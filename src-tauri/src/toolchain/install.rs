//! 安装流程：下载官方包 → 解压到临时目录 → 校验可执行文件 →
//! 原子替换 `tools/<dir>/` → 登记状态文件。
//!
//! 失败可重试：每次安装先清理上次残留的临时目录与下载缓存。

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

use crate::toolchain::archive::extract_archive;
use crate::toolchain::catalog::{current_source, ArchiveFormat, ToolKind};
use crate::toolchain::download::download_to;
use crate::toolchain::error::ToolchainError;
use crate::toolchain::status::{find_executable_in, register_managed};

/// 发给前端的进度事件（Tauri Channel）。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProgressEvent {
    pub tool: ToolKind,
    pub phase: Phase,
    pub received: u64,
    pub total: Option<u64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Phase {
    Downloading,
    Extracting,
    Installing,
}

/// 安装一个工具到 `tools_root`。完成后状态文件登记，UI 刷新为已托管。
/// 返回安装的版本号。
pub async fn install_tool(
    kind: ToolKind,
    tools_root: &Path,
    mut on_progress: impl FnMut(ProgressEvent) + Send,
) -> Result<String, ToolchainError> {
    let source = current_source(kind).ok_or_else(|| ToolchainError::NoSourceForPlatform {
        tool: kind.dir_name().to_string(),
        os: std::env::consts::OS.to_string(),
        arch: std::env::consts::ARCH.to_string(),
    })?;

    cleanup_stale(tools_root, kind)?;

    let file_name = source
        .url
        .rsplit('/')
        .next()
        .unwrap_or("package")
        .to_string();
    let archive_path = tools_root.join(".downloads").join(&file_name);
    let url = source.url.clone();

    download_to(&url, &archive_path, &mut |received, total| {
        on_progress(ProgressEvent {
            tool: kind,
            phase: Phase::Downloading,
            received,
            total,
        })
    })
    .await?;

    let extracted_to =
        extract_step(kind, tools_root, archive_path, source.format, source.strip_top, &mut on_progress)
            .await?;

    let exec =
        find_executable_in(kind, &extracted_to).ok_or_else(|| {
            ToolchainError::Archive("解压后未找到工具可执行文件".to_string())
        })?;
    let exec_rel = exec.strip_prefix(&extracted_to).unwrap_or(&exec).to_path_buf();

    on_progress(ProgressEvent {
        tool: kind,
        phase: Phase::Installing,
        received: 1,
        total: Some(1),
    });

    // 临时目录与最终目录同盘（都在 tools_root 下），rename 原子完成替换。
    let final_dir = tools_root.join(kind.dir_name());
    if final_dir.exists() {
        // 旧版本先挪走，改名成功后再删；失败可回滚。
        let backup = tools_root.join(format!(".old-{}", kind.dir_name()));
        let _ = std::fs::remove_dir_all(&backup);
        std::fs::rename(&final_dir, &backup)?;
        if let Err(e) = std::fs::rename(&extracted_to, &final_dir) {
            std::fs::rename(&backup, &final_dir)?;
            return Err(e.into());
        }
        let _ = std::fs::remove_dir_all(&backup);
    } else {
        std::fs::rename(&extracted_to, &final_dir)?;
    }

    let tools_rel = Path::new(kind.dir_name()).join(&exec_rel);
    register_managed(kind, tools_root, source.version, &tools_rel)?;

    let _ = std::fs::remove_file(tools_root.join(".downloads").join(&file_name));

    Ok(source.version.to_string())
}

/// 解压到 `tools_root/.tmp-<dir>`（与最终目录同盘，保证 rename 原子）。
/// 解压是 CPU/磁盘密集操作，放 blocking 线程池。
async fn extract_step(
    kind: ToolKind,
    tools_root: &Path,
    archive_path: PathBuf,
    format: ArchiveFormat,
    strip_top: bool,
    on_progress: &mut (impl FnMut(ProgressEvent) + Send),
) -> Result<PathBuf, ToolchainError> {
    on_progress(ProgressEvent {
        tool: kind,
        phase: Phase::Extracting,
        received: 0,
        total: None,
    });
    let tmp_dir = tools_root.join(format!(".tmp-{}", kind.dir_name()));
    tauri::async_runtime::spawn_blocking(move || {
        std::fs::create_dir_all(&tmp_dir)?;
        extract_archive(&archive_path, &tmp_dir, format, strip_top).map(|_| tmp_dir)
    })
    .await
    .map_err(|e| ToolchainError::Archive(format!("解压任务失败：{e}")))?
    .map_err(ToolchainError::from)
}

/// 清理上次失败留下的临时目录与下载缓存（重试入口）。
fn cleanup_stale(tools_root: &Path, kind: ToolKind) -> std::io::Result<()> {
    let tmp = tools_root.join(format!(".tmp-{}", kind.dir_name()));
    if tmp.exists() {
        std::fs::remove_dir_all(&tmp)?;
    }
    let downloads = tools_root.join(".downloads");
    if downloads.exists() {
        for entry in std::fs::read_dir(downloads)? {
            let path = entry?.path();
            let _ = std::fs::remove_file(path); // 缓存一律重新下载
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::toolchain::status::compute_status;
    use crate::toolchain::status::ToolState;

    fn temp_root(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "packsmith-install-test-{}-{}",
            tag,
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    #[tokio::test]
    async fn 无官方包的平台报错且不落地() {
        // git 在 macOS 无官方预编译包（catalog 已测）；验证 install 入口的报错路径。
        if std::env::consts::OS != "macos" {
            return;
        }
        let tools_root = temp_root("nosource");
        std::fs::create_dir_all(&tools_root).unwrap();

        let result = install_tool(ToolKind::Git, &tools_root, |_| {}).await;
        assert!(matches!(
            result.unwrap_err(),
            ToolchainError::NoSourceForPlatform { .. }
        ));
        assert!(!tools_root.join("git").exists());
        let _ = std::fs::remove_dir_all(&tools_root);
    }

    /// 真实网络 smoke：下载 mvnd 官方包（约 23MB）装进临时目录，
    /// 状态应变为已托管且可执行文件存在。手动运行：
    /// `cargo test -p packsmith -- --ignored smoke`
    #[tokio::test]
    #[ignore = "真实网络下载 smoke 测试，手动运行"]
    async fn smoke_真实下载mvnd并登记为已托管() {
        let tools_root = temp_root("smoke");
        std::fs::create_dir_all(&tools_root).unwrap();

        let mut phases = Vec::new();
        let version = install_tool(ToolKind::Mvnd, &tools_root, |e| phases.push(e.phase))
            .await
            .expect("真实下载安装 mvnd");

        assert_eq!(version, "1.0.6");
        assert!(tools_root.join("mvnd/bin/mvnd").is_file());
        assert!(phases.contains(&Phase::Downloading));
        assert!(phases.contains(&Phase::Extracting));
        assert!(matches!(
            compute_status(ToolKind::Mvnd, &tools_root),
            ToolState::Managed { .. }
        ));
        let _ = std::fs::remove_dir_all(&tools_root);
    }
}
