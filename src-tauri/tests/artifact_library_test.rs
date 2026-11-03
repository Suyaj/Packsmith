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
