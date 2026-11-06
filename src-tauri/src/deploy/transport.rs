//! Real russh transport for deploy execution (ticket 10).
//!
//! One [`SshTransport`] per target: connect (host-key policy identical to
//! the 连接测试 — a stored fingerprint is required, deploys never accept new
//! keys interactively), authenticate with the server's stored credential,
//! then either run a command group via an exec channel (as the SSH login
//! user) or upload the artifact via SFTP (russh-sftp), create + truncate —
//! i.e. original file name, overwrite.
//!
//! Exercised end to end by `tests/deploy_ssh_integration_test.rs`
//! (env-gated, network-free by default).

use std::path::Path;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use russh::ChannelMsg;
use russh::client::{AuthResult, Handle};
use russh_sftp::client::SftpSession;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

use super::engine::{DeployTransport, ExecOutput, TransportError};
use crate::server::ssh::{HostKeyCheck, check_host_key, fingerprint_of};

/// Generous ceiling for one command group / one SFTP operation; real
/// deployments finish far below this. Cancellation (v1) happens between
/// phases and per upload chunk, not mid-exec.
const OPERATION_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(600);

/// russh 的 `client::connect` 没有默认超时：TCP 被静默拦截/丢包（或对端收下
/// 连接却不回 SSH banner）时连接永远悬着——引擎在此之前不发任何事件，部署
/// 目标就一直停在“等待”且没有任何反馈。给整个建立阶段（TCP + banner +
/// KEX + 认证）一个上限，超时以 `Unreachable`（unreachable）上报。
const CONNECT_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(15);

const SFTP_WRITE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(300);
const UPLOAD_CHUNK: usize = 32 * 1024;

/// russh client handler applying the stored-fingerprint policy (reject
/// unknown — deploys have no interactive accept).
struct DeployHandler {
    stored: Option<String>,
    presented: Arc<Mutex<Option<String>>>,
}

impl russh::client::Handler for DeployHandler {
    type Error = russh::Error;

    async fn check_server_key(
        &mut self,
        server_public_key: &russh::keys::PublicKeyOrCertificate,
    ) -> Result<bool, Self::Error> {
        let fingerprint = fingerprint_of(server_public_key);
        *self.presented.lock().unwrap() = Some(fingerprint.clone());
        Ok(matches!(
            check_host_key(self.stored.as_deref(), &fingerprint, false),
            HostKeyCheck::Accept
        ))
    }
}

fn transport_error_with_context(
    presented: Option<String>,
    stored: Option<&str>,
    error: russh::Error,
) -> TransportError {
    match presented {
        Some(fingerprint) => match check_host_key(stored, &fingerprint, false) {
            HostKeyCheck::RejectUnknown => TransportError::HostKeyUnknown { fingerprint },
            HostKeyCheck::RejectChanged => TransportError::HostKeyChanged { fingerprint },
            HostKeyCheck::Accept => TransportError::Unreachable {
                detail: error.to_string(),
            },
        },
        None => TransportError::Unreachable {
            detail: error.to_string(),
        },
    }
}

fn unreachable(e: russh::Error) -> TransportError {
    TransportError::Unreachable {
        detail: e.to_string(),
    }
}

/// A connected + authenticated SSH session to one target's server.
pub struct SshTransport {
    handle: Handle<DeployHandler>,
}

impl SshTransport {
    /// Connects and authenticates with `conn`'s credential. Host-key errors
    /// surface as [`TransportError::HostKeyUnknown`] / [`HostKeyChanged`].
    pub async fn connect(conn: &super::engine::ConnInfo) -> Result<Self, TransportError> {
        Self::connect_with_timeout(conn, CONNECT_TIMEOUT).await
    }

    /// [`Self::connect`] with an injectable ceiling — the black-hole test uses
    /// a short one so CI stays bounded.
    pub async fn connect_with_timeout(
        conn: &super::engine::ConnInfo,
        timeout: std::time::Duration,
    ) -> Result<Self, TransportError> {
        tokio::time::timeout(timeout, Self::establish(conn))
            .await
            .unwrap_or_else(|_| {
                Err(TransportError::Unreachable {
                    detail: format!("connection timed out after {}s", timeout.as_secs()),
                })
            })
    }

    /// The unbounded establish itself (TCP + banner + KEX + auth), factored
    /// out so [`Self::connect_with_timeout`] can bound it.
    async fn establish(conn: &super::engine::ConnInfo) -> Result<Self, TransportError> {
        // Validate key material before any network I/O.
        if let crate::server::repo::Credential::PrivateKey { path, passphrase } = &conn.credential {
            if let Err(e) = russh::keys::load_secret_key(path, passphrase.as_deref()) {
                return Err(TransportError::KeyUnreadable {
                    detail: e.to_string(),
                });
            }
        }

        let presented: Arc<Mutex<Option<String>>> = Arc::new(Mutex::new(None));
        let handler = DeployHandler {
            stored: conn.fingerprint.clone(),
            presented: Arc::clone(&presented),
        };
        let config = Arc::new(russh::client::Config {
            nodelay: true,
            ..Default::default()
        });

        let mut handle =
            match russh::client::connect(config, (conn.host.as_str(), conn.port), handler).await {
                Ok(handle) => handle,
                Err(e) => {
                    return Err(transport_error_with_context(
                        presented.lock().unwrap().clone(),
                        conn.fingerprint.as_deref(),
                        e,
                    ));
                }
            };

        let auth_result: AuthResult = match &conn.credential {
            crate::server::repo::Credential::Password { password } => {
                match handle.authenticate_password(&conn.username, password).await {
                    Ok(result) => result,
                    Err(e) => {
                        return Err(TransportError::Unreachable {
                            detail: e.to_string(),
                        });
                    }
                }
            }
            crate::server::repo::Credential::PrivateKey { path, passphrase } => {
                let key = match russh::keys::load_secret_key(path, passphrase.as_deref()) {
                    Ok(key) => key,
                    Err(e) => {
                        return Err(TransportError::KeyUnreadable {
                            detail: e.to_string(),
                        });
                    }
                };
                let hash = match handle.best_supported_rsa_hash().await {
                    Ok(hash) => hash.flatten(),
                    Err(e) => {
                        return Err(TransportError::Unreachable {
                            detail: e.to_string(),
                        });
                    }
                };
                match handle
                    .authenticate_publickey(
                        &conn.username,
                        russh::keys::PrivateKeyWithHashAlg::new(Arc::new(key), hash),
                    )
                    .await
                {
                    Ok(result) => result,
                    Err(e) => {
                        return Err(TransportError::Unreachable {
                            detail: e.to_string(),
                        });
                    }
                }
            }
        };

        if !auth_result.success() {
            return Err(TransportError::AuthFailed);
        }

        Ok(SshTransport { handle })
    }
}

#[async_trait]
impl DeployTransport for SshTransport {
    async fn exec(&mut self, command: &str) -> Result<ExecOutput, TransportError> {
        let future = async {
            let mut channel = self
                .handle
                .channel_open_session()
                .await
                .map_err(unreachable)?;
            channel
                .exec(true, wrap_sh_c(command))
                .await
                .map_err(unreachable)?;

            let mut combined = String::new();
            let mut exit_code: Option<u32> = None;
            while let Some(msg) = channel.wait().await {
                match msg {
                    ChannelMsg::Data { data, .. } | ChannelMsg::ExtendedData { data, .. } => {
                        combined.push_str(&String::from_utf8_lossy(&data));
                    }
                    ChannelMsg::ExitStatus { exit_status } => {
                        exit_code = Some(exit_status);
                    }
                    // Eof/Close end the stream; the exit status arrives before
                    // both on OpenSSH servers.
                    ChannelMsg::Eof | ChannelMsg::Close => break,
                    _ => {}
                }
            }
            Ok(ExecOutput {
                exit_code: exit_code.unwrap_or(u32::MAX) as i32,
                combined,
            })
        };
        tokio::time::timeout(OPERATION_TIMEOUT, future)
            .await
            .unwrap_or_else(|_| {
                Err(TransportError::Unreachable {
                    detail: format!("command timed out after {}s", OPERATION_TIMEOUT.as_secs()),
                })
            })
    }

    async fn upload(
        &mut self,
        local: &Path,
        remote_dir: &str,
        file_name: &str,
        progress: &mut (dyn FnMut(u64, u64) -> bool + Send),
    ) -> Result<(), TransportError> {
        let total = tokio::fs::metadata(local)
            .await
            .map(|m| m.len())
            .map_err(|e| TransportError::Io(e.to_string()))?;

        let future = async {
            let channel = self
                .handle
                .channel_open_session()
                .await
                .map_err(unreachable)?;
            channel
                .request_subsystem(true, "sftp")
                .await
                .map_err(unreachable)?;
            let sftp = SftpSession::new(channel.into_stream())
                .await
                .map_err(|e| TransportError::Io(e.to_string()))?;
            sftp.set_timeout(SFTP_WRITE_TIMEOUT.as_secs());

            ensure_remote_dir(&sftp, remote_dir).await?;

            let remote_path = join_remote_path(remote_dir, file_name);
            // create = O_CREAT|O_TRUNC: original file name, overwrite.
            let mut remote = sftp
                .create(&remote_path)
                .await
                .map_err(|e| TransportError::Io(format!("{remote_path}: {e}")))?;

            let mut local = tokio::fs::File::open(local)
                .await
                .map_err(|e| TransportError::Io(e.to_string()))?;
            let mut buf = vec![0u8; UPLOAD_CHUNK];
            let mut done: u64 = 0;
            loop {
                if !progress(done, total) {
                    return Err(TransportError::Cancelled);
                }
                let n = local
                    .read(&mut buf)
                    .await
                    .map_err(|e| TransportError::Io(e.to_string()))?;
                if n == 0 {
                    break;
                }
                remote
                    .write_all(&buf[..n])
                    .await
                    .map_err(|e| TransportError::Io(e.to_string()))?;
                done += n as u64;
            }
            progress(done, total);
            let _ = remote.sync_all().await;
            // 数据完整性关口：write 的回执只在 shutdown 时统一核对
            // （russh-sftp 的已知弱点——drop 不等待回执，写错误会被静默
            // 丢弃），所以必须显式 await shutdown 并检查其结果。
            remote
                .shutdown()
                .await
                .map_err(|e| TransportError::Io(format!("{remote_path}: {e}")))?;
            Ok(())
        };
        tokio::time::timeout(OPERATION_TIMEOUT, future)
            .await
            .unwrap_or_else(|_| {
                Err(TransportError::Io(format!(
                    "upload timed out after {}s",
                    OPERATION_TIMEOUT.as_secs()
                )))
            })
    }
}

/// Creates `dir` (and missing parents) on the remote; existing dirs are left
/// untouched. Failures surface on the subsequent file create.
async fn ensure_remote_dir(sftp: &SftpSession, dir: &str) -> Result<(), TransportError> {
    let mut current = String::new();
    for part in dir.split('/').filter(|p| !p.is_empty()) {
        current = format!("{current}/{part}");
        if sftp.metadata(&current).await.is_err() {
            sftp.create_dir(&current)
                .await
                .map_err(|e| TransportError::Io(format!("mkdir {current}: {e}")))?;
        }
    }
    Ok(())
}

fn join_remote_path(dir: &str, file_name: &str) -> String {
    if dir.ends_with('/') {
        format!("{dir}{file_name}")
    } else {
        format!("{dir}/{file_name}")
    }
}

/// 把命令组包进显式 `sh -c '...'` 再交给 channel.exec：命令组按 POSIX sh
/// 语义（`&&` fail-fast、逐条串行）执行，不依赖登录 shell 的解释行为。
/// 单引号按 POSIX 方式转义（`'\''`）。
fn wrap_sh_c(command: &str) -> String {
    format!("sh -c '{}'", command.replace('\'', "'\\''"))
}

#[cfg(test)]
mod tests {
    use super::wrap_sh_c;

    #[test]
    fn wraps_command_group_in_explicit_sh_c() {
        // `&&` fail-fast 语义保留在引号内部，由 sh 解释，而非依赖登录 shell。
        assert_eq!(
            wrap_sh_c("cd /srv/app && sh stop.sh && sleep 1"),
            "sh -c 'cd /srv/app && sh stop.sh && sleep 1'"
        );
    }

    #[test]
    fn escapes_single_quotes_inside_the_group() {
        // POSIX 单引号转义：'\'' —— 外层引号闭合、转义引号、重新闭合。
        assert_eq!(
            wrap_sh_c("echo 'hello world'"),
            r"sh -c 'echo '\''hello world'\'''"
        );
    }

    #[test]
    fn plain_command_is_wrapped_unchanged() {
        assert_eq!(wrap_sh_c("sh start.sh --prod"), "sh -c 'sh start.sh --prod'");
    }
}
