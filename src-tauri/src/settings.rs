//! 应用设置（ticket 11）：关闭窗口行为。
//!
//! 设置项持久化在加密库的 `app_meta` 键值表（迁移 v7 播种默认值）。
//! 关窗行为的**决策**是纯函数 [`close_action`]：窗口关闭事件与托盘菜单共用
//! 它的结论——最小化到托盘（默认）或真正退出；托盘菜单的"退出应用"不经过
//! 该函数，始终退出。

use crate::store::{Store, StoreError};

/// `app_meta` 中关闭窗口行为设置项的键。
pub const CLOSE_WINDOW_BEHAVIOR_KEY: &str = "close_window_behavior";

/// 关闭窗口行为：最小化到托盘（默认）或退出应用。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum CloseWindowBehavior {
    /// 点窗口关闭按钮 = 隐藏到托盘，应用继续运行（默认）。
    #[default]
    Minimize,
    /// 点窗口关闭按钮 = 真正退出应用。
    Quit,
}

impl CloseWindowBehavior {
    /// 持久化字符串形式（前端 IPC 与迁移播种共用同一契约）。
    pub fn as_str(self) -> &'static str {
        match self {
            CloseWindowBehavior::Minimize => "minimize",
            CloseWindowBehavior::Quit => "quit",
        }
    }
}

/// 非法设置字符串（持久化契约只有 `'minimize' | 'quit'`）。
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("unknown close_window_behavior: {0:?}")]
pub struct CloseWindowBehaviorParseError(String);

impl std::str::FromStr for CloseWindowBehavior {
    type Err = CloseWindowBehaviorParseError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "minimize" => Ok(CloseWindowBehavior::Minimize),
            "quit" => Ok(CloseWindowBehavior::Quit),
            other => Err(CloseWindowBehaviorParseError(other.to_string())),
        }
    }
}

/// 窗口关闭事件的实际处置。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CloseAction {
    /// 拦截关闭，隐藏窗口（托盘常驻）。
    HideToTray,
    /// 放行关闭，应用退出。
    Quit,
}

/// 决策纯函数：给定设置，点关闭按钮时该做什么。
pub fn close_action(behavior: CloseWindowBehavior) -> CloseAction {
    match behavior {
        CloseWindowBehavior::Minimize => CloseAction::HideToTray,
        CloseWindowBehavior::Quit => CloseAction::Quit,
    }
}

/// 读取关闭窗口行为设置。缺行或值无法识别时回落到默认（最小化到托盘）——
/// 设置页永远能打开，行为永远有定义。
pub fn get_close_window_behavior(
    store: &Store,
) -> Result<CloseWindowBehavior, StoreError> {
    let raw: Option<String> = store.with(|conn| {
        conn.query_row(
            "SELECT value FROM app_meta WHERE key = ?1",
            [CLOSE_WINDOW_BEHAVIOR_KEY],
            |r| r.get(0),
        )
        .map(Some)
        .or_else(|e| match e {
            rusqlite::Error::QueryReturnedNoRows => Ok(None),
            other => Err(other),
        })
    })?;
    Ok(raw
        .and_then(|v| v.parse::<CloseWindowBehavior>().ok())
        .unwrap_or_default())
}

/// 保存关闭窗口行为设置（UPSERT）。
pub fn set_close_window_behavior(
    store: &Store,
    behavior: CloseWindowBehavior,
) -> Result<(), StoreError> {
    store.with(|conn| {
        conn.execute(
            "INSERT INTO app_meta (key, value) VALUES (?1, ?2)
             ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            [CLOSE_WINDOW_BEHAVIOR_KEY, behavior.as_str()],
        )?;
        Ok(())
    })
}
