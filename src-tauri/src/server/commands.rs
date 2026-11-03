//! Tauri commands exposing server CRUD + connection testing to the UI.
//!
//! Thin serialization boundary only: all behavior lives in `repo` and
//! `ssh` and is tested there. The command layer is covered by the frontend
//! tests (which mock `invoke` on the same DTO shapes).

use serde::{Deserialize, Serialize};
use tauri::State;

use crate::server::repo::{self, Credential, NewServer, Server};
use crate::server::ssh::{self, AuthMethod, TestOutcome, TestRequest};
use crate::store::Store;

/// Credential as sent by the frontend. Tagged so the JS side can use a
/// discriminated union of the same shape.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum CredentialPayload {
    PrivateKey {
        path: String,
        passphrase: Option<String>,
    },
    Password {
        password: String,
    },
}

impl From<&CredentialPayload> for Credential {
    fn from(p: &CredentialPayload) -> Self {
        match p {
            CredentialPayload::PrivateKey { path, passphrase } => Credential::PrivateKey {
                path: path.clone(),
                passphrase: passphrase.clone(),
            },
            CredentialPayload::Password { password } => Credential::Password {
                password: password.clone(),
            },
        }
    }
}

impl From<&Credential> for CredentialPayload {
    fn from(c: &Credential) -> Self {
        match c {
            Credential::PrivateKey { path, passphrase } => CredentialPayload::PrivateKey {
                path: path.clone(),
                passphrase: passphrase.clone(),
            },
            Credential::Password { password } => CredentialPayload::Password {
                password: password.clone(),
            },
        }
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ServerDto {
    pub id: i64,
    pub host: String,
    pub port: u16,
    pub username: String,
    pub credential: CredentialPayload,
}

impl From<&Server> for ServerDto {
    fn from(s: &Server) -> Self {
        ServerDto {
            id: s.id,
            host: s.host.clone(),
            port: s.port,
            username: s.username.clone(),
            credential: (&s.credential).into(),
        }
    }
}

/// Connection-test verdict for the UI. `status` is the machine-readable
/// discriminator; the frontend owns the Chinese wording per status.
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum TestOutcomeDto {
    Success { new_fingerprint: Option<String> },
    HostKeyUnknown { fingerprint: String },
    HostKeyChanged { fingerprint: String },
    Unreachable { detail: String },
    AuthFailed { method: AuthMethodDto },
    KeyUnreadable { detail: String },
}

#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AuthMethodDto {
    Password,
    PrivateKey,
}

impl From<AuthMethod> for AuthMethodDto {
    fn from(m: AuthMethod) -> Self {
        match m {
            AuthMethod::Password => AuthMethodDto::Password,
            AuthMethod::PrivateKey => AuthMethodDto::PrivateKey,
        }
    }
}

impl From<TestOutcome> for TestOutcomeDto {
    fn from(o: TestOutcome) -> Self {
        match o {
            TestOutcome::Success { new_fingerprint } => TestOutcomeDto::Success { new_fingerprint },
            TestOutcome::HostKeyUnknown { fingerprint } => {
                TestOutcomeDto::HostKeyUnknown { fingerprint }
            }
            TestOutcome::HostKeyChanged { fingerprint } => {
                TestOutcomeDto::HostKeyChanged { fingerprint }
            }
            TestOutcome::Unreachable { detail } => TestOutcomeDto::Unreachable { detail },
            TestOutcome::AuthFailed { method } => TestOutcomeDto::AuthFailed {
                method: method.into(),
            },
            TestOutcome::KeyUnreadable { detail } => TestOutcomeDto::KeyUnreadable { detail },
        }
    }
}

#[tauri::command]
pub fn list_servers(store: State<'_, Store>) -> Result<Vec<ServerDto>, String> {
    repo::list(&store)
        .map_err(|e| e.to_string())
        .map(|servers| servers.iter().map(ServerDto::from).collect())
}

#[tauri::command]
pub fn create_server(
    store: State<'_, Store>,
    host: String,
    port: u16,
    username: String,
    credential: CredentialPayload,
) -> Result<ServerDto, String> {
    let new = NewServer {
        host,
        port,
        username,
        credential: (&credential).into(),
    };
    repo::create(&store, &new)
        .map_err(|e| e.to_string())
        .map(|s| (&s).into())
}

#[tauri::command]
pub fn update_server(
    store: State<'_, Store>,
    id: i64,
    host: String,
    port: u16,
    username: String,
    credential: CredentialPayload,
) -> Result<ServerDto, String> {
    let server = Server {
        id,
        host,
        port,
        username,
        credential: (&credential).into(),
    };
    repo::update(&store, &server)
        .map_err(|e| e.to_string())
        .map(|s| (&s).into())
}

#[tauri::command]
pub fn delete_server(store: State<'_, Store>, id: i64) -> Result<bool, String> {
    repo::delete(&store, id).map_err(|e| e.to_string())
}

/// Connection test for either a saved server (id) or unsaved form values.
/// Runs the full connect + host-key + auth flow; potentially slow, hence async.
#[tauri::command]
pub async fn test_server_connection(
    store: State<'_, Store>,
    host: String,
    port: u16,
    username: String,
    credential: CredentialPayload,
    accept_unknown_host_key: bool,
) -> Result<TestOutcomeDto, String> {
    let stored_fingerprint =
        repo::known_host_fingerprint(&store, &host, port).map_err(|e| e.to_string())?;
    let request = TestRequest {
        host,
        port,
        username,
        credential: (&credential).into(),
    };
    let outcome = ssh::test_connection(
        &request,
        stored_fingerprint.as_deref(),
        accept_unknown_host_key,
    )
    .await;
    // Persist a fingerprint the user explicitly accepted during this test.
    if let TestOutcome::Success {
        new_fingerprint: Some(ref fingerprint),
    } = outcome
    {
        repo::store_host_fingerprint(&store, &request.host, request.port, fingerprint)
            .map_err(|e| e.to_string())?;
    }
    Ok(outcome.into())
}

/// Explicit "I know this host changed its key" action — never called
/// automatically; the UI must show the new fingerprint first.
#[tauri::command]
pub fn accept_host_key(
    store: State<'_, Store>,
    host: String,
    port: u16,
    fingerprint: String,
) -> Result<(), String> {
    repo::store_host_fingerprint(&store, &host, port, &fingerprint).map_err(|e| e.to_string())
}

// ---------------------------------------------------------------------------
// 首页服务器概览（ticket 12）
// ---------------------------------------------------------------------------

/// TCP 可达性结果：`reachable` + 失败细节（首页服务器卡片）。
#[derive(Debug, Clone, Serialize)]
pub struct ReachabilityDto {
    pub reachable: bool,
    pub detail: Option<String>,
}

/// 首页连接状态：便宜的 TCP 层探测（3s 超时）。只证明端口有响应，
/// 不含 SSH 认证——完整测试用服务器管理页的连接测试。
#[tauri::command]
pub async fn check_server_reachable(host: String, port: u16) -> Result<ReachabilityDto, String> {
    Ok(match super::reachability::tcp_probe(&host, port, super::reachability::PROBE_TIMEOUT).await {
        Ok(()) => ReachabilityDto {
            reachable: true,
            detail: None,
        },
        Err(detail) => ReachabilityDto {
            reachable: false,
            detail: Some(detail),
        },
    })
}

/// 最近成功部署时间（首页服务器卡片）：每台已知服务器一行，从未部署为 null。
#[tauri::command]
pub fn list_server_last_deploys(
    store: State<'_, Store>,
) -> Result<Vec<crate::store::deploy_history::ServerLastDeploy>, String> {
    crate::store::deploy_history::last_successful_per_server(&store).map_err(|e| e.to_string())
}
