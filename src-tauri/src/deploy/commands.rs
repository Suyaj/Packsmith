//! Tauri commands for 部署执行 (ticket 10): start/cancel a deployment,
//! run one target's start group alone (单独执行启动命令组), and the
//! 构建成功后自动连锁部署 (auto-chain) hook.
//!
//! Same long-running pattern as build/clone (research notes): the command
//! awaits the engine directly on the async runtime, streams per-target
//! phase/progress/output events through a `tauri::ipc::Channel`, and
//! cancellation goes through a registry-held [`CancelFlag`]. Auto-chain
//! deployments run as detached background tasks and announce themselves via
//! the `deploy-event` app event (envelope: `{ deployId, event }`).

use std::collections::HashMap;
use std::sync::Mutex;
use std::sync::atomic::{AtomicI64, Ordering};

use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager, State};

use crate::commands::ApiError;
use crate::deploy::engine::{self, ConnInfo, DeployEvent, DeployPlan, TargetJob};
use crate::deploy::transport::SshTransport;
use crate::server::repo as server_repo;
use crate::store::Store;
use crate::store::history;
use crate::store::projects::{CancelFlag, new_cancel_flag};
use crate::targets::repo as targets_repo;

/// Deploy task registry: ids → cancel flags. Snapshots themselves travel to
/// the UI as events; the registry only tracks cancellation.
#[derive(Default)]
pub struct DeployRegistry {
    next_id: AtomicI64,
    tasks: Mutex<HashMap<i64, CancelFlag>>,
}

impl DeployRegistry {
    fn register(&self) -> (i64, CancelFlag) {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed) + 1;
        let flag = new_cancel_flag();
        self.tasks
            .lock()
            .expect("deploy registry poisoned")
            .insert(id, flag.clone());
        (id, flag)
    }

    fn unregister(&self, id: i64) {
        self.tasks
            .lock()
            .expect("deploy registry poisoned")
            .remove(&id);
    }

    fn cancel(&self, id: i64) -> bool {
        match self
            .tasks
            .lock()
            .expect("deploy registry poisoned")
            .get(&id)
        {
            Some(flag) => {
                flag.store(true, Ordering::Relaxed);
                true
            }
            None => false,
        }
    }
}

/// Auto-chain event envelope for the `deploy-event` app event: the deploy id
/// plus the engine event itself.
#[derive(Debug, Clone, Serialize)]
pub struct AutoDeployEvent {
    pub deploy_id: i64,
    #[serde(flatten)]
    pub event: DeployEvent,
}

fn resolve_jobs(
    store: &Store,
    project_id: i64,
    target_ids: &[i64],
) -> Result<Vec<TargetJob>, ApiError> {
    let servers = server_repo::list(store).map_err(|e| ApiError::raw("store", e.to_string()))?;
    target_ids
        .iter()
        .map(|id| {
            let target = targets_repo::list_for_project(store, project_id)
                .map_err(|e| ApiError::raw("store", e.to_string()))?
                .into_iter()
                .find(|t| t.id == *id)
                .ok_or_else(|| {
                    ApiError::raw(
                        "invalid_target",
                        format!("target {id} does not belong to project {project_id}"),
                    )
                })?;
            let server = servers
                .iter()
                .find(|s| s.id == target.server_id)
                .ok_or_else(|| {
                    ApiError::raw(
                        "missing_server",
                        format!("server {} not found", target.server_id),
                    )
                })?;
            let fingerprint = server_repo::known_host_fingerprint(store, &server.host, server.port)
                .map_err(|e| ApiError::raw("store", e.to_string()))?;
            Ok(TargetJob {
                target_id: target.id,
                upload_dir: target.upload_dir,
                stop_commands: engine::split_commands(&target.stop_commands),
                start_commands: engine::split_commands(&target.start_commands),
                conn: ConnInfo {
                    server_id: server.id,
                    host: server.host.clone(),
                    port: server.port,
                    username: server.username.clone(),
                    credential: server.credential.clone(),
                    fingerprint,
                },
            })
        })
        .collect()
}

fn artifact_path(artifacts_dir: &str, file_name: &str) -> Result<std::path::PathBuf, ApiError> {
    let local_path = std::path::Path::new(artifacts_dir).join(file_name);
    if !local_path.is_file() {
        return Err(ApiError::raw(
            "artifact_not_found",
            format!("{file_name} not found in the artifact library"),
        ));
    }
    Ok(local_path)
}

#[tauri::command]
pub async fn start_deploy(
    app: AppHandle,
    history_id: i64,
    file_name: String,
    target_ids: Vec<i64>,
    on_event: tauri::ipc::Channel<DeployEvent>,
) -> Result<engine::DeploySummary, ApiError> {
    if target_ids.is_empty() {
        return Err(ApiError::raw(
            "no_targets_selected",
            "no deployment targets selected",
        ));
    }
    let store = (*app.state::<Store>()).clone();

    let (artifact, _project_id, jobs) = {
        let entry = history::list(&store, None)?
            .into_iter()
            .find(|e| e.id == history_id)
            .ok_or(history::HistoryError::NotFound(history_id))?;
        if !entry.success {
            return Err(ApiError::raw(
                "build_failed",
                "cannot deploy a failed build",
            ));
        }
        let artifacts_dir = entry
            .artifacts_dir
            .clone()
            .ok_or_else(|| ApiError::raw("artifacts_dir_missing", entry.build_id.clone()))?;
        let artifact = engine::ArtifactRef {
            local_path: artifact_path(&artifacts_dir, &file_name)?,
            file_name,
        };
        (
            Some(artifact),
            entry.project_id,
            resolve_jobs(&store, entry.project_id, &target_ids)?,
        )
    };

    let registry = app.state::<DeployRegistry>();
    let (deploy_id, cancel) = registry.register();
    let _ = deploy_id; // cancellation goes by id; per-target state travels as events

    let factory = |conn: ConnInfo| async move { SshTransport::connect(&conn).await };
    let mut sink = |event: DeployEvent| {
        // Channel 关闭（前端已离开）不构成失败理由。
        let _ = on_event.send(event);
    };
    let summary = engine::run_deploy(
        DeployPlan {
            artifact,
            run_stop: true,
            targets: jobs,
        },
        factory,
        cancel,
        &mut sink,
    )
    .await;

    registry.unregister(deploy_id);
    Ok(summary)
}

#[tauri::command]
pub fn cancel_deploy(registry: State<'_, DeployRegistry>, deploy_id: i64) -> bool {
    registry.cancel(deploy_id)
}

/// 单独执行启动命令组：only the start group of one target (no upload, no
/// stop). Events stream over the Channel; the summary comes back with the
/// command.
#[tauri::command]
pub async fn run_start_group(
    app: AppHandle,
    target_id: i64,
    on_event: tauri::ipc::Channel<DeployEvent>,
) -> Result<engine::DeploySummary, ApiError> {
    let store = (*app.state::<Store>()).clone();
    let target = targets_repo::get(&store, target_id)
        .map_err(|e| ApiError::raw("store", e.to_string()))?
        .ok_or_else(|| ApiError::raw("invalid_target", format!("target {target_id} not found")))?;
    let jobs = resolve_jobs(&store, target.project_id, &[target_id])?;

    let registry = app.state::<DeployRegistry>();
    let (deploy_id, cancel) = registry.register();

    let factory = |conn: ConnInfo| async move { SshTransport::connect(&conn).await };
    let mut sink = |event: DeployEvent| {
        let _ = on_event.send(event);
    };
    let summary = engine::run_deploy(
        DeployPlan {
            artifact: None,
            run_stop: false,
            targets: jobs,
        },
        factory,
        cancel,
        &mut sink,
    )
    .await;

    registry.unregister(deploy_id);
    Ok(summary)
}

/// 构建成功后自动连锁部署 hook: for every target of the project with the
/// toggle ON, spawn a detached deploy of the just-built artifact. Events go
/// out as `deploy-event` app events (`AutoDeployEvent` envelope). No-op when
/// no artifact can be chosen (multiple artifacts and no default selection).
pub fn auto_chain_after_build(
    app: &AppHandle,
    project_id: i64,
    history_id: i64,
    artifact_candidates: &[String],
    default_artifact: Option<&str>,
) -> Result<(), ApiError> {
    let store = (*app.state::<Store>()).clone();

    let enabled = targets_repo::auto_deploy_enabled(&store, project_id)
        .map_err(|e| ApiError::raw("store", e.to_string()))?;
    if enabled.is_empty() {
        return Ok(());
    }

    let file_name = match artifact_candidates {
        [single] => single.clone(),
        many => match default_artifact {
            Some(default) if many.iter().any(|c| c == default) => default.to_string(),
            _ => return Ok(()), // 无法确定要部署哪个构件：自动连锁跳过
        },
    };

    let entry = history::list(&store, None)?
        .into_iter()
        .find(|e| e.id == history_id && e.success)
        .ok_or_else(|| ApiError::raw("history_not_found", format!("history {history_id}")))?;
    let artifacts_dir = entry
        .artifacts_dir
        .clone()
        .ok_or_else(|| ApiError::raw("artifacts_dir_missing", entry.build_id.clone()))?;
    let artifact = engine::ArtifactRef {
        local_path: artifact_path(&artifacts_dir, &file_name)?,
        file_name,
    };

    let jobs = resolve_jobs(&store, project_id, &enabled)?;
    let plan = DeployPlan {
        artifact: Some(artifact),
        run_stop: true,
        targets: jobs,
    };

    let registry = app.state::<DeployRegistry>();
    let (deploy_id, cancel) = registry.register();

    let emitter = app.clone();
    let emitter_for_state = app.clone();
    tauri::async_runtime::spawn(async move {
        let factory = |conn: ConnInfo| async move { SshTransport::connect(&conn).await };
        let sink = move |event: DeployEvent| {
            let _ = emitter.emit("deploy-event", AutoDeployEvent { deploy_id, event });
        };
        let _summary = engine::run_deploy(plan, factory, cancel, sink).await;
        if let Some(reg) = emitter_for_state.try_state::<DeployRegistry>() {
            reg.unregister(deploy_id);
        }
    });
    Ok(())
}
