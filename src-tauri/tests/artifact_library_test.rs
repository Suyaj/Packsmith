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
//! - `projects::set_default_artifact` — the remembered default artifact
//!   selection of a project (migration v7).
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
    write_file(&ws.path().join("src/lib/not-a-build-output.jar"), b"src jar");

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
        vec!["module-a/target/a-api-2.0.jar", "module-b/target/b-web-2.0.war"]
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

use packsmith_lib::store::history::{self, NewHistoryEntry};
use packsmith_lib::store::projects::{self, ProjectType};
use packsmith_lib::store::settings;
use packsmith_lib::store::Store;

const TEST_KEY_HEX: &str = "a1b2c3d4e5f60718293a4b5c6d7e8f90a1b2c3d4e5f60718293a4b5c6d7e8f90";

fn open_store(dir: &Path) -> Store {
    Store::open(&dir.join("test.db"), TEST_KEY_HEX).unwrap()
}

/// 导入一个项目；本地路径必须存在且各不相同，用 `<dir>/<name>`。
fn import_project(store: &Store, dir: &Path, name: &str) -> packsmith_lib::store::projects::Project {
    let path = dir.join(name);
    std::fs::create_dir_all(&path).unwrap();
    projects::import(store, name, &path, ProjectType::Maven).unwrap()
}

fn new_entry(project_id: i64, build_id: &str, success: bool) -> NewHistoryEntry {    NewHistoryEntry {
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
    assert!(first.created_at.contains("T"), "created_at: {}", first.created_at);

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

    for (name, p, bid) in [
        ("one", &p1, "b1"),
        ("one", &p1, "b2"),
        ("two", &p2, "b3"),
    ] {
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
    assert!(!appdata.join("artifacts/one").exists(), "被清理项目的构件目录应消失");
    assert!(appdata.join("artifacts/two").exists());
}

#[test]
fn 删除项目时历史记录级联删除() {
    let dir = tempfile::tempdir().unwrap();
    let store = open_store(dir.path());
    let project = projects::import(&store, "gone", &dir.path(), ProjectType::Maven).unwrap();
    history::record(&store, &new_entry(project.id, "build-1", true)).unwrap();

    store
        .with(|conn| conn.execute("DELETE FROM projects WHERE id = ?1", [project.id]).map(|_| ()))
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
// 项目默认构件选择（迁移 v7：projects.default_artifact）
// ---------------------------------------------------------------------------

#[test]
fn 默认构件选择_被记住并可更新() {
    let dir = tempfile::tempdir().unwrap();
    let store = open_store(dir.path());
    let project = projects::import(&store, "demo", &dir.path(), ProjectType::Maven).unwrap();

    assert!(project.default_artifact.is_none());
    projects::set_default_artifact(&store, project.id, "app-1.0.jar").unwrap();
    assert_eq!(
        projects::get(&store, project.id).unwrap().unwrap().default_artifact.as_deref(),
        Some("app-1.0.jar")
    );
    projects::set_default_artifact(&store, project.id, "other.war").unwrap();
    assert_eq!(
        projects::get(&store, project.id).unwrap().unwrap().default_artifact.as_deref(),
        Some("other.war")
    );
}
