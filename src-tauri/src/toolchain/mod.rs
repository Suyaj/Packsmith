//! 托管工具链：检测与下载（票 03）。
//!
//! 对前端的公开接口：
//! - `toolchain_status` → 三个工具的三态（已托管 / 已检测 / 缺失）
//! - `toolchain_download(tool, on_progress)` → 下载官方包到 `<appdata>/tools/`，
//!   经 Channel 推送进度，完成后状态刷新为已托管
//!
//! ADR-0001：PATH 检测仅用于展示；执行（后续票）只用托管工具链。

mod archive;
mod catalog;
mod download;
mod error;
mod install;
mod state;
mod status;

use std::collections::HashSet;

use serde::Serialize;
use tauri::{AppHandle, Manager};

pub use catalog::ToolKind;
pub use status::ToolState;

use install::ProgressEvent;

use error::ToolchainError;

/// 下载互斥标记：同一工具同时只允许一个下载任务（UI 按钮禁用之外的双保险）。
#[derive(Default)]
pub struct DownloadGuard(std::sync::Mutex<HashSet<ToolKind>>);

fn guard_slot_acquire(app: &AppHandle, kind: ToolKind) -> Result<(), ToolchainError> {
    let guard = app.state::<DownloadGuard>();
    let mut set = guard.0.lock().expect("download guard lock");
    if set.contains(&kind) {
        return Err(ToolchainError::Busy(kind.dir_name().to_string()));
    }
    set.insert(kind);
    Ok(())
}

fn guard_slot_release(app: &AppHandle, kind: ToolKind) {
    let guard = app.state::<DownloadGuard>();
    guard
        .0
        .lock()
        .expect("download guard lock")
        .remove(&kind);
}

/// `<appdata>/tools/`：托管工具安装根目录。
pub fn tools_root(app: &AppHandle) -> Result<std::path::PathBuf, ToolchainError> {
    let data_dir = app
        .path()
        .app_data_dir()
        .map_err(|e| ToolchainError::Io(std::io::Error::other(e.to_string())))?;
    Ok(data_dir.join("tools"))
}

/// 单个工具的设置页展示视图。
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolStatusView {
    pub tool: ToolKind,
    pub state: ToolState,
    /// 是否提供下载按钮：无托管且有官方包来源。
    pub can_download: bool,
}

fn status_view(tool: ToolKind, state: ToolState) -> ToolStatusView {
    let can_download = !state.is_managed() && catalog::current_source(tool).is_some();
    ToolStatusView {
        tool,
        state,
        can_download,
    }
}

#[tauri::command]
pub async fn toolchain_status(app: AppHandle) -> Result<Vec<ToolStatusView>, ToolchainError> {
    let root = tools_root(&app)?;
    // spawn_blocking：涉及文件系统探测，避免阻塞主线程。
    tauri::async_runtime::spawn_blocking(move || {
        Ok(status::compute_all(&root)
            .into_iter()
            .map(|(tool, state)| status_view(tool, state))
            .collect())
    })
    .await
    .map_err(|e| ToolchainError::Io(std::io::Error::other(e.to_string())))?
}

#[tauri::command]
pub async fn toolchain_download(
    app: AppHandle,
    tool: ToolKind,
    on_progress: tauri::ipc::Channel<ProgressEvent>,
) -> Result<ToolStatusView, ToolchainError> {
    guard_slot_acquire(&app, tool)?;
    let root = tools_root(&app)?;
    let channel = on_progress.clone();
    let result = install::install_tool(tool, &root, move |event| {
        // 进度事件推送失败不影响安装本身（前端可能已关闭页面）。
        let _ = channel.send(event);
    })
    .await;
    guard_slot_release(&app, tool);
    result?;

    let state = status::compute_status(tool, &root);
    Ok(status_view(tool, state))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::toolchain::install::Phase;

    #[test]
    fn 有官方包且未托管才提供下载按钮() {
        // macOS：mvnd/jdk 有源、git 没有
        if std::env::consts::OS == "macos" {
            assert!(!status_view(ToolKind::Git, ToolState::Missing).can_download);
            assert!(status_view(ToolKind::Mvnd, ToolState::Missing).can_download);
            assert!(status_view(ToolKind::Jdk, ToolState::Missing).can_download);
            // 已检测（PATH）也可一键托管（票 03 决策）
            assert!(status_view(
                ToolKind::Mvnd,
                ToolState::Detected {
                    path: "/opt/homebrew/bin/mvnd".into()
                }
            )
            .can_download);
            // 已托管不再提供下载
            assert!(!status_view(
                ToolKind::Mvnd,
                ToolState::Managed {
                    version: "1.0.6".into(),
                    path: "mvnd/bin/mvnd".into()
                }
            )
            .can_download);
        }
    }

    #[test]
    fn 进度事件序列化形态稳定() {
        let json = serde_json::to_string(&ProgressEvent {
            tool: ToolKind::Mvnd,
            phase: Phase::Downloading,
            received: 1024,
            total: Some(2048),
        })
        .unwrap();
        assert_eq!(
            json,
            r#"{"tool":"mvnd","phase":"downloading","received":1024,"total":2048}"#
        );
    }

    #[test]
    fn 工具状态序列化tag形态稳定() {
        let managed = serde_json::to_string(&ToolState::Managed {
            version: "8u512".into(),
            path: "jdk/Contents/Home/bin/java".into(),
        })
        .unwrap();
        assert_eq!(
            managed,
            r#"{"state":"managed","version":"8u512","path":"jdk/Contents/Home/bin/java"}"#
        );
        let detected = serde_json::to_string(&ToolState::Detected {
            path: "/usr/bin/git".into(),
        })
        .unwrap();
        assert_eq!(detected, r#"{"state":"detected","path":"/usr/bin/git"}"#);
        assert_eq!(
            serde_json::to_string(&ToolState::Missing).unwrap(),
            r#"{"state":"missing"}"#
        );
    }
}
