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
        name: "ticket 06: projects.last_branch (记住上次打包分支)",
        sql: "ALTER TABLE projects ADD COLUMN last_branch TEXT;",
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
