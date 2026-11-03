//! Deployment-target persistence over the encrypted store.
//!
//! All access goes through [`Store::with`]; the connection never leaks
//! further out. Command groups are stored normalized: split on newlines,
//! each line trimmed, blank lines dropped, order preserved (see
//! [`split_commands`]).

use rusqlite::{params, OptionalExtension};

use crate::store::Store;

/// A new deployment target, not yet persisted.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewTarget {
    pub project_id: i64,
    pub server_id: i64,
    pub upload_dir: String,
    pub start_commands: String,
    pub stop_commands: String,
}

/// A persisted deployment target.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Target {
    pub id: i64,
    pub project_id: i64,
    pub server_id: i64,
    pub upload_dir: String,
    pub start_commands: String,
    pub stop_commands: String,
    /// 构建成功后自动连锁部署 (auto-chain deploy after a successful build) —
    /// default OFF (spec: 默认关).
    pub auto_deploy: bool,
}

/// Splits a 命令组 (command group) into its commands: one per line, each
/// trimmed, blank lines dropped, order preserved.
pub fn split_commands(text: &str) -> Vec<String> {
    text.lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(str::to_string)
        .collect()
}

/// Normalizes a command group for storage (what round-trips back out).
pub fn normalize_commands(text: &str) -> String {
    split_commands(text).join("\n")
}

const SELECT_COLUMNS: &str =
    "id, project_id, server_id, upload_dir, start_commands, stop_commands, auto_deploy";

fn row_to_target(row: &rusqlite::Row<'_>) -> rusqlite::Result<Target> {
    Ok(Target {
        id: row.get("id")?,
        project_id: row.get("project_id")?,
        server_id: row.get("server_id")?,
        upload_dir: row.get("upload_dir")?,
        start_commands: row.get("start_commands")?,
        stop_commands: row.get("stop_commands")?,
        auto_deploy: row.get::<_, i64>("auto_deploy")? != 0,
    })
}

/// Lists every deployment target bound to `project_id`, oldest first.
pub fn list_for_project(store: &Store, project_id: i64) -> Result<Vec<Target>, RepoError> {
    Ok(store
        .with(|conn| {
            let mut stmt = conn.prepare(&format!(
                "SELECT {SELECT_COLUMNS} FROM deployment_targets
                 WHERE project_id = ?1 ORDER BY id"
            ))?;
            let rows = stmt
                .query_map([project_id], row_to_target)?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            Ok(rows)
        })?)
}

/// Inserts a deployment target and returns the persisted record.
///
/// Validates that the referenced project and server exist and that the
/// project × server pair is not bound yet (the table also carries a
/// `UNIQUE (project_id, server_id)` constraint as a backstop).
pub fn create(store: &Store, target: &NewTarget) -> Result<Target, RepoError> {
    check_references(store, target.project_id, target.server_id)?;
    if binding_exists(store, target.project_id, target.server_id, None)? {
        return Err(RepoError::DuplicateBinding);
    }
    let normalized = Target {
        id: 0,
        project_id: target.project_id,
        server_id: target.server_id,
        upload_dir: target.upload_dir.trim().to_string(),
        start_commands: normalize_commands(&target.start_commands),
        stop_commands: normalize_commands(&target.stop_commands),
        auto_deploy: false,
    };
    let id = store.with(|conn| {
        conn.execute(
            "INSERT INTO deployment_targets
                 (project_id, server_id, upload_dir, start_commands, stop_commands, auto_deploy)
             VALUES (?1, ?2, ?3, ?4, ?5, 0)",
            params![
                normalized.project_id,
                normalized.server_id,
                normalized.upload_dir,
                normalized.start_commands,
                normalized.stop_commands
            ],
        )?;
        Ok(conn.last_insert_rowid())
    })?;
    Ok(Target { id, ..normalized })
}

/// Replaces all fields of the target with the given id. The row must exist.
///
/// Same validation as [`create`]; the pair uniqueness check ignores the row
/// being updated itself.
pub fn update(store: &Store, target: &Target) -> Result<Target, RepoError> {
    check_references(store, target.project_id, target.server_id)?;
    if binding_exists(store, target.project_id, target.server_id, Some(target.id))? {
        return Err(RepoError::DuplicateBinding);
    }
    let normalized = Target {
        upload_dir: target.upload_dir.trim().to_string(),
        start_commands: normalize_commands(&target.start_commands),
        stop_commands: normalize_commands(&target.stop_commands),
        ..target.clone()
    };
    let changed = store.with(|conn| {
        conn.execute(
            "UPDATE deployment_targets
             SET project_id = ?1, server_id = ?2, upload_dir = ?3,
                 start_commands = ?4, stop_commands = ?5, auto_deploy = ?6
             WHERE id = ?7",
            params![
                normalized.project_id,
                normalized.server_id,
                normalized.upload_dir,
                normalized.start_commands,
                normalized.stop_commands,
                normalized.auto_deploy as i64,
                normalized.id
            ],
        )
    })?;
    if changed == 0 {
        return Err(RepoError::Sql(rusqlite::Error::QueryReturnedNoRows));
    }
    Ok(normalized)
}

/// Deletes a deployment target. Returns whether a row was actually removed.
pub fn delete(store: &Store, id: i64) -> Result<bool, RepoError> {
    let changed = store.with(|conn| {
        conn.execute("DELETE FROM deployment_targets WHERE id = ?1", params![id])
    })?;
    Ok(changed > 0)
}

/// Toggles 构建成功后自动连锁部署 for one target (ticket 10). Returns
/// whether the target existed.
pub fn set_auto_deploy(store: &Store, id: i64, enabled: bool) -> Result<bool, RepoError> {
    let changed = store.with(|conn| {
        conn.execute(
            "UPDATE deployment_targets SET auto_deploy = ?1 WHERE id = ?2",
            params![enabled as i64, id],
        )
    })?;
    Ok(changed > 0)
}

/// Ids of the project's deployment targets with auto-chain enabled (构建
/// 成功后自动连锁部署), oldest first.
pub fn auto_deploy_enabled(store: &Store, project_id: i64) -> Result<Vec<i64>, RepoError> {
    Ok(store.with(|conn| {
        let mut stmt = conn.prepare(
            "SELECT id FROM deployment_targets
             WHERE project_id = ?1 AND auto_deploy != 0 ORDER BY id",
        )?;
        let rows = stmt
            .query_map([project_id], |row| row.get(0))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    })?)
}

#[derive(Debug)]
pub enum RepoError {
    Sql(rusqlite::Error),
    Store(crate::store::StoreError),
    /// The referenced project does not exist.
    MissingProject,
    /// The referenced server does not exist.
    MissingServer,
    /// This project already binds that server (project × server is unique).
    DuplicateBinding,
}

impl std::fmt::Display for RepoError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RepoError::Sql(e) => write!(f, "sqlite error: {e}"),
            RepoError::Store(e) => write!(f, "store error: {e}"),
            RepoError::MissingProject => write!(f, "the referenced project does not exist"),
            RepoError::MissingServer => write!(f, "the referenced server does not exist"),
            RepoError::DuplicateBinding => {
                write!(f, "this project already binds that server")
            }
        }
    }
}

impl RepoError {
    /// A stable machine-readable tag for the frontend to key i18n on.
    pub fn code(&self) -> &'static str {
        match self {
            RepoError::Sql(_) | RepoError::Store(_) => "store",
            RepoError::MissingProject => "missing_project",
            RepoError::MissingServer => "missing_server",
            RepoError::DuplicateBinding => "duplicate_binding",
        }
    }
}

/// Whether the project × server pair is already bound. `except_id` excludes
/// one row (the target being updated) from the check.
fn binding_exists(
    store: &Store,
    project_id: i64,
    server_id: i64,
    except_id: Option<i64>,
) -> Result<bool, RepoError> {
    let exists = store.with(|conn| {
        Ok(conn
            .query_row(
                "SELECT 1 FROM deployment_targets
                 WHERE project_id = ?1 AND server_id = ?2 AND id != ?3",
                params![project_id, server_id, except_id.unwrap_or(-1)],
                |_| Ok(()),
            )
            .optional()?
            .is_some())
    })?;
    Ok(exists)
}

/// Fails with [`RepoError::MissingProject`] / [`RepoError::MissingServer`]
/// when either referenced row does not exist.
fn check_references(store: &Store, project_id: i64, server_id: i64) -> Result<(), RepoError> {
    let (project_exists, server_exists) = store.with(|conn| {
        let project = conn
            .query_row(
                "SELECT 1 FROM projects WHERE id = ?1",
                [project_id],
                |_| Ok(()),
            )
            .optional()?;
        let server = conn
            .query_row("SELECT 1 FROM servers WHERE id = ?1", [server_id], |_| Ok(()))
            .optional()?;
        Ok((project.is_some(), server.is_some()))
    })?;
    if !project_exists {
        return Err(RepoError::MissingProject);
    }
    if !server_exists {
        return Err(RepoError::MissingServer);
    }
    Ok(())
}

impl std::error::Error for RepoError {}

impl From<rusqlite::Error> for RepoError {
    fn from(e: rusqlite::Error) -> Self {
        RepoError::Sql(e)
    }
}

impl From<crate::store::StoreError> for RepoError {
    fn from(e: crate::store::StoreError) -> Self {
        RepoError::Store(e)
    }
}
