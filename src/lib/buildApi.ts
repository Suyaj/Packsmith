// 打包（一）API：UI 与 Tauri 后端之间的类型化边界（ticket 06）。
// 每个函数底层都是 `invoke`，前端测试 mock `@tauri-apps/api/core`
// 并按这里的确切 payload 形状断言。

import { invoke, Channel } from "@tauri-apps/api/core";

export type ProjectSource = "git" | "local";
export type ProjectType = "maven" | "gradle";

export type Project = {
  id: number;
  name: string;
  source: ProjectSource;
  url?: string | null;
  path: string;
  project_type: ProjectType;
  /** 上次打包使用的分支；从未打包过为 undefined。 */
  last_branch?: string | null;
  created_at: string;
};

export type BuildProgress = {
  stage: string;
  received?: number;
  total?: number;
};

/** 临时工作区就绪结果（构建执行属于 ticket 07）。 */
export type PreparedWorkspace = { path: string };

/** 实时拉取项目分支列表（git 项目拉其 URL；本地导入项目拉本地路径）。 */
export function listRemoteBranches(projectId: number): Promise<string[]> {
  return invoke("list_remote_branches", { projectId });
}

/** 发起打包前半段：创建临时工作区并 clone 所选分支，成功即记住该分支。 */
export function startBuildWorkspace(
  projectId: number,
  branch: string,
  onProgress: Channel<BuildProgress>,
): Promise<PreparedWorkspace> {
  return invoke("start_build_workspace", { projectId, branch, onProgress });
}

/** 取消进行中的打包准备。 */
export function cancelBuildWorkspace(projectId: number): Promise<void> {
  return invoke("cancel_build_workspace", { projectId });
}
