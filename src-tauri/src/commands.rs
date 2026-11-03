//! Tauri commands for 项目管理 (ticket 05).
//!
//! Thin layer: validation / clone / persistence live in `store::projects`.
//! Errors cross the IPC boundary as `{ code, message }` so the frontend can
//! key i18n messages on `code`.
//!
//! `clone_project` is a long-running task (research notes: tauri2 long-running
//! pattern): it runs on the blocking thread pool, streams progress through a
//! `tauri::ipc::Channel`, and can be cancelled via `cancel_clone` which sets
//! the task's flag in the [`CloneRegistry`].

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::Ordering;
use std::sync::Mutex;

use serde::Serialize;
use tauri::{AppHandle, Manager, State};

use crate::store::projects::{
    self, new_cancel_flag, CancelFlag, Project, ProjectError, ProjectType,
};
use crate::store::Store;

/// Clone 任务的取消标志注册表：按项目名称存取（同名同时只能有一个 clone，
/// 由名称唯一性校验保证）。
#[derive(Default)]
pub struct CloneRegistry(Mutex<HashMap<String, CancelFlag>>);

/// IPC 错误：稳定 `code` 供前端 i18n，`message` 为补充细节。
#[derive(Debug, Serialize)]
pub struct ApiError {
    code: String,
    message: String,
}

impl From<ProjectError> for ApiError {
    fn from(e: ProjectError) -> Self {
        ApiError {
            code: e.code().to_string(),
            message: e.to_string(),
        }
    }
}

impl From<crate::store::StoreError> for ApiError {
    fn from(e: crate::store::StoreError) -> Self {
        ApiError::raw("store", e.to_string())
    }
}

impl ApiError {
    pub(crate) fn raw(code: &str, message: impl Into<String>) -> Self {
        ApiError {
            code: code.to_string(),
            message: message.into(),
        }
    }
}

#[tauri::command]
pub fn list_projects(store: State<'_, Store>) -> Result<Vec<Project>, ApiError> {
    projects::list(&store).map_err(Into::into)
}

#[tauri::command]
pub fn import_project(
    store: State<'_, Store>,
    name: String,
    path: PathBuf,
    project_type: ProjectType,
) -> Result<Project, ApiError> {
    projects::import(&store, &name, &path, project_type).map_err(Into::into)
}

#[tauri::command]
pub async fn clone_project(
    app: AppHandle,
    name: String,
    url: String,
    workspace_root: String,
    project_type: ProjectType,
    on_progress: tauri::ipc::Channel<projects::CloneProgress>,
) -> Result<Project, ApiError> {
    // 先做同步校验：名称/URL 冲突立刻反馈，不占用 clone 线程。
    {
        let store = app.state::<Store>();
        projects::validate_new_git(&store, &name, &url).map_err(ApiError::from)?;
    }

    let cancel = new_cancel_flag();
    {
        let registry = app.state::<CloneRegistry>();
        registry
            .0
            .lock()
            .expect("clone registry poisoned")
            .insert(name.clone(), cancel.clone());
    }

    let handle = app.clone();
    let (name, result) = tauri::async_runtime::spawn_blocking(move || {
        let store = handle.state::<Store>();
        let mut report = |p: projects::CloneProgress| {
            // Channel 关闭（前端已离开）不构成失败理由。
            let _ = on_progress.send(p);
        };
        let result = projects::clone_and_register(
            &store,
            &name,
            &url,
            &workspace_root,
            project_type,
            &cancel,
            &mut report,
        );
        (name, result)
    })
    .await
    .map_err(|e| ApiError::raw("clone_failed", e.to_string()))?;

    if let Some(registry) = app.try_state::<CloneRegistry>() {
        registry
            .0
            .lock()
            .expect("clone registry poisoned")
            .remove(&name);
    }

    result.map_err(Into::into)
}

/// 取消进行中的 clone（按项目名称）。
#[tauri::command]
pub fn cancel_clone(registry: State<'_, CloneRegistry>, name: String) {
    if let Some(flag) = registry.0.lock().expect("clone registry poisoned").get(&name) {
        flag.store(true, Ordering::Relaxed);
    }
}
