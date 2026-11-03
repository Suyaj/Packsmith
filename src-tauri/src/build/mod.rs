//! 打包（一）：分支选择与临时工作区 (ticket 06)。
//!
//! 本模块负责打包流程的前半段：实时拉取分支列表（[`list_branches`]）与
//! 临时工作区准备（[`prepare_workspace`]）。构建执行本身属于 ticket 07。
//!
//! - 分支列表为**实时**拉取：git2 匿名远端连接（`git ls-remote` 的进程内
//!   等价物，ADR-0001：不依赖 PATH 上的 git），远程新建分支立即可见。
//! - 临时工作区位于系统临时目录、由 Packsmith 统一命名管理
//!   （`packsmith-build-<项目名>-<时间戳>`）；clone 失败不留残留目录。
//! - 上次选择的分支持久化在项目记录上（`projects.last_branch`，迁移 v4）。

use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use git2::Direction;
use serde::{Deserialize, Serialize};

use crate::store::projects::{self, CancelFlag, ProjectError, Project};
use crate::store::Store;

pub mod commands;
pub mod exec;

/// 构建执行（ticket 07）错误：稳定 `code` 供前端 i18n。
#[derive(Debug, thiserror::Error)]
pub enum BuildError {
    /// 托管工具链缺 mvnd 或 JDK（ADR-0001：不回退 PATH）。
    #[error("managed toolchain missing: {0}")]
    ToolchainMissing(&'static str),
    /// 构建命令的第一个词不是 mvnd/mvn（v1 仅 Maven）。
    #[error("unsupported build command: {0}")]
    UnsupportedBuildCommand(String),
    /// 构建命令为空、引号未闭合等无法分词。
    #[error("invalid build command: {0}")]
    InvalidBuildCommand(String),
    /// 子进程启动失败。
    #[error("failed to spawn build process: {0}")]
    Spawn(String),
    /// 构建被用户取消（子进程已被杀死）。
    #[error("build was cancelled")]
    Cancelled,
    /// 同一项目已有打包任务进行中。
    #[error("a build is already running for this project")]
    AlreadyRunning,
    /// 构建日志不存在（id 非法或文件已被清理）。
    #[error("build log not found: {0}")]
    LogNotFound(String),
    /// 指定 id 的项目不存在。
    #[error("project {0} not found")]
    ProjectNotFound(i64),
    #[error("store error: {0}")]
    Store(#[from] crate::store::StoreError),
    #[error("project error: {0}")]
    Project(#[from] ProjectError),
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
}

impl BuildError {
    /// A stable machine-readable tag for the frontend to key i18n on.
    pub fn code(&self) -> &'static str {
        match self {
            BuildError::ToolchainMissing(_) => "toolchain_missing",
            BuildError::UnsupportedBuildCommand(_) => "unsupported_build_command",
            BuildError::InvalidBuildCommand(_) => "invalid_build_command",
            BuildError::Spawn(_) => "spawn_failed",
            BuildError::Cancelled => "cancelled",
            BuildError::AlreadyRunning => "already_running",
            BuildError::LogNotFound(_) => "log_not_found",
            BuildError::ProjectNotFound(_) => "project_not_found",
            BuildError::Store(_) => "store",
            BuildError::Project(_) => "project_not_found",
            BuildError::Io(_) => "io",
        }
    }
}

/// 打包临时工作区就绪后的信息。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PreparedWorkspace {
    /// 临时工作区目录（绝对路径）。ticket 07 在此目录执行构建，结束后删除。
    pub path: String,
}

// ---------------------------------------------------------------------------
// 实时分支列表（git ls-remote 的进程内等价物）
// ---------------------------------------------------------------------------

/// 实时拉取 `url` 的远端分支列表（仅 `refs/heads/*`，前缀剥除，不含 HEAD
/// 与 tag）。`url` 也接受本地仓库路径（本地导入的项目走同一通道）。
pub fn list_branches(url: &str) -> Result<Vec<String>, ProjectError> {
    // git2 需要一个仓库来创建匿名远端；开一个一次性的空 bare 仓库当跳板，
    // 用完即删。
    let scratch = std::env::temp_dir().join(format!("packsmith-lsremote-{}", unique_suffix()));
    let repo = git2::Repository::init_bare(&scratch)
        .map_err(|e| ProjectError::BranchListing(e.message().to_string()))?;
    let result = list_branches_via(&repo, url);
    let _ = std::fs::remove_dir_all(&scratch);
    result
}

fn list_branches_via(repo: &git2::Repository, url: &str) -> Result<Vec<String>, ProjectError> {
    let list = || -> Result<Vec<String>, ProjectError> {
        // 匿名远端：不写入仓库配置，等价于 `git ls-remote <url>`。
        let mut remote = repo
            .remote_anonymous(url)
            .map_err(|e| ProjectError::BranchListing(e.message().to_string()))?;
        let connection = remote
            .connect_auth(Direction::Fetch, None, None)
            .map_err(|e| ProjectError::BranchListing(e.message().to_string()))?;
        let heads = connection
            .list()
            .map_err(|e| ProjectError::BranchListing(e.message().to_string()))?;
        let mut branches: Vec<String> = heads
            .iter()
            .map(|head| head.name())
            .filter_map(|name| name.strip_prefix("refs/heads/"))
            .filter(|name| !name.is_empty())
            .map(str::to_string)
            .collect();        branches.sort();
        Ok(branches)
    };
    list()
}

fn unique_suffix() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0)
}

// ---------------------------------------------------------------------------
// 临时工作区准备
// ---------------------------------------------------------------------------

/// 打包临时工作区的根：系统临时目录（spec: 临时工作区位于系统临时目录）。
fn temp_root() -> PathBuf {
    std::env::temp_dir()
}

/// Packsmith 统一管理的临时工作区命名：`packsmith-build-<项目名>-<时间戳>`。
/// 统一前缀便于（后续）清理孤儿目录；项目名不含路径分隔符（入库时校验）。
pub fn temp_workspace_path(project_name: &str, root: &Path) -> PathBuf {
    root.join(format!(
        "packsmith-build-{}-{}",
        project_name,
        unique_suffix()
    ))
}

/// 完整的"准备临时工作区"流程：创建 Packsmith 命名的临时目录 → clone 指定
/// 分支 → 记住上次分支。
///
/// 失败（含取消、无效分支名）不留临时目录，也不更新 last_branch。成功返回
/// 就绪的临时工作区路径；删除它属于打包结束流程（ticket 07），本函数不删。
pub fn prepare_workspace(
    store: &Store,
    project_id: i64,
    branch: &str,
    cancel: &CancelFlag,
    on_progress: &mut dyn FnMut(projects::CloneProgress),
) -> Result<PreparedWorkspace, ProjectError> {
    prepare_workspace_in(
        store,
        project_id,
        branch,
        cancel,
        on_progress,
        &temp_root(),
    )
}

/// [`prepare_workspace`] 的可注入临时根版本（测试用）。
pub fn prepare_workspace_in(
    store: &Store,
    project_id: i64,
    branch: &str,
    cancel: &CancelFlag,
    on_progress: &mut dyn FnMut(projects::CloneProgress),
    root: &Path,
) -> Result<PreparedWorkspace, ProjectError> {
    let project = get_project(store, project_id)?;
    let url = source_url(&project);

    let dest = temp_workspace_path(&project.name, root);
    std::fs::create_dir_all(&dest)?;

    if let Err(e) = projects::clone_repo(&url, &dest, Some(branch), on_progress, cancel) {
        // 失败不留残留（验收：临时工作区被清理）。
        let _ = std::fs::remove_dir_all(&dest);
        return Err(e);
    }

    projects::set_last_branch(store, project_id, branch)?;
    Ok(PreparedWorkspace {
        path: dest.to_string_lossy().into_owned(),
    })
}

fn get_project(store: &Store, project_id: i64) -> Result<Project, ProjectError> {
    projects::get(store, project_id)?.ok_or(ProjectError::ProjectNotFound(project_id))
}

/// 克隆来源：git 项目用其 URL；本地导入的项目直接对本地路径做 ls-remote /
/// clone（同一 git2 通道，本地路径由 libgit2 本地传输支持）。
fn source_url(project: &Project) -> String {
    project
        .url
        .clone()
        .unwrap_or_else(|| project.path.clone())
}
