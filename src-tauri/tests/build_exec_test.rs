//! Integration tests for 打包（二）：构建执行与清理 (ticket 07).
//!
//! Seams under test:
//! - `build::run_build_flow_in` — the full end-to-end flow: temp workspace
//!   clone (ticket 06) → managed-toolchain build execution → always-cleanup,
//!   with live log events and a persisted per-build log file.
//!
//! All tests run against a local fixture git repository and a fake managed
//! toolchain (a shell script standing in for mvnd), so they are hermetic.
//! The one real-mvnd network test is `#[ignore]`-gated.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use packsmith_lib::build::{self, BuildError, BuildEvent, CloneStage};
use packsmith_lib::store::projects::{self, ProjectType};
use packsmith_lib::store::Store;

const TEST_KEY_HEX: &str = "a1b2c3d4e5f60718293a4b5c6d7e8f90a1b2c3d4e5f60718293a4b5c6d7e8f90";

fn open_store(dir: &Path) -> Store {
    Store::open(&dir.join("test.db"), TEST_KEY_HEX).unwrap()
}

/// A local git repo (project source) containing a `pom.xml` marker file.
fn fixture_project(dir: &Path, store: &Store, name: &str) -> PathBuf {
    let repo_path = dir.join(name);
    let repo = git2::Repository::init(&repo_path).unwrap();
    std::fs::write(repo_path.join("pom.xml"), "<project/>\n").unwrap();
    let mut index = repo.index().unwrap();
    index.add_path(Path::new("pom.xml")).unwrap();
    index.write().unwrap();
    let tree_id = index.write_tree().unwrap();
    let tree = repo.find_tree(tree_id).unwrap();
    let sig = git2::Signature::now("fixture", "fixture@example.com").unwrap();
    repo.commit(Some("refs/heads/main"), &sig, &sig, "initial", &tree, &[])
        .unwrap();

    projects::import(store, name, &repo_path, ProjectType::Maven).unwrap();
    repo_path
}

/// Installs a fake managed toolchain: a shell script standing in for mvnd
/// (printing its args and exiting with `code`) plus a dummy JDK 8 layout.
fn fake_toolchain(dir: &Path, script: &str) -> PathBuf {
    let tools = dir.join("tools");
    let mvnd = tools.join("mvnd/bin/mvnd");
    std::fs::create_dir_all(mvnd.parent().unwrap()).unwrap();
    std::fs::write(&mvnd, script).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&mvnd, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    let java = tools.join("jdk/Contents/Home/bin/java");
    std::fs::create_dir_all(java.parent().unwrap()).unwrap();
    std::fs::write(&java, "not a real jvm").unwrap();
    tools
}

#[cfg(unix)]
fn success_script() -> &'static str {
    "#!/bin/sh\nprintf 'arg:%s\\n' \"$@\"\necho 'fake build output'\nexit 0\n"
}

#[cfg(unix)]
fn failure_script() -> &'static str {
    "#!/bin/sh\necho 'boom' >&2\nexit 3\n"
}

type Events = Arc<Mutex<Vec<BuildEvent>>>;

fn event_collector() -> (Events, Arc<dyn Fn(BuildEvent) + Send + Sync>) {
    let events: Events = Arc::new(Mutex::new(Vec::new()));
    let sink = events.clone();
    let cb = Arc::new(move |e: BuildEvent| sink.lock().unwrap().push(e));
    (events, cb)
}

struct Fixture {
    _dir: tempfile::TempDir,
    store: Store,
    project_id: i64,
    temp_root: PathBuf,
    appdata: PathBuf,
    tools: PathBuf,
}

/// Sets up store + local project + fake toolchain + isolated temp/appdata
/// roots. `script` is the fake mvnd script body.
#[cfg(unix)]
fn setup(script: &str) -> Fixture {
    let dir = tempfile::tempdir().unwrap();
    let store = open_store(dir.path());
    fixture_project(dir.path(), &store, "demo-app");
    let project_id = projects::list(&store).unwrap()[0].id;
    let tools = fake_toolchain(dir.path(), script);
    let temp_root = dir.path().join("tmp");
    let appdata = dir.path().join("appdata");
    std::fs::create_dir_all(&temp_root).unwrap();
    Fixture {
        _dir: dir,
        store,
        project_id,
        temp_root,
        appdata,
        tools,
    }
}

fn workspaces_in(root: &Path) -> Vec<PathBuf> {
    std::fs::read_dir(root)
        .map(|entries| {
            entries
                .flatten()
                .map(|e| e.path())
                .filter(|p| p.file_name().unwrap().to_string_lossy().starts_with("packsmith-build-"))
                .collect()
        })
        .unwrap_or_default()
}

fn clone_events(events: &Events) -> Vec<CloneStage> {
    events
        .lock()
        .unwrap()
        .iter()
        .filter_map(|e| match e {
            BuildEvent::Clone { stage, .. } => Some(*stage),
            _ => None,
        })
        .collect()
}

fn log_lines(events: &Events) -> Vec<String> {
    events
        .lock()
        .unwrap()
        .iter()
        .filter_map(|e| match e {
            BuildEvent::Log { line } => Some(line.clone()),
            _ => None,
        })
        .collect()
}

#[cfg(unix)]
#[tokio::test]
async fn 成功构建_日志完整_工作区无残留_依赖仓库目录建立() {
    let fx = setup(success_script());

    let (events, on_event) = event_collector();
    let cancel = projects::new_cancel_flag();

    let outcome = build::run_build_flow_in(
        fx.store.clone(),
        &fx.tools,
        &fx.appdata,
        &fx.temp_root,
        fx.project_id,
        "main",
        cancel,
        on_event,
    )
    .await
    .expect("build flow succeeds");

    // 构建成功
    assert!(outcome.success);
    assert_eq!(outcome.exit_code, Some(0));

    // 构建命令（含追加的依赖仓库参数）确实传给了托管 mvnd
    let lines = log_lines(&events);
    assert!(lines.iter().any(|l| l.contains("arg:package")), "lines: {lines:?}");
    let repo_flag = lines
        .iter()
        .find(|l| l.starts_with("arg:-Dmaven.repo.local="))
        .expect("repo-local flag must be passed to mvnd");
    assert_eq!(
        repo_flag.as_str(),
        format!("arg:-Dmaven.repo.local={}", fx.appdata.join("m2-repo").display())
    );

    // 临时工作区一律删除
    assert!(workspaces_in(&fx.temp_root).is_empty(), "temp workspace must be removed");

    // 依赖仓库目录首次构建即创建，且持久保留
    assert!(fx.appdata.join("m2-repo").is_dir());

    // 日志持久化：文件存在且含构建输出
    let log_path = PathBuf::from(&outcome.log_path);
    assert!(log_path.is_file());
    let content = std::fs::read_to_string(&log_path).unwrap();
    assert!(content.contains("fake build output"));
    assert!(content.contains("-Dmaven.repo.local"));

    // 克隆进度事件也经同一通道上报
    assert!(clone_events(&events).contains(&CloneStage::Done));
}

#[cfg(unix)]
#[tokio::test]
async fn 失败构建_报告非零退出_错误日志留存_工作区无残留() {
    let fx = setup(failure_script());

    let (events, on_event) = event_collector();
    let cancel = projects::new_cancel_flag();

    let outcome = build::run_build_flow_in(
        fx.store.clone(),
        &fx.tools,
        &fx.appdata,
        &fx.temp_root,
        fx.project_id,
        "main",
        cancel,
        on_event,
    )
    .await
    .expect("a failed build is still a completed flow");

    assert!(!outcome.success);
    assert_eq!(outcome.exit_code, Some(3));

    // stderr 进了实时日志
    assert!(log_lines(&events).iter().any(|l| l.contains("boom")));

    // 失败只留构建日志：文件在，工作区不在
    let content = std::fs::read_to_string(&outcome.log_path).unwrap();
    assert!(content.contains("boom"), "failure log must keep stderr");
    assert!(workspaces_in(&fx.temp_root).is_empty());
}

#[cfg(unix)]
#[tokio::test]
async fn 取消构建_子进程被杀死_工作区无残留_报取消错误() {
    let script = "#!/bin/sh\necho 'started'\nexec sleep 30\n";
    let fx = setup(script);

    let (events, on_event) = event_collector();
    let cancel = projects::new_cancel_flag();

    // tokio::spawn 需要 'static：把所需值克隆成所有权值再移入任务块。
    let tools = fx.tools.clone();
    let appdata = fx.appdata.clone();
    let temp_root = fx.temp_root.clone();
    let project_id = fx.project_id;
    let store = fx.store.clone();
    let cancel_inner = cancel.clone();
    let flow = tokio::spawn(async move {
        build::run_build_flow_in(
            store,
            &tools,
            &appdata,
            &temp_root,
            project_id,
            "main",
            cancel_inner,
            on_event,
        )
        .await
    });

    // 等到构建真正开始（脚本输出了 started），再取消。
    let deadline = std::time::Instant::now() + Duration::from_secs(10);
    loop {
        if log_lines(&events).iter().any(|l| l.contains("started")) {
            break;
        }
        assert!(std::time::Instant::now() < deadline, "build never started");
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    cancel.store(true, std::sync::atomic::Ordering::Relaxed);

    let result = tokio::time::timeout(Duration::from_secs(10), flow)
        .await
        .expect("cancelled flow must return promptly (child killed, not leaked)")
        .unwrap();

    assert!(matches!(
        result,
        Err(BuildError::Cancelled)
    ));
    assert!(workspaces_in(&fx.temp_root).is_empty(), "cancel must clean the workspace");
    // 脚本先输出 started 再长睡：取消后日志里应有该行（进程被杀，不再有输出）。
    let lines = log_lines(&events);
    assert!(lines.iter().any(|l| l.contains("started")), "lines: {lines:?}");
}

#[cfg(unix)]
#[tokio::test]
async fn 工具链缺失_报缺失错误且不留工作区() {
    let fx = setup(success_script());
    let empty_tools = fx._dir.path().join("no-tools");
    std::fs::create_dir_all(&empty_tools).unwrap();

    let (_events, on_event) = event_collector();
    let cancel = projects::new_cancel_flag();

    let result = build::run_build_flow_in(
        fx.store.clone(),
        &empty_tools,
        &fx.appdata,
        &fx.temp_root,
        fx.project_id,
        "main",
        cancel,
        on_event,
    )
    .await;

    assert!(matches!(result, Err(BuildError::ToolchainMissing("mvnd"))));
    assert!(workspaces_in(&fx.temp_root).is_empty());
}

#[tokio::test]
async fn 构建日志可按id读回_非法id被拒绝() {
    let fx = setup(success_script());
    let (events, on_event) = event_collector();
    let cancel = projects::new_cancel_flag();

    let outcome = build::run_build_flow_in(
        fx.store.clone(),
        &fx.tools,
        &fx.appdata,
        &fx.temp_root,
        fx.project_id,
        "main",
        cancel,
        on_event,
    )
    .await
    .unwrap();

    let content = build::read_build_log(&fx.appdata, &outcome.build_id).unwrap();
    assert!(content.contains("fake build output"));

    // 路径穿越 / 任意 id 拒绝
    assert!(matches!(
        build::read_build_log(&fx.appdata, "../secrets"),
        Err(BuildError::LogNotFound(_))
    ));
}

// ---------------------------------------------------------------------------
// 真实托管 mvnd 网络测试（#[ignore]）：需要网络下载依赖插件。
// 运行：cargo test --test build_exec_test -- --ignored
// ---------------------------------------------------------------------------

#[tokio::test]
#[ignore = "network + real managed toolchain: downloads dependencies on first run"]
async fn 真实托管mvnd构建最小maven项目_依赖落在独立仓库() {
    // 从真实 appdata（macOS）解析托管工具链；未安装则跳过。
    let home = std::env::var("HOME").unwrap();
    let tools = PathBuf::from(format!("{home}/Library/Application Support/com.wyy.packsmith/tools"));
    if !tools.join("mvnd").is_dir() || !tools.join("jdk").is_dir() {
        eprintln!("skipping: managed toolchain not installed at {tools:?}");
        return;
    }

    let dir = tempfile::tempdir().unwrap();
    let store = open_store(dir.path());
    fixture_project(dir.path(), &store, "real-demo");
    let project_id = projects::list(&store).unwrap()[0].id;

    let temp_root = dir.path().join("tmp");
    let appdata = dir.path().join("appdata");
    std::fs::create_dir_all(&temp_root).unwrap();
    let (events, on_event) = event_collector();
    let cancel = projects::new_cancel_flag();

    let outcome = build::run_build_flow_in(
        store,
        &tools,
        &appdata,
        &temp_root,
        project_id,
        "main",
        cancel,
        on_event,
    )
    .await
    .expect("real mvnd build must succeed");

    assert!(outcome.success, "exit code {:?}", outcome.exit_code);
    assert!(workspaces_in(&temp_root).is_empty());
    // 依赖确实下载进了独立仓库（ADR-0001），而非 ~/.m2。
    let m2 = appdata.join("m2-repo");
    assert!(m2.join("org/apache/maven").is_dir() || m2.read_dir().unwrap().next().is_some(),
        "m2-repo must be populated");
    let _ = events.lock().unwrap().len();
}
