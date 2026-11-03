//! Tauri commands for 打包（一）：分支选择与临时工作区 (ticket 06).
//!
//! Thin layer: branch listing / temp-workspace preparation live in
//! [`crate::build`]. Errors cross the IPC boundary as `{ code, message }`
//! (same shape as ticket 05 commands) so the frontend can key i18n on `code`.
//!
//! `start_build_workspace`（票 06 前半段）与 [`start_build`]（票 07 全流程）
//! 都是长任务（research notes: tauri2 long-running pattern）：在阻塞线程池 /
//! tokio 运行时上执行，经 `tauri::ipc::Channel` 流式上报，可由
//! [`cancel_build_workspace`] 取消（置位注册表中的取消标志；构建阶段轮询到
//! 标志即杀死子进程，而非仅丢弃 future）。

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::sync::Mutex;

use tauri::{AppHandle, Manager, State};

use crate::build::{self, exec, BuildError, BuildEvent, BuildOutcome, PreparedWorkspace};
use crate::commands::ApiError;
use crate::store::projects::{self, new_cancel_flag, CancelFlag, Project, ProjectError};
use crate::store::Store;

impl From<BuildError> for ApiError {
    fn from(e: BuildError) -> Self {
        ApiError::raw(e.code(), e.to_string())
    }
}

/// 打包准备任务的取消标志注册表：按项目 id 存取（同一项目同时只能有一个
/// 进行中的打包准备）。
#[derive(Default)]
pub struct BuildRegistry(Mutex<HashMap<i64, CancelFlag>>);

/// 实时拉取项目的分支列表（打包面板打开时调用）。
///
/// git 项目拉取其 URL；本地导入的项目直接对本地路径做 ls-remote（libgit2
/// 本地传输），两者同为实时、进程内完成。
#[tauri::command]
pub async fn list_remote_branches(
    app: AppHandle,
    project_id: i64,
) -> Result<Vec<String>, ApiError> {
    let url = {
        let store = app.state::<Store>();
        let project =
            projects::get(&store, project_id)?.ok_or(ProjectError::ProjectNotFound(project_id))?;
        project
            .url
            .clone()
            .unwrap_or_else(|| project.path.clone())
    };

    // ls-remote 可能等网络；放到阻塞线程池，不占主线程。
    tauri::async_runtime::spawn_blocking(move || build::list_branches(&url))
        .await
        .map_err(|e| ApiError::raw("branch_listing_failed", e.to_string()))?
        .map_err(Into::into)
}

/// 发起打包（前半段）：创建临时工作区并 clone 所选分支。
///
/// 成功后记住该项目本次选择的分支（下次打开面板默认选中）。构建执行属于
/// ticket 07，本命令结束时只是"代码在临时工作区就绪"。
#[tauri::command]
pub async fn start_build_workspace(
    app: AppHandle,
    project_id: i64,
    branch: String,
    on_progress: tauri::ipc::Channel<projects::CloneProgress>,
) -> Result<PreparedWorkspace, ApiError> {
    let cancel = new_cancel_flag();
    if let Some(registry) = app.try_state::<BuildRegistry>() {
        registry
            .0
            .lock()
            .expect("build registry poisoned")
            .insert(project_id, cancel.clone());
    }

    let handle = app.clone();
    let result = tauri::async_runtime::spawn_blocking(move || {
        let store = handle.state::<Store>();
        let mut report = |p: projects::CloneProgress| {
            // Channel 关闭（前端已离开）不构成失败理由。
            let _ = on_progress.send(p);
        };
        build::prepare_workspace(&store, project_id, &branch, &cancel, &mut report)
    })
    .await
    .map_err(|e| ApiError::raw("clone_failed", e.to_string()))?;

    if let Some(registry) = app.try_state::<BuildRegistry>() {
        registry
            .0
            .lock()
            .expect("build registry poisoned")
            .remove(&project_id);
    }

    result.map_err(Into::into)
}

/// 取消进行中的打包任务（按项目 id）：置位取消标志。克隆阶段随之中断；
/// 构建阶段轮询到标志即杀死子进程。临时工作区由任务侧清理。
#[tauri::command]
pub fn cancel_build_workspace(registry: State<'_, BuildRegistry>, project_id: i64) {
    if let Some(flag) = registry
        .0
        .lock()
        .expect("build registry poisoned")
        .get(&project_id)
    {
        flag.store(true, Ordering::Relaxed);
    }
}

/// 发起打包（ticket 07 全流程）：准备临时工作区 → 用托管工具链执行项目的
/// 构建命令 → 无论成败删除临时工作区。
///
/// 进度与构建日志经同一 Channel 推送（[`BuildEvent`]）；返回值是构建结果
/// （失败构建 = `success:false`，不是 IPC 错误）。取消复用
/// [`cancel_build_workspace`]。同一项目同时只允许一个打包任务。
#[tauri::command]
pub async fn start_build(
    app: AppHandle,
    project_id: i64,
    branch: String,
    on_event: tauri::ipc::Channel<BuildEvent>,
) -> Result<BuildOutcome, ApiError> {
    let cancel = new_cancel_flag();
    if let Some(registry) = app.try_state::<BuildRegistry>() {
        let mut slots = registry.0.lock().expect("build registry poisoned");
        if slots.contains_key(&project_id) {
            return Err(ApiError::from(BuildError::AlreadyRunning));
        }
        slots.insert(project_id, cancel.clone());
    }

    let tools_root = crate::toolchain::tools_root(&app)
        .map_err(|e| ApiError::raw("toolchain_missing", e.to_string()))?;
    let appdata = app
        .path()
        .app_data_dir()
        .map_err(|e| ApiError::raw("io", e.to_string()))?;

    let store = (*app.state::<Store>()).clone();
    let sink = move |event: BuildEvent| {
        // Channel 关闭（前端已离开）不构成失败理由。
        let _ = on_event.send(event);
    };

    let result =
        build::run_build_flow(store, &tools_root, &appdata, project_id, &branch, cancel, Arc::new(sink))
            .await;

    if let Some(registry) = app.try_state::<BuildRegistry>() {
        registry
            .0
            .lock()
            .expect("build registry poisoned")
            .remove(&project_id);
    }

    result.map_err(Into::into)
}

/// 保存项目的构建命令（ticket 07：随项目可编辑并持久化）。保存前校验命令
/// 形态（v1 仅 mvnd/mvn 开头；执行时替换为托管 mvnd）。
#[tauri::command]
pub fn update_build_command(
    app: AppHandle,
    project_id: i64,
    command: String,
) -> Result<Project, ApiError> {
    exec::validate_build_command(&command).map_err(ApiError::from)?;
    let store = app.state::<Store>();
    projects::get(&store, project_id)?.ok_or(ProjectError::ProjectNotFound(project_id))?;
    projects::set_build_command(&store, project_id, &command)?;
    Ok(projects::get(&store, project_id)?.expect("project was just updated"))
}

/// 读取一次构建的持久化日志（失败后仍可查看；票 08 在此之上做历史 UI）。
#[tauri::command]
pub fn read_build_log(app: AppHandle, build_id: String) -> Result<String, ApiError> {
    let appdata = app
        .path()
        .app_data_dir()
        .map_err(|e| ApiError::raw("io", e.to_string()))?;
    build::read_build_log(&appdata, &build_id).map_err(Into::into)
}
