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

/// 反序列化 `Option<Option<T>>`：字段**缺席** → `None`（保持现状），
/// JSON `null` → `Some(None)`（清除），有值 → `Some(Some(v))`（更新）。
/// 用于私钥口令的"留空保持不变"编辑语义。
fn double_option<'de, T, D>(de: D) -> Result<Option<Option<T>>, D::Error>
where
    T: Deserialize<'de>,
    D: serde::Deserializer<'de>,
{
    Deserialize::deserialize(de).map(Some)
}

/// 凭据入参（create / update / 连接测试共用）。编辑语义（update）：
/// - `update_server` 的整个 `credential` 为 null → 保持现有凭据；
/// - 私钥口令字段缺席 → 保持现有口令；`null` → 清除；有值 → 更新。
/// 密码无逐字段"保持"标记：密码留空由前端整体传 `credential: null`。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum CredentialPayload {
    PrivateKey {
        path: String,
        #[serde(default, deserialize_with = "double_option")]
        passphrase: Option<Option<String>>,
    },
    Password {
        password: String,
    },
}

/// 把入参解析为实际要落库的凭据。`existing` 是库中的当前凭据（update 时
/// 提供），仅在"保持不变"编辑语义需要时读取——结果不回传前端。
fn resolve_credential(payload: CredentialPayload, existing: Option<&Credential>) -> Credential {
    match payload {
        CredentialPayload::Password { password } => Credential::Password { password },
        CredentialPayload::PrivateKey { path, passphrase } => Credential::PrivateKey {
            path,
            passphrase: match passphrase {
                Some(passphrase) => passphrase,
                // 字段缺席：现有凭据是私钥时保持其口令，否则视为无口令。
                None => match existing {
                    Some(Credential::PrivateKey { passphrase, .. }) => passphrase.clone(),
                    _ => None,
                },
            },
        },
    }
}

/// 凭据出参（列表 / 保存结果）：只含类型与"是否已设置"指示，永不含
/// 明文密码 / 口令。私钥路径不是机密，可直接展示。
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum CredentialSummary {
    PrivateKey {
        path: String,
        has_passphrase: bool,
    },
    /// 密码型服务器必然已设置密码（创建时必填），因此无额外字段。
    Password,
}

impl From<&Credential> for CredentialSummary {
    fn from(c: &Credential) -> Self {
        match c {
            Credential::PrivateKey { path, passphrase } => CredentialSummary::PrivateKey {
                path: path.clone(),
                has_passphrase: passphrase.is_some(),
            },
            Credential::Password { .. } => CredentialSummary::Password,
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
    pub credential: CredentialSummary,
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
        // 创建没有"现状"可保持：口令缺席 = 无口令。
        credential: resolve_credential(credential, None),
    };
    repo::create(&store, &new)
        .map_err(|e| e.to_string())
        .map(|s| (&s).into())
}

/// 更新服务器。`credential` 为 null（或字段缺席）时保持现有凭据 /
/// 口令不变——凭据永不明文回传前端，编辑表单留空即"保持不变"。
#[tauri::command]
pub fn update_server(
    store: State<'_, Store>,
    id: i64,
    host: String,
    port: u16,
    username: String,
    credential: Option<CredentialPayload>,
) -> Result<ServerDto, String> {
    let existing = repo::get(&store, id)
        .map_err(|e| e.to_string())?
        .ok_or_else(|| "server not found".to_string())?;
    let credential = match credential {
        // 整体保持现状（编辑表单密码留空）。
        None => existing.credential.clone(),
        Some(payload) => resolve_credential(payload, Some(&existing.credential)),
    };
    let server = Server {
        id,
        host,
        port,
        username,
        credential,
    };
    repo::update(&store, &server)
        .map_err(|e| e.to_string())
        .map(|s| (&s).into())
}

#[tauri::command]
pub fn delete_server(store: State<'_, Store>, id: i64) -> Result<bool, String> {
    repo::delete(&store, id).map_err(|e| e.to_string())
}

/// Connection test for unsaved form values. Runs the full connect + host-key
/// + auth flow; potentially slow, hence async.
#[tauri::command]
pub async fn test_server_connection(
    store: State<'_, Store>,
    host: String,
    port: u16,
    username: String,
    credential: CredentialPayload,
    accept_unknown_host_key: bool,
) -> Result<TestOutcomeDto, String> {
    let request = TestRequest {
        host,
        port,
        username,
        // 连接测试没有"现状"可保持：口令缺席 = 无口令。
        credential: resolve_credential(credential, None),
    };
    run_connection_test(&store, request, accept_unknown_host_key).await
}

/// Connection test for a saved server, using its stored credential. 编辑表单
/// 留空凭据时，"连接测试"走这里——凭据不出后端即可测试。
#[tauri::command]
pub async fn test_saved_server_connection(
    store: State<'_, Store>,
    id: i64,
    accept_unknown_host_key: bool,
) -> Result<TestOutcomeDto, String> {
    let server = repo::get(&store, id)
        .map_err(|e| e.to_string())?
        .ok_or_else(|| "server not found".to_string())?;
    let request = TestRequest {
        host: server.host,
        port: server.port,
        username: server.username,
        credential: server.credential,
    };
    run_connection_test(&store, request, accept_unknown_host_key).await
}

/// 连接测试共用流程：连接 + 主机指纹校验 + 认证；用户显式接受的新指纹
/// 在此持久化（从不自动接受）。
async fn run_connection_test(
    store: &State<'_, Store>,
    request: TestRequest,
    accept_unknown_host_key: bool,
) -> Result<TestOutcomeDto, String> {
    let stored_fingerprint =
        repo::known_host_fingerprint(store, &request.host, request.port).map_err(|e| e.to_string())?;
    let outcome = ssh::test_connection(&request, stored_fingerprint.as_deref(), accept_unknown_host_key).await;
    // Persist a fingerprint the user explicitly accepted during this test.
    if let TestOutcome::Success {
        new_fingerprint: Some(ref fingerprint),
    } = outcome
    {
        repo::store_host_fingerprint(store, &request.host, request.port, fingerprint)
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn password_payload_resolves_directly() {
        let c = resolve_credential(
            CredentialPayload::Password {
                password: "new-pass".into(),
            },
            Some(&Credential::PrivateKey {
                path: "/k".into(),
                passphrase: Some("old".into()),
            }),
        );
        assert_eq!(
            c,
            Credential::Password {
                password: "new-pass".into()
            }
        );
    }

    #[test]
    fn private_key_passphrase_absent_keeps_existing_passphrase() {
        let payload = serde_json::from_str::<CredentialPayload>(
            r#"{"type":"private_key","path":"/new-key"}"#,
        )
        .unwrap();
        let c = resolve_credential(
            payload,
            Some(&Credential::PrivateKey {
                path: "/old-key".into(),
                passphrase: Some("kept".into()),
            }),
        );
        assert_eq!(
            c,
            Credential::PrivateKey {
                path: "/new-key".into(),
                passphrase: Some("kept".into()),
            }
        );
    }

    #[test]
    fn private_key_passphrase_null_clears_it() {
        let payload = serde_json::from_str::<CredentialPayload>(
            r#"{"type":"private_key","path":"/new-key","passphrase":null}"#,
        )
        .unwrap();
        let c = resolve_credential(
            payload,
            Some(&Credential::PrivateKey {
                path: "/old-key".into(),
                passphrase: Some("kept".into()),
            }),
        );
        assert_eq!(
            c,
            Credential::PrivateKey {
                path: "/new-key".into(),
                passphrase: None,
            }
        );
    }

    #[test]
    fn private_key_passphrase_value_updates_it() {
        let payload = serde_json::from_str::<CredentialPayload>(
            r#"{"type":"private_key","path":"/k","passphrase":"fresh"}"#,
        )
        .unwrap();
        let c = resolve_credential(payload, None);
        assert_eq!(
            c,
            Credential::PrivateKey {
                path: "/k".into(),
                passphrase: Some("fresh".into()),
            }
        );
    }

    #[test]
    fn summary_never_contains_secrets() {
        let summary = CredentialSummary::from(&Credential::Password {
            password: "s3cret".into(),
        });
        let json = serde_json::to_string(&summary).unwrap();
        assert!(!json.contains("s3cret"));
        assert_eq!(json, r#"{"type":"password"}"#);

        let summary = CredentialSummary::from(&Credential::PrivateKey {
            path: "/home/me/id_ed25519".into(),
            passphrase: Some("s3cret".into()),
        });
        let json = serde_json::to_string(&summary).unwrap();
        assert!(!json.contains("s3cret"));
        assert_eq!(
            json,
            r#"{"type":"private_key","path":"/home/me/id_ed25519","has_passphrase":true}"#
        );
    }
}
