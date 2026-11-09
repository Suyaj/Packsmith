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
use std::time::Duration;

use serde::Serialize;
use tauri::{AppHandle, Manager};

pub use catalog::ToolKind;
pub use status::{ToolState, find_executable};

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
    guard.0.lock().expect("download guard lock").remove(&kind);
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
    // 代理策略在下载开始前解析一次，贯穿整个下载（票 27）：
    // 显式配置 > 环境变量 > 直连，配了代理失败不回退直连。
    let configured_proxy = {
        let store = app.state::<crate::store::Store>();
        crate::store::settings::get_toolchain_proxy_url(&store)?
    };
    let proxy = download::DownloadProxy::resolve(configured_proxy.as_deref())?;
    let channel = on_progress.clone();
    let result = install::install_tool(tool, &root, &proxy, move |event| {
        // 进度事件推送失败不影响安装本身（前端可能已关闭页面）。
        let _ = channel.send(event);
    })
    .await;
    guard_slot_release(&app, tool);
    result?;

    let state = status::compute_status(tool, &root);
    Ok(status_view(tool, state))
}

// ---------------------------------------------------------------------------
// 工具链下载代理（票 27）
// ---------------------------------------------------------------------------

/// 读取工具链下载代理 URL（未设置为 `null`；空串不会持久化）。
#[tauri::command]
pub fn toolchain_proxy_get(app: AppHandle) -> Result<Option<String>, ToolchainError> {
    let store = app.state::<crate::store::Store>();
    Ok(crate::store::settings::get_toolchain_proxy_url(&store)?)
}

/// 保存工具链下载代理 URL（空/空白即清除；非空须为 http(s) URL）。
#[tauri::command]
pub fn toolchain_proxy_set(app: AppHandle, proxy_url: String) -> Result<(), ToolchainError> {
    let store = app.state::<crate::store::Store>();
    Ok(crate::store::settings::set_toolchain_proxy_url(
        &store, &proxy_url,
    )?)
}

/// 测试代理连通性（设置页"测试连接"按钮）：对 `https://github.com` 发 HEAD
/// 请求，成功返回耗时毫秒。始终用入参代理——测试的是**输入框当前值**，不
/// 依赖是否已保存。
#[tauri::command]
pub async fn toolchain_proxy_test(proxy_url: String) -> Result<u64, ToolchainError> {
    const TEST_URL: &str = "https://github.com";
    const TOTAL_TIMEOUT: Duration = Duration::from_secs(15);
    let url = proxy_url.trim();
    if url.is_empty() {
        return Err(ToolchainError::Http("代理地址为空".to_string()));
    }
    let proxy =
        reqwest::Proxy::all(url).map_err(|e| ToolchainError::Http(format!("代理地址非法：{e}")))?;
    let client = reqwest::Client::builder()
        .connect_timeout(download::CONNECT_TIMEOUT)
        .timeout(TOTAL_TIMEOUT)
        .no_proxy()
        .proxy(proxy)
        .build()
        .map_err(|e| ToolchainError::Http(format!("{e}")))?;
    let started = std::time::Instant::now();
    client
        .head(TEST_URL)
        .send()
        .await
        .and_then(|r| r.error_for_status())
        .map_err(|e| ToolchainError::Http(format!("经代理连接 {TEST_URL} 失败：{e}")))?;
    Ok(started.elapsed().as_millis() as u64)
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
            assert!(
                status_view(
                    ToolKind::Mvnd,
                    ToolState::Detected {
                        path: "/opt/homebrew/bin/mvnd".into()
                    }
                )
                .can_download
            );
            // 已托管不再提供下载
            assert!(
                !status_view(
                    ToolKind::Mvnd,
                    ToolState::Managed {
                        version: "1.0.6".into(),
                        path: "mvnd/bin/mvnd".into()
                    }
                )
                .can_download
            );
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
