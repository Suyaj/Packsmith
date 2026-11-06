//! 流式下载：带进度回调，落盘到指定文件。
//!
//! 连接策略：hyper 不做 happy-eyeballs——对解析出的多个地址只连第一个，
//! 而本机网络会封锁 github.com 的部分地址（curl 能连上、reqwest 超时）。
//! 因此自行解析地址并逐个回退；连接超时 8s/地址。传输中途断流则按
//! HTTP Range 断点续传。

use std::net::{SocketAddr, ToSocketAddrs};
use std::path::Path;
use std::time::Duration;

use futures_util::StreamExt;
use tokio::io::{AsyncSeekExt, AsyncWriteExt};

use crate::toolchain::error::ToolchainError;

const CONNECT_TIMEOUT: Duration = Duration::from_secs(8);
const MAX_ADDRS: usize = 5;

/// 拆出 (host, port)。官方源一律 https；http 仅放行环回地址（测试用）。
fn host_port(url: &str) -> Result<(String, u16), ToolchainError> {
    let (scheme, rest) = url
        .split_once("://")
        .ok_or_else(|| ToolchainError::Http(format!("URL 缺少协议：{url}")))?;
    let authority = rest.split('/').next().unwrap_or_default();
    let host_no_port = authority
        .rsplit_once(':')
        .map(|(h, _)| h)
        .unwrap_or(authority);
    match (scheme, host_no_port) {
        ("https", _) => {}
        ("http", "localhost" | "127.0.0.1" | "::1") => {}
        _ => return Err(ToolchainError::Http(format!("仅支持 https 下载：{url}"))),
    }
    Ok(match authority.rsplit_once(':') {
        Some((host, port)) => (
            host.to_string(),
            port.parse()
                .map_err(|_| ToolchainError::Http(format!("端口非法：{url}")))?,
        ),
        None => (
            authority.to_string(),
            if scheme == "https" { 443 } else { 80 },
        ),
    })
}

/// 解析域名全部地址（阻塞解析放 blocking 线程池）。
async fn resolve_addrs(host: &str, port: u16) -> Result<Vec<SocketAddr>, ToolchainError> {
    let host = host.to_string();
    tauri::async_runtime::spawn_blocking(move || {
        Ok((host.as_str(), port).to_socket_addrs()?.collect::<Vec<_>>())
    })
    .await
    .map_err(|e| ToolchainError::Http(format!("解析任务失败：{e}")))?
    .map_err(|e: std::io::Error| ToolchainError::Http(format!("域名解析失败：{e}")))
}

/// 逐地址回退的 GET。连接级失败换下一个地址；HTTP 状态错误不重试
///（交由调用方按业务处理）。`range_from` 为 Some 时带 Range 头。
async fn send_get_inner(
    url: &str,
    range_from: Option<u64>,
) -> Result<reqwest::Response, ToolchainError> {
    let (host, port) = host_port(url)?;
    let addrs = resolve_addrs(&host, port).await?;
    if addrs.is_empty() {
        return Err(ToolchainError::Http(format!("域名解析为空：{host}")));
    }
    let mut last_err: Option<reqwest::Error> = None;
    for addr in addrs.iter().take(MAX_ADDRS) {
        let client = reqwest::Client::builder()
            .connect_timeout(CONNECT_TIMEOUT)
            .resolve_to_addrs(&host, std::slice::from_ref(addr))
            .build()
            .map_err(|e| ToolchainError::Http(format!("{e}")))?;
        let mut request = client.get(url);
        if let Some(offset) = range_from {
            request = request.header(reqwest::header::RANGE, format!("bytes={offset}-"));
        }
        match request.send().await {
            Ok(response) => return Ok(response),
            Err(e) => last_err = Some(e),
        }
    }
    Err(ToolchainError::Http(format!(
        "{last_err}",
        last_err = last_err.expect("at least one address attempted")
    )))
}

async fn send_get(url: &str) -> Result<reqwest::Response, ToolchainError> {
    send_get_inner(url, None).await
}

/// 下载 `url` 到 `dest`，每个数据块回调（已接收字节, 总字节）。
/// 返回总接收字节数。进度值是单调递增的累计值，供前端计算百分比。
///
/// 网络抖动下的韧性：连接失败按 send_get 的尝试序列重试；传输中途断流
/// 则用 HTTP Range 从已收字节断点续传（服务器不支持 Range 时整段重来）。
pub async fn download_to<F>(
    url: &str,
    dest: &Path,
    mut on_progress: F,
) -> Result<u64, ToolchainError>
where
    F: FnMut(u64, Option<u64>) + Send,
{
    let _ = tokio::fs::remove_file(dest).await;
    if let Some(parent) = dest.parent() {
        tokio::fs::create_dir_all(parent).await?;
    }
    let mut file = tokio::fs::OpenOptions::new()
        .create(true)
        .write(true)
        .open(dest)
        .await?;

    let mut received: u64 = 0;
    let mut total: Option<u64> = None;
    const MAX_ATTEMPTS: usize = 8;
    for attempt in 0..MAX_ATTEMPTS {
        if attempt > 0 {
            tokio::time::sleep(Duration::from_secs(2)).await;
        }
        let response = if attempt == 0 || received == 0 {
            if received > 0 {
                received = 0;
                file.set_len(0).await?;
                file.seek(std::io::SeekFrom::Start(0)).await?;
            }
            send_get(url).await?
        } else {
            // 断点续传
            send_get_range(url, received).await?
        };
        match response.status() {
            reqwest::StatusCode::PARTIAL_CONTENT => { /* 续传：追加写入 */ }
            reqwest::StatusCode::OK => {
                if attempt > 0 {
                    // 服务器不支持 Range：整段重来
                    received = 0;
                    file.set_len(0).await?;
                    file.seek(std::io::SeekFrom::Start(0)).await?;
                }
            }
            status => return Err(ToolchainError::Http(format!("意外的响应状态 {status}"))),
        }
        if attempt == 0 || total.is_none() {
            total = response.content_length();
        }
        on_progress(received, total);

        let mut stream = response.bytes_stream();
        let mut broken: Option<reqwest::Error> = None;
        while let Some(item) = stream.next().await {
            match item {
                Ok(chunk) => {
                    file.write_all(&chunk).await?;
                    received += chunk.len() as u64;
                    on_progress(received, total);
                }
                Err(e) => {
                    broken = Some(e);
                    break;
                }
            }
        }
        match broken {
            None => {
                file.flush().await?;
                if let Some(t) = total {
                    if received != t {
                        return Err(ToolchainError::Http(format!(
                            "下载不完整：{received}/{t} 字节"
                        )));
                    }
                }
                return Ok(received);
            }
            Some(e) => {
                if attempt + 1 == MAX_ATTEMPTS {
                    return Err(ToolchainError::Http(format!("{e}")));
                }
                // 落盘已收字节，进入下一轮断点续传
            }
        }
    }
    unreachable!("循环内必然 return")
}

/// 带断点续传请求头的 GET（期望 206 Partial Content）。
async fn send_get_range(url: &str, offset: u64) -> Result<reqwest::Response, ToolchainError> {
    send_get_inner(url, Some(offset)).await
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 真实网络 smoke：走 download_to 下载一个小文件（mvnd release 的
    /// 源码包，约 830KB，该 release 无独立校验和资产）。手动运行：
    /// `cargo test -p packsmith -- --ignored smoke`
    #[tokio::test]
    #[ignore = "真实网络下载 smoke 测试，手动运行"]
    async fn smoke_真实下载小文件并校验字节数() {
        let dir = std::env::temp_dir().join(format!("ps-download-smoke-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let dest = dir.join("maven-mvnd-1.0.6-src.zip");
        let url =
            "https://github.com/apache/maven-mvnd/releases/download/1.0.6/maven-mvnd-1.0.6-src.zip";

        let mut calls = 0;
        let received = download_to(url, &dest, |_, _| calls += 1)
            .await
            .expect("真实下载小文件");

        let bytes = std::fs::read(&dest).unwrap();
        assert_eq!(received as usize, bytes.len());
        assert!(calls > 0);
        assert!(bytes.len() > 800 * 1024); // 完整源码包
        assert_eq!(&bytes[..2], b"PK"); // zip 魔数
        std::fs::remove_dir_all(&dir).ok();
    }
}
