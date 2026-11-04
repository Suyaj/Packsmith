//! Integration tests for 构件库与构建历史 (ticket 08).
//!
//! Seams under test:
//! - `build::artifacts::scan_workspace_artifacts` — scanning the Maven output
//!   (`target` dirs of the built project, multi-module aware) for jar/war
//!   artifacts in the temp workspace.
//! - `store::history` — build history persistence: record / list (most recent
//!   first, across all projects) / delete (artifacts + log + row) / retention
//!   (keep the most recent N builds per project) / manual per-project cleanup.
//! - `store::settings` — small key/value settings on `app_meta` (retention N).
//! - `projects::set_default_artifacts` — the remembered default artifact
//!   list of a project (migration v13, ticket 21; ordered, empty = unset).
//! - `build::run_build_flow_in` — end-to-end: on success artifacts are scanned
//!   and copied into `<appdata>/artifacts/<project>/<build time>/` BEFORE the
//!   temp workspace is deleted, a history row is written and retention is
//!   enforced; failed builds leave a history row without artifacts. The build
//!   process environment is captured to prove the dependency-repository
//!   isolation of ADR-0001 (nothing points at the user's `~/.m2`).
//!
//! All tests run against local fixture git repositories and a fake managed
//! toolchain (a shell script standing in for mvnd), so they are hermetic.

#![allow(non_snake_case)]

use std::fs;
use std::path::{Path, PathBuf};

use packsmith_lib::build::artifacts::scan_workspace_artifacts;

// ---------------------------------------------------------------------------
// 产物扫描（target 目录，多模块感知）
// ---------------------------------------------------------------------------

fn write_file(path: &Path, content: &[u8]) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, content).unwrap();
}

#[test]
fn 扫描单模块target_只认jar与war构件() {
    let ws = tempfile::tempdir().unwrap();
    write_file(&ws.path().join("target/app-1.0.jar"), b"jar-bytes");
    write_file(&ws.path().join("target/app-1.0.war"), b"war-bytes");
    // 干扰项：classes、文本、源码目录里的 jar 都不算构件。
    write_file(&ws.path().join("target/classes/App.class"), b"class");
    write_file(&ws.path().join("target/README.txt"), b"txt");
    write_file(
        &ws.path().join("src/lib/not-a-build-output.jar"),
        b"src jar",
    );

    let found = scan_workspace_artifacts(ws.path());

    let mut names: Vec<&str> = found.iter().map(|a| a.file_name.as_str()).collect();
    names.sort();
    assert_eq!(names, vec!["app-1.0.jar", "app-1.0.war"]);
    let jar = found.iter().find(|a| a.file_name == "app-1.0.jar").unwrap();
    assert_eq!(jar.relative_path.replace('\\', "/"), "target/app-1.0.jar");
    assert_eq!(jar.size, 9);
}

#[test]
fn 扫描多模块工程_每个模块的target都被覆盖() {
    let ws = tempfile::tempdir().unwrap();
    write_file(&ws.path().join("pom.xml"), b"<project/>");
    write_file(&ws.path().join("module-a/target/a-api-2.0.jar"), b"a");
    write_file(&ws.path().join("module-b/target/b-web-2.0.war"), b"bb");

    let mut found = scan_workspace_artifacts(ws.path());
    found.sort_by(|x, y| x.relative_path.cmp(&y.relative_path));

    let rels: Vec<String> = found.iter().map(|a| a.relative_path.clone()).collect();
    assert_eq!(
        rels,
        vec![
            "module-a/target/a-api-2.0.jar",
            "module-b/target/b-web-2.0.war"
        ]
    );
}

#[test]
fn 没有构件时扫描结果为空() {
    let ws = tempfile::tempdir().unwrap();
    write_file(&ws.path().join("pom.xml"), b"<project/>");
    write_file(&ws.path().join("target/classes/App.class"), b"class");

    assert!(scan_workspace_artifacts(ws.path()).is_empty());
}

#[test]
fn 扫描对不存在的目录返回空而不是报错() {
    let missing: PathBuf = PathBuf::from("/nonexistent/packsmith-workspace");
    assert!(scan_workspace_artifacts(&missing).is_empty());
}

// ---------------------------------------------------------------------------
// 构建历史（持久化）
// ---------------------------------------------------------------------------

use packsmith_lib::store::Store;
use packsmith_lib::store::history::{self, NewHistoryEntry};
use packsmith_lib::store::projects::{self, ProjectType};
use packsmith_lib::store::settings;

const TEST_KEY_HEX: &str = "a1b2c3d4e5f60718293a4b5c6d7e8f90a1b2c3d4e5f60718293a4b5c6d7e8f90";

fn open_store(dir: &Path) -> Store {
    Store::open(&dir.join("test.db"), TEST_KEY_HEX).unwrap()
}

/// 导入一个项目；本地路径必须存在且各不相同，用 `<dir>/<name>`。
fn import_project(
    store: &Store,
    dir: &Path,
    name: &str,
) -> packsmith_lib::store::projects::Project {
    let path = dir.join(name);
    std::fs::create_dir_all(&path).unwrap();
    projects::import(store, name, &path, ProjectType::Maven).unwrap()
}

fn new_entry(project_id: i64, build_id: &str, success: bool) -> NewHistoryEntry {
    NewHistoryEntry {
        build_id: build_id.to_string(),
        project_id,
        branch: "main".to_string(),
        success,
        exit_code: if success { Some(0) } else { Some(1) },
        log_path: format!("/appdata/build-logs/{build_id}.log"),
        artifacts_dir: None,
    }
}

#[test]
fn 历史列表_跨项目_最近在前() {
    let dir = tempfile::tempdir().unwrap();
    let store = open_store(dir.path());
    let p1 = import_project(&store, dir.path(), "alpha");
    let p2 = import_project(&store, dir.path(), "beta");

    history::record(&store, &new_entry(p1.id, "build-1-a", true)).unwrap();
    history::record(&store, &new_entry(p2.id, "build-2-b", false)).unwrap();
    history::record(&store, &new_entry(p1.id, "build-3-c", true)).unwrap();

    let rows = history::list(&store, None).unwrap();
    let build_ids: Vec<&str> = rows.iter().map(|r| r.build_id.as_str()).collect();
    assert_eq!(build_ids, vec!["build-3-c", "build-2-b", "build-1-a"]);

    // 字段完整：项目名联查、成败、分支、日志路径。
    let first = &rows[0];
    assert_eq!(first.project_name, "alpha");
    assert!(first.success);
    assert_eq!(first.exit_code, Some(0));
    assert_eq!(first.branch, "main");
    assert_eq!(first.log_path, "/appdata/build-logs/build-3-c.log");
    assert!(
        first.created_at.contains("T"),
        "created_at: {}",
        first.created_at
    );

    // limit 只取最近 N 条。
    let top1 = history::list(&store, Some(1)).unwrap();
    assert_eq!(top1.len(), 1);
    assert_eq!(top1[0].build_id, "build-3-c");
}

#[test]
fn 删除历史条目_同时删除构件目录与构建日志() {
    let dir = tempfile::tempdir().unwrap();
    let store = open_store(dir.path());
    let appdata = dir.path().join("appdata");
    let project = projects::import(&store, "demo", &dir.path(), ProjectType::Maven).unwrap();

    let artifacts_dir = appdata.join("artifacts/demo/20261103-120000");
    std::fs::create_dir_all(&artifacts_dir).unwrap();
    std::fs::write(artifacts_dir.join("app-1.0.jar"), b"jar").unwrap();
    let logs_dir = appdata.join("build-logs");
    std::fs::create_dir_all(&logs_dir).unwrap();
    let log_path = logs_dir.join("build-9.log");
    std::fs::write(&log_path, b"log").unwrap();

    let id = history::record(
        &store,
        &NewHistoryEntry {
            build_id: "build-9".into(),
            artifacts_dir: Some(artifacts_dir.to_string_lossy().into_owned()),
            log_path: log_path.to_string_lossy().into_owned(),
            ..new_entry(project.id, "build-9", true)
        },
    )
    .unwrap();

    history::delete_entry(&store, &appdata, id).unwrap();

    assert!(history::list(&store, None).unwrap().is_empty());
    assert!(!artifacts_dir.exists(), "构件目录必须随之删除");
    assert!(!log_path.exists(), "构建日志必须随之删除");
    // 空的项目目录一并收敛，不留孤儿目录。
    assert!(
        !dir.path().join("appdata/artifacts").exists(),
        "空的构件库根应被收敛"
    );
}

#[test]
fn 删除不存在的条目报明确错误() {
    let dir = tempfile::tempdir().unwrap();
    let store = open_store(dir.path());
    let err = history::delete_entry(&store, &dir.path(), 999).unwrap_err();
    assert_eq!(err.code(), "history_not_found");
}

#[test]
fn 保留策略_每项目只留最近N次_超出的连文件一起清理() {
    let dir = tempfile::tempdir().unwrap();
    let store = open_store(dir.path());
    let appdata = dir.path().join("appdata");
    let project = projects::import(&store, "demo", &dir.path(), ProjectType::Maven).unwrap();

    settings::set(&store, settings::BUILD_RETENTION_KEY, "2").unwrap();

    for i in 1..=3 {
        let artifacts_dir = appdata.join(format!("artifacts/demo/20261103-12000{i}"));
        std::fs::create_dir_all(&artifacts_dir).unwrap();
        std::fs::write(artifacts_dir.join("a.jar"), b"x").unwrap();
        let log_path = appdata.join(format!("build-logs/build-{i}.log"));
        std::fs::create_dir_all(log_path.parent().unwrap()).unwrap();
        std::fs::write(&log_path, b"log").unwrap();
        history::record(
            &store,
            &NewHistoryEntry {
                build_id: format!("build-{i}"),
                artifacts_dir: Some(artifacts_dir.to_string_lossy().into_owned()),
                log_path: log_path.to_string_lossy().into_owned(),
                ..new_entry(project.id, "build-x", true)
            },
        )
        .unwrap();
    }

    history::enforce_retention(&store, &appdata, project.id).unwrap();

    let rows = history::list(&store, None).unwrap();
    let build_ids: Vec<&str> = rows.iter().map(|r| r.build_id.as_str()).collect();
    assert_eq!(build_ids, vec!["build-3", "build-2"], "只留最近 2 次");
    assert!(!appdata.join("artifacts/demo/20261103-120001").exists());
    assert!(appdata.join("artifacts/demo/20261103-120002").exists());
    assert!(!appdata.join("build-logs/build-1.log").exists());
    assert!(appdata.join("build-logs/build-2.log").exists());
}

#[test]
fn 手动清理_清空项目全部历史与文件_不影响其他项目() {
    let dir = tempfile::tempdir().unwrap();
    let store = open_store(dir.path());
    let appdata = dir.path().join("appdata");
    let p1 = import_project(&store, dir.path(), "one");
    let p2 = import_project(&store, dir.path(), "two");

    for (name, p, bid) in [("one", &p1, "b1"), ("one", &p1, "b2"), ("two", &p2, "b3")] {
        let artifacts_dir = appdata.join(format!("artifacts/{name}/20261103-120000-{bid}"));
        std::fs::create_dir_all(&artifacts_dir).unwrap();
        history::record(
            &store,
            &NewHistoryEntry {
                build_id: bid.into(),
                artifacts_dir: Some(artifacts_dir.to_string_lossy().into_owned()),
                ..new_entry(p.id, bid, true)
            },
        )
        .unwrap();
    }

    history::clear_project(&store, &appdata, p1.id).unwrap();

    let rows = history::list(&store, None).unwrap();
    let build_ids: Vec<&str> = rows.iter().map(|r| r.build_id.as_str()).collect();
    assert_eq!(build_ids, vec!["b3"], "其他项目的历史不受影响");
    assert!(
        !appdata.join("artifacts/one").exists(),
        "被清理项目的构件目录应消失"
    );
    assert!(appdata.join("artifacts/two").exists());
}

#[test]
fn 删除项目时历史记录级联删除() {
    let dir = tempfile::tempdir().unwrap();
    let store = open_store(dir.path());
    let project = projects::import(&store, "gone", &dir.path(), ProjectType::Maven).unwrap();
    history::record(&store, &new_entry(project.id, "build-1", true)).unwrap();

    store
        .with(|conn| {
            conn.execute("DELETE FROM projects WHERE id = ?1", [project.id])
                .map(|_| ())
        })
        .unwrap();

    assert!(history::list(&store, None).unwrap().is_empty());
}

#[test]
fn 项目构件列表_列出库内文件_按文件名排序() {
    let dir = tempfile::tempdir().unwrap();
    let store = open_store(dir.path());
    let project = projects::import(&store, "demo", &dir.path(), ProjectType::Maven).unwrap();
    let artifacts_dir = dir.path().join("appdata/artifacts/demo/20261103-120000");
    std::fs::create_dir_all(&artifacts_dir).unwrap();
    std::fs::write(artifacts_dir.join("b.jar"), b"xx").unwrap();
    std::fs::write(artifacts_dir.join("a.war"), b"x").unwrap();

    let id = history::record(
        &store,
        &NewHistoryEntry {
            artifacts_dir: Some(artifacts_dir.to_string_lossy().into_owned()),
            ..new_entry(project.id, "build-1", true)
        },
    )
    .unwrap();
    let _ = id;
    let entry = &history::list(&store, None).unwrap()[0];

    let files = history::list_artifacts(entry).unwrap();
    let names: Vec<&str> = files.iter().map(|f| f.file_name.as_str()).collect();
    assert_eq!(names, vec!["a.war", "b.jar"]);
    assert_eq!(files[0].size, 1);
}

// ---------------------------------------------------------------------------
// 设置（保留次数 N，存 app_meta）
// ---------------------------------------------------------------------------

#[test]
fn 保留次数设置_默认10_可写回() {
    let dir = tempfile::tempdir().unwrap();
    let store = open_store(dir.path());

    assert_eq!(settings::get_build_retention(&store).unwrap(), 10);
    settings::set_build_retention(&store, 3).unwrap();
    assert_eq!(settings::get_build_retention(&store).unwrap(), 3);
    // 非法值被拒绝，设置不变。
    assert!(settings::set_build_retention(&store, 0).is_err());
    assert!(settings::set_build_retention(&store, -1).is_err());
    assert_eq!(settings::get_build_retention(&store).unwrap(), 3);
    // 损坏值回退默认。
    settings::set(&store, settings::BUILD_RETENTION_KEY, "not-a-number").unwrap();
    assert_eq!(settings::get_build_retention(&store).unwrap(), 10);
}

// ---------------------------------------------------------------------------
// 项目默认构件列表（迁移 v13：project_default_artifacts，票 21）
// ---------------------------------------------------------------------------

#[test]
fn 默认构件列表_按保存顺序持久化并可整组替换() {
    let dir = tempfile::tempdir().unwrap();
    let store = open_store(dir.path());
    let project = projects::import(&store, "demo", &dir.path(), ProjectType::Maven).unwrap();

    // 从未保存过 = 空列表（与旧"未保存默认构件"一致）。
    assert!(project.default_artifacts.is_empty());

    projects::set_default_artifacts(
        &store,
        project.id,
        &["b-web-1.0.war".to_string(), "a-api-1.0.jar".to_string()],
    )
    .unwrap();
    assert_eq!(
        projects::get(&store, project.id)
            .unwrap()
            .unwrap()
            .default_artifacts,
        vec![
            "b-web-1.0.war".to_string(),
            "a-api-1.0.jar".to_string()
        ],
        "列表顺序即保存（勾选）顺序"
    );

    // 整组替换：第二次保存覆盖第一次。
    projects::set_default_artifacts(&store, project.id, &["only.jar".to_string()]).unwrap();
    assert_eq!(
        projects::get(&store, project.id)
            .unwrap()
            .unwrap()
            .default_artifacts,
        vec!["only.jar".to_string()]
    );
}

#[test]
fn 默认构件列表_空列表即清空_空名拒绝() {
    let dir = tempfile::tempdir().unwrap();
    let store = open_store(dir.path());
    let project = projects::import(&store, "demo", &dir.path(), ProjectType::Maven).unwrap();

    projects::set_default_artifacts(&store, project.id, &["app.jar".to_string()]).unwrap();
    // 空列表 = 清除选择（回到"未设置"）。
    projects::set_default_artifacts(&store, project.id, &[]).unwrap();
    assert!(
        projects::get(&store, project.id)
            .unwrap()
            .unwrap()
            .default_artifacts
            .is_empty()
    );

    // 空名/纯空白构件名拒绝。
    for bad in ["", "   "] {
        let err = projects::set_default_artifacts(&store, project.id, &[bad.to_string()]).unwrap_err();
        assert!(matches!(err, projects::ProjectError::InvalidArtifact(_)), "case {bad:?}");
    }
    // 列表中混入空名同样拒绝，且不清空既有列表。
    projects::set_default_artifacts(&store, project.id, &["keep.jar".to_string()]).unwrap();
    assert!(
        projects::set_default_artifacts(&store, project.id, &["ok.jar".to_string(), "".to_string()])
            .is_err()
    );
    assert_eq!(
        projects::get(&store, project.id)
            .unwrap()
            .unwrap()
            .default_artifacts,
        vec!["keep.jar".to_string()],
        "被拒绝的保存不得破坏既有列表"
    );
}

/// 迁移 v13：既有非空单值 `projects.default_artifact` 回填为单元素列表；
/// 空白/NULL 不回填。原列保留但废弃（与票 19 的 `upload_dir` 同法）。
#[test]
fn migration_v13_backfills_non_empty_default_artifact_as_one_element_list() {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("test.db");
    fs::create_dir(dir.path().join("a")).unwrap();
    fs::create_dir(dir.path().join("b")).unwrap();
    {
        let store = Store::open(&db, TEST_KEY_HEX).unwrap();
        let with = projects::import(&store, "with-default", &dir.path().join("a"), ProjectType::Maven)
            .unwrap();
        let without =
            projects::import(&store, "without-default", &dir.path().join("b"), ProjectType::Maven)
                .unwrap();
        // 模拟旧库：降回 v12，删掉 v13 子表，直接写旧单值列。
        store
            .with(|conn| {
                conn.execute_batch(&format!(
                    "PRAGMA user_version = 12;
                     DROP TABLE project_default_artifacts;
                     UPDATE projects SET default_artifact = 'app-1.0.jar' WHERE id = {};
                     UPDATE projects SET default_artifact = '   ' WHERE id = {};",
                    with.id, without.id
                ))
            })
            .unwrap();
    }

    let store = Store::open(&db, TEST_KEY_HEX).unwrap();
    let all = projects::list(&store).unwrap();
    let with = all.iter().find(|p| p.name == "with-default").unwrap();
    let without = all.iter().find(|p| p.name == "without-default").unwrap();
    assert_eq!(
        with.default_artifacts,
        vec!["app-1.0.jar".to_string()],
        "非空旧单值回填为单元素列表"
    );
    assert!(
        without.default_artifacts.is_empty(),
        "空白旧单值不回填（= 未设置）"
    );
}

// ---------------------------------------------------------------------------
// 打包流程集成：构件入库（先于临时目录删除）+ 历史记录 + 保留策略
// ---------------------------------------------------------------------------

use packsmith_lib::build::{self, BuildEvent};
use std::sync::{Arc, Mutex};

/// 本地 git 仓库（项目源）+ 已注册项目。
fn fixture_project(
    dir: &Path,
    store: &Store,
    name: &str,
) -> packsmith_lib::store::projects::Project {
    let repo_path = dir.join(name);
    let repo = git2::Repository::init(&repo_path).unwrap();
    std::fs::write(repo_path.join("pom.xml"), "<project/>").unwrap();
    let mut index = repo.index().unwrap();
    index.add_path(Path::new("pom.xml")).unwrap();
    index.write().unwrap();
    let tree = repo.find_tree(index.write_tree().unwrap()).unwrap();
    let sig = git2::Signature::now("fixture", "fixture@example.com").unwrap();
    repo.commit(Some("refs/heads/main"), &sig, &sig, "initial", &tree, &[])
        .unwrap();
    projects::import(store, name, &repo_path, ProjectType::Maven).unwrap()
}

/// 假托管工具链：mvnd 是一个 shell 脚本（`code` 为退出码），外加 JDK 目录布局。
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

/// 构建成功的假 mvnd：在 target/ 下产出 jar；支持 `-Denvfile=<path>` 导出环境。
#[cfg(unix)]
fn success_script(jars: &[&str]) -> String {
    let mut s = String::from("#!/bin/sh\nmkdir -p target\n");
    for jar in jars {
        s.push_str(&format!("echo jar-content-{jar} > target/{jar}\n"));
    }
    s.push_str(
        "for a in \"$@\"; do case \"$a\" in -Denvfile=*) { env; echo \"ARGS: $*\"; } > \"${a#-Denvfile=}\" ;; esac; done\nexit 0\n",
    );
    s
}

#[cfg(unix)]
fn failure_script() -> String {
    "#!/bin/sh\necho boom >&2\nexit 3\n".to_string()
}

struct FlowFixture {
    _dir: tempfile::TempDir,
    store: Store,
    project_id: i64,
    temp_root: PathBuf,
    appdata: PathBuf,
    tools: PathBuf,
}

#[cfg(unix)]
fn flow_setup(script: &str, jars: &[&str]) -> FlowFixture {
    let dir = tempfile::tempdir().unwrap();
    let store = open_store(dir.path());
    let project = fixture_project(dir.path(), &store, "demo-app");
    let tools = fake_toolchain(dir.path(), &success_script(jars));
    // 覆盖为指定的成败脚本。
    let mvnd = tools.join("mvnd/bin/mvnd");
    std::fs::write(&mvnd, script).unwrap();
    let temp_root = dir.path().join("tmp");
    let appdata = dir.path().join("appdata");
    std::fs::create_dir_all(&temp_root).unwrap();
    std::fs::write(tools.join("jdk/Contents/Home/bin/java"), "not a real jvm").unwrap();
    FlowFixture {
        _dir: dir,
        store,
        project_id: project.id,
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
                .filter(|p| {
                    p.file_name()
                        .unwrap()
                        .to_string_lossy()
                        .starts_with("packsmith-build-")
                })
                .collect()
        })
        .unwrap_or_default()
}

async fn run_flow(fx: &FlowFixture) -> Result<build::BuildOutcome, build::BuildError> {
    let (events, on_event) = {
        let events: Arc<Mutex<Vec<BuildEvent>>> = Arc::new(Mutex::new(Vec::new()));
        let sink = events.clone();
        (
            events,
            Arc::new(move |e: BuildEvent| sink.lock().unwrap().push(e))
                as Arc<dyn Fn(BuildEvent) + Send + Sync>,
        )
    };
    let _ = events;
    build::run_build_flow_in(
        fx.store.clone(),
        &fx.tools,
        &fx.appdata,
        &fx.temp_root,
        fx.project_id,
        "main",
        projects::new_cancel_flag(),
        on_event,
    )
    .await
}

#[cfg(unix)]
#[tokio::test]
async fn 成功构建_构件先入库后删工作区_历史落记录_单构件自动成为默认() {
    let fx = flow_setup(&success_script(&["app-1.0.jar"]), &["app-1.0.jar"]);

    let outcome = run_flow(&fx).await.expect("build succeeds");
    assert!(outcome.success);

    // 构件已入库：`<appdata>/artifacts/<项目>/<构建时间>/`，内容与 target 一致。
    let lib_dir = PathBuf::from(
        outcome
            .artifacts_dir
            .as_ref()
            .expect("成功构建必须有构件目录"),
    );
    assert!(
        lib_dir.starts_with(fx.appdata.join("artifacts/demo-app")),
        "库目录应在 artifacts/<项目>/ 下: {lib_dir:?}"
    );
    let jar = lib_dir.join("app-1.0.jar");
    let jar_content = String::from_utf8_lossy(&std::fs::read(&jar).unwrap()).into_owned();
    assert_eq!(jar_content.trim_end(), "jar-content-app-1.0.jar");

    // 临时工作区在入库之后才删除。
    assert!(workspaces_in(&fx.temp_root).is_empty());

    // 历史落记录：成功、分支、日志、构件目录。
    let rows = history::list(&fx.store, None).unwrap();
    assert_eq!(rows.len(), 1);
    let row = &rows[0];
    assert!(row.success);
    assert_eq!(row.branch, "main");
    assert_eq!(
        row.artifacts_dir.as_deref(),
        Some(lib_dir.to_str().unwrap())
    );
    assert!(Path::new(&row.log_path).is_file());
    assert_eq!(outcome.history_id, Some(row.id));

    // 单构件自动选中并被记住为项目默认（单元素列表，票 21）。
    assert_eq!(
        projects::get(&fx.store, fx.project_id)
            .unwrap()
            .unwrap()
            .default_artifacts,
        vec!["app-1.0.jar".to_string()]
    );
}

#[cfg(unix)]
#[tokio::test]
async fn 多构件构建_全部入库但不自动改默认() {
    let fx = flow_setup(
        &success_script(&["a-api-1.0.jar", "b-web-1.0.war"]),
        &["a-api-1.0.jar", "b-web-1.0.war"],
    );

    let outcome = run_flow(&fx).await.expect("build succeeds");

    assert_eq!(outcome.artifacts.len(), 2);
    let dir = PathBuf::from(outcome.artifacts_dir.as_ref().unwrap());
    assert!(dir.join("a-api-1.0.jar").is_file());
    assert!(dir.join("b-web-1.0.war").is_file());
    assert!(workspaces_in(&fx.temp_root).is_empty());

    // 多构件时选择权在用户：不自动改默认。
    assert!(
        projects::get(&fx.store, fx.project_id)
            .unwrap()
            .unwrap()
            .default_artifacts
            .is_empty()
    );
}

#[cfg(unix)]
#[tokio::test]
async fn 失败构建_历史落失败记录_无构件目录() {
    let fx = flow_setup(&failure_script(), &[]);

    let outcome = run_flow(&fx).await.expect("failed build still completes");
    assert!(!outcome.success);

    assert!(outcome.artifacts.is_empty());
    assert!(outcome.artifacts_dir.is_none());
    let rows = history::list(&fx.store, None).unwrap();
    assert_eq!(rows.len(), 1);
    assert!(!rows[0].success);
    assert_eq!(rows[0].exit_code, Some(3));
    assert!(rows[0].artifacts_dir.is_none());
    assert!(
        Path::new(&rows[0].log_path).is_file(),
        "失败构建的日志仍留存"
    );
    assert!(workspaces_in(&fx.temp_root).is_empty());
    // 失败构建不留构件目录。
    assert!(
        !fx.appdata.join("artifacts").exists()
            || fx
                .appdata
                .join("artifacts")
                .read_dir()
                .unwrap()
                .next()
                .is_none()
    );
}

#[cfg(unix)]
#[tokio::test]
async fn 保留策略在入库后执行_超出N次的旧构建连文件清理() {
    let fx = flow_setup(&success_script(&["app-1.0.jar"]), &["app-1.0.jar"]);
    settings::set_build_retention(&fx.store, 1).unwrap();

    let first = run_flow(&fx).await.unwrap();
    let second = run_flow(&fx).await.unwrap();
    assert_ne!(first.artifacts_dir, second.artifacts_dir);

    let rows = history::list(&fx.store, None).unwrap();
    let build_ids: Vec<&str> = rows.iter().map(|r| r.build_id.as_str()).collect();
    assert_eq!(build_ids.len(), 1, "保留 1 次：只剩最近一次");
    assert_eq!(rows[0].build_id, second.build_id);

    assert!(
        !PathBuf::from(first.artifacts_dir.unwrap()).exists(),
        "旧构件目录被清理"
    );
    assert!(!Path::new(first.log_path.as_str()).exists(), "旧日志被清理");
    assert!(
        PathBuf::from(second.artifacts_dir.unwrap())
            .join("app-1.0.jar")
            .is_file()
    );
    assert!(workspaces_in(&fx.temp_root).is_empty());
}

#[cfg(unix)]
#[tokio::test]
async fn 构建进程环境_依赖仓库指向应用数据目录_用户m2零污染() {
    let fx = flow_setup(&success_script(&["app-1.0.jar"]), &["app-1.0.jar"]);
    let env_file = fx._dir.path().join("env.txt");
    projects::set_build_command(
        &fx.store,
        fx.project_id,
        &format!("mvnd package -Denvfile={}", env_file.display()),
    )
    .unwrap();

    // 用户 ~/.m2 顶层两层快照（打包前后内容不变，ADR-0001 验收）。
    fn snapshot(root: &Path, depth: usize, out: &mut Vec<PathBuf>) {
        let Ok(entries) = std::fs::read_dir(root) else {
            return;
        };
        for e in entries.flatten() {
            out.push(e.path());
            if depth > 0 && e.file_type().map(|t| t.is_dir()).unwrap_or(false) {
                snapshot(&e.path(), depth - 1, out);
            }
        }
    }
    let m2 = std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".m2"));
    let mut before = Vec::new();
    if let Some(m2) = &m2 {
        snapshot(m2, 2, &mut before);
    }

    let outcome = run_flow(&fx).await.unwrap();
    assert!(outcome.success);

    let env = std::fs::read_to_string(&env_file).unwrap();
    // 构建命令统一追加依赖仓库参数，指向 <appdata>/m2-repo（非 ~/.m2）。
    let repo_flag = env
        .lines()
        .find(|l| l.starts_with("MVN_") && l.contains("maven.repo.local"))
        .or_else(|| env.lines().find(|l| l.contains("maven.repo.local=")))
        .expect("构建进程环境中必须出现 -Dmaven.repo.local")
        .to_string();
    assert!(
        repo_flag.contains(fx.appdata.join("m2-repo").to_string_lossy().as_ref()),
        "依赖仓库必须指向应用数据目录: {repo_flag}"
    );
    assert!(
        !repo_flag.contains("/.m2") || repo_flag.contains(fx.appdata.to_string_lossy().as_ref()),
        "依赖仓库不得指向用户 ~/.m2: {repo_flag}"
    );

    // JAVA_HOME 指向托管 JDK（执行只用托管工具链）。
    let java_home = env
        .lines()
        .find(|l| l.starts_with("JAVA_HOME="))
        .expect("JAVA_HOME must be injected");
    assert!(
        java_home.starts_with(&format!("JAVA_HOME={}", fx.tools.join("jdk").display())),
        "JAVA_HOME 必须指向托管 JDK: {java_home}"
    );

    // 打包前后用户 ~/.m2 内容不变。
    if let Some(m2) = &m2 {
        let mut after = Vec::new();
        snapshot(m2, 2, &mut after);
        let mut b = before.clone();
        let mut a = after.clone();
        b.sort();
        a.sort();
        assert_eq!(b, a, "用户 ~/.m2 在打包前后必须完全不变（ADR-0001）");
    }
}
