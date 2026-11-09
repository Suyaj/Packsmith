//! 流式下载：带进度回调，落盘到指定文件。
//!
//! 连接策略（票 27，ADR-0004）：代理优先级为设置页显式配置 > 环境变量
//! （reqwest 默认读取 `https_proxy`/`all_proxy`）> 直连。配置了代理（显式或
//! env）就走普通 client，**不回退直连**；未配置代理时走逐地址直连回退——
//! hyper 不做 happy-eyeballs，对解析出的多个地址只连第一个，而本机网络会
//! 封锁 github.com 的部分地址（curl 能连上、reqwest 超时），因此自行解析
//! 地址并逐个回退；连接超时 8s/地址。传输中途断流则按 HTTP Range 断点续传。

use std::net::{SocketAddr, ToSocketAddrs};
use std::path::Path;
use std::time::Duration;

use futures_util::StreamExt;
use tokio::io::{AsyncSeekExt, AsyncWriteExt};

use crate::toolchain::error::ToolchainError;

pub(crate) const CONNECT_TIMEOUT: Duration = Duration::from_secs(8);
const MAX_ADDRS: usize = 5;

/// 下载连接策略（票 27）：代理解析结果，一次下载解析一次、贯穿始终。
#[derive(Debug, Clone)]
pub enum DownloadProxy {
    /// 设置页显式配置的代理 URL。失败不回退直连（显式配置说明直连不通）。
    Explicit(reqwest::Proxy),
    /// 环境变量代理（`https_proxy`/`all_proxy`）：交给 reqwest 默认处理。
    Env,
    /// 无代理：逐地址直连回退。
    Direct,
}

impl DownloadProxy {
    /// 解析代理：显式配置 > 环境变量 > 直连。
    pub fn resolve(configured: Option<&str>) -> Result<Self, ToolchainError> {
        Self::resolve_with(configured, &|name| {
            std::env::var(name).ok().filter(|v| !v.trim().is_empty())
        })
    }

    /// 同 [`resolve`](Self::resolve)，env 读取可注入（测试用）。
    fn resolve_with(
        configured: Option<&str>,
        env: &dyn Fn(&str) -> Option<String>,
    ) -> Result<Self, ToolchainError> {
        if let Some(url) = configured.map(str::trim).filter(|s| !s.is_empty()) {
            // 仅支持 HTTP(S) 代理（票 27 决策）；reqwest 对 Proxy::all 的解析
            // 很宽松（"not-a-url" 也能过），协议前缀在这里显式把关。
            if !(url.starts_with("http://") || url.starts_with("https://")) {
                return Err(ToolchainError::Http(format!(
                    "代理地址必须以 http:// 或 https:// 开头：{url}"
                )));
            }
            let proxy = reqwest::Proxy::all(url)
                .map_err(|e| ToolchainError::Http(format!("代理地址非法：{e}")))?;
            return Ok(Self::Explicit(proxy));
        }
        // 与 reqwest 的 env 代理检测对齐：all_proxy 优先，https_proxy 次之
        //（大小写都查）。检测到就走普通 client，由 reqwest 读 env。
        for name in ["all_proxy", "ALL_PROXY", "https_proxy", "HTTPS_PROXY"] {
            if env(name).is_some() {
                return Ok(Self::Env);
            }
        }
        Ok(Self::Direct)
    }
}

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

/// 按 [`DownloadProxy`] 策略发起 GET。代理路径（显式/env）不解析本地
/// DNS、不做逐地址回退——连接目标由代理决定；直连路径逐地址回退。
/// HTTP 状态错误不重试（交由调用方按业务处理）。`range_from` 为 Some 时带
/// Range 头。
async fn send_get_inner(
    url: &str,
    range_from: Option<u64>,
    proxy: &DownloadProxy,
) -> Result<reqwest::Response, ToolchainError> {
    let mut request = match proxy {
        // 显式代理：no_proxy() 关掉 env 代理检测，保证只用用户配的这一个
        DownloadProxy::Explicit(p) => {
            let client = reqwest::Client::builder()
                .connect_timeout(CONNECT_TIMEOUT)
                .no_proxy()
                .proxy(p.clone())
                .build()
                .map_err(|e| ToolchainError::Http(format!("{e}")))?;
            client.get(url)
        }
        // env 代理：reqwest 默认读取；不做本地 DNS 解析（代理端解析）
        DownloadProxy::Env => {
            let client = reqwest::Client::builder()
                .connect_timeout(CONNECT_TIMEOUT)
                .build()
                .map_err(|e| ToolchainError::Http(format!("{e}")))?;
            client.get(url)
        }
        DownloadProxy::Direct => return send_get_direct(url, range_from).await,
    };
    if let Some(offset) = range_from {
        request = request.header(reqwest::header::RANGE, format!("bytes={offset}-"));
    }
    request
        .send()
        .await
        .map_err(|e| ToolchainError::Http(format!("{e}")))
}

/// 逐地址回退的直连 GET。连接级失败换下一个地址。
async fn send_get_direct(
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

async fn send_get(url: &str, proxy: &DownloadProxy) -> Result<reqwest::Response, ToolchainError> {
    send_get_inner(url, None, proxy).await
}

/// 下载 `url` 到 `dest`，每个数据块回调（已接收字节, 总字节）。
/// 返回总接收字节数。进度值是单调递增的累计值，供前端计算百分比。
/// `proxy` 为本次下载的连接策略（下载开始前解析一次）。
///
/// 网络抖动下的韧性：连接失败按 send_get 的尝试序列重试；传输中途断流
/// 则用 HTTP Range 从已收字节断点续传（服务器不支持 Range 时整段重来）。
pub async fn download_to<F>(
    url: &str,
    dest: &Path,
    proxy: &DownloadProxy,
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
            send_get(url, proxy).await?
        } else {
            // 断点续传
            send_get_range(url, received, proxy).await?
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
    proxy: &DownloadProxy,
) -> Result<reqwest::Response, ToolchainError> {
    send_get_inner(url, Some(offset), proxy).await
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
        let received = download_to(
            url,
            &dest,
            &DownloadProxy::resolve(None).expect("代理解析"),
            |_, _| calls += 1,
        )
        .await
        .expect("真实下载小文件");

        let bytes = std::fs::read(&dest).unwrap();
        assert_eq!(received as usize, bytes.len());
        assert!(calls > 0);
        assert!(bytes.len() > 800 * 1024); // 完整源码包
        assert_eq!(&bytes[..2], b"PK"); // zip 魔数
        std::fs::remove_dir_all(&dir).ok();
    }

    fn no_env(_: &str) -> Option<String> {
        None
    }

    #[test]
    fn 代理解析_显式配置优先于环境变量() {
        let env_all = |name: &str| (!name.starts_with("all_proxy")).then(|| "http://env:1".into());
        // 显式配置存在时 env 一律忽略
        assert!(matches!(
            DownloadProxy::resolve_with(Some("http://cfg:1"), &env_all).unwrap(),
            DownloadProxy::Explicit(_)
        ));
    }

    #[test]
    fn 代理解析_环境变量命中走env() {
        let with_env = |name: &str| (name == "all_proxy").then(|| "http://env:1".into());
        assert!(matches!(
            DownloadProxy::resolve_with(None, &with_env).unwrap(),
            DownloadProxy::Env
        ));
        // reqwest 检测对齐：https_proxy 大写也能命中
        let with_upper = |name: &str| (name == "HTTPS_PROXY").then(|| "http://env:1".into());
        assert!(matches!(
            DownloadProxy::resolve_with(None, &with_upper).unwrap(),
            DownloadProxy::Env
        ));
    }

    #[test]
    fn 代理解析_无配置无环境变量走直连() {
        assert!(matches!(
            DownloadProxy::resolve_with(None, &no_env).unwrap(),
            DownloadProxy::Direct
        ));
        // 空串/空白配置视同未配置
        assert!(matches!(
            DownloadProxy::resolve_with(Some("  "), &no_env).unwrap(),
            DownloadProxy::Direct
        ));
    }

    #[test]
    fn 代理解析_非法代理地址报错() {
        assert!(DownloadProxy::resolve_with(Some("not-a-url"), &no_env).is_err());
    }
}
