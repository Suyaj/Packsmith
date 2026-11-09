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
//!
//! ticket 19: the upload set was the target's 文件名→目录 mapping rows.
//! ticket 26: the dialog upgrades to 自由勾选 — every artifact of this build
//! is checkable; mapping rows only preset the checked entries (row dir), free
//! entries upload to the target's default dir. The payload is per-target
//! `entries: [{fileName, uploadDir}]` — validated against this build's
//! artifact listing; no row-id checks. Auto-chain keeps its row-based
//! resolution (预设) unchanged.

use std::collections::HashMap;
use std::sync::Mutex;
use std::sync::atomic::{AtomicI64, Ordering};

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, Manager, State};

use crate::commands::ApiError;
use crate::deploy::engine::{self, ConnInfo, DeployEvent, DeployPlan, TargetJob};
use crate::deploy::transport::SshTransport;
use crate::server::repo as server_repo;
use crate::store::Store;
use crate::store::deploy_history;
use crate::store::history;
use crate::store::projects;
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
/// plus the engine event itself. Also the wire shape of the
/// `tauri::ipc::Channel` messages for start_deploy / run_start_group — the
/// frontend's `AutoDeployMessage` (`{ deployId, event }`, camelCase). The
/// deploy id must travel with every message so the UI can cancel mid-deploy.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AutoDeployEvent {
    pub deploy_id: i64,
    /// 嵌套在 `event` 键下（不 flatten）：前端按 `message.event.type` 分派。
    pub event: DeployEvent,
}

impl AutoDeployEvent {
    /// Single wrapping point for every event leaving the backend (Channel +
    /// app-event emitter) so the wire shape can't drift from the frontend.
    pub fn envelope(deploy_id: i64, event: DeployEvent) -> Self {
        AutoDeployEvent { deploy_id, event }
    }
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
                stop_commands: engine::split_command_group(&target.stop_commands),
                start_commands: engine::split_command_group(&target.start_commands),
                conn: ConnInfo {
                    server_id: server.id,
                    host: server.host.clone(),
                    port: server.port,
                    username: server.username.clone(),
                    credential: server.credential.clone(),
                    fingerprint,
                },
                artifacts: Vec::new(),
            })
        })
        .collect()
}

fn artifact_path(artifacts_dir: &str, file_name: &str) -> Result<std::path::PathBuf, ApiError> {
    // The resolved file must name a file from this build's artifact listing
    // (ticket 17/19 校验): no path separators / traversal — anything else is
    // rejected up front with the i18n-able `artifact_not_found` code.
    if file_name.is_empty()
        || file_name.contains('/')
        || file_name.contains('\\')
        || file_name.contains("..")
    {
        return Err(ApiError::raw(
            "artifact_not_found",
            format!("artifact {file_name:?} is not in this build's artifact listing"),
        ));
    }
    let local_path = std::path::Path::new(artifacts_dir).join(file_name);
    if !local_path.is_file() {
        // 前端按 code 选中文案（deploy.errors.artifact_not_found）。
        return Err(ApiError::raw(
            "artifact_not_found",
            format!("artifact {file_name:?} is not in this build's artifact listing"),
        ));
    }
    Ok(local_path)
}

/// One target's 自由勾选条目 (ticket 26): the dialog submits the checked
/// artifacts with their upload dirs — preset entries (映射行预设) carry the
/// row's own dir, free entries the target's default dir. Mapping rows only
/// preset the dialog on the frontend; the backend never sees row ids.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeployEntry {
    pub file_name: String,
    pub upload_dir: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TargetAssignment {
    pub target_id: i64,
    /// 仅停止/仅启动（票 23）时缺省（= 空）：不传文件。
    #[serde(default)]
    pub entries: Vec<DeployEntry>,
}

/// 解析一个目标的映射行（ticket 19 / 21）:
/// - 具名行（`file_name` 非空）按本次构建的产物列表解析：`strict` 时缺失即
///   报错（手动部署的兜底校验）；否则缺失行跳过并记入 skipped（自动连锁）。
/// - 空名行使用 `picks`（部署对话框内联补选的文件名）；未补选时按 `defaults`
///   （项目默认构件列表，票 21）**依序**取第一个在本构建产物里可用的项，
///   列表用尽回退 `first_available`（本次构建的首个可用产物）；列表为空 =
///   未设置，跳过（与票 19 行为一致）。
///
/// Returns the resolved uploads (each with its own dir) plus the skipped-row
/// notices (空名行跳过有提示).
fn resolve_target_rows(
    rows: &[targets_repo::TargetFile],
    artifacts_dir: &str,
    picks: &HashMap<i64, String>,
    strict: bool,
    defaults: &[String],
    first_available: Option<&str>,
) -> Result<(Vec<engine::ArtifactRef>, Vec<String>), ApiError> {
    let mut artifacts = Vec::new();
    let mut skipped = Vec::new();
    for row in rows {
        let chosen = if row.file_name.trim().is_empty() {
            match picks.get(&row.id).map(|p| p.trim().to_string()) {
                Some(p) if !p.is_empty() => p,
                _ => match default_for_empty_row(artifacts_dir, defaults, first_available) {
                    Some(name) => name,
                    None => {
                        skipped.push(format!("row {} (部署时补选): 未选择构件，已跳过", row.id));
                        continue;
                    }
                },
            }
        } else {
            row.file_name.trim().to_string()
        };
        match artifact_path(artifacts_dir, &chosen) {
            Ok(local_path) => artifacts.push(engine::ArtifactRef {
                local_path,
                file_name: chosen,
                upload_dir: row.upload_dir.clone(),
            }),
            Err(e) => {
                if strict {
                    return Err(e);
                }
                skipped.push(format!("row {} ({chosen}): 产物缺失，已跳过", row.id));
            }
        }
    }
    Ok((artifacts, skipped))
}

/// 空名行的默认构件补选（票 21）：按默认列表顺序取第一个在本次构建产物里
/// 可用的项；列表用尽（全部缺失）回退首个可用产物；列表为空 = 未设置，
/// 返回 `None`（由调用方跳过该行）。
fn default_for_empty_row(
    artifacts_dir: &str,
    defaults: &[String],
    first_available: Option<&str>,
) -> Option<String> {
    if defaults.is_empty() {
        return None;
    }
    for name in defaults {
        let name = name.trim();
        if !name.is_empty() && artifact_path(artifacts_dir, name).is_ok() {
            return Some(name.to_string());
        }
    }
    let fallback = first_available?.trim().to_string();
    (!fallback.is_empty() && artifact_path(artifacts_dir, &fallback).is_ok()).then_some(fallback)
}

/// 解析一个目标提交的自由勾选条目（票 26）: 每个条目的文件名必须在本构建
/// 产物里（`artifact_not_found`，含路径穿越/空名），目录必须非空
/// （`empty_upload_dir`）。不校验 rowId——映射行只在前端驱动预设。
/// Returns the resolved uploads, each with the dir the dialog submitted.
fn resolve_entries(
    artifacts_dir: &str,
    entries: &[DeployEntry],
) -> Result<Vec<engine::ArtifactRef>, ApiError> {
    entries
        .iter()
        .map(|entry| {
            let upload_dir = entry.upload_dir.trim().to_string();
            if upload_dir.is_empty() {
                return Err(ApiError::raw(
                    "empty_upload_dir",
                    format!("entry {:?}: upload dir must not be empty", entry.file_name),
                ));
            }
            let file_name = entry.file_name.trim().to_string();
            let local_path = artifact_path(artifacts_dir, &file_name)?;
            Ok(engine::ArtifactRef {
                local_path,
                file_name,
                upload_dir,
            })
        })
        .collect()
}

#[tauri::command]
pub async fn start_deploy(
    app: AppHandle,
    history_id: i64,
    operation: engine::DeployOperation,
    assignments: Vec<TargetAssignment>,
    on_event: tauri::ipc::Channel<AutoDeployEvent>,
) -> Result<engine::DeploySummary, ApiError> {
    if assignments.is_empty() {
        return Err(ApiError::raw(
            "no_targets_selected",
            "no deployment targets selected",
        ));
    }
    let store = (*app.state::<Store>()).clone();
    // 上传相关的校验（构建成功、产物目录、条目解析）只对会传文件的操作生效
    // （ticket 23）：仅停止/仅启动只需要目标选择仍然有效。
    let uploads = operation.runs_upload();

    let jobs = {
        let entry = history::list(&store, None)?
            .into_iter()
            .find(|e| e.id == history_id)
            .ok_or(history::HistoryError::NotFound(history_id))?;
        let artifacts_dir =
            if uploads {
                if !entry.success {
                    return Err(ApiError::raw(
                        "build_failed",
                        "cannot deploy a failed build",
                    ));
                }
                Some(entry.artifacts_dir.clone().ok_or_else(|| {
                    ApiError::raw("artifacts_dir_missing", entry.build_id.clone())
                })?)
            } else {
                None
            };
        let target_ids: Vec<i64> = assignments.iter().map(|a| a.target_id).collect();
        let mut seen = std::collections::HashSet::new();
        if !target_ids.iter().all(|id| seen.insert(*id)) {
            return Err(ApiError::raw(
                "duplicate_target",
                "each deployment target may appear only once in the assignments",
            ));
        }
        let mut jobs = resolve_jobs(&store, entry.project_id, &target_ids)?;
        if let Some(artifacts_dir) = artifacts_dir {
            // 自由勾选条目（票 26）：逐条校验产物归属与目录非空，按条目提交的
            // 目录上传（预设条目 = 行目录，自由条目 = 目标默认目录）。
            for assignment in &assignments {
                let artifacts = resolve_entries(&artifacts_dir, &assignment.entries)?;
                let job = jobs
                    .iter_mut()
                    .find(|j| j.target_id == assignment.target_id)
                    .expect("resolve_jobs preserves target order");
                job.artifacts = artifacts;
            }
            if jobs.iter().all(|j| j.artifacts.is_empty()) {
                return Err(ApiError::raw(
                    "no_files_selected",
                    "no upload entries in the assignments",
                ));
            }
        }
        jobs
    };

    let registry = app.state::<DeployRegistry>();
    let (deploy_id, cancel) = registry.register();
    let _ = deploy_id; // cancellation goes by id; per-target state travels as events

    let factory = |conn: ConnInfo| async move { SshTransport::connect(&conn).await };
    let mut sink = |event: DeployEvent| {
        // Channel 关闭（前端已离开）不构成失败理由。信封带上 deployId——
        // 前端靠它取消进行中的部署。
        let _ = on_event.send(AutoDeployEvent::envelope(deploy_id, event));
    };
    let summary = engine::run_deploy(
        DeployPlan {
            operation,
            targets: jobs,
        },
        factory,
        cancel,
        &mut sink,
    )
    .await;

    registry.unregister(deploy_id);
    // 部署记录落库（ticket 12/23）：每个目标一行，附本次操作类型，供首页
    // "最近部署时间"。记录失败不影响部署结果本身。
    let _ = deploy_history::record_outcomes(&store, &summary, operation);
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
    on_event: tauri::ipc::Channel<AutoDeployEvent>,
) -> Result<engine::DeploySummary, ApiError> {
    let store = (*app.state::<Store>()).clone();
    let target = targets_repo::get(&store, target_id)
        .map_err(ApiError::from)?
        .ok_or_else(|| ApiError::raw("invalid_target", format!("target {target_id} not found")))?;
    let jobs = resolve_jobs(&store, target.project_id, &[target_id])?;

    let registry = app.state::<DeployRegistry>();
    let (deploy_id, cancel) = registry.register();

    let factory = |conn: ConnInfo| async move { SshTransport::connect(&conn).await };
    let mut sink = |event: DeployEvent| {
        let _ = on_event.send(AutoDeployEvent::envelope(deploy_id, event));
    };
    let summary = engine::run_deploy(
        DeployPlan {
            operation: engine::DeployOperation::Start,
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
/// toggle ON, resolve its mapping rows (ticket 19) against the just-built
/// build's artifact directory. Named rows must resolve to upload; 空名行
/// (部署时补选) fall back to the project's ordered default artifact list
/// (票 21): the first list entry available in this build, or — once the list
/// is exhausted — the build's first available artifact (empty list = unset:
/// the row is skipped, each with a `Skipped` notice event). A target with no
/// resolvable rows is skipped entirely (never restart without a fresh
/// artifact). Events go out as `deploy-event` app events (`AutoDeployEvent`
/// envelope).
pub fn auto_chain_after_build(
    app: &AppHandle,
    project_id: i64,
    history_id: i64,
) -> Result<(), ApiError> {
    let store = (*app.state::<Store>()).clone();

    let enabled = targets_repo::auto_deploy_enabled(&store, project_id)
        .map_err(|e| ApiError::raw("store", e.to_string()))?;
    if enabled.is_empty() {
        return Ok(());
    }

    let entry = history::list(&store, None)?
        .into_iter()
        .find(|e| e.id == history_id && e.success)
        .ok_or_else(|| ApiError::raw("history_not_found", format!("history {history_id}")))?;
    let artifacts_dir = entry
        .artifacts_dir
        .clone()
        .ok_or_else(|| ApiError::raw("artifacts_dir_missing", entry.build_id.clone()))?;

    let rows = targets_repo::files_for_targets(&store, &enabled)
        .map_err(|e| ApiError::raw("store", e.to_string()))?;
    let mut jobs = Vec::new();
    let mut skipped_notices: Vec<(i64, String)> = Vec::new();
    for target_id in enabled {
        let rows: Vec<targets_repo::TargetFile> = rows
            .iter()
            .filter(|r| r.target_id == target_id)
            .cloned()
            .collect();
        // Lenient: missing rows skip with a notice instead of failing the
        // auto-chain (自动连锁只传可解析行). 空名行按项目默认构件列表顺序
        // 补选（票 21），列表用尽回退本次构建的首个可用产物；列表为空 =
        // 未设置，照旧跳过。
        let defaults = projects::get(&store, project_id)
            .map_err(ApiError::from)?
            .map(|p| p.default_artifacts)
            .unwrap_or_default();
        let first_available = history::list_artifacts(&entry)
            .ok()
            .and_then(|files| files.first().map(|f| f.file_name.clone()));
        let (artifacts, skipped) = resolve_target_rows(
            &rows,
            &artifacts_dir,
            &HashMap::new(),
            false,
            &defaults,
            first_available.as_deref(),
        )?;
        skipped_notices.extend(skipped.into_iter().map(|m| (target_id, m)));
        if artifacts.is_empty() {
            skipped_notices.push((target_id, "没有可解析的上传行，已跳过该目标".to_string()));
            continue;
        }
        let mut job = resolve_jobs(&store, project_id, &[target_id])?.remove(0);
        job.artifacts = artifacts;
        jobs.push(job);
    }
    if jobs.is_empty() {
        // No target has a resolvable row: nothing to deploy (no-op, ticket 19).
        return Ok(());
    }

    // 自动连锁仍是完整"上传 + 重启"（ticket 23 语义不回退）。
    let plan = DeployPlan {
        operation: engine::DeployOperation::Restart,
        targets: jobs,
    };

    let registry = app.state::<DeployRegistry>();
    let (deploy_id, cancel) = registry.register();

    let emitter = app.clone();
    let emitter_for_state = app.clone();
    tauri::async_runtime::spawn(async move {
        let factory = |conn: ConnInfo| async move { SshTransport::connect(&conn).await };
        let sink = move |event: DeployEvent| {
            let _ = emitter.emit("deploy-event", AutoDeployEvent::envelope(deploy_id, event));
        };
        for (target_id, message) in skipped_notices {
            sink(DeployEvent::Skipped { target_id, message });
        }
        let summary = engine::run_deploy(plan, factory, cancel, sink).await;
        // 自动连锁部署同样落部署记录（ticket 12/23，操作 = 重启）；记录失败
        // 不影响部署。
        let _ = deploy_history::record_outcomes(&store, &summary, engine::DeployOperation::Restart);
        if let Some(reg) = emitter_for_state.try_state::<DeployRegistry>() {
            reg.unregister(deploy_id);
        }
    });
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Channel / app-event 信封的线上形状（golden）：前端 `AutoDeployMessage`
    /// = `{ deployId, event }`，`deployId` 必须 camelCase，事件本体嵌套在
    /// `event` 键下。形状漂移 = 前端状态机永远停在"等待"。golden 字符串与
    /// `src/components/DeployDialog.test.tsx` 的线上契约测试互为镜像。
    #[test]
    fn event_envelope_serializes_to_the_frontend_contract() {
        let json = serde_json::to_string(&AutoDeployEvent::envelope(
            7,
            DeployEvent::Phase {
                target_id: 1,
                phase: "upload",
                status: "running",
            },
        ))
        .unwrap();
        assert_eq!(
            json,
            r#"{"deployId":7,"event":{"type":"phase","target_id":1,"phase":"upload","status":"running"}}"#
        );
    }

    /// ticket 17/19 校验: only files actually in this build's artifact listing
    /// resolve; anything else (missing file, path traversal) is rejected with
    /// the stable `artifact_not_found` code the frontend keys Chinese i18n on.
    #[test]
    fn artifact_path_rejects_files_outside_the_builds_artifact_listing() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("a.jar"), b"x").unwrap();

        // A file in the listing resolves to its path in the 产物库.
        let ok = artifact_path(dir.path().to_str().unwrap(), "a.jar").unwrap();
        assert_eq!(ok, dir.path().join("a.jar"));

        for bad in [
            "missing.jar",
            "",
            "../evil.jar",
            "sub/dir.jar",
            "..\\evil.jar",
        ] {
            let err = artifact_path(dir.path().to_str().unwrap(), bad).unwrap_err();
            assert_eq!(err.code(), "artifact_not_found", "case {bad:?}");
        }
    }

    fn row(id: i64, target_id: i64, file_name: &str, upload_dir: &str) -> targets_repo::TargetFile {
        targets_repo::TargetFile {
            id,
            target_id,
            file_name: file_name.to_string(),
            upload_dir: upload_dir.to_string(),
            position: 0,
        }
    }

    /// ticket 19 解析: a named row present in the artifact listing resolves to
    /// an upload into its own dir with the original file name.
    #[test]
    fn resolve_named_row_matching_the_listing() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("app.jar"), b"x").unwrap();
        let rows = vec![row(1, 10, "app.jar", "/srv/app")];

        let (artifacts, skipped) = resolve_target_rows(
            &rows,
            dir.path().to_str().unwrap(),
            &HashMap::new(),
            true,
            &[],
            None,
        )
        .unwrap();
        assert_eq!(artifacts.len(), 1);
        assert_eq!(artifacts[0].file_name, "app.jar");
        assert_eq!(artifacts[0].upload_dir, "/srv/app");
        assert!(skipped.is_empty());
    }

    /// ticket 19 解析: a named row whose product is missing — strict (manual
    /// deploy backstop) rejects with `artifact_not_found`; lenient (auto-chain)
    /// skips it with a 产物缺失 notice.
    #[test]
    fn resolve_named_row_missing_product_strict_vs_lenient() {
        let dir = tempfile::tempdir().unwrap();
        let rows = vec![row(2, 10, "gone.jar", "/srv/app")];

        let err = resolve_target_rows(
            &rows,
            dir.path().to_str().unwrap(),
            &HashMap::new(),
            true,
            &[],
            None,
        )
        .unwrap_err();
        assert_eq!(err.code(), "artifact_not_found");

        let (artifacts, skipped) = resolve_target_rows(
            &rows,
            dir.path().to_str().unwrap(),
            &HashMap::new(),
            false,
            &[],
            None,
        )
        .unwrap();
        assert!(artifacts.is_empty());
        assert_eq!(skipped.len(), 1);
        assert!(skipped[0].contains("产物缺失"));
    }

    /// ticket 19 解析: 空名行（旧数据迁移行）需要对话框补选；未补选则跳过。
    #[test]
    fn resolve_empty_row_requires_pick_and_skips_without_one() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("picked.jar"), b"x").unwrap();
        let rows = vec![row(3, 10, "", "/srv/app")];

        // 未补选：跳过（有提示），什么也不上传。
        let (artifacts, skipped) = resolve_target_rows(
            &rows,
            dir.path().to_str().unwrap(),
            &HashMap::new(),
            true,
            &[],
            None,
        )
        .unwrap();
        assert!(artifacts.is_empty());
        assert_eq!(skipped.len(), 1);

        // 补选后按补选的文件名解析，上传目录仍是该行自己的目录。
        let picks = HashMap::from([(3i64, "picked.jar".to_string())]);
        let (artifacts, skipped) =
            resolve_target_rows(&rows, dir.path().to_str().unwrap(), &picks, true, &[], None)
                .unwrap();
        assert_eq!(artifacts.len(), 1);
        assert_eq!(artifacts[0].file_name, "picked.jar");
        assert_eq!(artifacts[0].upload_dir, "/srv/app");
        assert!(skipped.is_empty());
    }

    /// 票 21 自动连锁：空名行按项目默认构件列表**顺序**取用——取列表中第一
    /// 个在本构建产物里可用的项；列表用尽（全部缺失）回退首个可用产物。
    #[test]
    fn auto_chain_empty_row_uses_default_list_in_order_with_fallback() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("a.jar"), b"x").unwrap();
        std::fs::write(dir.path().join("b.jar"), b"x").unwrap();
        let dir_str = dir.path().to_str().unwrap();
        let rows = vec![row(1, 10, "", "/srv/app")];

        // 列表头项可用：直接取用。
        let defaults = vec!["a.jar".to_string(), "b.jar".to_string()];
        let (artifacts, skipped) = resolve_target_rows(
            &rows,
            dir_str,
            &HashMap::new(),
            false,
            &defaults,
            Some("a.jar"),
        )
        .unwrap();
        assert_eq!(artifacts.len(), 1);
        assert_eq!(artifacts[0].file_name, "a.jar");
        assert!(skipped.is_empty());

        // 列表前项缺失：依序取到第一个可用的项。
        let defaults = vec!["gone.jar".to_string(), "b.jar".to_string()];
        let (artifacts, _) = resolve_target_rows(
            &rows,
            dir_str,
            &HashMap::new(),
            false,
            &defaults,
            Some("a.jar"),
        )
        .unwrap();
        assert_eq!(artifacts[0].file_name, "b.jar");

        // 列表用尽（全部缺失）：回退首个可用产物。
        let defaults = vec!["gone1.jar".to_string(), "gone2.jar".to_string()];
        let (artifacts, _) = resolve_target_rows(
            &rows,
            dir_str,
            &HashMap::new(),
            false,
            &defaults,
            Some("a.jar"),
        )
        .unwrap();
        assert_eq!(artifacts[0].file_name, "a.jar");

        // 构建无任何产物：列表再非空也无处可取，跳过。
        let (artifacts, skipped) =
            resolve_target_rows(&rows, dir_str, &HashMap::new(), false, &defaults, None).unwrap();
        assert!(artifacts.is_empty());
        assert_eq!(skipped.len(), 1);

        // 空列表 = 未设置：跳过（与票 19 行为一致），不凭空回退。
        let (artifacts, skipped) =
            resolve_target_rows(&rows, dir_str, &HashMap::new(), false, &[], Some("a.jar"))
                .unwrap();
        assert!(artifacts.is_empty());
        assert_eq!(skipped.len(), 1);

        // 对话框补选（picks）优先于默认列表。
        let picks = HashMap::from([(1i64, "b.jar".to_string())]);
        let defaults = vec!["a.jar".to_string()];
        let (artifacts, _) =
            resolve_target_rows(&rows, dir_str, &picks, false, &defaults, Some("a.jar")).unwrap();
        assert_eq!(artifacts[0].file_name, "b.jar");
    }

    /// 自动连锁（lenient）只保留可解析行：具名且产物存在；空名行与缺失行
    /// 都被跳过并给出提示，目标仍有可解析行时照常部署。
    #[test]
    fn auto_chain_resolution_keeps_only_resolvable_named_rows() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("a.jar"), b"x").unwrap();
        let rows = vec![
            row(1, 10, "a.jar", "/srv/a"),
            row(2, 10, "", "/srv/empty"),
            row(3, 10, "missing.jar", "/srv/missing"),
        ];

        // 项目未设置默认构件列表（空列表 = 未设置）：空名行跳过（票 19 行为）。
        let (artifacts, skipped) = resolve_target_rows(
            &rows,
            dir.path().to_str().unwrap(),
            &HashMap::new(),
            false,
            &[],
            None,
        )
        .unwrap();
        assert_eq!(
            artifacts
                .iter()
                .map(|a| a.file_name.as_str())
                .collect::<Vec<_>>(),
            vec!["a.jar"]
        );
        assert_eq!(skipped.len(), 2, "空名行与缺失行都要有跳过提示");
    }

    // ---- 票 26：部署 payload 改为每目标自由勾选条目 entries{fileName,uploadDir}
    // ----（映射行只在前端驱动预设，rowId 校验移除）。

    fn entry(file_name: &str, upload_dir: &str) -> DeployEntry {
        DeployEntry {
            file_name: file_name.to_string(),
            upload_dir: upload_dir.to_string(),
        }
    }

    /// 自由勾选条目按提交原样解析：文件名必须在本构建产物里，目录按条目
    /// （预设条目 = 行目录，自由条目 = 目标默认目录），原文件名覆盖上传。
    #[test]
    fn resolve_entries_maps_checked_entries_to_uploads_with_their_dirs() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("a.jar"), b"x").unwrap();
        std::fs::write(dir.path().join("b.jar"), b"x").unwrap();
        let dir_str = dir.path().to_str().unwrap();

        let artifacts = resolve_entries(
            dir_str,
            &[entry("a.jar", "/srv/row"), entry("b.jar", "/srv/default")],
        )
        .unwrap();
        assert_eq!(artifacts.len(), 2);
        assert_eq!(artifacts[0].file_name, "a.jar");
        assert_eq!(artifacts[0].upload_dir, "/srv/row");
        assert_eq!(artifacts[0].local_path, dir.path().join("a.jar"));
        assert_eq!(artifacts[1].file_name, "b.jar");
        assert_eq!(artifacts[1].upload_dir, "/srv/default");

        // 目录两侧空白规整；空条目列表合法（该目标没有勾选任何构件）。
        let artifacts = resolve_entries(dir_str, &[entry("a.jar", "  /srv/x  ")]).unwrap();
        assert_eq!(artifacts[0].upload_dir, "/srv/x");
        assert!(resolve_entries(dir_str, &[]).unwrap().is_empty());
    }

    /// 校验：文件名不在本次构建产物里 → `artifact_not_found`（含路径穿越与
    /// 空名）；目录为空（含纯空白）→ `empty_upload_dir`。
    #[test]
    fn resolve_entries_rejects_unknown_artifacts_and_empty_dirs() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("a.jar"), b"x").unwrap();
        let dir_str = dir.path().to_str().unwrap();

        for bad_name in ["missing.jar", "", "../evil.jar", "sub/dir.jar"] {
            let err = resolve_entries(dir_str, &[entry(bad_name, "/srv/a")]).unwrap_err();
            assert_eq!(err.code(), "artifact_not_found", "case {bad_name:?}");
        }
        for bad_dir in ["", "   "] {
            let err = resolve_entries(dir_str, &[entry("a.jar", bad_dir)]).unwrap_err();
            assert_eq!(err.code(), "empty_upload_dir", "case {bad_dir:?}");
        }
    }

    /// 线上形状（golden）：部署 payload 每目标 `{ targetId, entries:
    /// [{fileName, uploadDir}] }`，camelCase；仅停止/仅启动时 entries 可缺省。
    #[test]
    fn target_assignment_deserializes_from_the_frontend_payload() {
        let json = r#"{"targetId":1,"entries":[{"fileName":"a.jar","uploadDir":"/srv/a"}]}"#;
        let parsed: TargetAssignment = serde_json::from_str(json).unwrap();
        assert_eq!(parsed.target_id, 1);
        assert_eq!(parsed.entries.len(), 1);
        assert_eq!(parsed.entries[0].file_name, "a.jar");
        assert_eq!(parsed.entries[0].upload_dir, "/srv/a");

        let stop_only: TargetAssignment = serde_json::from_str(r#"{"targetId":2}"#).unwrap();
        assert_eq!(stop_only.target_id, 2);
        assert!(stop_only.entries.is_empty());
    }
}
