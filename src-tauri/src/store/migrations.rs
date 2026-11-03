//! Schema migrations: the shared migration entry point for all later tickets.
//!
//! Migrations are declared as an ordered, append-only list. `Store::open`
//! applies every migration newer than the database's recorded
//! `PRAGMA user_version`, so reopening an old database upgrades it in place.

use rusqlite::Connection;

pub(crate) struct Migration {
    pub version: i32,
    #[allow(dead_code)]
    pub name: &'static str,
    pub sql: &'static str,
}

/// Append-only list: never edit an applied migration, only add new ones.
pub(crate) const MIGRATIONS: &[Migration] = &[
    Migration {
        version: 1,
        name: "baseline: app_meta key/value table",
        sql: "CREATE TABLE app_meta (key TEXT PRIMARY KEY, value TEXT NOT NULL);",
    },
    Migration {
        version: 2,
        name: "ticket 04: servers + known_hosts (accepted fingerprints)",
        sql: "CREATE TABLE servers (
            id INTEGER PRIMARY KEY,
            host TEXT NOT NULL,
            port INTEGER NOT NULL DEFAULT 22,
            username TEXT NOT NULL,
            auth_method TEXT NOT NULL CHECK (auth_method IN ('private_key', 'password')),
            password TEXT,
            private_key_path TEXT,
            private_key_passphrase TEXT
        );
        CREATE TABLE known_hosts (
            host TEXT NOT NULL,
            port INTEGER NOT NULL,
            fingerprint TEXT NOT NULL,
            PRIMARY KEY (host, port)
        );",
    },
    Migration {
        version: 3,
        name: "项目: projects table",
        sql: "CREATE TABLE projects (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            name TEXT NOT NULL UNIQUE,
            source TEXT NOT NULL CHECK (source IN ('git', 'local')),
            url TEXT,
            path TEXT NOT NULL,
            project_type TEXT NOT NULL DEFAULT 'maven'
                CHECK (project_type IN ('maven', 'gradle')),
            created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
        );",
    },
    Migration {
        version: 4,
        name: "ticket 09: deployment_targets (项目 × 服务器 bindings)",
        sql: "CREATE TABLE deployment_targets (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            project_id INTEGER NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
            server_id INTEGER NOT NULL REFERENCES servers(id) ON DELETE CASCADE,
            upload_dir TEXT NOT NULL,
            start_commands TEXT NOT NULL DEFAULT '',
            stop_commands TEXT NOT NULL DEFAULT '',
            created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
            UNIQUE (project_id, server_id)
        );",
    },
    Migration {
        version: 5,
        name: "ticket 06: projects.last_branch (记住上次打包分支)",
        sql: "ALTER TABLE projects ADD COLUMN last_branch TEXT;",
    },
    Migration {
        version: 6,
        name: "ticket 07: projects.build_command (随项目可编辑的构建命令)",
        sql: "ALTER TABLE projects ADD COLUMN build_command TEXT NOT NULL DEFAULT 'mvnd package -DskipTests';",
    },
    Migration {
        version: 7,
        name: "ticket 08: build_history (构建历史) + projects.default_artifact (记住默认构件选择)",
        sql: "ALTER TABLE projects ADD COLUMN default_artifact TEXT;
        CREATE TABLE build_history (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            build_id TEXT NOT NULL UNIQUE,
            project_id INTEGER NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
            branch TEXT NOT NULL,
            success INTEGER NOT NULL CHECK (success IN (0, 1)),
            exit_code INTEGER,
            log_path TEXT NOT NULL,
            artifacts_dir TEXT,
            created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
        );",
    },
    Migration {
        version: 8,
        name: "ticket 11: 关窗行为设置默认值 (close_window_behavior)",
        sql: "INSERT OR IGNORE INTO app_meta (key, value) VALUES ('close_window_behavior', 'minimize');",
    },
    Migration {
        version: 9,
        name: "ticket 10: deployment_targets.auto_deploy (构建成功后自动连锁部署, 默认关)",
        sql: "ALTER TABLE deployment_targets ADD COLUMN auto_deploy INTEGER NOT NULL DEFAULT 0;",
    },
    Migration {
        version: 10,
        name: "ticket 12: deploy_records (每次部署的目标结果, 供首页最近部署时间)",
        sql: "CREATE TABLE deploy_records (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            target_id INTEGER NOT NULL REFERENCES deployment_targets(id) ON DELETE CASCADE,
            success INTEGER NOT NULL CHECK (success IN (0, 1)),
            created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
        );
        CREATE INDEX idx_deploy_records_target ON deploy_records(target_id);",
    },
    Migration {
        version: 11,
        name: "ticket 17: deployment_targets.last_artifact (记住该目标上次部署选择的构件)",
        sql: "ALTER TABLE deployment_targets ADD COLUMN last_artifact TEXT;",
    },
];

pub(crate) const LATEST_SCHEMA_VERSION: i32 = MIGRATIONS[MIGRATIONS.len() - 1].version;

pub(crate) fn run(conn: &Connection) -> Result<(), rusqlite::Error> {
    let current: i32 = conn.query_row("PRAGMA user_version", [], |r| r.get(0))?;

    for m in MIGRATIONS.iter().filter(|m| m.version > current) {
        // Each migration is atomic: schema change + version bump together.
        let tx = conn.unchecked_transaction()?;
        tx.execute_batch(m.sql)?;
        tx.pragma_update(None, "user_version", m.version)?;
        tx.commit()?;
    }

    Ok(())
}
