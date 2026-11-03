//! 应用设置（ticket 08）：以 `app_meta` key/value 表为存储的小型设置层。
//!
//! 设置项：构件库保留次数 [`BUILD_RETENTION_KEY`]（每个项目保留最近 N 次
//! 构建的构件与日志，未设置/值损坏时回退 [`DEFAULT_BUILD_RETENTION`]）、
//! 关闭窗口行为、Git 账号（ticket 13：用户名 + 密码/访问令牌，入加密库
//! ADR-0002 —— `app_meta` 迁移 v1 起即提供加密 KV 存储，因此无需新迁移）。
//! Git 令牌从不回传前端明文（掩码模式：编辑留空=保持不变，见命令层）。

use rusqlite::OptionalExtension;

use super::{Store, StoreError};

/// 构件库保留次数（每项目保留最近 N 次构建）。
pub const BUILD_RETENTION_KEY: &str = "retention_builds";

/// 保留次数默认值。
pub const DEFAULT_BUILD_RETENTION: i64 = 10;

/// 读取通用设置项。
pub fn get(store: &Store, key: &str) -> Result<Option<String>, StoreError> {
    store.with(|conn| {
        Ok(conn
            .query_row("SELECT value FROM app_meta WHERE key = ?1", [key], |r| {
                r.get::<_, String>(0)
            })
            .optional()?)
    })
}

/// 写入通用设置项（存在则覆盖）。
pub fn set(store: &Store, key: &str, value: &str) -> Result<(), StoreError> {
    store.with(|conn| {
        conn.execute(
            "INSERT INTO app_meta (key, value) VALUES (?1, ?2)
             ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            rusqlite::params![key, value],
        )?;
        Ok(())
    })?;
    Ok(())
}

/// 读取保留次数：未设置/损坏回退默认值。
pub fn get_build_retention(store: &Store) -> Result<i64, StoreError> {
    Ok(get(store, BUILD_RETENTION_KEY)?
        .and_then(|v| v.trim().parse::<i64>().ok())
        .filter(|n| *n >= 1)
        .unwrap_or(DEFAULT_BUILD_RETENTION))
}

/// 写入保留次数（必须 ≥ 1）。
pub fn set_build_retention(store: &Store, n: i64) -> Result<(), StoreError> {
    if n < 1 {
        return Err(StoreError::InvalidSetting(format!(
            "retention must be >= 1, got {n}"
        )));
    }
    set(store, BUILD_RETENTION_KEY, &n.to_string())
}

// ---------------------------------------------------------------------------
// Git 账号 (ticket 13)
// ---------------------------------------------------------------------------

/// Git 账号用户名（clone / 分支拉取的 HTTPS 认证）。
pub const GIT_USERNAME_KEY: &str = "git_username";
/// Git 账号密码 / 访问令牌。仅存于加密库（ADR-0002），从不回传前端明文。
pub const GIT_TOKEN_KEY: &str = "git_token";

/// 读取 Git 账号用户名；未设置为 `None`。
pub fn get_git_username(store: &Store) -> Result<Option<String>, StoreError> {
    get(store, GIT_USERNAME_KEY)
}

/// 读取 Git 账号密码 / 访问令牌；未设置为 `None`。
pub fn get_git_token(store: &Store) -> Result<Option<String>, StoreError> {
    get(store, GIT_TOKEN_KEY)
}

/// 写入 Git 账号用户名（去首尾空白；空白即视为清除）。
pub fn set_git_username(store: &Store, username: &str) -> Result<(), StoreError> {
    let trimmed = username.trim();
    if trimmed.is_empty() {
        return remove(store, GIT_USERNAME_KEY);
    }
    set(store, GIT_USERNAME_KEY, trimmed)
}

/// 写入 Git 账号密码 / 访问令牌（去首尾空白；空白即视为清除）。
pub fn set_git_token(store: &Store, token: &str) -> Result<(), StoreError> {
    let trimmed = token.trim();
    if trimmed.is_empty() {
        return remove(store, GIT_TOKEN_KEY);
    }
    set(store, GIT_TOKEN_KEY, trimmed)
}

/// 清除 Git 账号（用户名与密码/令牌一起删除）。
pub fn clear_git_account(store: &Store) -> Result<(), StoreError> {
    remove(store, GIT_USERNAME_KEY)?;
    remove(store, GIT_TOKEN_KEY)
}

fn remove(store: &Store, key: &str) -> Result<(), StoreError> {
    store.with(|conn| {
        conn.execute("DELETE FROM app_meta WHERE key = ?1", [key])?;
        Ok(())
    })
}
