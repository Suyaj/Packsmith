//! Integration tests for 部署目标 (Deployment Target) CRUD (ticket 09).
//!
//! Seams under test:
//! - `targets::repo::{list_for_project, create, update, delete}` — persistence
//!   over `&Store` (same seam shape as `server::repo`)
//! - `targets::repo::split_command_group` — 命令组 line handling (trim, blank-line
//!   removal, order preservation)
//! - the schema created by migration v4 (`deployment_targets` table) and its
//!   foreign keys (project × server integrity)
//!
//! Nothing here touches SSH/SFTP: execution belongs to ticket 10.

use packsmith_lib::store::Store;
use packsmith_lib::targets::repo::{self, NewTarget, Target};

const TEST_KEY_HEX: &str = "a1b2c3d4e5f60718293a4b5c6d7e8f90a1b2c3d4e5f60718293a4b5c6d7e8f90";

fn open_store() -> (tempfile::TempDir, Store) {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("test.db");
    let store = Store::open(&db, TEST_KEY_HEX).unwrap();
    (dir, store)
}

fn seed_server(store: &Store, host: &str) -> i64 {
    let server = packsmith_lib::server::repo::create(
        store,
        &packsmith_lib::server::repo::NewServer {
            host: host.to_string(),
            port: 22,
            username: "deploy".to_string(),
            credential: packsmith_lib::server::repo::Credential::Password {
                password: "s3cret".to_string(),
            },
        },
    )
    .unwrap();
    server.id
}

fn seed_project(store: &Store, name: &str) -> i64 {
    store
        .with(|conn| {
            conn.execute(
                "INSERT INTO projects (name, source, url, path, project_type)
                 VALUES (?1, 'git', ?2, ?3, 'maven')",
                rusqlite::params![
                    name,
                    format!("https://example.com/{name}.git"),
                    format!("/tmp/{name}")
                ],
            )?;
            Ok(conn.last_insert_rowid())
        })
        .unwrap()
}

fn new_target(project_id: i64, server_id: i64) -> NewTarget {
    NewTarget {
        project_id,
        server_id,
        start_commands: "cd /home/app\nsh start.sh start".to_string(),
        stop_commands: "sh start.sh stop".to_string(),
    }
}

#[test]
fn migration_creates_deployment_targets_table() {
    let (_dir, store) = open_store();

    let count: i64 = store
        .with(|conn| {
            conn.query_row(
                "SELECT count(*) FROM sqlite_master WHERE type='table'
                 AND name = 'deployment_targets'",
                [],
                |r| r.get(0),
            )
        })
        .unwrap();
    assert_eq!(
        count, 1,
        "migration must create the deployment_targets table"
    );
}

#[test]
fn created_target_round_trips_all_fields() {
    let (_dir, store) = open_store();
    let project = seed_project(&store, "app");
    let server = seed_server(&store, "web-1");

    let created = repo::create(&store, &new_target(project, server)).unwrap();

    assert!(created.id > 0);
    let listed = repo::list_for_project(&store, project).unwrap();
    assert_eq!(listed.len(), 1);
    assert_eq!(
        listed[0],
        Target {
            id: created.id,
            project_id: project,
            server_id: server,
            // 废弃列（ticket 19）：新目标不再写上传目录，映射行才是数据。
            upload_dir: String::new(),
            start_commands: "cd /home/app\nsh start.sh start".to_string(),
            stop_commands: "sh start.sh stop".to_string(),
            auto_deploy: false,
            last_artifact: None,
        }
    );
}

#[test]
fn command_groups_are_normalized_but_order_preserved() {
    let (_dir, store) = open_store();
    let project = seed_project(&store, "app");
    let server = seed_server(&store, "web-1");

    let target = repo::create(
        &store,
        &NewTarget {
            project_id: project,
            server_id: server,
            // Blank lines dropped, surrounding whitespace trimmed, order kept.
            start_commands: "\n  cd /home/app  \n\n   \nsh start.sh start\n".to_string(),
            stop_commands: "  \n".to_string(),
        },
    )
    .unwrap();

    assert_eq!(target.start_commands, "cd /home/app\nsh start.sh start");
    assert_eq!(target.stop_commands, "");
}

#[test]
fn split_commands_splits_trims_and_drops_blank_lines_in_order() {
    assert_eq!(
        repo::split_command_group("cd /home/app\nsh start.sh start"),
        vec!["cd /home/app".to_string(), "sh start.sh start".to_string()]
    );
    assert_eq!(
        repo::split_command_group("  a \n\n  b  \n\t\nc"),
        vec!["a".to_string(), "b".to_string(), "c".to_string()]
    );
    assert_eq!(repo::split_command_group("   \n\t "), Vec::<String>::new());
    assert_eq!(repo::split_command_group(""), Vec::<String>::new());
}

#[test]
fn one_project_can_bind_multiple_servers_and_a_server_multiple_projects() {
    let (_dir, store) = open_store();
    let project_a = seed_project(&store, "app-a");
    let project_b = seed_project(&store, "app-b");
    let server_1 = seed_server(&store, "web-1");
    let server_2 = seed_server(&store, "web-2");

    repo::create(&store, &new_target(project_a, server_1)).unwrap();
    repo::create(&store, &new_target(project_a, server_2)).unwrap();
    repo::create(&store, &new_target(project_b, server_1)).unwrap();

    let for_a = repo::list_for_project(&store, project_a).unwrap();
    assert_eq!(for_a.len(), 2, "one project may bind several servers");
    let a_server_ids: Vec<i64> = for_a.iter().map(|t| t.server_id).collect();
    assert_eq!(a_server_ids, vec![server_1, server_2], "oldest first");

    let for_b = repo::list_for_project(&store, project_b).unwrap();
    assert_eq!(
        for_b.len(),
        1,
        "one server may be bound by several projects"
    );
    assert_eq!(for_b[0].server_id, server_1);
}

#[test]
fn update_replaces_fields_and_keeps_id() {
    let (_dir, store) = open_store();
    let project = seed_project(&store, "app");
    let server_1 = seed_server(&store, "web-1");
    let server_2 = seed_server(&store, "web-2");
    let created = repo::create(&store, &new_target(project, server_1)).unwrap();

    let updated = repo::update(
        &store,
        &Target {
            id: created.id,
            project_id: project,
            server_id: server_2,
            upload_dir: "/srv/app2".to_string(),
            start_commands: "sh run.sh".to_string(),
            stop_commands: "".to_string(),
            auto_deploy: false,
            last_artifact: None,
        },
    )
    .unwrap();

    assert_eq!(updated.id, created.id);
    let listed = repo::list_for_project(&store, project).unwrap();
    assert_eq!(listed.len(), 1, "update must not create a new row");
    assert_eq!(listed[0].server_id, server_2);
    assert_eq!(listed[0].upload_dir, "/srv/app2");
    assert_eq!(listed[0].start_commands, "sh run.sh");
}

#[test]
fn update_unknown_id_fails() {
    let (_dir, store) = open_store();
    let result = repo::update(
        &store,
        &Target {
            id: 4242,
            project_id: 1,
            server_id: 1,
            upload_dir: "/x".to_string(),
            start_commands: String::new(),
            stop_commands: String::new(),
            auto_deploy: false,
            last_artifact: None,
        },
    );
    assert!(result.is_err());
}

#[test]
fn delete_removes_the_row_and_unknown_id_returns_false() {
    let (_dir, store) = open_store();
    let project = seed_project(&store, "app");
    let server = seed_server(&store, "web-1");
    let created = repo::create(&store, &new_target(project, server)).unwrap();

    assert!(repo::delete(&store, created.id).unwrap());
    assert!(repo::list_for_project(&store, project).unwrap().is_empty());
    assert!(!repo::delete(&store, created.id).unwrap());
}

#[test]
fn binding_a_missing_project_or_server_is_rejected() {
    let (_dir, store) = open_store();
    let project = seed_project(&store, "app");
    let server = seed_server(&store, "web-1");

    let no_project = repo::create(&store, &new_target(99_999, server)).unwrap_err();
    assert!(
        matches!(no_project, repo::RepoError::MissingProject),
        "got {no_project:?}"
    );
    let no_server = repo::create(&store, &new_target(project, 99_999)).unwrap_err();
    assert!(
        matches!(no_server, repo::RepoError::MissingServer),
        "got {no_server:?}"
    );
}

#[test]
fn the_same_project_server_pair_cannot_be_bound_twice() {
    let (_dir, store) = open_store();
    let project = seed_project(&store, "app");
    let server = seed_server(&store, "web-1");
    repo::create(&store, &new_target(project, server)).unwrap();

    let err = repo::create(&store, &new_target(project, server)).unwrap_err();
    assert!(
        matches!(err, repo::RepoError::DuplicateBinding),
        "got {err:?}"
    );
}

/// Documented referential-integrity choice: deleting the referenced project or
/// server CASCADES to deleting its deployment targets (bindings are dependent
/// rows; the alternative was blocking deletion until unbound).
#[test]
fn deleting_the_server_cascades_to_its_deployment_targets() {
    let (_dir, store) = open_store();
    let project = seed_project(&store, "app");
    let server = seed_server(&store, "web-1");
    repo::create(&store, &new_target(project, server)).unwrap();

    packsmith_lib::server::repo::delete(&store, server).unwrap();

    assert!(repo::list_for_project(&store, project).unwrap().is_empty());
}

#[test]
fn deleting_the_project_cascades_to_its_deployment_targets() {
    let (_dir, store) = open_store();
    let project = seed_project(&store, "app");
    let server = seed_server(&store, "web-1");
    repo::create(&store, &new_target(project, server)).unwrap();

    store
        .with(|conn| conn.execute("DELETE FROM projects WHERE id = ?1", [project]))
        .unwrap();

    let orphans: i64 = store
        .with(|conn| conn.query_row("SELECT count(*) FROM deployment_targets", [], |r| r.get(0)))
        .unwrap();
    assert_eq!(orphans, 0, "project deletion must cascade to its targets");
}

#[test]
fn targets_survive_an_app_restart() {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("test.db");

    let store = Store::open(&db, TEST_KEY_HEX).unwrap();
    let project = seed_project(&store, "app");
    let server = seed_server(&store, "web-1");
    let created = repo::create(&store, &new_target(project, server)).unwrap();
    drop(store);

    let store = Store::open(&db, TEST_KEY_HEX).unwrap();
    let listed = repo::list_for_project(&store, project).unwrap();
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].id, created.id);
    assert_eq!(listed[0].start_commands, "cd /home/app\nsh start.sh start");
}

// ---------------------------------------------------------------------------
// 构建成功后自动连锁部署 (auto-deploy toggle, ticket 10)
// ---------------------------------------------------------------------------

/// The auto-chain toggle defaults to OFF (spec: 默认关).
#[test]
fn auto_deploy_defaults_to_off() {
    let (_dir, store) = open_store();
    let project = seed_project(&store, "app");
    let server = seed_server(&store, "web-1");
    let target = repo::create(&store, &new_target(project, server)).unwrap();
    assert!(!target.auto_deploy);
    assert!(!repo::list_for_project(&store, project).unwrap()[0].auto_deploy);
}

#[test]
fn auto_deploy_can_be_toggled_on_and_off() {
    let (_dir, store) = open_store();
    let project = seed_project(&store, "app");
    let server = seed_server(&store, "web-1");
    let target = repo::create(&store, &new_target(project, server)).unwrap();

    assert!(repo::set_auto_deploy(&store, target.id, true).unwrap());
    let listed = repo::list_for_project(&store, project).unwrap();
    assert!(listed[0].auto_deploy);

    assert!(repo::set_auto_deploy(&store, target.id, false).unwrap());
    assert!(!repo::list_for_project(&store, project).unwrap()[0].auto_deploy);
}

/// Only targets with the toggle ON are picked up for auto-chain deployment.
#[test]
fn auto_deploy_enabled_targets_lists_only_toggled_targets_of_the_project() {
    let (_dir, store) = open_store();
    let project_a = seed_project(&store, "app-a");
    let project_b = seed_project(&store, "app-b");
    let s1 = seed_server(&store, "web-1");
    let s2 = seed_server(&store, "web-2");
    let a1 = repo::create(&store, &new_target(project_a, s1)).unwrap();
    let _a2 = repo::create(&store, &new_target(project_a, s2)).unwrap();
    let b1 = repo::create(&store, &new_target(project_b, s1)).unwrap();

    repo::set_auto_deploy(&store, a1.id, true).unwrap();
    repo::set_auto_deploy(&store, b1.id, true).unwrap();

    let enabled = repo::auto_deploy_enabled(&store, project_a).unwrap();
    assert_eq!(enabled, vec![a1.id]);
}

#[test]
fn set_auto_deploy_reports_missing_target() {
    let (_dir, store) = open_store();
    assert!(!repo::set_auto_deploy(&store, 999, true).unwrap());
}

/// update() replaces all fields and must preserve the toggle round-trip.
#[test]
fn update_preserves_and_sets_auto_deploy() {
    let (_dir, store) = open_store();
    let project = seed_project(&store, "app");
    let server = seed_server(&store, "web-1");
    let target = repo::create(&store, &new_target(project, server)).unwrap();

    let mut updated = target.clone();
    updated.auto_deploy = true;
    let updated = repo::update(&store, &updated).unwrap();
    assert!(updated.auto_deploy);
    assert!(repo::list_for_project(&store, project).unwrap()[0].auto_deploy);
}

#[test]
fn get_returns_one_target_by_id_across_projects() {
    let (_dir, store) = open_store();
    let project_a = seed_project(&store, "app-a");
    let server = seed_server(&store, "web-1");
    let created = repo::create(&store, &new_target(project_a, server)).unwrap();
    let fetched = repo::get(&store, created.id)
        .unwrap()
        .expect("target exists");
    assert_eq!(fetched.project_id, project_a);
    assert!(repo::get(&store, 999).unwrap().is_none());
}

// ---------------------------------------------------------------------------
// 文件名→上传目录映射行 (ticket 19)
// ---------------------------------------------------------------------------

use packsmith_lib::targets::repo::NewTargetFile;

fn file_row(file_name: &str, upload_dir: &str) -> NewTargetFile {
    NewTargetFile {
        file_name: file_name.to_string(),
        upload_dir: upload_dir.to_string(),
    }
}

/// 旧数据迁移（migration v12）：每个既有目标回填一行
/// {file_name: '', upload_dir: 旧目录}；空文件名 = 部署时补选。
/// 通过把库降级回 v11（删掉子表 + 写入旧形态目标行）再重开模拟升级路径。
#[test]
fn migration_v12_backfills_one_unnamed_row_per_existing_target() {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("test.db");

    {
        let store = Store::open(&db, TEST_KEY_HEX).unwrap();
        let project = seed_project(&store, "app");
        let server = seed_server(&store, "web-1");
        store
            .with(|conn| {
                conn.execute_batch(&format!(
                    "PRAGMA user_version = 11;
                     DROP TABLE deployment_target_files;
                     INSERT INTO deployment_targets (project_id, server_id, upload_dir)
                         VALUES ({project}, {server}, '/legacy/dir');"
                ))
            })
            .unwrap();
    }

    let store = Store::open(&db, TEST_KEY_HEX).unwrap();
    let rows = repo::list_files(&store, 1).unwrap();
    assert_eq!(rows.len(), 1, "legacy target must gain exactly one row");
    assert_eq!(rows[0].file_name, "", "legacy row is a 部署时补选 row");
    assert_eq!(rows[0].upload_dir, "/legacy/dir");
    assert_eq!(rows[0].position, 0);
}

#[test]
fn replace_files_round_trips_rows_in_order() {
    let (_dir, store) = open_store();
    let project = seed_project(&store, "app");
    let server = seed_server(&store, "web-1");
    let target = repo::create(&store, &new_target(project, server)).unwrap();

    let rows = repo::replace_files(
        &store,
        target.id,
        &[
            file_row("app-1.0.jar", "/srv/app"),
            file_row("", "/srv/pick-later"),
            file_row("config.properties", "/srv/etc"),
        ],
    )
    .unwrap();

    assert_eq!(
        rows.iter().map(|r| r.file_name.as_str()).collect::<Vec<_>>(),
        vec!["app-1.0.jar", "", "config.properties"]
    );
    assert_eq!(
        rows.iter().map(|r| r.upload_dir.as_str()).collect::<Vec<_>>(),
        vec!["/srv/app", "/srv/pick-later", "/srv/etc"]
    );
    assert!(rows.iter().enumerate().all(|(i, r)| r.position == i as i64));

    // 整组替换：第二次保存覆盖第一次。
    let rows = repo::replace_files(&store, target.id, &[file_row("only.jar", "/srv/x")]).unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(repo::list_files(&store, target.id).unwrap().len(), 1);

    // files_for_targets 按目标分组带出。
    let all = repo::files_for_targets(&store, &[target.id]).unwrap();
    assert_eq!(all.len(), 1);
    assert_eq!(all[0].target_id, target.id);
}

#[test]
fn empty_file_names_may_repeat_but_named_duplicates_are_rejected() {
    let (_dir, store) = open_store();
    let project = seed_project(&store, "app");
    let server = seed_server(&store, "web-1");
    let target = repo::create(&store, &new_target(project, server)).unwrap();

    // 两个空名行（部署时补选）允许共存。
    repo::replace_files(
        &store,
        target.id,
        &[file_row("", "/srv/a"), file_row("", "/srv/b")],
    )
    .unwrap();

    let err = repo::replace_files(
        &store,
        target.id,
        &[file_row("a.jar", "/srv/a"), file_row("a.jar", "/srv/b")],
    )
    .unwrap_err();
    assert!(
        matches!(err, repo::RepoError::DuplicateFileName(ref n) if n == "a.jar"),
        "got {err:?}"
    );
    // 校验失败 = 原子：旧行集不变。
    assert_eq!(repo::list_files(&store, target.id).unwrap().len(), 2);
}

#[test]
fn row_upload_dirs_must_be_single_nonempty_lines() {
    let (_dir, store) = open_store();
    let project = seed_project(&store, "app");
    let server = seed_server(&store, "web-1");
    let target = repo::create(&store, &new_target(project, server)).unwrap();

    let empty = repo::replace_files(&store, target.id, &[file_row("a.jar", "   ")])
        .unwrap_err();
    assert!(matches!(empty, repo::RepoError::EmptyUploadDir), "got {empty:?}");

    let multiline =
        repo::replace_files(&store, target.id, &[file_row("a.jar", "/a\n/b")]).unwrap_err();
    assert!(
        matches!(multiline, repo::RepoError::UploadDirMultiline),
        "got {multiline:?}"
    );

    // 目录两侧空白会被规整。
    let rows = repo::replace_files(&store, target.id, &[file_row("  a.jar  ", "  /srv/app  ")])
        .unwrap();
    assert_eq!(rows[0].file_name, "a.jar");
    assert_eq!(rows[0].upload_dir, "/srv/app");
}

/// 删除目标时映射行随 FK 级联消失（migration v12 的 ON DELETE CASCADE）。
#[test]
fn deleting_the_target_cascades_to_its_mapping_rows() {
    let (_dir, store) = open_store();
    let project = seed_project(&store, "app");
    let server = seed_server(&store, "web-1");
    let target = repo::create(&store, &new_target(project, server)).unwrap();
    repo::replace_files(&store, target.id, &[file_row("a.jar", "/srv/a")]).unwrap();
    assert_eq!(repo::list_files(&store, target.id).unwrap().len(), 1);

    assert!(repo::delete(&store, target.id).unwrap());
    assert!(repo::list_files(&store, target.id).unwrap().is_empty());

    let orphans: i64 = store
        .with(|conn| conn.query_row("SELECT count(*) FROM deployment_target_files", [], |r| r.get(0)))
        .unwrap();
    assert_eq!(orphans, 0, "target deletion must cascade to its rows");
}

/// 未知目标上的映射行写入被 FK 拒绝（外键开启）。
#[test]
fn replace_files_on_a_missing_target_is_rejected() {
    let (_dir, store) = open_store();
    assert!(repo::replace_files(&store, 99_999, &[file_row("a.jar", "/srv/a")]).is_err());
}
