//! Tauri commands exposing deployment-target CRUD to the UI (ticket 09;
//! 文件名→目录映射行 since ticket 19).

use serde::{Deserialize, Serialize};
use tauri::State;

use crate::commands::ApiError;
use crate::store::Store;
use crate::targets::repo::{self, NewTarget, NewTargetFile, RepoError, Target};

/// Targets 错误 → IPC `{code, message}`：前端按 code 选 i18n 文案。
impl From<RepoError> for ApiError {
    fn from(e: RepoError) -> Self {
        ApiError::raw(e.code(), e.to_string())
    }
}

/// 一行 文件名→上传目录 映射 (ticket 19) as sent to the frontend.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TargetFileDto {
    pub id: i64,
    pub target_id: i64,
    /// 空字符串 = 旧数据迁移出的"部署时补选"行（对话框内补选）。
    pub file_name: String,
    pub upload_dir: String,
    pub position: i64,
}

/// 新建/保存目标时随表单提交的映射行（无 id，整组替换）。
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TargetFileInput {
    pub file_name: String,
    pub upload_dir: String,
}

/// Deployment target as sent to the frontend (`camelCase`).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TargetDto {
    pub id: i64,
    pub project_id: i64,
    pub server_id: i64,
    /// 默认上传目录（票 26）：自由勾选的构件上传到这里。
    pub upload_dir: String,
    pub start_commands: String,
    pub stop_commands: String,
    /// 构建成功后自动连锁部署 (ticket 10, 默认关).
    pub auto_deploy: bool,
    /// 上次该目标部署选择的构件 (ticket 17)；ticket 19 起废弃，UI 不再使用。
    pub last_artifact: Option<String>,
    /// 文件名→上传目录映射行（票 19；票 26 起降级为部署对话框的预设，可选）。
    pub files: Vec<TargetFileDto>,
}

impl TargetDto {
    fn build(target: &Target, files: Vec<TargetFileDto>) -> Self {
        TargetDto {
            id: target.id,
            project_id: target.project_id,
            server_id: target.server_id,
            upload_dir: target.upload_dir.clone(),
            start_commands: target.start_commands.clone(),
            stop_commands: target.stop_commands.clone(),
            auto_deploy: target.auto_deploy,
            last_artifact: target.last_artifact.clone(),
            files,
        }
    }
}

fn file_dtos(store: &Store, target_id: i64) -> Vec<TargetFileDto> {
    repo::list_files(store, target_id)
        .map(|rows| {
            rows.iter()
                .map(|f| TargetFileDto {
                    id: f.id,
                    target_id: f.target_id,
                    file_name: f.file_name.clone(),
                    upload_dir: f.upload_dir.clone(),
                    position: f.position,
                })
                .collect()
        })
        .unwrap_or_default()
}

#[tauri::command]
pub fn list_deployment_targets(
    store: State<'_, Store>,
    project_id: i64,
) -> Result<Vec<TargetDto>, ApiError> {
    let targets = repo::list_for_project(&store, project_id)?;
    Ok(targets
        .iter()
        .map(|t| TargetDto::build(t, file_dtos(&store, t.id)))
        .collect())
}

#[tauri::command]
pub fn create_deployment_target(
    store: State<'_, Store>,
    project_id: i64,
    server_id: i64,
    // upload_dir：默认上传目录（票 26，必填）。
    upload_dir: String,
    // files：映射行（票 19；票 26 起可选，可全删——预设驱动部署对话框）。
    files: Option<Vec<TargetFileInput>>,
    start_commands: String,
    stop_commands: String,
) -> Result<TargetDto, ApiError> {
    let new = NewTarget {
        project_id,
        server_id,
        upload_dir,
        start_commands,
        stop_commands,
    };
    let created = repo::create(&store, &new)?;
    replace_files(&store, created.id, files.unwrap_or_default())?;
    Ok(TargetDto::build(&created, file_dtos(&store, created.id)))
}

#[tauri::command]
pub fn update_deployment_target(
    store: State<'_, Store>,
    id: i64,
    project_id: i64,
    server_id: i64,
    // upload_dir：默认上传目录（票 26，必填）。
    upload_dir: String,
    // files：映射行（票 19；票 26 起可选，可全删——预设驱动部署对话框）。
    files: Option<Vec<TargetFileInput>>,
    start_commands: String,
    stop_commands: String,
    auto_deploy: bool,
) -> Result<TargetDto, ApiError> {
    // 编辑目标不得丢失“上次选择的构件”（repo::update 不写该列，这里回填）。
    let existing = repo::get(&store, id)?;
    let target = Target {
        id,
        project_id,
        server_id,
        upload_dir,
        start_commands,
        stop_commands,
        auto_deploy,
        last_artifact: existing.and_then(|t| t.last_artifact),
    };
    let updated = repo::update(&store, &target)?;
    replace_files(&store, id, files.unwrap_or_default())?;
    Ok(TargetDto::build(&updated, file_dtos(&store, id)))
}

/// Saves the mapping-row set (atomic per `repo::replace_files`); validation
/// failures surface with the row's stable error code.
fn replace_files(
    store: &Store,
    target_id: i64,
    files: Vec<TargetFileInput>,
) -> Result<(), ApiError> {
    let rows: Vec<NewTargetFile> = files
        .into_iter()
        .map(|f| NewTargetFile {
            file_name: f.file_name,
            upload_dir: f.upload_dir,
        })
        .collect();
    repo::replace_files(store, target_id, &rows)?;
    Ok(())
}

#[tauri::command]
pub fn delete_deployment_target(store: State<'_, Store>, id: i64) -> Result<bool, ApiError> {
    repo::delete(&store, id).map_err(Into::into)
}

/// 切换“构建成功后自动连锁部署”开关（ticket 10）。返回目标是否存在。
#[tauri::command]
pub fn set_target_auto_deploy(
    store: State<'_, Store>,
    id: i64,
    enabled: bool,
) -> Result<bool, ApiError> {
    repo::set_auto_deploy(&store, id, enabled).map_err(Into::into)
}

/// 记住某目标上次部署选择的构件（ticket 17）。ticket 19 起废弃：映射行自带
/// 文件名，UI 不再调用；命令与列仅为兼容保留。
#[tauri::command]
pub fn set_target_last_artifact(
    store: State<'_, Store>,
    id: i64,
    file_name: String,
) -> Result<bool, ApiError> {
    repo::set_last_artifact(&store, id, &file_name).map_err(Into::into)
}
