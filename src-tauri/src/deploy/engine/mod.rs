//! 部署执行编排 (deploy orchestration, ticket 10).
//!
//! For each 部署目标 (deployment target) in plan order:
//! 1. **上传** — SFTP the build artifact from the 产物库 to the target's
//!    upload dir under its **original file name** (overwrite).
//! 2. **停止** — run the 停止命令组 as one serial shell unit.
//! 3. 停止成功 → **启动** — run the 启动命令组 the same way.
//!
//! A failed stop group aborts the restart (GLOSSARY: 重启): the start group
//! never runs, the stop-phase output/error is surfaced via
//! [`DeployEvent::Output`] / [`DeployEvent::TargetFinished`] with
//! `stop_failed = true`, and the user may later run the start group alone.
//!
//! The SSH/SFTP work happens behind the [`DeployTransport`] seam so the whole
//! orchestration is testable hermetically; the real russh implementation is
//! [`super::transport::SshTransport`].

use std::future::Future;
use std::path::Path;

use async_trait::async_trait;
use serde::Serialize;

use crate::store::projects::CancelFlag;
use crate::targets::repo::split_commands as split_commands_stored;

/// One phase's output snapshot (upload/stop/start). `combined` is the
/// command's stdout + stderr (or an engine-written line for the upload).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ExecOutput {
    pub exit_code: i32,
    pub combined: String,
}

/// Transport-level failure. The frontend keys i18n on `code()`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TransportError {
    /// No stored fingerprint and deploys never accept new keys silently —
    /// the user must run a 连接测试 once and accept the fingerprint.
    HostKeyUnknown { fingerprint: String },
    /// The server's key differs from the stored fingerprint (possible MITM).
    HostKeyChanged { fingerprint: String },
    Unreachable { detail: String },
    AuthFailed,
    KeyUnreadable { detail: String },
    /// SFTP / local-file I/O failure during upload.
    Io(String),
    /// The user cancelled; checked via the upload progress callback.
    Cancelled,
}

impl TransportError {
    /// A stable machine-readable tag for the frontend to key i18n on.
    pub fn code(&self) -> &'static str {
        match self {
            TransportError::HostKeyUnknown { .. } => "host_key_unknown",
            TransportError::HostKeyChanged { .. } => "host_key_changed",
            TransportError::Unreachable { .. } => "unreachable",
            TransportError::AuthFailed => "auth_failed",
            TransportError::KeyUnreadable { .. } => "key_unreadable",
            TransportError::Io(_) => "io",
            TransportError::Cancelled => "cancelled",
        }
    }
}

impl std::fmt::Display for TransportError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            TransportError::HostKeyUnknown { fingerprint } => {
                write!(f, "unknown host key ({fingerprint}); accept it via a connection test first")
            }
            TransportError::HostKeyChanged { fingerprint } => {
                write!(f, "host key changed ({fingerprint}); possible MITM")
            }
            TransportError::Unreachable { detail } => write!(f, "connection failed: {detail}"),
            TransportError::AuthFailed => write!(f, "authentication failed"),
            TransportError::KeyUnreadable { detail } => {
                write!(f, "private key unreadable: {detail}")
            }
            TransportError::Io(detail) => write!(f, "upload failed: {detail}"),
            TransportError::Cancelled => write!(f, "cancelled"),
        }
    }
}

/// How to reach one target's server: everything the transport needs to
/// connect (resolved from the encrypted store by the command layer).
#[derive(Debug, Clone)]
pub struct ConnInfo {
    pub server_id: i64,
    pub host: String,
    pub port: u16,
    pub username: String,
    pub credential: crate::server::repo::Credential,
    /// Stored accepted fingerprint; `None` means never accepted (deploys
    /// reject, unlike the interactive 连接测试).
    pub fingerprint: Option<String>,
}

/// One per-target job: where to connect and what to run.
#[derive(Debug, Clone)]
pub struct TargetJob {
    pub target_id: i64,
    pub upload_dir: String,
    pub stop_commands: Vec<String>,
    pub start_commands: Vec<String>,
    pub conn: ConnInfo,
}

/// The artifact to upload (from the 产物库), under its original file name.
#[derive(Debug, Clone)]
pub struct ArtifactRef {
    pub local_path: std::path::PathBuf,
    pub file_name: String,
}

/// What [`run_deploy`] should do. `artifact: None` + `run_stop: false` is the
/// "单独执行启动命令组" (start-group-only) operation.
#[derive(Debug, Clone)]
pub struct DeployPlan {
    pub artifact: Option<ArtifactRef>,
    pub run_stop: bool,
    pub targets: Vec<TargetJob>,
}

/// Per-target final state. `upload`/`stop`/`start` are the output snapshots
/// the UI shows; a phase that never ran is `None`.
#[derive(Debug, Clone, Serialize)]
pub struct TargetOutcome {
    pub target_id: i64,
    pub status: TargetStatus,
    pub upload: Option<ExecOutput>,
    pub stop: Option<ExecOutput>,
    pub start: Option<ExecOutput>,
    /// True when the stop group (or its connection) failed — the start group
    /// was withheld and the user may run it manually.
    pub stop_failed: bool,
    pub error: Option<String>,
}

/// Status values as sent to the frontend (uploading/stop/starting/success/…).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum TargetStatus {
    Pending,
    Uploading,
    Stopping,
    Starting,
    Success,
    Failed,
    Cancelled,
}

impl TargetStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            TargetStatus::Pending => "pending",
            TargetStatus::Uploading => "uploading",
            TargetStatus::Stopping => "stopping",
            TargetStatus::Starting => "starting",
            TargetStatus::Success => "success",
            TargetStatus::Failed => "failed",
            TargetStatus::Cancelled => "cancelled",
        }
    }
}

/// Overall result returned by [`run_deploy`].
#[derive(Debug, Clone, Serialize)]
pub struct DeploySummary {
    pub cancelled: bool,
    pub targets: Vec<TargetOutcome>,
}

/// Streaming events for the UI: per-target phase changes, upload progress and
/// output snapshots (输出快照). Serialized over the Tauri `Channel`.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum DeployEvent {
    Started { target_ids: Vec<i64> },
    Phase { target_id: i64, phase: &'static str, status: &'static str },
    UploadProgress { target_id: i64, transferred: u64, total: u64 },
    Output { target_id: i64, phase: &'static str, output: String, exit_code: i32 },
    TargetFinished {
        target_id: i64,
        status: &'static str,
        stop_failed: bool,
        error: Option<String>,
    },
    Finished { cancelled: bool },
}

/// The SSH/SFTP seam: one connected transport per target (one target = one
/// connection lifetime). Commands run as the SSH login user.
#[async_trait]
pub trait DeployTransport: Send {
    /// Runs one command group script (`sh -c` semantics on the server) and
    /// collects stdout+stderr plus the exit code.
    async fn exec(&mut self, command: &str) -> Result<ExecOutput, TransportError>;

    /// Uploads the local file into `remote_dir` as `file_name` (create or
    /// overwrite). The `progress` callback receives `(transferred, total)`
    /// and returns whether to continue; returning `false` aborts the upload
    /// with [`TransportError::Cancelled`].
    async fn upload(
        &mut self,
        local: &Path,
        remote_dir: &str,
        file_name: &str,
        progress: &mut (dyn FnMut(u64, u64) -> bool + Send),
    ) -> Result<(), TransportError>;
}

/// Splits a 命令组 into its commands (same normalization as storage,
/// `targets::repo::split_commands`).
pub fn split_commands(text: &str) -> Vec<String> {
    split_commands_stored(text)
}

/// Joins a command group into one serial shell script: commands run in order
/// and the group fails fast (`&&`) — 命令组 is N commands serial as one unit.
/// An empty group is `None` (no-op phase).
pub fn join_group(commands: &[String]) -> Option<String> {
    if commands.is_empty() {
        None
    } else {
        Some(commands.join(" && "))
    }
}

/// Runs the whole deployment plan to completion (cancellable between targets,
/// between phases and per upload chunk). Never panics on transport failures —
/// every failure lands in a [`TargetOutcome`].
pub async fn run_deploy<T, F, Fut, S>(
    plan: DeployPlan,
    make_transport: F,
    cancel: CancelFlag,
    mut sink: S,
) -> DeploySummary
where
    F: Fn(ConnInfo) -> Fut,
    Fut: Future<Output = Result<T, TransportError>> + Send,
    T: DeployTransport,
    S: FnMut(DeployEvent) + Send,
{
    sink(DeployEvent::Started {
        target_ids: plan.targets.iter().map(|t| t.target_id).collect(),
    });

    let mut outcomes = Vec::with_capacity(plan.targets.len());
    let mut cancelled = false;

    for job in &plan.targets {
        // Between-target cancellation: everything still pending is cancelled.
        if cancel.load(std::sync::atomic::Ordering::Relaxed) {
            cancelled = true;
            let outcome = TargetOutcome {
                target_id: job.target_id,
                status: TargetStatus::Cancelled,
                upload: None,
                stop: None,
                start: None,
                stop_failed: false,
                error: None,
            };
            finish(&mut sink, &outcome);
            outcomes.push(outcome);
            continue;
        }

        let mut outcome = TargetOutcome {
            target_id: job.target_id,
            status: TargetStatus::Uploading,
            upload: None,
            stop: None,
            start: None,
            stop_failed: false,
            error: None,
        };

        let mut transport = match make_transport(job.conn.clone()).await {
            Ok(t) => t,
            Err(e) => {
                outcome.status = TargetStatus::Failed;
                outcome.error = Some(e.to_string());
                finish(&mut sink, &outcome);
                outcomes.push(outcome);
                continue;
            }
        };

        // Phase 1: upload (original file name, overwrite).
        if let Some(artifact) = &plan.artifact {
            sink(DeployEvent::Phase {
                target_id: job.target_id,
                phase: "upload",
                status: "running",
            });
            let cancel2 = cancel.clone();
            let mut progress = |transferred: u64, total: u64| {
                sink(DeployEvent::UploadProgress {
                    target_id: job.target_id,
                    transferred,
                    total,
                });
                !cancel2.load(std::sync::atomic::Ordering::Relaxed)
            };
            match transport
                .upload(
                    &artifact.local_path,
                    &job.upload_dir,
                    &artifact.file_name,
                    &mut progress,
                )
                .await
            {
                Ok(()) => {
                    let total = std::fs::metadata(&artifact.local_path)
                        .map(|m| m.len())
                        .unwrap_or(0);
                    let output = ExecOutput {
                        exit_code: 0,
                        combined: format!(
                            "uploaded {total} bytes to {} as {}",
                            job.upload_dir, artifact.file_name
                        ),
                    };
                    sink(DeployEvent::Output {
                        target_id: job.target_id,
                        phase: "upload",
                        output: output.combined.clone(),
                        exit_code: 0,
                    });
                    outcome.upload = Some(output);
                }
                Err(TransportError::Cancelled) => {
                    cancelled = true;
                    outcome.status = TargetStatus::Cancelled;
                    finish(&mut sink, &outcome);
                    outcomes.push(outcome);
                    continue;
                }
                Err(e) => {
                    outcome.status = TargetStatus::Failed;
                    outcome.error = Some(e.to_string());
                    finish(&mut sink, &outcome);
                    outcomes.push(outcome);
                    continue;
                }
            }
        }

        // Phase 2: stop group. 停止失败则中止重启 — never run the start group.
        if plan.run_stop {
            if let Some(script) = join_group(&job.stop_commands) {
                outcome.status = TargetStatus::Stopping;
                sink(DeployEvent::Phase {
                    target_id: job.target_id,
                    phase: "stop",
                    status: "running",
                });
                match transport.exec(&script).await {
                    Ok(out) => {
                        let failed = out.exit_code != 0;
                        sink(DeployEvent::Output {
                            target_id: job.target_id,
                            phase: "stop",
                            output: out.combined.clone(),
                            exit_code: out.exit_code,
                        });
                        outcome.stop = Some(out);
                        if failed {
                            outcome.status = TargetStatus::Failed;
                            outcome.stop_failed = true;
                            outcome.error = Some("stop command group failed".into());
                            finish(&mut sink, &outcome);
                            outcomes.push(outcome);
                            continue;
                        }
                    }
                    Err(e) => {
                        outcome.status = TargetStatus::Failed;
                        outcome.stop_failed = true;
                        outcome.error = Some(e.to_string());
                        finish(&mut sink, &outcome);
                        outcomes.push(outcome);
                        continue;
                    }
                }
            }
        }

        // Phase 3: start group (only reached when the stop group succeeded).
        if let Some(script) = join_group(&job.start_commands) {
            outcome.status = TargetStatus::Starting;
            sink(DeployEvent::Phase {
                target_id: job.target_id,
                phase: "start",
                status: "running",
            });
            match transport.exec(&script).await {
                Ok(out) => {
                    let failed = out.exit_code != 0;
                    sink(DeployEvent::Output {
                        target_id: job.target_id,
                        phase: "start",
                        output: out.combined.clone(),
                        exit_code: out.exit_code,
                    });
                    outcome.start = Some(out);
                    if failed {
                        outcome.status = TargetStatus::Failed;
                        outcome.error = Some("start command group failed".into());
                        finish(&mut sink, &outcome);
                        outcomes.push(outcome);
                        continue;
                    }
                }
                Err(e) => {
                    outcome.status = TargetStatus::Failed;
                    outcome.error = Some(e.to_string());
                    finish(&mut sink, &outcome);
                    outcomes.push(outcome);
                    continue;
                }
            }
        }

        outcome.status = TargetStatus::Success;
        finish(&mut sink, &outcome);
        outcomes.push(outcome);
    }

    sink(DeployEvent::Finished { cancelled });
    DeploySummary { cancelled, targets: outcomes }
}

fn finish(sink: &mut impl FnMut(DeployEvent), outcome: &TargetOutcome) {
    sink(DeployEvent::TargetFinished {
        target_id: outcome.target_id,
        status: outcome.status.as_str(),
        stop_failed: outcome.stop_failed,
        error: outcome.error.clone(),
    });
}

#[cfg(test)]
mod tests;
