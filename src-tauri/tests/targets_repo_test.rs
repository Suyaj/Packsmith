//! Integration tests for 部署目标 (Deployment Target) CRUD (ticket 09).
//!
//! Seams under test:
//! - `targets::repo::{list_for_project, create, update, delete}` — persistence
//!   over `&Store` (same seam shape as `server::repo`)
//! - `targets::repo::split_commands` — 命令组 line handling (trim, blank-line
//!   removal, order preservation)
//! - the schema created by migration v4 (`deployment_targets` table) and its
//!   foreign keys (project × server integrity)
//!
//! Nothing here touches SSH/SFTP: execution belongs to ticket 10.

use packsmith_lib::store::Store;
use packsmith_lib::targets::repo::{self, NewTarget, Target};

const TEST_KEY_HEX: &str =
    "a1b2c3d4e5f60718293a4b5c6d7e8f90a1b2c3d4e5f60718293a4b5c6d7e8f90";

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
                rusqlite::params![name, format!("https://example.com/{name}.git"), format!("/tmp/{name}")],
            )?;
            Ok(conn.last_insert_rowid())
        })
        .unwrap()
}

fn new_target(project_id: i64, server_id: i64) -> NewTarget {
    NewTarget {
        project_id,
        server_id,
        upload_dir: "/home/app".to_string(),
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
    assert_eq!(count, 1, "migration must create the deployment_targets table");
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
            upload_dir: "/home/app".to_string(),
            start_commands: "cd /home/app\nsh start.sh start".to_string(),
            stop_commands: "sh start.sh stop".to_string(),
            auto_deploy: false,
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
            upload_dir: "  /home/app  \n".to_string(),
            // Blank lines dropped, surrounding whitespace trimmed, order kept.
            start_commands: "\n  cd /home/app  \n\n   \nsh start.sh start\n".to_string(),
            stop_commands: "  \n".to_string(),
        },
    )
    .unwrap();

    assert_eq!(target.upload_dir, "/home/app");
    assert_eq!(target.start_commands, "cd /home/app\nsh start.sh start");
    assert_eq!(target.stop_commands, "");
}

#[test]
fn split_commands_splits_trims_and_drops_blank_lines_in_order() {
    assert_eq!(
        repo::split_commands("cd /home/app\nsh start.sh start"),
        vec!["cd /home/app".to_string(), "sh start.sh start".to_string()]
    );
    assert_eq!(
        repo::split_commands("  a \n\n  b  \n\t\nc"),
        vec!["a".to_string(), "b".to_string(), "c".to_string()]
    );
    assert_eq!(repo::split_commands("   \n\t "), Vec::<String>::new());
    assert_eq!(repo::split_commands(""), Vec::<String>::new());
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
    assert_eq!(for_b.len(), 1, "one server may be bound by several projects");
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
    let fetched = repo::get(&store, created.id).unwrap().expect("target exists");
    assert_eq!(fetched.project_id, project_a);
    assert!(repo::get(&store, 999).unwrap().is_none());
}
