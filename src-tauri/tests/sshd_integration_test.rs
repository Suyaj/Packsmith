//! Integration tests against a REAL SSH server (ticket 04).
//!
//! These never run in plain `cargo test`. They are gated on environment
//! variables so CI and local default runs stay network-free:
//!
//! ```text
//! PACKSMITH_SSH_TEST_HOST=127.0.0.1
//! PACKSMITH_SSH_TEST_PORT=2222
//! PACKSMITH_SSH_TEST_USER=wyy
//! # one of:
//! PACKSMITH_SSH_TEST_PASSWORD=...
//! PACKSMITH_SSH_TEST_KEY=/path/to/private/key          (optionally
//! PACKSMITH_SSH_TEST_KEY_PASSPHRASE=...)
//! ```
//!
//! What they cover, end to end against real sshd:
//! - correct credential -> Success, fingerprint accepted-once-then-stored
//! - wrong password -> AuthFailed (the concrete reason)
//! - host-key flow: first test without opt-in -> HostKeyUnknown; with opt-in
//!   -> Success + fingerprint persisted in the encrypted store
//! - key-file credential -> Success (only when PACKSMITH_SSH_TEST_KEY set)

use packsmith_lib::server::repo::{self, Credential};
use packsmith_lib::server::ssh::{test_connection, TestOutcome, TestRequest};

const TEST_KEY_HEX: &str =
    "a1b2c3d4e5f60718293a4b5c6d7e8f90a1b2c3d4e5f60718293a4b5c6d7e8f90";

struct SshEnv {
    host: String,
    port: u16,
    username: String,
    password: Option<String>,
    key_path: Option<String>,
}

fn ssh_env() -> Option<SshEnv> {
    let host = std::env::var("PACKSMITH_SSH_TEST_HOST").ok()?;
    Some(SshEnv {
        host,
        port: std::env::var("PACKSMITH_SSH_TEST_PORT")
            .ok()
            .and_then(|p| p.parse().ok())
            .unwrap_or(22),
        username: std::env::var("PACKSMITH_SSH_TEST_USER").ok()?,
        password: std::env::var("PACKSMITH_SSH_TEST_PASSWORD").ok(),
        key_path: std::env::var("PACKSMITH_SSH_TEST_KEY").ok(),
    })
}

fn password_request(env: &SshEnv, password: &str) -> TestRequest {
    TestRequest {
        host: env.host.clone(),
        port: env.port,
        username: env.username.clone(),
        credential: Credential::Password {
            password: password.to_string(),
        },
    }
}

fn block_on<F: std::future::Future>(fut: F) -> F::Output {
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    rt.block_on(fut)
}

#[test]
fn correct_password_succeeds_and_stores_accepted_fingerprint() {
    let Some(env) = ssh_env() else { return };
    let Some(password) = env.password.clone() else { return };
    let dir = tempfile::tempdir().unwrap();
    let store = packsmith_lib::store::Store::open(&dir.path().join("db"), TEST_KEY_HEX).unwrap();

    // First attempt without opt-in: the unknown host key must stop us even
    // though the credential is correct.
    let outcome = block_on(test_connection(&password_request(&env, &password), None, false));
    let fingerprint = match outcome {
        TestOutcome::HostKeyUnknown { fingerprint } => fingerprint,
        other => panic!("expected HostKeyUnknown on first contact, got {other:?}"),
    };
    assert!(repo::known_host_fingerprint(&store, &env.host, env.port)
        .unwrap()
        .is_none(), "a rejected host key must not be stored");

    // User accepts -> re-test with opt-in -> success, and the fingerprint is
    // handed back for storage.
    let outcome = block_on(test_connection(&password_request(&env, &password), None, true));
    match outcome {
        TestOutcome::Success { new_fingerprint } => {
            assert_eq!(new_fingerprint.as_deref(), Some(fingerprint.as_str()));
        }
        other => panic!("expected Success after opt-in, got {other:?}"),
    }
    repo::store_host_fingerprint(&store, &env.host, env.port, &fingerprint).unwrap();

    // Later tests against the same server succeed with no opt-in: the
    // stored fingerprint is accepted transparently.
    let stored = repo::known_host_fingerprint(&store, &env.host, env.port).unwrap();
    let outcome = block_on(test_connection(&password_request(&env, &password), stored.as_deref(), false));
    assert!(
        matches!(outcome, TestOutcome::Success { new_fingerprint: None }),
        "known fingerprint must not be re-stored, got {outcome:?}"
    );
}

#[test]
fn wrong_password_fails_with_concrete_auth_reason() {
    let Some(env) = ssh_env() else { return };
    let dir = tempfile::tempdir().unwrap();
    let store = packsmith_lib::store::Store::open(&dir.path().join("db"), TEST_KEY_HEX).unwrap();
    let stored = repo::known_host_fingerprint(&store, &env.host, env.port).unwrap()
        .or_else(|| std::env::var("PACKSMITH_SSH_TEST_FINGERPRINT").ok());

    let outcome = block_on(test_connection(
        &password_request(&env, "definitely-not-the-password"),
        stored.as_deref(),
        true,
    ));

    match outcome {
        TestOutcome::AuthFailed { method } => {
            assert_eq!(method, packsmith_lib::server::ssh::AuthMethod::Password);
        }
        other => panic!("expected AuthFailed for a wrong password, got {other:?}"),
    }
}

#[test]
fn correct_key_succeeds_and_stores_accepted_fingerprint() {
    let Some(env) = ssh_env() else { return };
    let Some(key_path) = env.key_path.clone() else { return };
    let passphrase = std::env::var("PACKSMITH_SSH_TEST_KEY_PASSPHRASE").ok();
    let dir = tempfile::tempdir().unwrap();
    let store = packsmith_lib::store::Store::open(&dir.path().join("db"), TEST_KEY_HEX).unwrap();

    let request = TestRequest {
        host: env.host.clone(),
        port: env.port,
        username: env.username.clone(),
        credential: Credential::PrivateKey {
            path: key_path,
            passphrase,
        },
    };

    // First contact without opt-in: unknown host key blocks the connection.
    let outcome = block_on(test_connection(&request, None, false));
    let fingerprint = match outcome {
        TestOutcome::HostKeyUnknown { fingerprint } => fingerprint,
        other => panic!("expected HostKeyUnknown on first contact, got {other:?}"),
    };

    // User accepts: success, and the fingerprint comes back for storage.
    let outcome = block_on(test_connection(&request, None, true));
    match outcome {
        TestOutcome::Success { new_fingerprint } => {
            assert_eq!(new_fingerprint.as_deref(), Some(fingerprint.as_str()));
        }
        other => panic!("expected Success after opt-in, got {other:?}"),
    }
    repo::store_host_fingerprint(&store, &env.host, env.port, &fingerprint).unwrap();

    // Re-test with the stored fingerprint: transparent success.
    let stored = repo::known_host_fingerprint(&store, &env.host, env.port).unwrap();
    let outcome = block_on(test_connection(&request, stored.as_deref(), false));
    assert!(
        matches!(outcome, TestOutcome::Success { new_fingerprint: None }),
        "known fingerprint must not be re-stored, got {outcome:?}"
    );
}

#[test]
fn wrong_key_fails_with_concrete_auth_reason() {
    let Some(env) = ssh_env() else { return };
    // Generate a key the server does not know.
    let key_dir = tempfile::tempdir().unwrap();
    let unknown_key = key_dir.path().join("unknown_key");
    let status = std::process::Command::new("ssh-keygen")
        .args(["-t", "ed25519", "-N", "", "-f", unknown_key.to_str().unwrap(), "-q"])
        .status()
        .expect("ssh-keygen must be available on dev machines");
    assert!(status.success());
    let stored = std::env::var("PACKSMITH_SSH_TEST_FINGERPRINT").ok();

    let outcome = block_on(test_connection(
        &TestRequest {
            host: env.host.clone(),
            port: env.port,
            username: env.username.clone(),
            credential: Credential::PrivateKey {
                path: unknown_key.to_str().unwrap().to_string(),
                passphrase: None,
            },
        },
        stored.as_deref(),
        true,
    ));

    match outcome {
        TestOutcome::AuthFailed { method } => {
            assert_eq!(method, packsmith_lib::server::ssh::AuthMethod::PrivateKey);
        }
        other => panic!("expected AuthFailed for an unknown key, got {other:?}"),
    }
}

