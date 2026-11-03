// 部署目标 API: the typed boundary between the UI and the Tauri backend
// (ticket 09 — CRUD only; execution lives in ticket 10). Every call is
// `invoke` under the hood; frontend tests mock `@tauri-apps/api/core` and
// verify these exact payload shapes.

import { invoke } from "@tauri-apps/api/core";

export type Target = {
  id: number;
  projectId: number;
  serverId: number;
  uploadDir: string;
  startCommands: string;
  stopCommands: string;
};

export type TargetInput = {
  serverId: number;
  uploadDir: string;
  startCommands: string;
  stopCommands: string;
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
    uploadDir: input.uploadDir,
    startCommands: input.startCommands,
    stopCommands: input.stopCommands,
  });
}

export function updateDeploymentTarget(
  id: number,
  input: TargetInput,
): Promise<Target> {
  return invoke("update_deployment_target", {
    id,
    serverId: input.serverId,
    uploadDir: input.uploadDir,
    startCommands: input.startCommands,
    stopCommands: input.stopCommands,
  });
}

export function deleteDeploymentTarget(id: number): Promise<boolean> {
  return invoke("delete_deployment_target", { id });
}
