// 托管工具链的前端接口封装（票 03）。
// ADR-0001：PATH 检测仅展示；执行只用托管工具链。

import { Channel, invoke } from "@tauri-apps/api/core";

export type ToolKind = "git" | "jdk" | "mvnd";

export type ToolState =
  | { state: "managed"; version: string; path: string }
  | { state: "detected"; path: string }
  | { state: "missing" };

export type ToolStatus = {
  tool: ToolKind;
  state: ToolState;
  canDownload: boolean;
};

export type ProgressEvent = {
  tool: ToolKind;
  phase: "downloading" | "extracting" | "installing";
  received: number;
  total: number | null;
};

export function fetchToolchainStatus(): Promise<ToolStatus[]> {
  return invoke<ToolStatus[]>("toolchain_status");
}

export function downloadTool(
  tool: ToolKind,
  onProgress: Channel<ProgressEvent>,
): Promise<ToolStatus> {
  return invoke<ToolStatus>("toolchain_download", { tool, onProgress });
}
