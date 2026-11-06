// Server management API: the typed boundary between the UI and the Tauri
// backend. Every call is `invoke` under the hood, so frontend tests mock
// `@tauri-apps/api/core` and verify these exact payload shapes.

import { invoke } from "@tauri-apps/api/core";

/**
 * 保存服务器返回的凭据摘要：只含类型与"是否已设置"指示，永不含明文
 * 密码 / 口令（后端凭据不回传，编辑留空即保持现状）。私钥路径不是机密。
 */
export type StoredCredential =
  | { type: "private_key"; path: string; has_passphrase: boolean }
  | { type: "password" };

export type Server = {
  id: number;
  host: string;
  port: number;
  username: string;
  credential: StoredCredential;
};

export type TestOutcome =
  | { status: "success"; new_fingerprint: string | null }
  | { status: "host_key_unknown"; fingerprint: string }
  | { status: "host_key_changed"; fingerprint: string }
  | { status: "unreachable"; detail: string }
  | { status: "auth_failed"; method: "password" | "private_key" }
  | { status: "key_unreadable"; detail: string };

/**
 * 凭据入参。私钥口令可选字段是三态：
 * - 缺省（undefined）→ 保持现有口令（编辑留空）；
 * - null → 无口令 / 清除口令；
 * - 字符串 → 设置口令。
 */
export type CredentialInput =
  | { type: "private_key"; path: string; passphrase?: string | null }
  | { type: "password"; password: string };

export type ServerInput = {
  host: string;
  port: number;
  username: string;
  credential: CredentialInput;
};

/** 更新入参：credential 为 null = 保持现有凭据不变（密码留空）。 */
export type ServerUpdateInput = Omit<ServerInput, "credential"> & {
  credential: CredentialInput | null;
};

export function listServers(): Promise<Server[]> {
  return invoke("list_servers");
}

export function createServer(input: ServerInput): Promise<Server> {
  return invoke("create_server", {
    host: input.host,
    port: input.port,
    username: input.username,
    credential: input.credential,
  });
}

export function updateServer(id: number, input: ServerUpdateInput): Promise<Server> {
  return invoke("update_server", {
    id,
    host: input.host,
    port: input.port,
    username: input.username,
    credential: input.credential,
  });
}

export function deleteServer(id: number): Promise<boolean> {
  return invoke("delete_server", { id });
}

export function testServerConnection(
  input: ServerInput,
  acceptUnknownHostKey: boolean,
): Promise<TestOutcome> {
  return invoke("test_server_connection", {
    host: input.host,
    port: input.port,
    username: input.username,
    credential: input.credential,
    acceptUnknownHostKey,
  });
}

export function acceptHostKey(host: string, port: number, fingerprint: string): Promise<void> {
  return invoke("accept_host_key", { host, port, fingerprint });
}

/** 对已保存服务器做连接测试：用后端存储的凭据，凭据不出后端。 */
export function testSavedServerConnection(
  id: number,
  acceptUnknownHostKey: boolean,
): Promise<TestOutcome> {
  return invoke("test_saved_server_connection", { id, acceptUnknownHostKey });
}

// ---------------------------------------------------------------------------
// 首页服务器概览（ticket 12）
// ---------------------------------------------------------------------------

/** 一台服务器的最近成功部署时间（从未部署过为 null）。 */
export type ServerLastDeploy = {
  serverId: number;
  lastDeployAt: string | null;
};

/** TCP 层可达性结果（真实探测，但不含 SSH 认证）。 */
export type ServerReachability = {
  reachable: boolean;
  detail: string | null;
};

/** 每台已知服务器一行最近成功部署时间（首页服务器卡片）。 */
export function listServerLastDeploys(): Promise<ServerLastDeploy[]> {
  return invoke("list_server_last_deploys");
}

/** 对 host:port 做一次 TCP 可达性探测（首页加载时与手动"重新检测"）。 */
export function checkServerReachable(host: string, port: number): Promise<ServerReachability> {
  return invoke("check_server_reachable", { host, port });
}
