//! Unit tests for the homepage server-card reachability probe (ticket 12).
//!
//! Seam under test: `server::reachability::tcp_probe` — a cheap, real
//! reachability check. It is deliberately TCP-level only: it proves the host
//! answers on the port, not that SSH auth would succeed (the full SSH flow
//! stays behind the 连接测试 button on the servers page).

use std::time::Duration;

use packsmith_lib::server::reachability::tcp_probe;

#[tokio::test]
async fn a_listening_port_is_reachable() {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();

    let result = tcp_probe(
        addr.ip().to_string().as_str(),
        addr.port(),
        Duration::from_secs(2),
    )
    .await;

    assert!(result.is_ok(), "expected reachable, got {result:?}");
}

#[tokio::test]
async fn a_closed_port_is_unreachable_with_a_detail() {
    // Pick a port with no listener by binding then dropping the socket.
    let addr = {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        listener.local_addr().unwrap()
    };

    let result = tcp_probe(
        addr.ip().to_string().as_str(),
        addr.port(),
        Duration::from_secs(2),
    )
    .await;

    let detail = result.expect_err("closed port must be unreachable");
    assert!(!detail.is_empty(), "unreachable must carry a detail");
}

#[tokio::test]
async fn a_host_that_never_answers_times_out() {
    // RFC 5737 documentation address; nothing can connect within the budget.
    let result = tcp_probe("192.0.2.1", 22, Duration::from_millis(200)).await;

    assert!(result.is_err(), "timeout must surface as unreachable");
}
