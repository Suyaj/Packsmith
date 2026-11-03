//! 部署执行编排 (deploy orchestration) — tested entirely against a hermetic
//! [`MockTransport`]; the real russh transport is exercised by
//! `tests/deploy_ssh_integration_test.rs` (env-gated, `#[ignore]`).

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;

use super::{
    ArtifactRef, ConnInfo, DeployEvent, DeployPlan, DeploySummary, DeployTransport, ExecOutput,
    TargetJob, TransportError,
};
use crate::store::projects::new_cancel_flag;

// ---------------------------------------------------------------------------
// Hermetic transport: records every call so tests assert on the public
// behavior of the orchestration, never on engine internals. Recordings live
// behind Arc so clones made by the per-target factory share them.
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq)]
struct RecordedUpload {
    remote_dir: String,
    file_name: String,
    bytes: Vec<u8>,
}

#[derive(Clone, Default)]
struct MockTransport {
    uploads: Arc<Mutex<Vec<RecordedUpload>>>,
    execs: Arc<Mutex<Vec<String>>>,
    /// Exit code 1 for any invoked script containing one of these needles.
    fail_when_contains: Arc<Vec<String>>,
    /// Simulates an SFTP write error.
    fail_upload: bool,
}

impl MockTransport {
    fn failing_on(needle: &str) -> Self {
        MockTransport {
            fail_when_contains: Arc::new(vec![needle.to_string()]),
            ..Default::default()
        }
    }
}

#[async_trait]
impl DeployTransport for MockTransport {
    async fn exec(&mut self, command: &str) -> Result<ExecOutput, TransportError> {
        self.execs.lock().unwrap().push(command.to_string());
        let failed = self
            .fail_when_contains
            .iter()
            .any(|needle| command.contains(needle));
        Ok(ExecOutput {
            exit_code: if failed { 1 } else { 0 },
            combined: if failed {
                format!("ERROR: {command}")
            } else {
                format!("ok: {command}")
            },
        })
    }

    async fn upload(
        &mut self,
        local: &Path,
        remote_dir: &str,
        file_name: &str,
        progress: &mut (dyn FnMut(u64, u64) -> bool + Send),
    ) -> Result<(), TransportError> {
        if self.fail_upload {
            return Err(TransportError::Io("disk full".to_string()));
        }
        let bytes = std::fs::read(local).map_err(|e| TransportError::Io(e.to_string()))?;
        progress(bytes.len() as u64, bytes.len() as u64);
        self.uploads.lock().unwrap().push(RecordedUpload {
            remote_dir: remote_dir.to_string(),
            file_name: file_name.to_string(),
            bytes,
        });
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Test fixtures
// ---------------------------------------------------------------------------

fn artifact(path: &Path) -> ArtifactRef {
    ArtifactRef {
        local_path: path.to_path_buf(),
        file_name: path.file_name().unwrap().to_string_lossy().into_owned(),
    }
}

fn conn(id: i64) -> ConnInfo {
    ConnInfo {
        server_id: id,
        host: "10.0.0.1".into(),
        port: 22,
        username: "deploy".into(),
        credential: crate::server::repo::Credential::Password {
            password: "pw".into(),
        },
        fingerprint: Some("SHA256:known".into()),
    }
}

fn job(id: i64, upload_dir: &str, stop: &str, start: &str) -> TargetJob {
    TargetJob {
        target_id: id,
        upload_dir: upload_dir.into(),
        stop_commands: super::split_commands(stop),
        start_commands: super::split_commands(start),
        conn: conn(id),
    }
}

fn plan(artifact: Option<ArtifactRef>, run_stop: bool, targets: Vec<TargetJob>) -> DeployPlan {
    DeployPlan {
        artifact,
        run_stop,
        targets,
    }
}

type FactoryResult = Result<MockTransport, TransportError>;

/// Per-target mock factory: servers listed in `fail_connect` fail to connect;
/// otherwise a fresh mock (optionally `failing_on` the given needle) is
/// created once per server and its recordings shared via clones.
fn make_factory(
    fail_connect: Vec<i64>,
    fail_on: Vec<(i64, &str)>,
) -> (
    impl Fn(ConnInfo) -> std::future::Ready<FactoryResult>,
    Arc<Mutex<HashMap<i64, MockTransport>>>,
) {
    let mocks: Arc<Mutex<HashMap<i64, MockTransport>>> = Arc::new(Mutex::new(HashMap::new()));
    let m2 = Arc::clone(&mocks);
    (
        move |conn: ConnInfo| {
            let result = if fail_connect.contains(&conn.server_id) {
                Err(TransportError::Unreachable {
                    detail: "connection refused".into(),
                })
            } else {
                let mut map = m2.lock().unwrap();
                let transport = map.entry(conn.server_id).or_insert_with(|| {
                    match fail_on.iter().find(|(id, _)| *id == conn.server_id) {
                        Some((_, needle)) => MockTransport::failing_on(needle),
                        None => MockTransport::default(),
                    }
                });
                Ok(transport.clone())
            };
            std::future::ready(result)
        },
        mocks,
    )
}

#[derive(Default)]
struct RecordedEvents(Mutex<Vec<DeployEvent>>);

impl RecordedEvents {
    fn push(&self, e: DeployEvent) {
        self.0.lock().unwrap().push(e);
    }

    fn of_kind(&self, kind: &str) -> Vec<DeployEvent> {
        self.0
            .lock()
            .unwrap()
            .iter()
            .filter(|e| event_kind(e) == kind)
            .cloned()
            .collect()
    }
}

fn event_kind(e: &DeployEvent) -> &'static str {
    match e {
        DeployEvent::Started { .. } => "started",
        DeployEvent::Phase { .. } => "phase",
        DeployEvent::UploadProgress { .. } => "upload_progress",
        DeployEvent::Output { .. } => "output",
        DeployEvent::TargetFinished { .. } => "target_finished",
        DeployEvent::Finished { .. } => "finished",
    }
}

async fn run(
    plan: DeployPlan,
    factory: impl Fn(ConnInfo) -> std::future::Ready<FactoryResult>,
) -> DeploySummary {
    let events = RecordedEvents::default();
    let sink = |e: DeployEvent| events.push(e);
    let summary = super::run_deploy(plan, factory, new_cancel_flag(), sink).await;
    // Events must mirror the summary exactly.
    let finished: Vec<(i64, String)> = events
        .of_kind("target_finished")
        .iter()
        .filter_map(|e| match e {
            DeployEvent::TargetFinished {
                target_id, status, ..
            } => Some((*target_id, status.to_string())),
            _ => None,
        })
        .collect();
    let expected: Vec<(i64, String)> = summary
        .targets
        .iter()
        .map(|o| (o.target_id, o.status.as_str().to_string()))
        .collect();
    assert_eq!(finished, expected, "events must mirror the summary");
    summary
}

fn tmp_artifact(name: &str, content: &[u8]) -> (tempfile::TempDir, PathBuf) {
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path().join(name);
    std::fs::write(&p, content).unwrap();
    (dir, p)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[tokio::test]
async fn uploads_artifact_with_original_filename_to_upload_dir() {
    let (_dir, path) = tmp_artifact("app-service-1.0.jar", b"jar-bytes");
    let (factory, mocks) = make_factory(vec![], vec![]);
    let summary = run(
        plan(
            Some(artifact(&path)),
            true,
            vec![job(1, "/srv/app", "sh stop.sh", "sh start.sh")],
        ),
        factory,
    )
    .await;

    let map = mocks.lock().unwrap();
    let uploads = map.get(&1).unwrap().uploads.lock().unwrap();
    assert_eq!(uploads.len(), 1);
    assert_eq!(uploads[0].remote_dir, "/srv/app");
    // 原文件名: exactly the artifact library's file name, no renaming.
    assert_eq!(uploads[0].file_name, "app-service-1.0.jar");
    assert_eq!(uploads[0].bytes, b"jar-bytes");
    assert_eq!(summary.targets[0].status.as_str(), "success");
}

#[tokio::test]
async fn stop_group_runs_serially_before_start_group() {
    let (_dir, path) = tmp_artifact("a.jar", b"x");
    let (factory, mocks) = make_factory(vec![], vec![]);
    let summary = run(
        plan(
            Some(artifact(&path)),
            true,
            vec![job(
                1,
                "/srv/app",
                "cd /srv/app\nsh stop.sh\nsleep 1",
                "sh start.sh --prod",
            )],
        ),
        factory,
    )
    .await;

    let map = mocks.lock().unwrap();
    let execs = map.get(&1).unwrap().execs.lock().unwrap();
    // Both groups run as one serial shell unit (命令组: N 条命令按序串行).
    assert_eq!(execs.len(), 2);
    assert!(
        execs[0].contains("cd /srv/app")
            && execs[0].contains("sh stop.sh")
            && execs[0].contains("sleep 1"),
        "stop group must run its commands serially together, got {:?}",
        execs[0]
    );
    assert_eq!(execs[1], "sh start.sh --prod");
    assert_eq!(summary.targets[0].status.as_str(), "success");
    assert_eq!(summary.targets[0].stop.as_ref().unwrap().exit_code, 0);
    assert_eq!(summary.targets[0].start.as_ref().unwrap().exit_code, 0);
}

#[tokio::test]
async fn stop_failure_aborts_restart_and_surfaces_stop_output() {
    let (_dir, path) = tmp_artifact("a.jar", b"x");
    let (factory, mocks) = make_factory(vec![], vec![(1, "sh stop.sh")]);
    let summary = run(
        plan(
            Some(artifact(&path)),
            true,
            vec![job(1, "/srv/app", "sh stop.sh", "sh start.sh")],
        ),
        factory,
    )
    .await;

    let outcome = &summary.targets[0];
    assert_eq!(outcome.status.as_str(), "failed");
    // 重启中止: the start group must NOT have run.
    assert!(outcome.start.is_none());
    assert!(outcome.stop_failed);
    let stop = outcome.stop.as_ref().expect("stop output must be visible");
    assert_eq!(stop.exit_code, 1);
    assert!(stop.combined.contains("ERROR"));

    let map = mocks.lock().unwrap();
    let execs = map.get(&1).unwrap().execs.lock().unwrap();
    assert!(
        execs.iter().all(|c| c.contains("stop")),
        "only the stop group may run, got {execs:?}"
    );
}

#[tokio::test]
async fn multiple_targets_deploy_sequentially_in_order() {
    let (_dir, path) = tmp_artifact("a.jar", b"x");
    let (factory, mocks) = make_factory(vec![], vec![]);
    let summary = run(
        plan(
            Some(artifact(&path)),
            true,
            vec![
                job(1, "/srv/one", "sh stop-one", "sh start-one"),
                job(2, "/srv/two", "sh stop-two", "sh start-two"),
            ],
        ),
        factory,
    )
    .await;

    assert!(summary.targets.iter().all(|o| o.status.as_str() == "success"));
    let map = mocks.lock().unwrap();
    assert_eq!(map.get(&1).unwrap().execs.lock().unwrap().len(), 2);
    assert_eq!(map.get(&2).unwrap().execs.lock().unwrap().len(), 2);
    // Original filename per target.
    let uploads = map.get(&2).unwrap().uploads.lock().unwrap();
    assert_eq!(uploads[0].remote_dir, "/srv/two");
    assert_eq!(uploads[0].file_name, "a.jar");
}

#[tokio::test]
async fn one_targets_failure_does_not_stop_the_next_target() {
    let (_dir, path) = tmp_artifact("a.jar", b"x");
    let (factory, mocks) = make_factory(vec![], vec![(1, "broken-stop")]);
    let summary = run(
        plan(
            Some(artifact(&path)),
            true,
            vec![
                job(1, "/srv/one", "sh broken-stop", "sh start-one"),
                job(2, "/srv/two", "sh stop-two", "sh start-two"),
            ],
        ),
        factory,
    )
    .await;

    assert_eq!(summary.targets[0].status.as_str(), "failed");
    assert_eq!(summary.targets[1].status.as_str(), "success");
    // Target 2 saw all its phases.
    assert_eq!(
        mocks.lock().unwrap().get(&2).unwrap().execs.lock().unwrap().len(),
        2
    );
}

#[tokio::test]
async fn connection_failure_fails_only_that_target() {
    let (_dir, path) = tmp_artifact("a.jar", b"x");
    let (factory, mocks) = make_factory(vec![1], vec![]);
    let summary = run(
        plan(
            Some(artifact(&path)),
            true,
            vec![job(1, "/a", "s1", "g1"), job(2, "/b", "s2", "g2")],
        ),
        factory,
    )
    .await;

    assert_eq!(summary.targets[0].status.as_str(), "failed");
    assert!(summary.targets[0].stop.is_none());
    assert!(summary.targets[0].error.is_some());
    assert_eq!(summary.targets[1].status.as_str(), "success");
    let map = mocks.lock().unwrap();
    assert!(!map.contains_key(&1), "failed connect must not record calls");
    assert!(map.contains_key(&2));
}

#[tokio::test]
async fn upload_failure_fails_target_before_any_command_group() {
    let (_dir, path) = tmp_artifact("a.jar", b"x");
    let mocks: Arc<Mutex<HashMap<i64, MockTransport>>> = Arc::new(Mutex::new(HashMap::new()));
    let summary = {
        let factory = move |conn: ConnInfo| {
            let _ = conn;
            std::future::ready(FactoryResult::Ok(MockTransport {
                fail_upload: true,
                ..Default::default()
            }))
        };
        super::run_deploy(
            plan(Some(artifact(&path)), true, vec![job(1, "/a", "sh stop", "sh start")]),
            factory,
            new_cancel_flag(),
            |_| {},
        )
        .await
    };

    let outcome = &summary.targets[0];
    assert_eq!(outcome.status.as_str(), "failed");
    assert!(outcome.upload.is_none());
    assert!(outcome.stop.is_none());
    assert!(outcome.error.is_some());
}

#[tokio::test]
async fn cancel_between_targets_marks_remaining_cancelled() {
    let (_dir, path) = tmp_artifact("a.jar", b"x");
    let cancel = crate::store::projects::new_cancel_flag();
    let summary = {
        let cancel2 = cancel.clone();
        let sink = move |e: DeployEvent| {
            // Cancel while the first target's stop group output arrives.
            if let DeployEvent::Output { phase, .. } = &e {
                if *phase == "stop" {
                    cancel2.store(true, Ordering::Relaxed);
                }
            }
        };
        let factory = |conn: ConnInfo| {
            let _ = conn;
            std::future::ready(Ok(MockTransport::default()))
        };
        super::run_deploy(
            plan(
                Some(artifact(&path)),
                true,
                vec![job(1, "/a", "s1", "g1"), job(2, "/b", "s2", "g2")],
            ),
            factory,
            cancel,
            sink,
        )
        .await
    };

    assert_eq!(summary.targets[0].status.as_str(), "success");
    assert_eq!(summary.targets[1].status.as_str(), "cancelled");
    assert!(summary.cancelled);
}

#[tokio::test]
async fn cancel_before_start_skips_everything() {
    let (_dir, path) = tmp_artifact("a.jar", b"x");
    let cancel = crate::store::projects::new_cancel_flag();
    cancel.store(true, Ordering::Relaxed);
    let (factory, mocks) = make_factory(vec![], vec![]);
    let summary = super::run_deploy(
        plan(Some(artifact(&path)), true, vec![job(1, "/a", "s1", "g1")]),
        factory,
        cancel,
        |_| {},
    )
    .await;
    assert_eq!(summary.targets[0].status.as_str(), "cancelled");
    assert!(summary.targets[0].upload.is_none());
    assert!(mocks.lock().unwrap().is_empty());
    assert!(summary.cancelled);
}

#[tokio::test]
async fn start_only_plan_runs_the_start_group_alone() {
    // 单独执行启动命令组: no upload, no stop — only the start group.
    let (factory, mocks) = make_factory(vec![], vec![]);
    let summary = run(
        plan(None, false, vec![job(1, "/a", "sh stop", "sh start")]),
        factory,
    )
    .await;

    let outcome = &summary.targets[0];
    assert_eq!(outcome.status.as_str(), "success");
    assert!(outcome.upload.is_none());
    assert!(outcome.stop.is_none());
    assert_eq!(outcome.start.as_ref().unwrap().exit_code, 0);
    let map = mocks.lock().unwrap();
    let execs = map.get(&1).unwrap().execs.lock().unwrap();
    assert_eq!(execs.len(), 1);
    assert_eq!(execs[0], "sh start");
}

#[tokio::test]
async fn run_stop_false_skips_stop_but_runs_start_after_upload() {
    let (_dir, path) = tmp_artifact("a.jar", b"x");
    let (factory, mocks) = make_factory(vec![], vec![]);
    let summary = run(
        plan(Some(artifact(&path)), false, vec![job(1, "/a", "sh stop", "sh start")]),
        factory,
    )
    .await;
    assert_eq!(summary.targets[0].status.as_str(), "success");
    let map = mocks.lock().unwrap();
    let execs = map.get(&1).unwrap().execs.lock().unwrap();
    assert_eq!(execs.len(), 1);
    assert_eq!(execs[0], "sh start");
    assert!(summary.targets[0].stop.is_none());
    assert!(summary.targets[0].upload.is_some());
}

#[tokio::test]
async fn empty_command_groups_are_no_op_success() {
    let (_dir, path) = tmp_artifact("a.jar", b"x");
    let (factory, mocks) = make_factory(vec![], vec![]);
    let summary = run(
        plan(Some(artifact(&path)), true, vec![job(1, "/a", "", "")]),
        factory,
    )
    .await;
    assert_eq!(summary.targets[0].status.as_str(), "success");
    assert!(summary.targets[0].stop.is_none());
    assert!(summary.targets[0].start.is_none());
    assert_eq!(
        mocks.lock().unwrap().get(&1).unwrap().execs.lock().unwrap().len(),
        0
    );
}

#[tokio::test]
async fn events_report_phase_and_output_snapshots() {
    let (_dir, path) = tmp_artifact("a.jar", b"x");
    let events = RecordedEvents::default();
    let sink = |e: DeployEvent| events.push(e);
    let factory = |conn: ConnInfo| {
        let _ = conn;
        std::future::ready(Ok(MockTransport::default()))
    };
    super::run_deploy(
        plan(
            Some(artifact(&path)),
            true,
            vec![job(7, "/a", "sh stop", "sh start")],
        ),
        factory,
        new_cancel_flag(),
        sink,
    )
    .await;

    let phases: Vec<(i64, &str, &str)> = events
        .of_kind("phase")
        .iter()
        .filter_map(|e| match e {
            DeployEvent::Phase {
                target_id, phase, status, ..
            } => Some((*target_id, *phase, *status)),
            _ => None,
        })
        .collect();
    assert_eq!(
        phases,
        vec![
            (7, "upload", "running"),
            (7, "stop", "running"),
            (7, "start", "running"),
        ]
    );
    assert_eq!(events.of_kind("output").len(), 3, "each phase leaves an output snapshot");
    assert_eq!(events.of_kind("finished").len(), 1);
}

#[tokio::test]
async fn upload_progress_is_reported() {
    let (_dir, path) = tmp_artifact("a.jar", b"0123456789");
    let events = RecordedEvents::default();
    let sink = |e: DeployEvent| events.push(e);
    let factory = |conn: ConnInfo| {
        let _ = conn;
        std::future::ready(Ok(MockTransport::default()))
    };
    super::run_deploy(
        plan(Some(artifact(&path)), true, vec![job(1, "/a", "s", "g")]),
        factory,
        new_cancel_flag(),
        sink,
    )
    .await;
    let progress = events.of_kind("upload_progress");
    assert!(!progress.is_empty());
    match &progress[progress.len() - 1] {
        DeployEvent::UploadProgress {
            target_id,
            transferred,
            total,
            ..
        } => {
            assert_eq!(*target_id, 1);
            assert_eq!(*total, 10);
            assert_eq!(*transferred, 10);
        }
        other => panic!("expected upload progress, got {other:?}"),
    }
}

#[test]
fn split_commands_trims_and_drops_blank_lines() {
    assert_eq!(
        super::split_commands("  cd /app  \n\n sh start.sh \n"),
        vec!["cd /app", "sh start.sh"]
    );
    assert!(super::split_commands(" \n ").is_empty());
}

#[test]
fn join_group_runs_commands_serially_as_one_unit() {
    let joined = super::join_group(&super::split_commands("cd /app\nsh stop.sh"));
    assert_eq!(joined.as_deref(), Some("cd /app && sh stop.sh"));
    assert!(super::join_group(&[]).is_none());
}
