//! 服务器可达性探测（ticket 12）：首页服务器卡片显示连接状态。
//!
//! 刻意做成 TCP 层的探测（异步 `TcpStream::connect` + 超时）：便宜、真实、
//! 无需凭据。它证明"主机在端口上有响应"，不证明 SSH 认证可用——完整连接
//! 测试（connect + host key + auth）仍是服务器管理页"连接测试"按钮的职责。

use std::time::Duration;

/// 探测预算：一次 TCP 连接最多等这么久。
pub const PROBE_TIMEOUT: Duration = Duration::from_secs(3);

/// TCP 连接探测：`Ok(())` = 可达；`Err(detail)` = 连接失败或超时。
pub async fn tcp_probe(host: &str, port: u16, timeout: Duration) -> Result<(), String> {
    match tokio::time::timeout(timeout, tokio::net::TcpStream::connect((host, port))).await {
        Ok(Ok(_)) => Ok(()),
        Ok(Err(e)) => Err(e.to_string()),
        Err(_) => Err(format!("timeout after {}s", timeout.as_secs_f32())),
    }
}
