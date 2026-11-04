// 部署执行 API（ticket 10）：UI 与 Tauri 后端之间的类型化边界。
// 每个函数底层都是 `invoke`，前端测试 mock `@tauri-apps/api/core` 并按这里
// 的确切 payload 形状断言。事件经 Channel 推送，信封为 AutoDeployMessage
// （deployId + 事件本体），使前端可在部署进行中按 id 取消。

import { invoke, Channel } from "@tauri-apps/api/core";

/** 部署阶段：上传 / 停止 / 启动。 */
export type DeployPhase = "upload" | "stop" | "start";

/**
 * 部署操作（票 23，对话框级，对本次勾选的所有目标统一生效）：
 * 重启（默认）= 上传 + 停止 + 启动；仅上传 / 仅停止 / 仅启动各跑对应阶段。
 */
export type DeployOperation = "restart" | "upload" | "stop" | "start";

/** 单个部署目标的状态（与后端 TargetStatus 一一对应）。 */
export type TargetStatus =
  | "pending"
  | "uploading"
  | "stopping"
  | "starting"
  | "success"
  | "failed"
  | "cancelled";

/** 部署过程事件（后端 DeployEvent）。 */
export type DeployEvent =
  | { type: "started"; target_ids: number[] }
  | { type: "phase"; target_id: number; phase: DeployPhase; status: string }
  | {
      type: "upload_progress";
      target_id: number;
      transferred: number;
      total: number;
    }
  | {
      type: "output";
      target_id: number;
      phase: DeployPhase;
      output: string;
      exit_code: number;
    }
  | {
      type: "target_finished";
      target_id: number;
      status: TargetStatus;
      stop_failed: boolean;
      error: string | null;
    }
  /** 一条上传行被跳过的提示（ticket 19：空名行/缺失行跳过有提示）。 */
  | { type: "skipped"; target_id: number; message: string }
  | { type: "finished"; cancelled: boolean };

/** 事件信封：deployId 让前端能在任务进行中取消。 */
export type AutoDeployMessage = {
  deployId: number;
  event: DeployEvent;
};

/** 一个阶段的输出快照（后端 ExecOutput）。 */
export type ExecOutput = { exit_code: number; combined: string };

/** 部署结束后单个目标的结果。 */
export type TargetOutcome = {
  target_id: number;
  status: TargetStatus;
  upload: ExecOutput | null;
  stop: ExecOutput | null;
  start: ExecOutput | null;
  stop_failed: boolean;
  error: string | null;
};

/** 部署任务整体结果。 */
export type DeploySummary = {
  cancelled: boolean;
  targets: TargetOutcome[];
};

/**
 * 发起部署（票 19 上传计划确认 / 票 23 操作选择）：`operation` 为对话框级
 * 操作；仅重启/仅上传时每个目标提交其勾选且可解析的文件名→目录映射行
 * （上传集合 = 勾选集合，原文件名覆盖，传到各自行目录），仅停止/仅启动时
 * `files` 为空、只按目标执行命令组。`fileName` 仅对空名行（部署时补选）
 * 有意义——具名行以后端库内文件名为准。
 */
export type RowAssignment = {
  rowId: number;
  fileName: string;
};

export type TargetAssignment = {
  targetId: number;
  files: RowAssignment[];
};

export function startDeploy(
  historyId: number,
  operation: DeployOperation,
  assignments: TargetAssignment[],
  onEvent: Channel<AutoDeployMessage>,
): Promise<DeploySummary> {
  return invoke("start_deploy", {
    historyId,
    operation,
    assignments,
    onEvent,
  });
}

/** 取消进行中的部署任务。 */
export function cancelDeploy(deployId: number): Promise<boolean> {
  return invoke("cancel_deploy", { deployId });
}

/** 单独执行某部署目标的启动命令组（停止失败后的手动恢复操作）。 */
export function runStartGroup(
  targetId: number,
  onEvent: Channel<AutoDeployMessage>,
): Promise<DeploySummary> {
  return invoke("run_start_group", { targetId, onEvent });
}

/** 切换某部署目标的“构建成功后自动连锁部署”开关。 */
export function setTargetAutoDeploy(id: number, enabled: boolean): Promise<boolean> {
  return invoke("set_target_auto_deploy", { id, enabled });
}

/** 订阅自动连锁部署的应用事件（构建成功后台触发）。 */
export async function listenAutoDeploy(
  handler: (message: AutoDeployMessage) => void,
): Promise<() => void> {
  const { listen } = await import("@tauri-apps/api/event");
  return listen<AutoDeployMessage>("deploy-event", (event) => handler(event.payload));
}
