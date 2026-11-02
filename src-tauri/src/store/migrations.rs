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
pub(crate) const MIGRATIONS: &[Migration] = &[Migration {
    version: 1,
    name: "baseline: app_meta key/value table",
    sql: "CREATE TABLE app_meta (key TEXT PRIMARY KEY, value TEXT NOT NULL);",
}];

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
