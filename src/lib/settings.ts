// 应用设置的前端接口封装（票 11：关闭窗口行为）。
// 行为值是 Rust 侧的持久化契约：'minimize'（默认）| 'quit'。

import { invoke } from "@tauri-apps/api/core";

export type CloseWindowBehavior = "minimize" | "quit";

export function fetchCloseWindowBehavior(): Promise<CloseWindowBehavior> {
  return invoke<CloseWindowBehavior>("get_close_window_behavior");
}

export function saveCloseWindowBehavior(
  behavior: CloseWindowBehavior,
): Promise<void> {
  return invoke("set_close_window_behavior", { behavior });
}

/** 收窄 IPC 返回值：非法值一律回落默认（minimize），保证 UI 永远有定义。 */
export function normalizeCloseWindowBehavior(
  value: unknown,
): CloseWindowBehavior {
  return value === "quit" ? "quit" : "minimize";
}
