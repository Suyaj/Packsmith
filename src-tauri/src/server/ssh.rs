//! Real SSH connection testing via russh (ticket 04).
//!
//! The 连接测试 button performs a genuine connect + host-key check + auth and
//! returns a machine-readable [`TestOutcome`] the UI maps to Chinese text.
//!
//! Host-key policy is accept-once-store-fingerprint (see the russh research
//! notes): a fingerprint the user has accepted before is accepted; an unknown
//! fingerprint is rejected unless the caller explicitly accepts it (the UI
//! asks and re-runs the test with `accept_unknown_host_key = true`); a
//! *changed* fingerprint is always rejected (possible MITM) — the UI may
//! offer an explicit "update fingerprint" action, never silent acceptance.
//!
//! The pure decision logic ([`check_host_key`]) is unit-testable without any
//! network; the full flow needs a real SSH server and is exercised by
//! `tests/sshd_integration_test.rs` (local or env-provided server).

use std::sync::{Arc, Mutex};

use russh::client::{AuthResult, Handle};
use russh::keys::{HashAlg, PrivateKeyWithHashAlg};

use super::repo::Credential;

/// How long the whole connect+auth flow may take before reporting the server
/// as unreachable.
const TEST_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(15);

/// Everything needed to attempt a connection. Mirrors the server form.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TestRequest {
    pub host: String,
    pub port: u16,
    pub username: String,
    pub credential: Credential,
}

/// Result of a connection test. The UI maps each variant to concrete Chinese
/// text; string details carry the underlying technical cause.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TestOutcome {
    /// Connected, host key verified, authenticated. Carries the fingerprint
    /// to persist when the user accepted an unknown host key during this
    /// test (`None` when it was already known).
    Success { new_fingerprint: Option<String> },
    /// No stored fingerprint and the caller did not opt in to accepting.
    HostKeyUnknown { fingerprint: String },
    /// A different key than the stored fingerprint — possible MITM.
    HostKeyChanged { fingerprint: String },
    /// TCP/DNS/version-exchange failure: the host did not speak to us.
    Unreachable { detail: String },
    /// The server rejected the offered password or key.
    AuthFailed { method: AuthMethod },
    /// The private key file could not be loaded (missing, bad format, wrong
    /// passphrase).
    KeyUnreadable { detail: String },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuthMethod {
    Password,
    PrivateKey,
}

/// Decision for the presented host fingerprint.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HostKeyCheck {
    /// Proceed with the connection.
    Accept,
    /// No stored fingerprint; caller did not opt in.
    RejectUnknown,
    /// Stored fingerprint differs from the presented key.
    RejectChanged,
}

/// Pure host-key policy. Unit-tested without network.
pub fn check_host_key(
    stored: Option<&str>,
    presented: &str,
    accept_unknown: bool,
) -> HostKeyCheck {
    match stored {
        Some(fp) if fp == presented => HostKeyCheck::Accept,
        Some(_) => HostKeyCheck::RejectChanged,
        None if accept_unknown => HostKeyCheck::Accept,
        None => HostKeyCheck::RejectUnknown,
    }
}

/// SHA-256 fingerprint of a presented key in OpenSSH known_hosts format
/// (`SHA256:...`). Shared with the deploy transport (ticket 10).
pub(crate) fn fingerprint_of(key: &russh::keys::PublicKeyOrCertificate) -> String {
    key.public_key().fingerprint(HashAlg::Sha256).to_string()
}

/// `russh::client::Handler` recording what it saw, so the caller can
/// distinguish "unknown host key" from other transport failures.
struct RecordingHandler {
    stored: Option<String>,
    accept_unknown: bool,
    presented: Arc<Mutex<Option<String>>>,
}

impl russh::client::Handler for RecordingHandler {
    type Error = russh::Error;

    async fn check_server_key(
        &mut self,
        server_public_key: &russh::keys::PublicKeyOrCertificate,
    ) -> Result<bool, Self::Error> {
        let fingerprint = fingerprint_of(server_public_key);
        *self.presented.lock().unwrap() = Some(fingerprint.clone());
        Ok(
            matches!(
                check_host_key(self.stored.as_deref(), &fingerprint, self.accept_unknown),
                HostKeyCheck::Accept
            ),
        )
    }
}

/// Runs a full connect + host-key + auth attempt against `req`.
///
/// `stored_fingerprint` comes from the encrypted store (see
/// `repo::known_host_fingerprint`); `accept_unknown_host_key` must be `true`
/// only after the user has explicitly accepted the presented fingerprint.
pub async fn test_connection(
    req: &TestRequest,
    stored_fingerprint: Option<&str>,
    accept_unknown_host_key: bool,
) -> TestOutcome {
    match tokio::time::timeout(TEST_TIMEOUT, run_test(req, stored_fingerprint, accept_unknown_host_key)).await {
        Ok(outcome) => outcome,
        Err(_) => TestOutcome::Unreachable {
            detail: format!("connection attempt timed out after {}s", TEST_TIMEOUT.as_secs()),
        },
    }
}

async fn run_test(
    req: &TestRequest,
    stored_fingerprint: Option<&str>,
    accept_unknown_host_key: bool,
) -> TestOutcome {
    // Validate key material before any network I/O: a missing or unloadable
    // key file never means "unreachable".
    if let Credential::PrivateKey { path, passphrase } = &req.credential {
        if let Err(e) = russh::keys::load_secret_key(path, passphrase.as_deref()) {
            return TestOutcome::KeyUnreadable {
                detail: e.to_string(),
            };
        }
    }

    let presented: Arc<Mutex<Option<String>>> = Arc::new(Mutex::new(None));
    let handler = RecordingHandler {
        stored: stored_fingerprint.map(|s| s.to_string()),
        accept_unknown: accept_unknown_host_key,
        presented: Arc::clone(&presented),
    };
    let config = Arc::new(russh::client::Config {
        nodelay: true,
        ..Default::default()
    });

    let mut handle: Handle<RecordingHandler> =
        match russh::client::connect(config, (req.host.as_str(), req.port), handler).await {
            Ok(handle) => handle,
            Err(e) => {
                // If we got far enough to see the host key, the failure is a
                // host-key rejection — report the fingerprint, not the
                // transport error.
                if let Some(fingerprint) = presented.lock().unwrap().clone() {
                    return match check_host_key(
                        stored_fingerprint,
                        &fingerprint,
                        accept_unknown_host_key,
                    ) {
                        HostKeyCheck::RejectUnknown => TestOutcome::HostKeyUnknown { fingerprint },
                        HostKeyCheck::RejectChanged => TestOutcome::HostKeyChanged { fingerprint },
                        HostKeyCheck::Accept => TestOutcome::Unreachable {
                            detail: e.to_string(),
                        },
                    };
                }
                return TestOutcome::Unreachable {
                    detail: e.to_string(),
                };
            }
        };

    let fingerprint = presented.lock().unwrap().clone();
    let accepted_new = fingerprint.is_some() && stored_fingerprint.is_none();

    let auth_result: AuthResult = match &req.credential {
        Credential::Password { password } => {
            match handle.authenticate_password(&req.username, password).await {
                Ok(result) => result,
                Err(e) => return TestOutcome::Unreachable { detail: e.to_string() },
            }
        }
        Credential::PrivateKey { path, passphrase } => {
            // Loaded once above for validation; reload here so the key is
            // owned for the auth call (load is cheap and keeps this function
            // free of shared mutable key state).
            let key = match russh::keys::load_secret_key(path, passphrase.as_deref()) {
                Ok(key) => key,
                Err(e) => return TestOutcome::KeyUnreadable { detail: e.to_string() },
            };
            let hash = match handle.best_supported_rsa_hash().await {
                Ok(hash) => hash.flatten(),
                Err(e) => return TestOutcome::Unreachable { detail: e.to_string() },
            };
            match handle
                .authenticate_publickey(&req.username, PrivateKeyWithHashAlg::new(Arc::new(key), hash))
                .await
            {
                Ok(result) => result,
                Err(e) => return TestOutcome::Unreachable { detail: e.to_string() },
            }
        }
    };

    if !auth_result.success() {
        return TestOutcome::AuthFailed {
            method: match req.credential {
                Credential::Password { .. } => AuthMethod::Password,
                Credential::PrivateKey { .. } => AuthMethod::PrivateKey,
            },
        };
    }

    let _ = handle
        .disconnect(russh::Disconnect::ByApplication, "", "english")
        .await;

    TestOutcome::Success {
        new_fingerprint: if accepted_new {
            fingerprint
        } else {
            None
        },
    }
}
