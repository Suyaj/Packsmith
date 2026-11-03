//! Integration tests for 部署记录 (deploy records, ticket 12).
//!
//! Seams under test:
//! - `store::deploy_history::{record, record_outcomes, last_successful_per_server}`
//!   — the persistence layer the homepage server cards read from
//! - the schema created by migration v10 (`deploy_records` table)
//!
//! 首页服务器卡片显示"最近部署时间"。Ticket 10 的部署执行不持久化任何记录，
//! 所以这里选择最简单且真实的方案：每次部署（手动 start_deploy 与自动连锁）
//! 结束后，把每个部署目标的结果落一行（success 标记成败）；"最近部署时间"
//! = 该服务器所有部署目标里最近一次**成功**部署的时间。

use packsmith_lib::deploy::engine::{DeploySummary, TargetOutcome, TargetStatus};
use packsmith_lib::store::deploy_history;
use packsmith_lib::store::Store;
use packsmith_lib::targets::repo::{self, NewTarget};

const TEST_KEY_HEX: &str = "a1b2c3d4e5f60718293a4b5c6d7e8f90a1b2c3d4e5f60718293a4b5c6d7e8f90";

fn open_store() -> (tempfile::TempDir, Store) {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("test.db");
    let store = Store::open(&db, TEST_KEY_HEX).unwrap();
    (dir, store)
}

fn seed_server(store: &Store, host: &str) -> i64 {
    packsmith_lib::server::repo::create(
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
    .unwrap()
    .id
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

fn seed_target(store: &Store, project_id: i64, server_id: i64) -> i64 {
    repo::create(
        store,
        &NewTarget {
            project_id,
            server_id,
            upload_dir: "/home/app".to_string(),
            start_commands: "sh start.sh".to_string(),
            stop_commands: "sh stop.sh".to_string(),
        },
    )
    .unwrap()
    .id
}

#[test]
fn migration_creates_deploy_records_table() {
    let (_dir, store) = open_store();

    let count: i64 = store
        .with(|conn| {
            conn.query_row(
                "SELECT count(*) FROM sqlite_master WHERE type='table'
                 AND name = 'deploy_records'",
                [],
                |r| r.get(0),
            )
        })
        .unwrap();
    assert_eq!(count, 1, "migration must create the deploy_records table");
}

#[test]
fn a_server_without_deploys_has_no_last_deploy_time() {
    let (_dir, store) = open_store();
    let server = seed_server(&store, "web-1");

    let rows = deploy_history::last_successful_per_server(&store).unwrap();
    assert_eq!(rows.len(), 1, "one row per known server");
    assert_eq!(rows[0].server_id, server);
    assert_eq!(rows[0].last_deploy_at, None);
}

#[test]
fn last_deploy_time_is_the_most_recent_successful_deploy() {
    let (_dir, store) = open_store();
    let project = seed_project(&store, "app");
    let server = seed_server(&store, "web-1");
    let target = seed_target(&store, project, server);

    deploy_history::record(&store, target, false).unwrap();
    deploy_history::record(&store, target, true).unwrap();
    deploy_history::record(&store, target, false).unwrap();

    let rows = deploy_history::last_successful_per_server(&store).unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].server_id, server);
    let at = rows[0].last_deploy_at.as_ref().expect("a success exists");
    assert_eq!(at.len(), 24, "created_at is an ISO timestamp: {at}");
}

/// 失败的部署不算"最近部署时间"：只有成功部署才推进该时间。
#[test]
fn failed_deploys_do_not_advance_last_deploy_time() {
    let (_dir, store) = open_store();
    let project = seed_project(&store, "app");
    let server = seed_server(&store, "web-1");
    let target = seed_target(&store, project, server);

    deploy_history::record(&store, target, true).unwrap();
    deploy_history::record(&store, target, false).unwrap();

    let rows = deploy_history::last_successful_per_server(&store).unwrap();
    assert!(rows[0].last_deploy_at.is_some());
}

/// 服务器卡片的"最近部署时间"取该服务器上所有部署目标的最近一次成功部署。
#[test]
fn last_deploy_time_covers_all_targets_of_the_server() {
    let (_dir, store) = open_store();
    let project_a = seed_project(&store, "app-a");
    let project_b = seed_project(&store, "app-b");
    let server = seed_server(&store, "web-1");
    let other = seed_server(&store, "web-2");
    let t1 = seed_target(&store, project_a, server);
    let t2 = seed_target(&store, project_b, server);
    let t_other = seed_target(&store, project_a, other);

    deploy_history::record(&store, t1, true).unwrap();
    deploy_history::record(&store, t2, true).unwrap();
    deploy_history::record(&store, t_other, true).unwrap();

    let rows = deploy_history::last_successful_per_server(&store).unwrap();
    assert_eq!(rows.len(), 2, "one row per server");
    let web1 = rows.iter().find(|r| r.server_id == server).unwrap();
    let web2 = rows.iter().find(|r| r.server_id == other).unwrap();
    assert!(web1.last_deploy_at.is_some());
    assert!(web2.last_deploy_at.is_some());
}

/// record_outcomes 落盘一次部署的每个目标结果（成功与失败都记，
/// 时间戳相同不影响——查询按 success=1 过滤）。
#[test]
fn record_outcomes_persists_one_row_per_target() {
    let (_dir, store) = open_store();
    let project = seed_project(&store, "app");
    let server = seed_server(&store, "web-1");
    let t1 = seed_target(&store, project, server);
    let t2 = seed_target(&store, seed_project(&store, "app-b"), server);

    let summary = DeploySummary {
        cancelled: false,
        targets: vec![
            TargetOutcome {
                target_id: t1,
                status: TargetStatus::Success,
                upload: None,
                stop: None,
                start: None,
                stop_failed: false,
                error: None,
            },
            TargetOutcome {
                target_id: t2,
                status: TargetStatus::Failed,
                upload: None,
                stop: None,
                start: None,
                stop_failed: true,
                error: Some("stop failed".to_string()),
            },
            TargetOutcome {
                target_id: 999, // 目标被并发删除也不该让记录崩溃
                status: TargetStatus::Failed,
                upload: None,
                stop: None,
                start: None,
                stop_failed: true,
                error: Some("stop failed".to_string()),
            },
        ],
    };

    deploy_history::record_outcomes(&store, &summary).unwrap();

    let total: i64 = store
        .with(|conn| conn.query_row("SELECT count(*) FROM deploy_records", [], |r| r.get(0)))
        .unwrap();
    assert_eq!(total, 2, "only existing targets are recorded");

    let rows: Vec<(i64, i64)> = store
        .with(|conn| {
            let mut stmt = conn.prepare(
                "SELECT target_id, success FROM deploy_records ORDER BY target_id",
            )?;
            let rows = stmt
                .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            Ok(rows)
        })
        .unwrap();
    assert_eq!(
        rows,
        vec![(t1, 1), (t2, 0)],
        "success and failure outcomes recorded faithfully"
    );
}

/// 删除部署目标（或其级联删除）不能让历史查询崩溃：记录随外键级联清理。
#[test]
fn deleting_a_server_cleans_up_its_deploy_records() {
    let (_dir, store) = open_store();
    let project = seed_project(&store, "app");
    let server = seed_server(&store, "web-1");
    let target = seed_target(&store, project, server);
    deploy_history::record(&store, target, true).unwrap();

    packsmith_lib::server::repo::delete(&store, server).unwrap();

    let total: i64 = store
        .with(|conn| conn.query_row("SELECT count(*) FROM deploy_records", [], |r| r.get(0)))
        .unwrap();
    assert_eq!(total, 0, "records cascade with their target");
    let rows = deploy_history::last_successful_per_server(&store).unwrap();
    assert!(rows.is_empty(), "no servers left");
}
