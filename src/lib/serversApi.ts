// Server management API: the typed boundary between the UI and the Tauri
// backend. Every call is `invoke` under the hood, so frontend tests mock
// `@tauri-apps/api/core` and verify these exact payload shapes.

import { invoke } from "@tauri-apps/api/core";

export type Credential =
  | { type: "private_key"; path: string; passphrase: string | null }
  | { type: "password"; password: string };

export type Server = {
  id: number;
  host: string;
  port: number;
  username: string;
  credential: Credential;
};

export type TestOutcome =
  | { status: "success"; new_fingerprint: string | null }
  | { status: "host_key_unknown"; fingerprint: string }
  | { status: "host_key_changed"; fingerprint: string }
  | { status: "unreachable"; detail: string }
  | { status: "auth_failed"; method: "password" | "private_key" }
  | { status: "key_unreadable"; detail: string };

export type ServerInput = {
  host: string;
  port: number;
  username: string;
  credential: Credential;
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

export function updateServer(id: number, input: ServerInput): Promise<Server> {
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
