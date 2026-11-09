//! 安装流程：下载官方包 → 解压到临时目录 → 校验可执行文件 →
//! 原子替换 `tools/<dir>/` → 登记状态文件。
//!
//! 失败可重试：每次安装先清理上次残留的临时目录与下载缓存。

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

use crate::toolchain::archive::extract_archive;
use crate::toolchain::catalog::{ArchiveFormat, ToolKind, ToolSource, current_source};
use crate::toolchain::download::{DownloadProxy, download_to};
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
/// 返回安装的版本号。`proxy` 为本次下载的连接策略（票 27）。
pub async fn install_tool(
    kind: ToolKind,
    tools_root: &Path,
    proxy: &DownloadProxy,
    on_progress: impl FnMut(ProgressEvent) + Send,
) -> Result<String, ToolchainError> {
    let source = current_source(kind).ok_or_else(|| ToolchainError::NoSourceForPlatform {
        tool: kind.dir_name().to_string(),
        os: std::env::consts::OS.to_string(),
        arch: std::env::consts::ARCH.to_string(),
    })?;
    install_from(kind, tools_root, &source, proxy, on_progress).await
}

/// 按给定安装包来源安装（测试可指向本地服务，覆盖 catalog 查表）。
pub async fn install_from(
    kind: ToolKind,
    tools_root: &Path,
    source: &ToolSource,
    proxy: &DownloadProxy,
    mut on_progress: impl FnMut(ProgressEvent) + Send,
) -> Result<String, ToolchainError> {
    cleanup_stale(tools_root, kind)?;

    let file_name = source
        .url
        .rsplit('/')
        .next()
        .unwrap_or("package")
        .to_string();
    let archive_path = tools_root.join(".downloads").join(&file_name);
    let url = source.url.clone();

    download_to(&url, &archive_path, proxy, &mut |received, total| {
        on_progress(ProgressEvent {
            tool: kind,
            phase: Phase::Downloading,
            received,
            total,
        })
    })
    .await?;

    let extracted_to = extract_step(
        kind,
        tools_root,
        archive_path,
        source.format,
        source.strip_top,
        &mut on_progress,
    )
    .await?;

    let exec = find_executable_in(kind, &extracted_to)
        .ok_or_else(|| ToolchainError::Archive("解压后未找到工具可执行文件".to_string()))?;
    ensure_owner_exec(&exec)?;
    let exec_rel = exec
        .strip_prefix(&extracted_to)
        .unwrap_or(&exec)
        .to_path_buf();

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

/// 校验注册的可执行文件有属主执行位；缺失则 chmod 0755 自愈——覆盖无
/// unix_mode 信息的安装包（Windows 打包常见，曾致 spawn os error 13）。
/// 自愈失败即安装失败：坏安装不允许流到 spawn 阶段。
#[cfg(unix)]
fn ensure_owner_exec(path: &Path) -> Result<(), ToolchainError> {
    use std::os::unix::fs::PermissionsExt;
    ensure_owner_exec_with(path, |p, mode| {
        std::fs::set_permissions(p, std::fs::Permissions::from_mode(mode))
    })
}

/// chmod 可注入的变体（单测覆盖自愈失败路径）。
#[cfg(unix)]
fn ensure_owner_exec_with(
    path: &Path,
    chmod: impl Fn(&Path, u32) -> std::io::Result<()>,
) -> Result<(), ToolchainError> {
    use std::os::unix::fs::PermissionsExt;
    let mode = std::fs::metadata(path)?.permissions().mode();
    if mode & 0o100 == 0 {
        chmod(path, 0o755).map_err(|e| ToolchainError::ExecBit {
            path: path.display().to_string(),
            detail: e.to_string(),
        })?;
    }
    Ok(())
}

/// 非 Unix 平台无执行位概念，原样通过。
#[cfg(not(unix))]
fn ensure_owner_exec(_path: &Path) -> Result<(), ToolchainError> {
    Ok(())
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
    use crate::toolchain::status::ToolState;
    use crate::toolchain::status::compute_status;

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

        let result = install_tool(ToolKind::Git, &tools_root, &DownloadProxy::Direct, |_| {}).await;
        assert!(matches!(
            result.unwrap_err(),
            ToolchainError::NoSourceForPlatform { .. }
        ));
        assert!(!tools_root.join("git").exists());
        let _ = std::fs::remove_dir_all(&tools_root);
    }

    /// 起单连接本地 HTTP 服务返回给定字节（真实 socket），返回 URL。
    fn serve_once(body: Vec<u8>) -> String {
        use std::io::{Read as _, Write as _};
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        std::thread::spawn(move || {
            if let Ok((mut stream, _)) = listener.accept() {
                let mut buf = [0u8; 4096];
                let _ = stream.read(&mut buf);
                let response = format!(
                    "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    body.len()
                );
                let _ = stream.write_all(response.as_bytes());
                let _ = stream.write_all(&body);
            }
        });
        format!("http://127.0.0.1:{port}/package.zip")
    }

    /// 组装官方布局的安装包字节（顶层目录 + bin/mvnd）。
    fn build_official_zip(dir: &Path, mode: Option<u32>) -> Vec<u8> {
        let zip_path = dir.join("mvnd.zip");
        {
            let file = std::fs::File::create(&zip_path).unwrap();
            let mut zip = zip::ZipWriter::new(file);
            let mut options = zip::write::SimpleFileOptions::default();
            if let Some(mode) = mode {
                options = options.unix_permissions(mode);
            }
            zip.start_file("maven-mvnd-1.0.6-darwin-aarch64/bin/mvnd", options)
                .unwrap();
            std::io::Write::write_all(&mut zip, b"#!/bin/sh\necho mvnd").unwrap();
            zip.finish().unwrap();
        }
        std::fs::read(&zip_path).unwrap()
    }

    /// 全链路 smoke（确定性）：起本地 HTTP 服务提供与官方包完全相同
    /// 布局（顶层目录 + bin/mvnd）的安装包，走 download_to（真实 socket、
    /// 真实 HTTP）→ 解压 → 登记 → 三态判定。远程下载部分由两个
    /// 真实网络 smoke 分别覆盖。
    #[tokio::test]
    async fn 全链路_本地服务安装mvnd并登记为已托管() {
        let dir = temp_root("local-srv");
        std::fs::create_dir_all(&dir).unwrap();
        let body = build_official_zip(&dir, None);
        let url = serve_once(body);

        let tools_root = temp_root("local-install");
        std::fs::create_dir_all(&tools_root).unwrap();
        let source = ToolSource {
            url,
            format: ArchiveFormat::Zip,
            version: "1.0.6",
            strip_top: true,
        };

        let mut events = Vec::new();
        let version = install_from(
            ToolKind::Mvnd,
            &tools_root,
            &source,
            &DownloadProxy::Direct,
            |e| events.push(e),
        )
        .await
        .expect("本地全链路安装");

        assert_eq!(version, "1.0.6");
        assert_eq!(
            std::fs::read_to_string(tools_root.join("mvnd/bin/mvnd")).unwrap(),
            "#!/bin/sh\necho mvnd"
        );
        assert!(events.iter().any(|e| e.phase == Phase::Downloading));
        assert!(events.iter().any(|e| e.phase == Phase::Extracting));
        assert!(matches!(
            compute_status(ToolKind::Mvnd, &tools_root),
            ToolState::Managed { .. }
        ));
        // 状态文件已登记
        let state = crate::toolchain::state::ManagedState::load(&tools_root);
        assert_eq!(state.tools.get("mvnd").unwrap().version, "1.0.6");
        let _ = std::fs::remove_dir_all(&tools_root);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// 回归：官方包在 Windows 打包时条目无 unix_mode，解压落 0644；
    /// 安装校验应 chmod 0755 自愈，而非把坏安装流到 spawn 阶段
    /// （spawn Permission denied, os error 13）。
    #[tokio::test]
    #[cfg(unix)]
    async fn 无模式信息的zip安装自愈出执行位() {
        let dir = temp_root("modeless-zip");
        std::fs::create_dir_all(&dir).unwrap();
        let body = build_official_zip(&dir, None);
        let url = serve_once(body);

        let tools_root = temp_root("modeless-install");
        std::fs::create_dir_all(&tools_root).unwrap();
        let source = ToolSource {
            url,
            format: ArchiveFormat::Zip,
            version: "1.0.6",
            strip_top: true,
        };

        install_from(
            ToolKind::Mvnd,
            &tools_root,
            &source,
            &DownloadProxy::Direct,
            |_| {},
        )
        .await
        .expect("modeless 安装应自愈成功");

        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(tools_root.join("mvnd/bin/mvnd"))
            .unwrap()
            .permissions()
            .mode();
        assert_eq!(mode & 0o111, 0o111, "自愈后应有执行位，实际 mode={mode:o}");
        let _ = std::fs::remove_dir_all(&tools_root);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// chmod 失败必须报安装错误（坏安装不允许产生），而非静默通过。
    #[test]
    #[cfg(unix)]
    fn 自愈chmod失败时报安装错误() {
        let dir = temp_root("exec-bit-fail");
        std::fs::create_dir_all(&dir).unwrap();
        let exec = dir.join("mvnd"); // 0644：无属主执行位，才会走到 chmod
        std::fs::write(&exec, "#!/bin/sh").unwrap();

        let result = ensure_owner_exec_with(&exec, |_p, _m| {
            Err(std::io::Error::other("注入的 chmod 失败"))
        });
        match result {
            Err(ToolchainError::ExecBit { path, detail }) => {
                assert_eq!(path, exec.to_string_lossy());
                assert!(detail.contains("注入的 chmod 失败"));
            }
            other => panic!("应为 ExecBit 错误，实际 {other:?}"),
        }
        let _ = std::fs::remove_dir_all(&dir);
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
        let version = install_tool(ToolKind::Mvnd, &tools_root, &DownloadProxy::Direct, |e| {
            phases.push(e.phase)
        })
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
