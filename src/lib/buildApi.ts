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
  /** 项目构建命令（ticket 07），随项目可编辑。 */
  build_command: string;
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

// ---------------------------------------------------------------------------
// 构建执行（ticket 07）
// ---------------------------------------------------------------------------

/** 打包过程事件：克隆进度或构建日志行（后端 BuildEvent）。 */
export type BuildEvent =
  | {
      type: "clone";
      stage: string;
      received?: number | null;
      total?: number | null;
    }
  | { type: "log"; line: string };

/** 一次打包的最终结果（失败构建 = success:false，不是 IPC 错误）。 */
export type BuildOutcome = {
  success: boolean;
  exit_code: number | null;
  build_id: string;
  /** 持久化构建日志的绝对路径（失败后仍可读取）。 */
  log_path: string;
};

/** 发起完整打包：准备临时工作区 → 托管工具链构建 → 清理，事件实时推送。 */
export function startBuild(
  projectId: number,
  branch: string,
  onEvent: Channel<BuildEvent>,
): Promise<BuildOutcome> {
  return invoke("start_build", { projectId, branch, onEvent });
}

/** 保存项目构建命令（随项目可编辑并持久化）。 */
export function updateBuildCommand(projectId: number, command: string): Promise<Project> {
  return invoke("update_build_command", { projectId, command });
}

/** 读取一次构建的持久化日志（按 build_id）。 */
export function readBuildLog(buildId: string): Promise<string> {
  return invoke("read_build_log", { buildId });
}
