//! 流式下载：带进度回调，落盘到指定文件。
//!
//! 连接策略：默认客户端 + 15s 连接超时；连接失败时回退强制 IPv4。
//! 背景：hyper 无 happy-eyeballs，部分网络环境下 github.com 的 IPv6
//! 路由会长时间挂起（实测 IPv4 直连正常），回退可避免"永远转圈"。

use std::net::IpAddr;
use std::path::Path;
use std::time::Duration;

use futures_util::StreamExt;
use tokio::io::{AsyncSeekExt, AsyncWriteExt};

use crate::toolchain::error::ToolchainError;

const CONNECT_TIMEOUT: Duration = Duration::from_secs(15);

fn http_client(ipv4_only: bool) -> reqwest::Client {
    let mut builder = reqwest::Client::builder().connect_timeout(CONNECT_TIMEOUT);
    if ipv4_only {
        builder = builder.local_address(IpAddr::from([0, 0, 0, 0]));
    }
    builder.build().expect("http client builds")
}

async fn send_get(url: &str) -> Result<reqwest::Response, ToolchainError> {
    // 尝试序列：默认（双栈）→ 强制 IPv4 → 稍候再强制 IPv4。
    // 本机网络对 github 的 IPv6 路由会挂起、IPv4 也偶发抖动，重试可吸收。
    let mut last_err: Option<reqwest::Error> = None;
    for (attempt, ipv4_only) in [false, true, true].into_iter().enumerate() {
        if attempt > 1 {
            tokio::time::sleep(Duration::from_secs(2)).await;
        }
        match http_client(ipv4_only).get(url).send().await {
            Ok(response) => {
                return response
                    .error_for_status()
                    .map_err(|e| ToolchainError::Http(format!("{e}")));
            }
            Err(e) => last_err = Some(e),
        }
    }
    Err(ToolchainError::Http(format!(
        "{last_err}",
        last_err = last_err.expect("at least one attempt")
    )))
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
    const MAX_ATTEMPTS: usize = 4;
    for attempt in 0..MAX_ATTEMPTS {
        if attempt > 0 {
            tokio::time::sleep(Duration::from_secs(2)).await;
        }
        let mut response = if attempt == 0 || received == 0 {
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
async fn send_get_range(
    url: &str,
    offset: u64,
) -> Result<reqwest::Response, ToolchainError> {
    let mut last_err: Option<reqwest::Error> = None;
    for ipv4_only in [false, true] {
        match http_client(ipv4_only)
            .get(url)
            .header(reqwest::header::RANGE, format!("bytes={offset}-"))
            .send()
            .await
        {
            Ok(response) => {
                return response
                    .error_for_status()
                    .map_err(|e| ToolchainError::Http(format!("{e}")));
            }
            Err(e) => last_err = Some(e),
        }
    }
    Err(ToolchainError::Http(format!(
        "{last_err}",
        last_err = last_err.expect("at least one attempt")
    )))
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
