//! Tauri commands for 打包（一）：分支选择与临时工作区 (ticket 06).
//!
//! Thin layer: branch listing / temp-workspace preparation live in
//! [`crate::build`]. Errors cross the IPC boundary as `{ code, message }`
//! (same shape as ticket 05 commands) so the frontend can key i18n on `code`.
//!
//! `start_build_workspace` is a long-running task (research notes: tauri2
//! long-running pattern): it runs on the blocking thread pool, streams clone
//! progress through a `tauri::ipc::Channel`, and can be cancelled via
//! [`cancel_build_workspace`] which sets the task's flag in the
//! [`BuildRegistry`].

use std::collections::HashMap;
use std::sync::atomic::Ordering;
use std::sync::Mutex;

use tauri::{AppHandle, Manager, State};

use crate::build::{self, PreparedWorkspace};
use crate::commands::ApiError;
use crate::store::projects::{self, new_cancel_flag, CancelFlag, ProjectError};
use crate::store::Store;

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

/// 取消进行中的打包准备（按项目 id）。临时工作区由任务侧清理。
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
