// 构建历史与构件库 API（ticket 08）：UI 与 Tauri 后端之间的类型化边界。
// 每个函数底层都是 `invoke`，前端测试 mock `@tauri-apps/api/core` 并按这里
// 的确切 payload 形状断言。

import { invoke } from "@tauri-apps/api/core";

/** 一条构建历史记录（后端 HistoryEntry，项目名已联查）。 */
export type HistoryEntry = {
  id: number;
  build_id: string;
  project_id: number;
  project_name: string;
  branch: string;
  success: boolean;
  exit_code: number | null;
  log_path: string;
  /** 构件库目录；失败构建为 null。 */
  artifacts_dir: string | null;
  created_at: string;
};

/** 库内一个构件文件（文件名即上传部署使用的原文件名）。 */
export type ArtifactFile = {
  file_name: string;
  size: number;
};

/** 构建历史列表（跨项目，最近在前）；`limit` 只取最近 N 条。 */
export function listBuildHistory(limit?: number | null): Promise<HistoryEntry[]> {
  return invoke("list_build_history", { limit: limit ?? null });
}

/** 某项目的构建历史（最近在前）。 */
export function listProjectBuildHistory(projectId: number): Promise<HistoryEntry[]> {
  return invoke("list_project_build_history", { projectId });
}

/** 一次构建入库的构件文件列表（按文件名排序）。 */
export function listBuildArtifacts(historyId: number): Promise<ArtifactFile[]> {
  return invoke("list_build_artifacts", { historyId });
}

/** 删除一条构建历史（构件目录 + 构建日志 + 记录行）。 */
export function deleteBuildHistory(historyId: number): Promise<void> {
  return invoke("delete_build_history", { historyId });
}

/** 手动清理：清空某项目的全部构建历史与构件。 */
export function cleanupProjectHistory(projectId: number): Promise<void> {
  return invoke("cleanup_project_history", { projectId });
}

/** 读取构件库保留次数（每项目保留最近 N 次构建）。 */
export function getBuildRetention(): Promise<number> {
  return invoke("get_build_retention");
}

/** 保存构件库保留次数。 */
export function setBuildRetention(retention: number): Promise<void> {
  return invoke("set_build_retention", { retention });
}
