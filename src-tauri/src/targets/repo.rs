//! Deployment-target persistence over the encrypted store.
//!
//! All access goes through [`Store::with`]; the connection never leaks
//! further out. Command groups are stored normalized: split on newlines,
//! each line trimmed, blank lines dropped, order preserved (see
//! [`split_command_group`]).

use rusqlite::{OptionalExtension, params};

use crate::store::Store;

/// A new deployment target, not yet persisted.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewTarget {
    pub project_id: i64,
    pub server_id: i64,
    /// 默认上传目录（票 26）：自由勾选的构件（非预设条目）上传到这里。必填，
    /// 单行非空。
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
    /// 默认上传目录（票 26 恢复为业务字段）：自由勾选的构件上传到这里。
    /// 票 19–25 期间废弃（映射行承载目录），迁移 v15 从首个映射行回填。
    pub upload_dir: String,
    pub start_commands: String,
    pub stop_commands: String,
    /// 构建成功后自动连锁部署 (auto-chain deploy after a successful build) —
    /// default OFF (spec: 默认关).
    pub auto_deploy: bool,
    /// 上次该目标部署选择的构件文件名 (ticket 17)；ticket 19 起废弃（映射行
    /// 自带文件名），列保留、UI 不再使用。
    pub last_artifact: Option<String>,
}

/// 一行 文件名→上传目录 映射（票 19；票 26 起降级为部署对话框的**预设**）：
/// 部署对话框把具名行对应的构件默认勾选并沿用该行的目录。`file_name` 为空
/// 的行不产生预设（票 26：空名行不再单独展示，可在对话框内自由勾选替代）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TargetFile {
    pub id: i64,
    pub target_id: i64,
    pub file_name: String,
    pub upload_dir: String,
    pub position: i64,
}

/// A new mapping row (ticket 19), not yet persisted.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewTargetFile {
    pub file_name: String,
    pub upload_dir: String,
}

/// Splits a 命令组 (command group) into its commands: one per line, each
/// trimmed, blank lines dropped, order preserved.
pub fn split_command_group(text: &str) -> Vec<String> {
    text.lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(str::to_string)
        .collect()
}

/// Normalizes a command group for storage (what round-trips back out).
pub fn normalize_commands(text: &str) -> String {
    split_command_group(text).join("\n")
}

const SELECT_COLUMNS: &str =
    "id, project_id, server_id, upload_dir, start_commands, stop_commands, auto_deploy, last_artifact";

fn row_to_target(row: &rusqlite::Row<'_>) -> rusqlite::Result<Target> {
    Ok(Target {
        id: row.get("id")?,
        project_id: row.get("project_id")?,
        server_id: row.get("server_id")?,
        upload_dir: row.get("upload_dir")?,
        start_commands: row.get("start_commands")?,
        stop_commands: row.get("stop_commands")?,
        auto_deploy: row.get::<_, i64>("auto_deploy")? != 0,
        last_artifact: row.get("last_artifact")?,
    })
}

/// Lists every deployment target bound to `project_id`, oldest first.
pub fn list_for_project(store: &Store, project_id: i64) -> Result<Vec<Target>, RepoError> {
    Ok(store.with(|conn| {
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
/// Validates that the referenced project and server exist, that the
/// project × server pair is not bound yet (the table also carries a
/// `UNIQUE (project_id, server_id)` constraint as a backstop), and that the
/// 默认上传目录 (票 26) is a single non-empty line.
pub fn create(store: &Store, target: &NewTarget) -> Result<Target, RepoError> {
    check_references(store, target.project_id, target.server_id)?;
    if binding_exists(store, target.project_id, target.server_id, None)? {
        return Err(RepoError::DuplicateBinding);
    }
    let upload_dir = normalize_upload_dir(&target.upload_dir)?;
    let normalized = Target {
        id: 0,
        project_id: target.project_id,
        server_id: target.server_id,
        upload_dir,
        start_commands: normalize_commands(&target.start_commands),
        stop_commands: normalize_commands(&target.stop_commands),
        auto_deploy: false,
        last_artifact: None,
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
/// being updated itself. `last_artifact` is deliberately **not** touched
/// (ticket 17): editing a target's commands must not forget its last choice.
pub fn update(store: &Store, target: &Target) -> Result<Target, RepoError> {
    check_references(store, target.project_id, target.server_id)?;
    if binding_exists(store, target.project_id, target.server_id, Some(target.id))? {
        return Err(RepoError::DuplicateBinding);
    }
    let normalized = Target {
        // 默认上传目录（票 26）：业务字段，随表单更新。
        upload_dir: normalize_upload_dir(&target.upload_dir)?,
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
/// The target's 文件名→目录 mapping rows cascade away via the FK.
pub fn delete(store: &Store, id: i64) -> Result<bool, RepoError> {
    let changed = store
        .with(|conn| conn.execute("DELETE FROM deployment_targets WHERE id = ?1", params![id]))?;
    Ok(changed > 0)
}

const FILE_COLUMNS: &str = "id, target_id, file_name, upload_dir, position";

fn row_to_file(row: &rusqlite::Row<'_>) -> rusqlite::Result<TargetFile> {
    Ok(TargetFile {
        id: row.get("id")?,
        target_id: row.get("target_id")?,
        file_name: row.get("file_name")?,
        upload_dir: row.get("upload_dir")?,
        position: row.get("position")?,
    })
}

/// Lists one target's 文件名→目录 mapping rows in display order (position, id).
pub fn list_files(store: &Store, target_id: i64) -> Result<Vec<TargetFile>, RepoError> {
    Ok(store.with(|conn| {
        let mut stmt = conn.prepare(&format!(
            "SELECT {FILE_COLUMNS} FROM deployment_target_files
             WHERE target_id = ?1 ORDER BY position, id"
        ))?;
        let rows = stmt
            .query_map([target_id], row_to_file)?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    })?)
}

/// Lists the mapping rows of several targets at once (grouped by the caller
/// via [`TargetFile::target_id`]), display order within each target.
pub fn files_for_targets(
    store: &Store,
    target_ids: &[i64],
) -> Result<Vec<TargetFile>, RepoError> {
    if target_ids.is_empty() {
        return Ok(Vec::new());
    }
    let placeholders = target_ids.iter().map(|_| "?").collect::<Vec<_>>().join(", ");
    let params: Vec<i64> = target_ids.to_vec();
    Ok(store.with(|conn| {
        let mut stmt = conn.prepare(&format!(
            "SELECT {FILE_COLUMNS} FROM deployment_target_files
             WHERE target_id IN ({placeholders})
             ORDER BY target_id, position, id"
        ))?;
        let rows = stmt
            .query_map(rusqlite::params_from_iter(params.iter()), row_to_file)?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    })?)
}

/// 规整默认上传目录（票 26）：两侧空白去除，必须是非空单行。
fn normalize_upload_dir(upload_dir: &str) -> Result<String, RepoError> {
    let upload_dir = upload_dir.trim().to_string();
    if upload_dir.is_empty() {
        return Err(RepoError::EmptyUploadDir);
    }
    if upload_dir.lines().count() > 1 {
        return Err(RepoError::UploadDirMultiline);
    }
    Ok(upload_dir)
}

/// Validates one mapping row (ticket 19; 票 26 起行语义不变、映射行集合可选):
/// the upload dir must be a single non-empty line; the file name may be empty
/// (legacy unnamed row — 不产生预设) but the same non-empty name may not
/// repeat within one target.
fn normalize_file_rows(files: &[NewTargetFile]) -> Result<Vec<(String, String)>, RepoError> {
    let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
    files
        .iter()
        .map(|file| {
            let upload_dir = file.upload_dir.trim().to_string();
            if upload_dir.is_empty() {
                return Err(RepoError::EmptyUploadDir);
            }
            if upload_dir.lines().count() > 1 {
                return Err(RepoError::UploadDirMultiline);
            }
            let file_name = file.file_name.trim().to_string();
            if !file_name.is_empty() && !seen.insert(file_name.clone()) {
                return Err(RepoError::DuplicateFileName(file_name));
            }
            Ok((file_name, upload_dir))
        })
        .collect()
}

/// Replaces one target's whole mapping-row set (ticket 19): delete + insert
/// atomically. Editing the table therefore cannot leave the target with a
/// half-saved mapping. Returns the persisted rows in display order.
pub fn replace_files(
    store: &Store,
    target_id: i64,
    files: &[NewTargetFile],
) -> Result<Vec<TargetFile>, RepoError> {
    let normalized = normalize_file_rows(files)?;
    Ok(store.with(|conn| {
        let tx = conn.unchecked_transaction()?;
        tx.execute(
            "DELETE FROM deployment_target_files WHERE target_id = ?1",
            params![target_id],
        )?;
        for (position, (file_name, upload_dir)) in normalized.iter().enumerate() {
            tx.execute(
                "INSERT INTO deployment_target_files
                     (target_id, file_name, upload_dir, position)
                 VALUES (?1, ?2, ?3, ?4)",
                params![target_id, file_name, upload_dir, position as i64],
            )?;
        }
        let rows = list_files_in(&tx, target_id)?;
        tx.commit()?;
        Ok(rows)
    })?)
}

fn list_files_in(conn: &rusqlite::Connection, target_id: i64) -> rusqlite::Result<Vec<TargetFile>> {
    let mut stmt = conn.prepare(&format!(
        "SELECT {FILE_COLUMNS} FROM deployment_target_files
         WHERE target_id = ?1 ORDER BY position, id"
    ))?;
    let rows = stmt
        .query_map([target_id], row_to_file)?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}

/// Fetches one deployment target by id, any project.
pub fn get(store: &Store, id: i64) -> Result<Option<Target>, RepoError> {
    Ok(store.with(|conn| {
        conn.query_row(
            &format!("SELECT {SELECT_COLUMNS} FROM deployment_targets WHERE id = ?1"),
            [id],
            row_to_target,
        )
        .optional()
    })?)
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

/// Remembers the artifact file name this target was last deployed with
/// (ticket 17: 多产物时按目标记住上次选择，部署对话框的默认选中项).
/// Returns whether the target exists.
pub fn set_last_artifact(store: &Store, id: i64, file_name: &str) -> Result<bool, RepoError> {
    let changed = store.with(|conn| {
        conn.execute(
            "UPDATE deployment_targets SET last_artifact = ?1 WHERE id = ?2",
            params![file_name.trim(), id],
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
    /// 默认上传目录（票 26）或一行映射行的目录为空——不允许。
    EmptyUploadDir,
    /// 默认上传目录（票 26）或一行映射行的目录跨多行——不允许。
    UploadDirMultiline,
    /// The same non-empty file name appears twice within one target's rows.
    DuplicateFileName(String),
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
            RepoError::EmptyUploadDir => write!(f, "an upload dir must not be empty"),
            RepoError::UploadDirMultiline => {
                write!(f, "an upload dir must be a single line")
            }
            RepoError::DuplicateFileName(name) => {
                write!(f, "the file name {name:?} appears twice in this target's rows")
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
            RepoError::EmptyUploadDir => "empty_upload_dir",
            RepoError::UploadDirMultiline => "upload_dir_multiline",
            RepoError::DuplicateFileName(_) => "duplicate_file_name",
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
            .query_row("SELECT 1 FROM projects WHERE id = ?1", [project_id], |_| {
                Ok(())
            })
            .optional()?;
        let server = conn
            .query_row("SELECT 1 FROM servers WHERE id = ?1", [server_id], |_| {
                Ok(())
            })
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
