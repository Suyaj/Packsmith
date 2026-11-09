//! 应用设置（ticket 08）：以 `app_meta` key/value 表为存储的小型设置层。
//!
//! 设置项：构件库保留次数 [`BUILD_RETENTION_KEY`]（每个项目保留最近 N 次
//! 构建的构件与日志，未设置/值损坏时回退 [`DEFAULT_BUILD_RETENTION`]）、
//! 关闭窗口行为、Git 账号（ticket 13：用户名 + 密码/访问令牌，入加密库
//! ADR-0002 —— `app_meta` 迁移 v1 起即提供加密 KV 存储，因此无需新迁移）。
//! Git 令牌从不回传前端明文（掩码模式：编辑留空=保持不变，见命令层）。
//! mvnd settings.xml（ticket 18）内容同样入加密库，由命令层物化为
//! `<appdata>/mvnd-settings.xml`（构建 spawn 从磁盘读取）。
//! 工具链下载代理 URL（ticket 27）为普通 KV（非机密），空 = 不使用代理。

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

// ---------------------------------------------------------------------------
// mvnd settings.xml (ticket 18)
// ---------------------------------------------------------------------------

/// mvnd 自定义 settings.xml 内容（私服依赖）。入加密库（ADR-0002），命令层
/// 在保存/清除时把它物化到 `<appdata>/mvnd-settings.xml`（构建 spawn 从磁盘
/// 读取）。
pub const MVND_SETTINGS_XML_KEY: &str = "mvnd_settings_xml";

/// 读取 mvnd settings.xml 内容；未设置为 `None`。
pub fn get_mvnd_settings_xml(store: &Store) -> Result<Option<String>, StoreError> {
    get(store, MVND_SETTINGS_XML_KEY)
}

/// 写入 mvnd settings.xml 内容（原样存储；空白即视为清除）。
pub fn set_mvnd_settings_xml(store: &Store, content: &str) -> Result<(), StoreError> {
    if content.trim().is_empty() {
        return remove(store, MVND_SETTINGS_XML_KEY);
    }
    set(store, MVND_SETTINGS_XML_KEY, content)
}

/// 清除 mvnd settings.xml 内容。
pub fn clear_mvnd_settings_xml(store: &Store) -> Result<(), StoreError> {
    remove(store, MVND_SETTINGS_XML_KEY)
}

// ---------------------------------------------------------------------------
// 工具链下载代理 (ticket 27)
// ---------------------------------------------------------------------------

/// 工具链下载代理 URL（如 `http://127.0.0.1:7890`）。空/未设置 = 不使用代理。
pub const TOOLCHAIN_PROXY_URL_KEY: &str = "toolchain_proxy_url";

/// 读取工具链下载代理 URL；未设置为 `None`。
pub fn get_toolchain_proxy_url(store: &Store) -> Result<Option<String>, StoreError> {
    get(store, TOOLCHAIN_PROXY_URL_KEY)
}

/// 写入工具链下载代理 URL（去首尾空白；空白即视为清除；非空必须是
/// `http://`/`https://` 开头的 URL——下载层只支持 HTTP(S) 代理，票 27 决策）。
pub fn set_toolchain_proxy_url(store: &Store, url: &str) -> Result<(), StoreError> {
    let trimmed = url.trim();
    if trimmed.is_empty() {
        return remove(store, TOOLCHAIN_PROXY_URL_KEY);
    }
    if !(trimmed.starts_with("http://") || trimmed.starts_with("https://")) {
        return Err(StoreError::InvalidSetting(format!(
            "代理地址必须以 http:// 或 https:// 开头，got {trimmed}"
        )));
    }
    set(store, TOOLCHAIN_PROXY_URL_KEY, trimmed)
}

fn remove(store: &Store, key: &str) -> Result<(), StoreError> {
    store.with(|conn| {
        conn.execute("DELETE FROM app_meta WHERE key = ?1", [key])?;
        Ok(())
    })
}
