// 部署目标 API: the typed boundary between the UI and the Tauri backend
// (ticket 09 — CRUD only; execution lives in ticket 10). Every call is
// `invoke` under the hood; frontend tests mock `@tauri-apps/api/core` and
// verify these exact payload shapes.
//
// ticket 19: 上传目录表格化——目标携带 文件名→目录 映射行 (files)；旧的
// 单一 uploadDir 字段已废弃，不再出现在接口里。

import { invoke } from "@tauri-apps/api/core";

/** 一行 文件名→上传目录 映射（ticket 19）。fileName 为空 = 部署时补选。 */
export type TargetFile = {
  id: number;
  targetId: number;
  fileName: string;
  uploadDir: string;
  position: number;
};

/** 新建/保存目标时提交的映射行（整组替换，无 id）。 */
export type TargetFileInput = {
  fileName: string;
  uploadDir: string;
};

export type Target = {
  id: number;
  projectId: number;
  serverId: number;
  startCommands: string;
  stopCommands: string;
  /** 构建成功后自动连锁部署（ticket 10，默认关）。 */
  autoDeploy: boolean;
  /** 文件名→上传目录映射行（ticket 19）。 */
  files: TargetFile[];
};

export type TargetInput = {
  serverId: number;
  files: TargetFileInput[];
  startCommands: string;
  stopCommands: string;
  autoDeploy?: boolean;
};

export function listDeploymentTargets(projectId: number): Promise<Target[]> {
  return invoke("list_deployment_targets", { projectId });
}

export function createDeploymentTarget(
  projectId: number,
  input: TargetInput,
): Promise<Target> {
  return invoke("create_deployment_target", {
    projectId,
    serverId: input.serverId,
    files: input.files,
    startCommands: input.startCommands,
    stopCommands: input.stopCommands,
  });
}

export function updateDeploymentTarget(
  id: number,
  input: TargetInput,
  projectId: number,
): Promise<Target> {
  return invoke("update_deployment_target", {
    id,
    projectId,
    serverId: input.serverId,
    files: input.files,
    startCommands: input.startCommands,
    stopCommands: input.stopCommands,
    autoDeploy: input.autoDeploy ?? false,
  });
}

export function deleteDeploymentTarget(id: number): Promise<boolean> {
  return invoke("delete_deployment_target", { id });
}
