// Git 账号 (票 13) 的前端接口封装：clone / 分支拉取的 HTTPS 认证凭据。
// 后端出参是掩码模式——username 可直接展示，令牌只给 hasToken 指示，
// 明文令牌永不过 IPC（保存是单向写入，编辑留空 = 保持不变）。

import { invoke } from "@tauri-apps/api/core";

/** Git 账号状态（设置页卡片）。username 为 null 表示未配置账号。 */
export type GitAccount = {
  username: string | null;
  hasToken: boolean;
};

export function fetchGitAccount(): Promise<GitAccount> {
  return invoke("get_git_account");
}

/**
 * 保存 Git 账号。`token` 缺省（undefined）= 保持现有令牌不变（编辑留空）；
 * 传值即更新。单独清令牌无入口——显式清除账号用 clearGitAccount。
 */
export function saveGitAccount(username: string, token?: string): Promise<GitAccount> {
  const args: Record<string, unknown> = { username };
  if (token !== undefined) args.token = token;
  return invoke("set_git_account", args);
}

/** 显式清除 Git 账号（用户名与令牌一起删除）。 */
export function clearGitAccount(): Promise<GitAccount> {
  return invoke("clear_git_account");
}
