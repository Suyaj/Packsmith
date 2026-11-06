//! SSH 连接必须有超时（部署“一直等待”bug 的传输层回归测试）。
//!
//! russh 的 `client::connect` 没有默认超时：对端收下 TCP 却不回 SSH banner
//! 时（网络拦截/防火墙静默丢弃/半死服务），连接永远悬着——引擎在此之前
//! 不发任何事件，前端部署目标就一直停在“等待”，没有任何错误反馈。
//!
//! 本测试完全封闭、无外部网络：本地起一个“黑洞”TCP 监听——接受连接但
//! 永不读写（不回 SSH banner），复现的正是线上挂死的形状。断言
//! [`SshTransport::connect_with_timeout`] 在注入的短超时内以
//! `TransportError::Unreachable`（"connection timed out"）返回，整体有界，
//! 不会挂死 CI。

use std::time::{Duration, Instant};

use packsmith_lib::deploy::engine::ConnInfo;
use packsmith_lib::deploy::transport::SshTransport;
use packsmith_lib::server::repo::Credential;

/// 接受连接后什么都不做：持有 socket（不 drop、不读、不写），客户端永远
/// 等不到 SSH 版本 banner。
fn spawn_blackhole() -> String {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind 127.0.0.1:0");
    let addr = listener.local_addr().expect("local_addr").to_string();
    std::thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            // 故意泄漏：连接保持打开但静默。
            std::mem::forget(stream);
        }
    });
    addr
}

#[tokio::test]
async fn connect_to_a_silent_server_errors_within_the_timeout() {
    let addr = spawn_blackhole();
    let (host, port) = addr
        .rsplit_once(':')
        .map(|(h, p)| (h.to_string(), p.parse::<u16>().expect("port")))
        .expect("host:port");

    let conn = ConnInfo {
        server_id: 0,
        host,
        port,
        username: "deploy".into(),
        credential: Credential::Password {
            password: "x".into(),
        },
        fingerprint: None,
    };

    let started = Instant::now();
    // 外层 20s 兜底：若连接超时回归（永远悬着），测试在 20s 内红掉而不是挂死。
    let result =
        tokio::time::timeout(Duration::from_secs(20), async {
            SshTransport::connect_with_timeout(&conn, Duration::from_secs(1)).await
        })
        .await;
    let elapsed = started.elapsed();

    let err = match result {
        Ok(Ok(_transport)) => {
            panic!("a silent server must not yield a transport")
        }
        Ok(Err(e)) => e,
        Err(_) => panic!(
            "connect_with_timeout never returned (elapsed {elapsed:?}) — the connect timeout regressed"
        ),
    };
    assert_eq!(err.code(), "unreachable");
    assert!(
        err.to_string().contains("timed out"),
        "expected the connect-timeout detail, got: {err}"
    );
    assert!(
        elapsed < Duration::from_secs(5),
        "bounded by the 1s ceiling, took {elapsed:?}"
    );
}
