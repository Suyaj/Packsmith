//! Git HTTPS 认证 (ticket 13)：设置页配置的 Git 账号在 clone / 实时拉取分支
//! 列表时自动供给 git2 认证回调（v1.1 仅 HTTPS 认证）。
//!
//! - SSH 协议 URL（`git@host:path` / `ssh://…`）不支持认证：调用方在上游
//!   一次性拒绝并给出明确中文提示（[`is_ssh_url`]）。
//! - 凭据存加密库（ADR-0002，`store::settings`）；[`credentials_from_store`]
//!   是唯一的读取口：用户名与令牌齐备才算已配置，**查找失败按未配置处理**
//!   ——凭据查找错误绝不让 clone / 分支拉取崩溃。
//! - 令牌只进 git2 认证回调，从不进日志、错误信息或 IPC 出参。
//!
//! git2 本地传输忽略凭据，端到端"私有仓库认证"无法密闭测试；此处只放纯
//! 决策逻辑，选择逻辑的测试见 `tests/git_auth_test.rs`。

use std::cell::Cell;

use git2::{Cred, CredentialType, Error as GitError};

use crate::store::Store;
use crate::store::settings;

/// 已配置的 Git 账号（HTTPS 认证用用户名 + 密码/访问令牌）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GitCredentials {
    pub username: String,
    pub token: String,
}

/// URL 是否为 SSH 协议（v1.1 不支持 SSH 认证，明确拒绝而非静默失败）：
/// - `ssh://…`、`git+ssh://…`
/// - scp 语法 `user@host:path`（冒号前有 `@` 且不含 `://`；Windows 盘符
///   路径 `C:\…` 的冒号前没有 `@`，不受影响）
pub fn is_ssh_url(url: &str) -> bool {
    let u = url.trim();
    if u.starts_with("ssh://") || u.starts_with("git+ssh://") {
        return true;
    }
    // scp 语法：scheme-less，且 '@' 出现在第一个 ':' 之前（排除 "C:\x"）。
    if u.contains("://") {
        return false;
    }
    match (u.find('@'), u.find(':')) {
        (Some(at), Some(colon)) => at < colon,
        _ => false,
    }
}

/// 从加密库读取 Git 账号；未设置（或读取失败）返回 `None`——调用方保持
/// 匿名访问（公开仓库不受影响）。
pub fn credentials_from_store(store: &Store) -> Option<GitCredentials> {
    let username = settings::get_git_username(store).ok().flatten()?;
    let token = settings::get_git_token(store).ok().flatten()?;
    if username.is_empty() || token.is_empty() {
        return None;
    }
    Some(GitCredentials { username, token })
}

/// 一次 git 操作内的认证提示计数：libgit2 在凭据被拒时会再次回调；继续
/// 供给同样凭据会无限循环，因此第二次起直接报错中止。
pub struct AuthAttempt(Cell<u32>);

impl AuthAttempt {
    pub fn new() -> Self {
        AuthAttempt(Cell::new(0))
    }

    /// 为一次认证提示选择凭据（内部计数：同一操作第二次提示即视为凭据被拒）。
    ///
    /// v1 一律用配置的用户名（GitHub 等令牌认证忽略 URL 中的用户名提示）。
    /// SSH URL 已在上游被拒绝；若服务器仍要求非用户名/令牌认证（或默认凭据），
    /// 返回匿名默认凭据或明确报错。
    pub fn try_credential(
        &self,
        creds: &GitCredentials,
        _url: Option<&str>,
        username_hint: Option<&str>,
        allowed: CredentialType,
    ) -> Result<Cred, GitError> {
        let _ = username_hint;
        if allowed.contains(CredentialType::USER_PASS_PLAINTEXT) {
            if self.0.get() > 0 {
                return Err(GitError::new(
                    git2::ErrorCode::Auth,
                    git2::ErrorClass::Http,
                    "服务器拒绝了已配置的 Git 凭据",
                ));
            }
            self.0.set(self.0.get() + 1);
            return Cred::userpass_plaintext(&creds.username, &creds.token);
        }
        if allowed.contains(CredentialType::DEFAULT) {
            // 服务器未指明认证方式：先试匿名默认凭据（公开仓库照常工作）。
            return Cred::default();
        }
        Err(GitError::new(
            git2::ErrorCode::Auth,
            git2::ErrorClass::Http,
            "服务器要求 SSH 密钥认证，Packsmith v1.1 仅支持 HTTPS 用户名/令牌认证",
        ))
    }
}

impl Default for AuthAttempt {
    fn default() -> Self {
        Self::new()
    }
}
