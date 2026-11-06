//! Local tests for the SSH connection-testing logic (ticket 04).
//!
//! Seams under test:
//! - `ssh::check_host_key` — pure host-key policy (no network at all)
//! - `ssh::test_connection` — full flow; exercised here only against local
//!   socket failures (closed port, non-SSH server, unreadable key file), so
//!   these run in plain `cargo test`.
//!
//! The success path and auth-failure path against a *real* SSH server live in
//! `sshd_integration_test.rs` and never run unless explicitly requested.

use packsmith_lib::server::repo::Credential;
use packsmith_lib::server::ssh::{
    HostKeyCheck, TestOutcome, TestRequest, check_host_key, test_connection,
};

const FINGERPRINT_A: &str = "SHA256:AAAA1111aaaaBBBBccccDDDDeeeeFFFF00001111";
const FINGERPRINT_B: &str = "SHA256:BBBB2222bbbbCCCCddddEEEEffff000011112222";

#[test]
fn host_key_policy_accepts_the_known_fingerprint() {
    assert_eq!(
        check_host_key(Some(FINGERPRINT_A), FINGERPRINT_A, false),
        HostKeyCheck::Accept
    );
}

#[test]
fn host_key_policy_rejects_unknown_fingerprint_unless_opted_in() {
    assert_eq!(
        check_host_key(None, FINGERPRINT_A, false),
        HostKeyCheck::RejectUnknown,
        "default must reject: never trust an unseen host silently"
    );
    assert_eq!(
        check_host_key(None, FINGERPRINT_A, true),
        HostKeyCheck::Accept,
        "explicit user opt-in accepts the new fingerprint"
    );
}

#[test]
fn host_key_policy_rejects_a_changed_fingerprint_even_when_opted_in() {
    assert_eq!(
        check_host_key(Some(FINGERPRINT_A), FINGERPRINT_B, true),
        HostKeyCheck::RejectChanged,
        "a different key is a possible MITM: the opt-in flag must not bypass it"
    );
}

#[test]
fn connection_to_a_closed_port_is_unreachable() {
    // Grab a free port, then release it so nothing listens there.
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    drop(listener);

    let outcome = tokio_test_block_on(test_connection(
        &password_request("127.0.0.1", port),
        None,
        true,
    ));

    assert!(
        matches!(outcome, TestOutcome::Unreachable { .. }),
        "closed port must be Unreachable, got {outcome:?}"
    );
}

#[test]
fn a_server_that_speaks_non_ssh_garbage_is_unreachable() {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let server = std::thread::spawn(move || {
        // Accept one connection and answer with an HTTP status line — a real
        // port that is simply not running SSH.
        let (stream, _) = listener.accept().unwrap();
        let _ = std::io::Write::write_all(&mut &stream, b"HTTP/1.1 400 Bad Request\r\n\r\n");
    });

    let outcome = tokio_test_block_on(test_connection(
        &password_request("127.0.0.1", port),
        None,
        true,
    ));

    server.join().unwrap();
    assert!(
        matches!(outcome, TestOutcome::Unreachable { .. }),
        "a non-SSH server must be Unreachable, got {outcome:?}"
    );
}

#[test]
fn a_missing_key_file_is_key_unreadable_not_unreachable() {
    let outcome = tokio_test_block_on(test_connection(
        &TestRequest {
            host: "127.0.0.1".to_string(),
            port: 1,
            username: "deploy".to_string(),
            credential: Credential::PrivateKey {
                path: "/nonexistent/id_ed25519".to_string(),
                passphrase: None,
            },
        },
        None,
        true,
    ));

    assert!(
        matches!(outcome, TestOutcome::KeyUnreadable { .. }),
        "missing key file must be KeyUnreadable, got {outcome:?}"
    );
}

#[test]
fn a_wrong_passphrase_is_key_unreadable_not_auth_failed() {
    let dir = tempfile::tempdir().unwrap();
    let key_path = dir.path().join("encrypted_key");
    let status = std::process::Command::new("ssh-keygen")
        .args([
            "-t",
            "ed25519",
            "-N",
            "right-passphrase",
            "-f",
            key_path.to_str().unwrap(),
            "-q",
            "-C",
            "packsmith-test",
        ])
        .status()
        .expect("ssh-keygen must be available on dev machines");
    assert!(status.success());

    let outcome = tokio_test_block_on(test_connection(
        &TestRequest {
            host: "127.0.0.1".to_string(),
            port: 1,
            username: "deploy".to_string(),
            credential: Credential::PrivateKey {
                path: key_path.to_str().unwrap().to_string(),
                passphrase: Some("wrong-passphrase".to_string()),
            },
        },
        None,
        true,
    ));

    assert!(
        matches!(outcome, TestOutcome::KeyUnreadable { .. }),
        "wrong passphrase must surface as KeyUnreadable, got {outcome:?}"
    );
}

#[test]
fn an_unresolvable_host_is_unreachable() {
    let outcome = tokio_test_block_on(test_connection(
        &password_request("packsmith-no-such-host.invalid", 22),
        None,
        true,
    ));

    assert!(
        matches!(outcome, TestOutcome::Unreachable { .. }),
        "DNS failure must be Unreachable, got {outcome:?}"
    );
}

fn password_request(host: &str, port: u16) -> TestRequest {
    TestRequest {
        host: host.to_string(),
        port,
        username: "deploy".to_string(),
        credential: Credential::Password {
            password: "no-such-password".to_string(),
        },
    }
}

/// Minimal single-threaded tokio runner (the lib's tokio features are
/// available; this avoids pulling test-only runtime config into the lib).
fn tokio_test_block_on<F: std::future::Future>(fut: F) -> F::Output {
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    rt.block_on(fut)
}
