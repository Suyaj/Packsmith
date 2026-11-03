//! 项目 (Project) records: persistence, validation and creation flows.
//!
//! A project is either cloned from a git URL (source `git`, created via
//! [`clone_and_register`]) or imported from an existing local path (source
//! `local`, [`import`]). All records live in the encrypted store (ADR-0002).
//!
//! v1 scope: only Maven (Java) is a selectable project type; no pull /
//! branch management.

use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use rusqlite::OptionalExtension;
use serde::{Deserialize, Serialize};

use super::{Store, StoreError};

mod clone;

pub use clone::{clone_and_register, clone_repo, expand_workspace_root, CloneProgress, CloneStage};

// ---------------------------------------------------------------------------
// Domain types
// ---------------------------------------------------------------------------

/// 项目类型。v1 仅 Maven 可选；其余值为将来扩展预留（前端以 disabled 占位）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ProjectType {
    Maven,
    Gradle,
}

impl ProjectType {
    pub fn as_str(self) -> &'static str {
        match self {
            ProjectType::Maven => "maven",
            ProjectType::Gradle => "gradle",
        }
    }

    fn from_str(s: &str) -> Result<Self, ProjectError> {
        match s {
            "maven" => Ok(ProjectType::Maven),
            "gradle" => Ok(ProjectType::Gradle),
            other => Err(ProjectError::InvalidType(other.to_string())),
        }
    }
}

/// 项目来源：git clone 或本地导入。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ProjectSource {
    Git,
    Local,
}

impl ProjectSource {
    fn from_str(s: &str) -> Self {
        match s {
            "local" => ProjectSource::Local,
            _ => ProjectSource::Git,
        }
    }
}

/// 一个已注册的项目。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Project {
    pub id: i64,
    pub name: String,
    pub source: ProjectSource,
    /// Git 仓库地址；本地导入的项目为 `None`。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    /// 项目在磁盘上的目录（clone 目标或导入路径，绝对路径）。
    pub path: String,
    pub project_type: ProjectType,
    /// 上次打包选择的分支（ticket 06）；从未打包过为 `None`。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_branch: Option<String>,
    /// 项目构建命令（ticket 07），随项目可编辑；默认
    /// [`DEFAULT_BUILD_COMMAND`]（迁移 v6 落库默认值）。
    #[serde(default)]
    pub build_command: String,
    pub created_at: String,
}

/// 新项目的默认构建命令（spec: 默认 `mvnd package -DskipTests`）。
pub const DEFAULT_BUILD_COMMAND: &str = "mvnd package -DskipTests";

// ---------------------------------------------------------------------------
// Errors
// ---------------------------------------------------------------------------

#[derive(Debug, thiserror::Error)]
pub enum ProjectError {
    /// 已存在同名项目。
    #[error("a project named {0:?} already exists")]
    DuplicateName(String),
    /// 已存在相同 Git 地址的项目。
    #[error("a project with URL {0:?} already exists")]
    DuplicateUrl(String),
    /// 已存在相同本地路径的项目。
    #[error("a project at path {0:?} already exists")]
    DuplicatePath(String),
    /// 项目名称为空或含路径分隔符。
    #[error("invalid project name {0:?}")]
    InvalidName(String),
    /// 导入的本地路径不存在或不是目录。
    #[error("local path {0:?} does not exist or is not a directory")]
    PathNotFound(String),
    /// clone 目标目录已存在。
    #[error("destination directory {0:?} already exists")]
    DirectoryExists(String),
    /// 未支持的项目类型值。
    #[error("unsupported project type {0:?}")]
    InvalidType(String),
    /// 克隆被用户取消。
    #[error("clone was cancelled")]
    Cancelled,
    /// 克隆失败（git2 / URL 无效等）。
    #[error("clone failed: {0}")]
    Clone(String),
    /// 实时拉取分支列表失败（`git ls-remote` 等价操作；URL 不可达 / 不是 git 仓库等）。
    #[error("failed to list remote branches: {0}")]
    BranchListing(String),
    /// 指定 id 的项目不存在。
    #[error("project {0} not found")]
    ProjectNotFound(i64),
    /// 构建命令为空或仅空白（ticket 07）。
    #[error("invalid build command {0:?}")]
    InvalidBuildCommand(String),
    #[error("store error: {0}")]
    Store(#[from] StoreError),
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
}

impl ProjectError {
    /// A stable machine-readable tag for the frontend to key i18n on.
    pub fn code(&self) -> &'static str {
        match self {
            ProjectError::DuplicateName(_) => "duplicate_name",
            ProjectError::DuplicateUrl(_) => "duplicate_url",
            ProjectError::DuplicatePath(_) => "duplicate_path",
            ProjectError::InvalidName(_) => "invalid_name",
            ProjectError::PathNotFound(_) => "path_not_found",
            ProjectError::DirectoryExists(_) => "directory_exists",
            ProjectError::InvalidType(_) => "invalid_type",
            ProjectError::Cancelled => "cancelled",
            ProjectError::Clone(_) => "clone_failed",
            ProjectError::BranchListing(_) => "branch_listing_failed",
            ProjectError::ProjectNotFound(_) => "project_not_found",
            ProjectError::InvalidBuildCommand(_) => "invalid_build_command",
            ProjectError::Store(_) => "store",
            ProjectError::Io(_) => "io",
        }
    }
}

// ---------------------------------------------------------------------------
// Persistence
// ---------------------------------------------------------------------------

fn row_to_project(row: &rusqlite::Row<'_>) -> rusqlite::Result<Project> {
    Ok(Project {
        id: row.get("id")?,
        name: row.get("name")?,
        source: ProjectSource::from_str(&row.get::<_, String>("source")?),
        url: row.get("url")?,
        path: row.get("path")?,
        project_type: ProjectType::from_str(&row.get::<_, String>("project_type")?)
            .expect("project_type is CHECK-constrained to known values"),
        last_branch: row.get("last_branch")?,
        build_command: row.get("build_command")?,
        created_at: row.get("created_at")?,
    })
}

/// 列出全部项目，按创建时间先后（最早在前）。
pub fn list(store: &Store) -> Result<Vec<Project>, ProjectError> {
    let rows = store.with(|conn| {
        let mut stmt = conn.prepare(
            "SELECT id, name, source, url, path, project_type, last_branch, build_command, created_at
             FROM projects ORDER BY created_at, id",
        )?;
        let rows = stmt
            .query_map([], row_to_project)?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    })?;
    Ok(rows)
}

fn name_exists(store: &Store, name: &str) -> Result<Option<String>, ProjectError> {
    let existing = store.with(|conn| {
        Ok(conn
            .query_row(
                "SELECT name FROM projects WHERE name = ?1",
                [name],
                |r| r.get::<_, String>(0),
            )
            .optional()?)
    })?;
    Ok(existing)
}

/// 校验项目名称：非空、不含路径分隔符（名称即 clone 目标目录名）。
fn validate_name(name: &str) -> Result<(), ProjectError> {
    let trimmed = name.trim();
    if trimmed.is_empty()
        || trimmed != name
        || trimmed == "."
        || trimmed == ".."
        || trimmed.contains('/')
        || trimmed.contains('\\')
        || name.contains(std::path::MAIN_SEPARATOR)
    {
        return Err(ProjectError::InvalidName(name.to_string()));
    }
    Ok(())
}

fn insert_git_project(
    store: &Store,
    name: &str,
    url: &str,
    path: &str,
    project_type: ProjectType,
) -> Result<Project, ProjectError> {
    let id = store.with(|conn| {
        conn.execute(
            "INSERT INTO projects (name, source, url, path, project_type)
             VALUES (?1, 'git', ?2, ?3, ?4)",
            rusqlite::params![name, url, path, project_type.as_str()],
        )?;
        Ok(conn.last_insert_rowid())
    })?;
    Ok(get(store, id)?.expect("row was just inserted"))
}

/// 读取单个项目。
pub fn get(store: &Store, id: i64) -> Result<Option<Project>, ProjectError> {
    let project = store.with(|conn| {
        Ok(conn
            .query_row(
                "SELECT id, name, source, url, path, project_type, last_branch, build_command, created_at
                 FROM projects WHERE id = ?1",
                [id],
                row_to_project,
            )
            .optional()?)
    })?;
    Ok(project)
}

/// 校验名称与 URL 的唯一性（clone 与直接建记录共用）。
pub fn validate_new_git(store: &Store, name: &str, url: &str) -> Result<(), ProjectError> {
    validate_name(name)?;
    if name_exists(store, name)?.is_some() {
        return Err(ProjectError::DuplicateName(name.to_string()));
    }
    if store
        .with(|conn| {
            Ok(conn
                .query_row(
                    "SELECT name FROM projects WHERE url = ?1",
                    [url],
                    |r| r.get::<_, String>(0),
                )
                .optional()?)
        })?
        .is_some()
    {
        return Err(ProjectError::DuplicateUrl(url.to_string()));
    }
    Ok(())
}

/// 新建（git 来源）项目记录。仅写库，不 clone；clone 流程见 [`clone_and_register`]。
///
/// 校验：名称合法且唯一、URL 未被其他项目使用。
pub fn create_git_record(
    store: &Store,
    name: &str,
    url: &str,
    path: &str,
    project_type: ProjectType,
) -> Result<Project, ProjectError> {
    validate_new_git(store, name, url)?;
    insert_git_project(store, name, url, path, project_type)
}

/// 导入已有本地路径为项目（source = local）。路径必须是已存在的目录。
///
/// 校验：名称合法且唯一、路径存在、路径未被其他项目使用。
pub fn import(
    store: &Store,
    name: &str,
    path: &Path,
    project_type: ProjectType,
) -> Result<Project, ProjectError> {
    validate_name(name)?;
    let path = path
        .canonicalize()
        .map_err(|_| ProjectError::PathNotFound(path.to_string_lossy().into_owned()))?;
    if !path.is_dir() {
        return Err(ProjectError::PathNotFound(path.to_string_lossy().into_owned()));
    }
    if name_exists(store, name)?.is_some() {
        return Err(ProjectError::DuplicateName(name.to_string()));
    }
    let path_str = path.to_string_lossy().into_owned();
    if store
        .with(|conn| {
            Ok(conn
                .query_row(
                    "SELECT name FROM projects WHERE path = ?1 AND source = 'local'",
                    [&path_str],
                    |r| r.get::<_, String>(0),
                )
                .optional()?)
        })?
        .is_some()
    {
        return Err(ProjectError::DuplicatePath(path_str));
    }

    let project = store.with(|conn| {
        conn.execute(
            "INSERT INTO projects (name, source, url, path, project_type)
             VALUES (?1, 'local', NULL, ?2, ?3)",
            rusqlite::params![name, path_str, project_type.as_str()],
        )?;
        Ok(get_in(conn, conn.last_insert_rowid())?.expect("row was just inserted"))
    })?;
    Ok(project)
}

fn get_in(conn: &rusqlite::Connection, id: i64) -> rusqlite::Result<Option<Project>> {
    conn.query_row(
        "SELECT id, name, source, url, path, project_type, last_branch, build_command, created_at
         FROM projects WHERE id = ?1",
        [id],
        row_to_project,
    )
    .optional()
}

/// 记住项目上次打包使用的分支（ticket 06）：下次打开打包面板时默认选中。
pub fn set_last_branch(store: &Store, project_id: i64, branch: &str) -> Result<(), ProjectError> {
    store.with(|conn| {
        conn.execute(
            "UPDATE projects SET last_branch = ?1 WHERE id = ?2",
            rusqlite::params![branch, project_id],
        )?;
        Ok(())
    })?;
    Ok(())
}

/// 保存项目构建命令（ticket 07）：随项目可编辑并持久化。空/纯空白拒绝。
pub fn set_build_command(
    store: &Store,
    project_id: i64,
    command: &str,
) -> Result<(), ProjectError> {
    let trimmed = command.trim();
    if trimmed.is_empty() {
        return Err(ProjectError::InvalidBuildCommand(command.to_string()));
    }
    store.with(|conn| {
        conn.execute(
            "UPDATE projects SET build_command = ?1 WHERE id = ?2",
            rusqlite::params![command.trim(), project_id],
        )?;
        Ok(())
    })?;
    Ok(())
}

/// 共享的取消标志，供 clone 任务与 `cancel_clone` 命令使用。
pub type CancelFlag = Arc<AtomicBool>;

pub fn new_cancel_flag() -> CancelFlag {
    Arc::new(AtomicBool::new(false))
}

pub fn is_cancelled(flag: &CancelFlag) -> bool {
    flag.load(Ordering::Relaxed)
}
