//! 设置相关 Tauri commands（ticket 11）：关闭窗口行为。
//!
//! 跨 IPC 的值是 `'minimize' | 'quit'` 字符串（[`crate::settings`] 的持久化
//! 契约）；非法值在写入侧即被拒绝，读取侧永远返回合法值。

use tauri::{AppHandle, Manager};

use crate::commands::ApiError;
use crate::settings::{self, CloseWindowBehavior, CloseWindowBehaviorParseError};
use crate::store::Store;

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
    Ok(settings::get_close_window_behavior(&store)?.as_str().to_string())
}

/// 保存关闭窗口行为设置（设置页切换后立即生效——下一次关窗即按新值处置）。
#[tauri::command]
pub fn set_close_window_behavior(app: AppHandle, behavior: String) -> Result<(), ApiError> {
    let behavior: CloseWindowBehavior = behavior.parse()?;
    let store = app.state::<Store>();
    settings::set_close_window_behavior(&store, behavior)?;
    Ok(())
}
