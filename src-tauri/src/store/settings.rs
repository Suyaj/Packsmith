//! 应用设置（ticket 08）：以 `app_meta` key/value 表为存储的小型设置层。
//!
//! v1 仅一个设置项：构件库保留次数 [`BUILD_RETENTION_KEY`]（每个项目保留
//! 最近 N 次构建的构件与日志）。读取侧永远给出可用值：未设置或值损坏时
//! 回退 [`DEFAULT_BUILD_RETENTION`]。

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
            .query_row(
                "SELECT value FROM app_meta WHERE key = ?1",
                [key],
                |r| r.get::<_, String>(0),
            )
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
