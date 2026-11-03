//! 设置相关 Tauri commands（ticket 11：关闭窗口行为；ticket 13：Git 账号）。
//!
//! 跨 IPC 的值是 `'minimize' | 'quit'` 字符串（[`crate::settings`] 的持久化
//! 契约）；非法值在写入侧即被拒绝，读取侧永远返回合法值。

use serde::Serialize;
use tauri::{AppHandle, Manager};

use crate::commands::ApiError;
use crate::settings::{self, CloseWindowBehavior, CloseWindowBehaviorParseError};
use crate::store::Store;
use crate::store::settings as store_settings;

impl From<CloseWindowBehaviorParseError> for ApiError {
    fn from(e: CloseWindowBehaviorParseError) -> Self {
        ApiError::raw("invalid_close_window_behavior", e.to_string())
    }
}

impl From<crate::store::StoreError> for ApiError {
    fn from(e: crate::store::StoreError) -> Self {
        ApiError::raw("store", e.to_string())
    }
}

/// 读取关闭窗口行为设置（`'minimize'`（默认）或 `'quit'`）。
#[tauri::command]
pub fn get_close_window_behavior(app: AppHandle) -> Result<String, ApiError> {
    let store = app.state::<Store>();
    Ok(settings::get_close_window_behavior(&store)?
        .as_str()
        .to_string())
}

/// 保存关闭窗口行为设置（设置页切换后立即生效——下一次关窗即按新值处置）。
#[tauri::command]
pub fn set_close_window_behavior(app: AppHandle, behavior: String) -> Result<(), ApiError> {
    let behavior: CloseWindowBehavior = behavior.parse()?;
    let store = app.state::<Store>();
    settings::set_close_window_behavior(&store, behavior)?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Git 账号 (ticket 13)：clone / 分支拉取的 HTTPS 认证凭据，入加密库
// ---------------------------------------------------------------------------

/// Git 账号出参（掩码模式，沿用服务器凭据模式）：用户名非机密可直接展示；
/// 令牌只给"是否已设置"指示，**永不明文回传前端**。编辑时令牌留空 =
/// 保持不变（前端不回传），显式"清除账号"走 [`clear_git_account`]。
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GitAccountDto {
    pub username: Option<String>,
    pub has_token: bool,
}

/// 读取 Git 账号状态（设置页卡片展示）。
#[tauri::command]
pub fn get_git_account(app: AppHandle) -> Result<GitAccountDto, ApiError> {
    let store = app.state::<Store>();
    git_account_dto(&store)
}

/// 保存 Git 账号。
///
/// `token` 两态：字段缺席或 `null` → **保持现有令牌不变**（编辑表单留空即
/// 保持）；有值 → 更新。单独清令牌无 UI 入口——显式"清除账号"用
/// [`clear_git_account`]（用户名与令牌一起删除）。用户名必填（非空白），
/// 否则返回 `git_username_required`——不落库，现有账号原样保留。
#[tauri::command]
pub fn set_git_account(
    app: AppHandle,
    username: String,
    token: Option<String>,
) -> Result<GitAccountDto, ApiError> {
    let store = app.state::<Store>();
    apply_git_account(&store, &username, token)
}

/// 显式清除 Git 账号（用户名与令牌一起删除）。
#[tauri::command]
pub fn clear_git_account(app: AppHandle) -> Result<GitAccountDto, ApiError> {
    let store = app.state::<Store>();
    clear_git_account_in(&store)
}

fn git_account_dto(store: &Store) -> Result<GitAccountDto, ApiError> {
    let username = store_settings::get_git_username(store).map_err(ApiError::from)?;
    let has_token = store_settings::get_git_token(store)
        .map_err(ApiError::from)?
        .is_some();
    Ok(GitAccountDto { username, has_token })
}

fn apply_git_account(
    store: &Store,
    username: &str,
    token: Option<String>,
) -> Result<GitAccountDto, ApiError> {
    if username.trim().is_empty() {
        return Err(ApiError::raw(
            "git_username_required",
            "Git 用户名不能为空",
        ));
    }
    store_settings::set_git_username(store, username).map_err(ApiError::from)?;
    // token 为 None（字段缺席）：保持现有令牌不变（编辑表单令牌留空）。
    if let Some(token) = token {
        store_settings::set_git_token(store, &token).map_err(ApiError::from)?;
    }
    git_account_dto(store)
}

fn clear_git_account_in(store: &Store) -> Result<GitAccountDto, ApiError> {
    store_settings::clear_git_account(store).map_err(ApiError::from)?;
    git_account_dto(store)
}

#[cfg(test)]
mod git_account_tests {
    use super::*;
    use crate::store::settings as store_settings;

    const TEST_KEY_HEX: &str =
        "a1b2c3d4e5f60718293a4b5c6d7e8f90a1b2c3d4e5f60718293a4b5c6d7e8f90";

    fn open(dir: &std::path::Path) -> crate::store::Store {
        crate::store::Store::open(&dir.join("test.db"), TEST_KEY_HEX).unwrap()
    }

    #[test]
    fn apply_saves_username_and_token_and_dto_never_contains_token_plaintext() {
        let dir = tempfile::tempdir().unwrap();
        let store = open(dir.path());

        let dto = apply_git_account(&store, "octocat", Some("ghp_secret".into())).unwrap();

        assert_eq!(dto.username.as_deref(), Some("octocat"));
        assert!(dto.has_token);
        let json = serde_json::to_string(&dto).unwrap();
        assert!(!json.contains("ghp_secret"), "令牌明文绝不回传前端: {json}");
        assert_eq!(
            store_settings::get_git_token(&store).unwrap().as_deref(),
            Some("ghp_secret")
        );
    }

    #[test]
    fn token_field_absent_keeps_existing_token() {
        let dir = tempfile::tempdir().unwrap();
        let store = open(dir.path());
        apply_git_account(&store, "octocat", Some("ghp_secret".into())).unwrap();

        let dto = apply_git_account(&store, "renamed", None).unwrap();

        assert_eq!(dto.username.as_deref(), Some("renamed"));
        assert!(dto.has_token, "编辑留空令牌 = 保持不变");
        assert_eq!(
            store_settings::get_git_token(&store).unwrap().as_deref(),
            Some("ghp_secret")
        );
    }

    #[test]
    fn token_null_also_keeps_existing_token() {
        // 两态契约：None（字段缺席或 null）= 保持不变；单独清令牌无 UI 入口，
        // 清除账号走 clear_git_account。
        let dir = tempfile::tempdir().unwrap();
        let store = open(dir.path());
        apply_git_account(&store, "octocat", Some("ghp_secret".into())).unwrap();

        let dto = apply_git_account(&store, "octocat", None).unwrap();

        assert!(dto.has_token);
        assert_eq!(
            store_settings::get_git_token(&store).unwrap().as_deref(),
            Some("ghp_secret")
        );
    }

    #[test]
    fn blank_username_is_rejected_without_touching_the_account() {
        let dir = tempfile::tempdir().unwrap();
        let store = open(dir.path());
        apply_git_account(&store, "octocat", Some("ghp_secret".into())).unwrap();

        let err = apply_git_account(&store, "   ", None).unwrap_err();

        assert_eq!(err.code(), "git_username_required");
        // 校验失败不落库：现有账号原样保留。
        assert_eq!(store_settings::get_git_username(&store).unwrap().as_deref(), Some("octocat"));
        assert!(store_settings::get_git_token(&store).unwrap().is_some());
    }

    #[test]
    fn clear_git_account_removes_username_and_token() {
        let dir = tempfile::tempdir().unwrap();
        let store = open(dir.path());
        apply_git_account(&store, "octocat", Some("ghp_secret".into())).unwrap();

        let dto = clear_git_account_in(&store).unwrap();

        assert_eq!(dto.username, None);
        assert!(!dto.has_token);
        assert_eq!(store_settings::get_git_username(&store).unwrap(), None);
        assert_eq!(store_settings::get_git_token(&store).unwrap(), None);
    }
}
