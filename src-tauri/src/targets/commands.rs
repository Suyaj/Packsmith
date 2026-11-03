//! Tauri commands exposing deployment-target CRUD to the UI (ticket 09).

use serde::{Deserialize, Serialize};
use tauri::State;

use crate::store::Store;
use crate::targets::repo::{self, NewTarget, Target};

/// Deployment target as sent to the frontend (`camelCase`).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TargetDto {
    pub id: i64,
    pub project_id: i64,
    pub server_id: i64,
    pub upload_dir: String,
    pub start_commands: String,
    pub stop_commands: String,
    /// 构建成功后自动连锁部署 (ticket 10, 默认关).
    pub auto_deploy: bool,
    /// 上次该目标部署选择的构件 (ticket 17, 部署对话框默认选中项).
    pub last_artifact: Option<String>,
}

impl From<&Target> for TargetDto {
    fn from(t: &Target) -> Self {
        TargetDto {
            id: t.id,
            project_id: t.project_id,
            server_id: t.server_id,
            upload_dir: t.upload_dir.clone(),
            start_commands: t.start_commands.clone(),
            stop_commands: t.stop_commands.clone(),
            auto_deploy: t.auto_deploy,
            last_artifact: t.last_artifact.clone(),
        }
    }
}

#[tauri::command]
pub fn list_deployment_targets(
    store: State<'_, Store>,
    project_id: i64,
) -> Result<Vec<TargetDto>, String> {
    repo::list_for_project(&store, project_id)
        .map_err(|e| e.to_string())
        .map(|targets| targets.iter().map(TargetDto::from).collect())
}

#[tauri::command]
pub fn create_deployment_target(
    store: State<'_, Store>,
    project_id: i64,
    server_id: i64,
    upload_dir: String,
    start_commands: String,
    stop_commands: String,
) -> Result<TargetDto, String> {
    let new = NewTarget {
        project_id,
        server_id,
        upload_dir,
        start_commands,
        stop_commands,
    };
    repo::create(&store, &new)
        .map_err(|e| e.to_string())
        .map(|t| (&t).into())
}

#[tauri::command]
pub fn update_deployment_target(
    store: State<'_, Store>,
    id: i64,
    project_id: i64,
    server_id: i64,
    upload_dir: String,
    start_commands: String,
    stop_commands: String,
    auto_deploy: bool,
) -> Result<TargetDto, String> {
    // 编辑目标不得丢失“上次选择的构件”（repo::update 不写该列，这里回填）。
    let last_artifact = repo::get(&store, id)
        .map_err(|e| e.to_string())?
        .and_then(|t| t.last_artifact);
    let target = Target {
        id,
        project_id,
        server_id,
        upload_dir,
        start_commands,
        stop_commands,
        auto_deploy,
        last_artifact,
    };
    repo::update(&store, &target)
        .map_err(|e| e.to_string())
        .map(|t| (&t).into())
}

#[tauri::command]
pub fn delete_deployment_target(store: State<'_, Store>, id: i64) -> Result<bool, String> {
    repo::delete(&store, id).map_err(|e| e.to_string())
}

/// 切换“构建成功后自动连锁部署”开关（ticket 10）。返回目标是否存在。
#[tauri::command]
pub fn set_target_auto_deploy(
    store: State<'_, Store>,
    id: i64,
    enabled: bool,
) -> Result<bool, String> {
    repo::set_auto_deploy(&store, id, enabled).map_err(|e| e.to_string())
}

/// 记住某目标上次部署选择的构件（ticket 17 一一对应上传）。返回目标是否存在。
#[tauri::command]
pub fn set_target_last_artifact(
    store: State<'_, Store>,
    id: i64,
    file_name: String,
) -> Result<bool, String> {
    repo::set_last_artifact(&store, id, &file_name).map_err(|e| e.to_string())
}
