//! 构建历史（ticket 08）：每次打包（成败）落一条记录，供历史列表、日志
//! 查看与构件列表使用。
//!
//! 删除语义：删除一条历史 = 删除其构件目录（`<appdata>/artifacts/<项目>/
//! <构建时间>/`）+ 构建日志 + 记录行。保留策略（每项目保留最近 N 次，N 在
//! 设置页配置，默认 [`crate::store::settings::DEFAULT_BUILD_RETENTION`]）在
//! 每次成功构建入库后由 [`enforce_retention`] 执行；手动清理用
//! [`clear_project`]。

use std::path::{Path, PathBuf};

use rusqlite::OptionalExtension;
use serde::Serialize;

use super::{settings, Store, StoreError};

#[derive(Debug, thiserror::Error)]
pub enum HistoryError {
    /// 指定 id 的历史条目不存在。
    #[error("build history entry {0} not found")]
    NotFound(i64),
    /// 构建日志文件缺失（已被外部清理）。
    #[error("artifact directory missing: {0}")]
    ArtifactsDirMissing(String),
    #[error("store error: {0}")]
    Store(#[from] StoreError),
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
}

impl HistoryError {
    /// A stable machine-readable tag for the frontend to key i18n on.
    pub fn code(&self) -> &'static str {
        match self {
            HistoryError::NotFound(_) => "history_not_found",
            HistoryError::ArtifactsDirMissing(_) => "artifacts_dir_missing",
            HistoryError::Store(_) => "store",
            HistoryError::Io(_) => "io",
        }
    }
}

/// 一条构建历史记录（项目名已联查）。
#[derive(Debug, Clone, Serialize)]
pub struct HistoryEntry {
    pub id: i64,
    pub build_id: String,
    pub project_id: i64,
    pub project_name: String,
    pub branch: String,
    pub success: bool,
    pub exit_code: Option<i32>,
    pub log_path: String,
    /// 构件库目录（`<appdata>/artifacts/<项目>/<构建时间>/`）；失败构建没有。
    pub artifacts_dir: Option<String>,
    pub created_at: String,
}

/// 新建一条历史记录所需的数据。
#[derive(Debug, Clone)]
pub struct NewHistoryEntry {
    pub build_id: String,
    pub project_id: i64,
    pub branch: String,
    pub success: bool,
    pub exit_code: Option<i32>,
    pub log_path: String,
    pub artifacts_dir: Option<String>,
}

/// 库内一个构件文件（平铺，文件名即上传部署使用的原文件名）。
#[derive(Debug, Clone, Serialize)]
pub struct ArtifactFile {
    pub file_name: String,
    pub size: u64,
}

fn row_to_entry(row: &rusqlite::Row<'_>) -> rusqlite::Result<HistoryEntry> {
    Ok(HistoryEntry {
        id: row.get("id")?,
        build_id: row.get("build_id")?,
        project_id: row.get("project_id")?,
        project_name: row.get("project_name")?,
        branch: row.get("branch")?,
        success: row.get::<_, i64>("success")? != 0,
        exit_code: row.get("exit_code")?,
        log_path: row.get("log_path")?,
        artifacts_dir: row.get("artifacts_dir")?,
        created_at: row.get("created_at")?,
    })
}

const SELECT: &str = "SELECT h.id, h.build_id, h.project_id, p.name AS project_name,
        h.branch, h.success, h.exit_code, h.log_path, h.artifacts_dir, h.created_at
     FROM build_history h JOIN projects p ON p.id = h.project_id";

/// 记录一次打包结果，返回记录 id。
pub fn record(store: &Store, entry: &NewHistoryEntry) -> Result<i64, StoreError> {
    store.with(|conn| {
        conn.execute(
            "INSERT INTO build_history (build_id, project_id, branch, success, exit_code, log_path, artifacts_dir)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            rusqlite::params![
                entry.build_id,
                entry.project_id,
                entry.branch,
                entry.success as i64,
                entry.exit_code,
                entry.log_path,
                entry.artifacts_dir,
            ],
        )?;
        Ok(conn.last_insert_rowid())
    })
}

/// 全部构建历史（跨项目），最近在前；`limit` 只取最近 N 条。
pub fn list(store: &Store, limit: Option<i64>) -> Result<Vec<HistoryEntry>, StoreError> {
    let sql = match limit {
        Some(_) => format!("{SELECT} ORDER BY h.created_at DESC, h.id DESC LIMIT ?1"),
        None => format!("{SELECT} ORDER BY h.created_at DESC, h.id DESC"),
    };
    store.with(|conn| {
        let mut stmt = conn.prepare(&sql)?;
        let rows = match limit {
            Some(n) => stmt
                .query_map([n], row_to_entry)?
                .collect::<rusqlite::Result<Vec<_>>>()?,
            None => stmt
                .query_map([], row_to_entry)?
                .collect::<rusqlite::Result<Vec<_>>>()?,
        };
        Ok(rows)
    })
}

/// 指定项目的构建历史，最近在前。
pub fn list_for_project(
    store: &Store,
    project_id: i64,
) -> Result<Vec<HistoryEntry>, StoreError> {
    store.with(|conn| {
        let mut stmt = conn.prepare(&format!(
            "{SELECT} WHERE h.project_id = ?1 ORDER BY h.created_at DESC, h.id DESC"
        ))?;
        let rows = stmt
            .query_map([project_id], row_to_entry)?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    })
}

/// 按项目列出指定历史条目库内目录中的构件文件，按文件名排序。
pub fn list_artifacts(entry: &HistoryEntry) -> Result<Vec<ArtifactFile>, HistoryError> {
    let dir = entry
        .artifacts_dir
        .as_ref()
        .map(PathBuf::from)
        .ok_or_else(|| HistoryError::ArtifactsDirMissing(entry.build_id.clone()))?;
    let mut files = Vec::new();
    for item in std::fs::read_dir(&dir)? {
        let item = item?;
        if item.file_type()?.is_file() {
            files.push(ArtifactFile {
                file_name: item.file_name().to_string_lossy().into_owned(),
                size: item.metadata().map(|m| m.len()).unwrap_or(0),
            });
        }
    }
    files.sort_by(|a, b| a.file_name.cmp(&b.file_name));
    Ok(files)
}

/// 删除一条历史：构件目录 + 构建日志 + 记录行。
pub fn delete_entry(store: &Store, appdata: &Path, id: i64) -> Result<(), HistoryError> {
    let entry = get(store, id)?.ok_or(HistoryError::NotFound(id))?;
    remove_entry_files(appdata, &entry)?;
    store.with(|conn| {
        conn.execute("DELETE FROM build_history WHERE id = ?1", [id])?;
        Ok(())
    })?;
    Ok(())
}

fn get(store: &Store, id: i64) -> Result<Option<HistoryEntry>, StoreError> {
    store.with(|conn| {
        Ok(conn
            .query_row(&format!("{SELECT} WHERE h.id = ?1"), [id], row_to_entry)
            .optional()?)
    })
}

/// 删除一条记录对应的构件目录与日志文件（目录不存在时容忍，幂等）。
fn remove_entry_files(appdata: &Path, entry: &HistoryEntry) -> Result<(), HistoryError> {
    if let Some(dir) = &entry.artifacts_dir {
        let dir = PathBuf::from(dir);
        if dir.starts_with(library_root(appdata)) {
            let _ = std::fs::remove_dir_all(&dir);
            // 顺带收敛空出的项目目录与构件库根。
            if let Some(parent) = dir.parent() {
                remove_dir_if_empty(parent);
                remove_dir_if_empty(parent.parent().unwrap_or(parent));
            }
        }
    }
    let log = PathBuf::from(&entry.log_path);
    if log.starts_with(appdata.join("build-logs")) {
        let _ = std::fs::remove_file(&log);
    }
    Ok(())
}

fn remove_dir_if_empty(dir: &Path) {
    if let Ok(mut items) = std::fs::read_dir(dir) {
        if items.next().is_none() {
            let _ = std::fs::remove_dir(dir);
        }
    }
}

/// 构件库根目录（与 [`crate::build::artifacts::library_root`] 一致）。
fn library_root(appdata: &Path) -> PathBuf {
    appdata.join("artifacts")
}

/// 保留策略：每项目只保留最近 N 次构建（N 来自设置，默认 10）。超出的
/// 条目连同其构件目录与日志一起删除。
pub fn enforce_retention(store: &Store, appdata: &Path, project_id: i64) -> Result<(), HistoryError> {
    let keep = settings::get_build_retention(store)?;
    let entries = list_for_project(store, project_id)?;
    for entry in entries.iter().skip(keep.max(0) as usize) {
        remove_entry_files(appdata, entry)?;
        store.with(|conn| {
            conn.execute("DELETE FROM build_history WHERE id = ?1", [entry.id])?;
            Ok(())
        })?;
    }
    Ok(())
}

/// 手动清理：清空指定项目的全部历史（构件目录 + 日志 + 记录行）。
pub fn clear_project(store: &Store, appdata: &Path, project_id: i64) -> Result<(), HistoryError> {
    for entry in list_for_project(store, project_id)? {
        remove_entry_files(appdata, &entry)?;
        store.with(|conn| {
            conn.execute("DELETE FROM build_history WHERE id = ?1", [entry.id])?;
            Ok(())
        })?;
    }
    Ok(())
}
