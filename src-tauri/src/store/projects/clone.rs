//! Clone 项目到工作区根目录（git2 / libgit2，进程内完成，不依赖 PATH 上的
//! git —— ADR-0001 自包含）。
//!
//! clone 是长任务：进度经 [`CloneProgress`] 回调上报（Tauri 命令层把它接到
//! `tauri::ipc::Channel`）；取消经共享的 `AtomicBool`（命令层存入任务注册表，
//! 由 `cancel_clone` 命令置位）。取消后不残留目录、不写库。
//!
//! 认证 (ticket 13)：调用方传入加密库中配置的 Git 账号（HTTPS 用户名/令牌，
//! 见 [`crate::git_auth`]）；未配置时匿名访问公开仓库。SSH 协议 URL 明确
//! 拒绝（v1.1 仅 HTTPS 认证）。

use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::atomic::Ordering;

use git2::build::RepoBuilder;
use git2::{FetchOptions, RemoteCallbacks};
use serde::{Deserialize, Serialize};

use super::{CancelFlag, ProjectError, ProjectType, Store, validate_new_git};
// ---------------------------------------------------------------------------
// Progress
// ---------------------------------------------------------------------------

/// 克隆阶段。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum CloneStage {
    Preparing,
    Transferring,
    CheckingOut,
    Done,
}

/// 一次进度上报。`received` / `total` 仅在传输阶段有意义。
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct CloneProgress {
    pub stage: CloneStage,
    pub received: Option<u64>,
    pub total: Option<u64>,
}

impl CloneProgress {
    fn stage(stage: CloneStage) -> Self {
        Self {
            stage,
            received: None,
            total: None,
        }
    }
}

// ---------------------------------------------------------------------------
// Workspace root
// ---------------------------------------------------------------------------

/// 展开工作区根目录：`~` / `~/…` 展开为用户主目录；其余按原样（绝对或相对路径）。
pub fn expand_workspace_root(root: &str) -> PathBuf {
    if root == "~" {
        if let Some(home) = home_dir() {
            return home;
        }
    } else if let Some(rest) = root.strip_prefix("~/").or(root.strip_prefix("~\\")) {
        if let Some(home) = home_dir() {
            return home.join(rest);
        }
    }
    PathBuf::from(root)
}

fn home_dir() -> Option<PathBuf> {
    // std::env::home_dir 在 macOS/Windows 上去符号链接等问题已修复（1.85+ 取消
    // deprecation）；unix 用 HOME，windows 用 USERPROFILE，均为其行为。
    #[allow(deprecated)]
    std::env::home_dir()
}

// ---------------------------------------------------------------------------
// Clone
// ---------------------------------------------------------------------------

/// 把 `url` 克隆到 `dest`。`branch` 指定要检出的分支（打包流程按所选分支
/// clone，ticket 06）；`None` 用远端默认分支。取消标志在传输回调中轮询，
/// 置位后中止并返回 [`ProjectError::Cancelled`]（调用方负责清理半成品目录）。
/// 不存在的分支名会返回 [`ProjectError::Clone`]（libgit2 报 reference not found）。
///
/// 认证 (ticket 13)：`creds` 为已配置的 Git 账号（HTTPS 用户名/令牌）时，
/// 远程要求认证即供给 `Cred::userpass_plaintext`；`None` 保持匿名（公开
/// 仓库不受影响）。SSH 协议 URL 一律 [`ProjectError::SshNotSupported`]；
/// Auth 类失败映射为 [`ProjectError::GitAuth`]（引导去 设置 → Git 账号）。
pub fn clone_repo(
    url: &str,
    dest: &Path,
    branch: Option<&str>,
    on_progress: &mut dyn FnMut(CloneProgress),
    cancel: &CancelFlag,
    creds: Option<&crate::git_auth::GitCredentials>,
) -> Result<(), ProjectError> {
    if crate::git_auth::is_ssh_url(url) {
        return Err(ProjectError::SshNotSupported(url.to_string()));
    }
    if super::is_cancelled(cancel) {
        return Err(ProjectError::Cancelled);
    }

    // 回调是同步的，通过 Rc<RefCell<…>> 在两个 git2 回调间共享上报与取消检查。
    let reporter = Rc::new(RefCell::new(on_progress));
    let cancel = Rc::new(cancel.clone());

    let mut remote_callbacks = RemoteCallbacks::new();
    if let Some(creds) = creds {
        let attempt = crate::git_auth::AuthAttempt::new();
        let creds = creds.clone();
        // 凭据被拒时 libgit2 会再次回调；AuthAttempt 第二次起报错中止。
        remote_callbacks.credentials(move |url, username, allowed| {
            attempt.try_credential(&creds, Some(url), username, allowed)
        });
    }
    {
        let reporter = Rc::clone(&reporter);
        let cancel = Rc::clone(&cancel);
        // 返回 false 让 libgit2 中止操作（GIT_EUSER）；取消检查以此为取消通道。
        remote_callbacks.sideband_progress(move |_data| {
            if cancel.load(Ordering::Relaxed) {
                return false;
            }
            reporter.borrow_mut()(CloneProgress::stage(CloneStage::Transferring));
            true
        });
    }
    {
        let reporter = Rc::clone(&reporter);
        let cancel = Rc::clone(&cancel);
        remote_callbacks.transfer_progress(move |stats| {
            // 取消检查：返回 false 让 libgit2 中止整个操作。
            if cancel.load(Ordering::Relaxed) {
                return false;
            }
            reporter.borrow_mut()(CloneProgress {
                stage: CloneStage::Transferring,
                received: Some(stats.received_objects() as u64),
                total: Some(stats.total_objects() as u64),
            });
            true
        });
    }

    let mut fetch_options = FetchOptions::new();
    fetch_options.remote_callbacks(remote_callbacks);

    let mut builder = RepoBuilder::new();
    if let Some(branch) = branch {
        builder.branch(branch);
    }
    builder.fetch_options(fetch_options);

    on_progress_in(&reporter, CloneProgress::stage(CloneStage::Preparing));
    if let Err(e) = builder.clone(url, dest) {
        // 回调返回 false 触发 GIT_EUSER；以取消标志为准判断是否为用户取消。
        if cancel.load(Ordering::Relaxed) {
            return Err(ProjectError::Cancelled);
        }
        return Err(super::map_git_error(e));
    }
    on_progress_in(&reporter, CloneProgress::stage(CloneStage::CheckingOut));
    on_progress_in(&reporter, CloneProgress::stage(CloneStage::Done));
    Ok(())
}

fn on_progress_in(reporter: &RefCell<&mut dyn FnMut(CloneProgress)>, p: CloneProgress) {
    (reporter.borrow_mut())(p);
}

/// 完整的"新建项目"流程：校验 → clone 到工作区根目录 → 入库。
///
/// 失败（含取消）不留记录；clone 失败时清理半成品目录。`workspace_root`
/// 支持 `~` 展开；目标目录为 `<root>/<name>`，已存在则拒绝。
#[allow(clippy::too_many_arguments)]
pub fn clone_and_register(
    store: &Store,
    name: &str,
    url: &str,
    workspace_root: &str,
    project_type: ProjectType,
    cancel: &CancelFlag,
    on_progress: &mut dyn FnMut(CloneProgress),
) -> Result<super::Project, ProjectError> {
    super::validate_name(name)?;
    // clone 之前先拒绝不可创建的类型：不给无效项目留下半成品目录。
    project_type.assert_creatable()?;
    validate_new_git(store, name, url)?;

    let root = expand_workspace_root(workspace_root);
    let dest = root.join(name);
    if dest.symlink_metadata().is_ok() {
        return Err(ProjectError::DirectoryExists(
            dest.to_string_lossy().into_owned(),
        ));
    }
    if let Some(parent) = dest.parent() {
        std::fs::create_dir_all(parent)?;
    }

    // 凭据查找错误不得让 clone 崩溃：credentials_from_store 失败按匿名处理。
    let creds = crate::git_auth::credentials_from_store(store);
    if let Err(e) = clone_repo(url, &dest, None, on_progress, cancel, creds.as_ref()) {
        // 半成品目录不残留（验收：clone 失败无残留）。
        let _ = std::fs::remove_dir_all(&dest);
        return Err(e);
    }

    let dest = dest.canonicalize()?;
    super::insert_git_project(store, name, url, &dest.to_string_lossy(), project_type)
}
