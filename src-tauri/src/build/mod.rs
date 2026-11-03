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
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use git2::Direction;
use serde::{Deserialize, Serialize};

use crate::store::projects::{self, CancelFlag, ProjectError, Project};
use crate::store::Store;

pub use crate::store::projects::CloneStage;

pub mod artifacts;
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
    /// 后台任务异常结束（join error）。
    #[error("background task failed: {0}")]
    Join(String),
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
            BuildError::Join(_) => "internal",
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

// ---------------------------------------------------------------------------
// 构建执行流程（ticket 07）：clone → 构建 → 清理（无论成败）
// ---------------------------------------------------------------------------

/// 单次打包过程推送给前端的事件流（一个 Channel 复用：克隆进度 + 构建日志）。
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum BuildEvent {
    Clone {
        stage: CloneStage,
        received: Option<u64>,
        total: Option<u64>,
    },
    Log {
        line: String,
    },
}

/// 一次打包的最终结果。`success=false` 表示构建命令以非零码退出（流程本身
/// 正常完成）；取消/系统错误走 `Err`。
#[derive(Debug, Clone, Serialize)]
pub struct BuildOutcome {
    pub success: bool,
    pub exit_code: Option<i32>,
    pub build_id: String,
    /// 持久化构建日志的绝对路径（失败后仍可读取）。
    pub log_path: String,
}

/// 构建日志文件：逐行追加，进程读取任务与流程本身共享。
struct LogFile(std::sync::Mutex<std::fs::File>);

impl LogFile {
    fn open(path: &Path) -> std::io::Result<Self> {
        Ok(LogFile(std::sync::Mutex::new(
            std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(path)?,
        )))
    }

    fn write(&self, line: &str) {
        use std::io::Write;
        if let Ok(mut f) = self.0.lock() {
            let _ = writeln!(f, "{line}");
        }
    }
}

/// 完整打包流程：准备临时工作区（clone）→ 用托管工具链执行项目构建命令 →
/// 无论成败删除临时工作区，日志持久化到 `<appdata>/build-logs/<id>.log`。
///
/// 依赖仓库 `<appdata>/m2-repo` 首次构建即创建且从不删除（ADR-0001）。
/// `on_event` 收到 [`BuildEvent::Clone`]（克隆进度）与 [`BuildEvent::Log`]
/// （构建输出逐行）。
pub async fn run_build_flow(
    store: Store,
    tools_root: &Path,
    appdata: &Path,
    project_id: i64,
    branch: &str,
    cancel: CancelFlag,
    on_event: Arc<dyn Fn(BuildEvent) + Send + Sync>,
) -> Result<BuildOutcome, BuildError> {
    run_build_flow_in(
        store,
        tools_root,
        appdata,
        &temp_root(),
        project_id,
        branch,
        cancel,
        on_event,
    )
    .await
}

/// [`run_build_flow`] 的可注入临时根版本（测试用）。
pub async fn run_build_flow_in(
    store: Store,
    tools_root: &Path,
    appdata: &Path,
    temp_root: &Path,
    project_id: i64,
    branch: &str,
    cancel: CancelFlag,
    on_event: Arc<dyn Fn(BuildEvent) + Send + Sync>,
) -> Result<BuildOutcome, BuildError> {
    let project = projects::get(&store, project_id)?
        .ok_or(BuildError::ProjectNotFound(project_id))?;

    // 依赖仓库目录：首次构建即创建，持久保留以复用缓存（ADR-0001）。
    let repo_local = appdata.join("m2-repo");
    std::fs::create_dir_all(&repo_local)?;

    // 每次构建一个日志文件：失败后仍可查看（票 08 做历史 UI）。
    let build_id = format!("build-{project_id}-{}", unique_suffix());
    let logs_dir = appdata.join("build-logs");
    std::fs::create_dir_all(&logs_dir)?;
    let log_path = logs_dir.join(format!("{build_id}.log"));
    let log = Arc::new(LogFile::open(&log_path)?);

    // —— 前半段：临时工作区 clone（阻塞，走 blocking 线程池）——
    let emit = on_event.clone();
    let cancel2 = cancel.clone();
    let store2 = store.clone();
    let temp_root2 = temp_root.to_path_buf();
    let branch2 = branch.to_string();
    let prepared = tokio::task::spawn_blocking(move || {
        let mut report = |p: projects::CloneProgress| {
            let _ = emit(BuildEvent::Clone {
                stage: p.stage,
                received: p.received,
                total: p.total,
            });
        };
        prepare_workspace_in(
            &store2,
            project_id,
            &branch2,
            &cancel2,
            &mut report,
            &temp_root2,
        )
    })
    .await
    .map_err(|e| BuildError::Join(e.to_string()))?;

    let workspace = match prepared {
        Ok(ws) => ws,
        Err(e) => {
            log.write(&format!("[Packsmith] 准备临时工作区失败: {e}"));
            return Err(e.into());
        }
    };
    let ws_path = PathBuf::from(&workspace.path);
    log.write(&format!("[Packsmith] 临时工作区就绪: {}", workspace.path));
    log.write(&format!("[Packsmith] 构建命令: {}", project.build_command));

    // 结果路径上都必须清理工作区；用显式清理避免遗漏分支。
    let cleanup = || {
        let _ = std::fs::remove_dir_all(&ws_path);
    };

    if projects::is_cancelled(&cancel) {
        log.write("[Packsmith] 已取消");
        cleanup();
        return Err(BuildError::Cancelled);
    }

    // —— 后半段：托管工具链执行构建 ——
    let result = (|| async {
        let tc = exec::resolve_toolchain(tools_root)
            .inspect_err(|_| log.write("[Packsmith] 托管工具链缺失，无法执行构建"))?;
        let spec = exec::build_spawn_spec(&project.build_command, &repo_local, &tc)
            .inspect_err(|e| log.write(&format!("[Packsmith] 构建命令无效: {e}")))?;

        let on_log = {
            let log = log.clone();
            let on_event = on_event.clone();
            move |line: String| {
                log.write(&line);
                let _ = on_event(BuildEvent::Log { line });
            }
        };
        exec::execute_build(spec, &ws_path, Arc::new(on_log), cancel.clone()).await
    })()
    .await;

    match result {
        Ok(outcome) => {
            if outcome.success {
                log.write(&format!(
                    "[Packsmith] 构建成功 (exit code {:?})",
                    outcome.exit_code
                ));
            } else {
                log.write(&format!(
                    "[Packsmith] 构建失败 (exit code {:?})，详见上方日志",
                    outcome.exit_code
                ));
            }
            cleanup();
            Ok(BuildOutcome {
                success: outcome.success,
                exit_code: outcome.exit_code,
                build_id,
                log_path: log_path.to_string_lossy().into_owned(),
            })
        }
        Err(e) => {
            log.write(&format!("[Packsmith] {e}"));
            cleanup();
            Err(e)
        }
    }
}

/// 读取一次构建的持久化日志。`build_id` 只允许字母数字与连字符（防路径穿越）。
pub fn read_build_log(appdata: &Path, build_id: &str) -> Result<String, BuildError> {
    let valid = !build_id.is_empty()
        && build_id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_');
    if !valid {
        return Err(BuildError::LogNotFound(build_id.to_string()));
    }
    let path = appdata.join("build-logs").join(format!("{build_id}.log"));
    std::fs::read_to_string(&path).map_err(|_| BuildError::LogNotFound(build_id.to_string()))
}
