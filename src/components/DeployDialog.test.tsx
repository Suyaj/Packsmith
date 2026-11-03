// 部署对话框（ticket 10 / 17）行为测试：逐目标选择构件（一一对应上传）、
// 默认值（单产物自动选中 / 记住上次选择）、目标多选、状态/输出快照展示、
// 停止失败中止重启 + 单独执行启动命令组、取消。

import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";

import type { Target } from "@/lib/targetsApi";
import type { AutoDeployMessage, DeployEvent } from "@/lib/deployApi";

type ChannelStub = { onmessage: ((msg: AutoDeployMessage) => void) | null };

const invokeMock = vi.fn();

// Channel 桩：测试用 messageSink 捕获每个 invoke 创建的 Channel，模拟后端
// 推送事件。
const channels: ChannelStub[] = [];

vi.mock("@tauri-apps/api/core", () => ({
  invoke: (...args: unknown[]) => invokeMock(...args),
  Channel: class {
    onmessage: ((msg: AutoDeployMessage) => void) | null = null;
    constructor() {
      channels.push(this);
    }
  },
}));

import DeployDialog from "./DeployDialog";
import "@/i18n";

const servers = [
  { id: 11, host: "web-1", port: 22, username: "deploy", credential: { type: "password" as const, password: "x" } },
  { id: 12, host: "web-2", port: 22, username: "deploy", credential: { type: "password" as const, password: "x" } },
];

const targets: Target[] = [
  {
    id: 1,
    projectId: 5,
    serverId: 11,
    uploadDir: "/srv/one",
    startCommands: "sh start.sh",
    stopCommands: "sh stop.sh",
    autoDeploy: false,
    lastArtifact: null,
  },
  {
    id: 2,
    projectId: 5,
    serverId: 12,
    uploadDir: "/srv/two",
    startCommands: "sh start.sh",
    stopCommands: "sh stop.sh",
    autoDeploy: true,
    lastArtifact: null,
  },
];

const artifacts = [
  { file_name: "app-1.0.jar", size: 10 },
  { file_name: "other-1.0.jar", size: 20 },
];

function push(targetId: number, event: DeployEvent) {
  for (const ch of channels) ch.onmessage?.({ deployId: 1, event: { ...event, target_id: targetId } as DeployEvent });
}

function pushSuccess(id: number) {
  push(id, { type: "phase", target_id: id, phase: "upload", status: "running" });
  push(id, { type: "upload_progress", target_id: id, transferred: 10, total: 10 });
  push(id, { type: "output", target_id: id, phase: "upload", output: "uploaded 10 bytes", exit_code: 0 });
  push(id, { type: "phase", target_id: id, phase: "stop", status: "running" });
  push(id, { type: "output", target_id: id, phase: "stop", output: "stopping ok", exit_code: 0 });
  push(id, { type: "phase", target_id: id, phase: "start", status: "running" });
  push(id, { type: "output", target_id: id, phase: "start", output: "started ok", exit_code: 0 });
  push(id, {
    type: "target_finished",
    target_id: id,
    status: "success",
    stop_failed: false,
    error: null,
  });
}

function mockBackend(overrides: {
  targets?: Target[];
  artifacts?: { file_name: string; size: number }[];
} = {}) {
  invokeMock.mockImplementation(async (command: string, args: Record<string, unknown>) => {
    switch (command) {
      case "list_deployment_targets":
        return overrides.targets ?? targets;
      case "list_servers":
        return servers;
      case "list_build_artifacts":
        return overrides.artifacts ?? artifacts;
      case "start_deploy": {
        const assignments = args.assignments as { targetId: number; fileName: string }[];
        for (const a of assignments) {
          push(a.targetId, { type: "started", target_ids: assignments.map((x) => x.targetId) });
          pushSuccess(a.targetId);
        }
        push(0, { type: "finished", cancelled: false });
        return { cancelled: false, targets: [] };
      }
      case "set_target_last_artifact":
        return true;
      case "cancel_deploy":
        return true;
      case "run_start_group": {
        const id = args.targetId as number;
        push(id, { type: "phase", target_id: id, phase: "start", status: "running" });
        push(id, { type: "output", target_id: id, phase: "start", output: "manual start ok", exit_code: 0 });
        push(id, {
          type: "target_finished",
          target_id: id,
          status: "success",
          stop_failed: false,
          error: null,
        });
        return { cancelled: false, targets: [] };
      }
      default:
        return null;
    }
  });
}

beforeEach(() => {
  invokeMock.mockReset();
  channels.length = 0;
  mockBackend();
});

describe("DeployDialog", () => {
  it("列出部署目标（可多选）与逐目标构件选择器，未选择时部署按钮禁用", async () => {
    render(
      <DeployDialog projectId={5} projectName="演示项目" historyId={7} open onClose={vi.fn()} />,
    );

    expect(await screen.findByText("web-1:22 — deploy")).toBeInTheDocument();
    expect(screen.getByText("web-2:22 — deploy")).toBeInTheDocument();
    // 每个目标一行，行内有自己的构件选择器（单产物场景下自动选中）。
    expect(screen.getByTestId("deploy-artifact-select-1")).toBeInTheDocument();
    expect(screen.getByTestId("deploy-artifact-select-2")).toBeInTheDocument();

    const submit = screen.getByTestId("deploy-submit");
    expect(submit).toBeDisabled();

    await userEvent.click(screen.getByTestId("deploy-target-1"));
    expect(submit).toBeEnabled();
  });

  it("单产物时每个目标默认自动选中该产物", async () => {
    mockBackend({ artifacts: [{ file_name: "only-1.0.jar", size: 10 }] });
    render(
      <DeployDialog projectId={5} projectName="演示项目" historyId={7} open onClose={vi.fn()} />,
    );

    const select1 = (await screen.findByTestId(
      "deploy-artifact-select-1",
    )) as HTMLSelectElement;
    const select2 = screen.getByTestId("deploy-artifact-select-2") as HTMLSelectElement;
    expect(select1.value).toBe("only-1.0.jar");
    expect(select2.value).toBe("only-1.0.jar");
  });

  it("多产物时默认记住该目标上次的选择，无记录则退回第一个产物", async () => {
    mockBackend({
      targets: [
        { ...targets[0], lastArtifact: "other-1.0.jar" },
        targets[1],
      ],
    });
    render(
      <DeployDialog projectId={5} projectName="演示项目" historyId={7} open onClose={vi.fn()} />,
    );

    const select1 = (await screen.findByTestId(
      "deploy-artifact-select-1",
    )) as HTMLSelectElement;
    const select2 = screen.getByTestId("deploy-artifact-select-2") as HTMLSelectElement;
    expect(select1.value).toBe("other-1.0.jar");
    expect(select2.value).toBe("app-1.0.jar");
  });

  it("提交的 payload 中每个目标对应自己选择的文件（一一对应上传）", async () => {
    render(
      <DeployDialog projectId={5} projectName="演示项目" historyId={7} open onClose={vi.fn()} />,
    );

    await userEvent.click(await screen.findByTestId("deploy-target-1"));
    await userEvent.click(screen.getByTestId("deploy-target-2"));
    // 目标 2 改选另一个产物。
    await userEvent.selectOptions(screen.getByTestId("deploy-artifact-select-2"), "other-1.0.jar");
    await userEvent.click(screen.getByTestId("deploy-submit"));

    await waitFor(() =>
      expect(screen.getByTestId("deploy-status-1")).toHaveTextContent("部署成功"),
    );
    expect(screen.getByTestId("deploy-status-2")).toHaveTextContent("部署成功");
    // 一次调用携带逐目标的 assignments，且提交时记住各目标的选择。
    const call = invokeMock.mock.calls.find(([cmd]) => cmd === "start_deploy");
    expect(call?.[1]).toMatchObject({
      historyId: 7,
      assignments: [
        { targetId: 1, fileName: "app-1.0.jar" },
        { targetId: 2, fileName: "other-1.0.jar" },
      ],
    });
    expect(invokeMock).toHaveBeenCalledWith("set_target_last_artifact", {
      id: 1,
      fileName: "app-1.0.jar",
    });
    expect(invokeMock).toHaveBeenCalledWith("set_target_last_artifact", {
      id: 2,
      fileName: "other-1.0.jar",
    });
  });

  it("未被选中的目标不进入 payload（不存在全部上传路径）", async () => {
    render(
      <DeployDialog projectId={5} projectName="演示项目" historyId={7} open onClose={vi.fn()} />,
    );

    await userEvent.click(await screen.findByTestId("deploy-target-2"));
    await userEvent.click(screen.getByTestId("deploy-submit"));

    const call = invokeMock.mock.calls.find(([cmd]) => cmd === "start_deploy");
    expect(call?.[1]).toMatchObject({
      assignments: [{ targetId: 2, fileName: "app-1.0.jar" }],
    });
    expect(call?.[1].assignments).toHaveLength(1);
    // 未被选中的目标 1 也不被记住选择。
    expect(invokeMock).not.toHaveBeenCalledWith("set_target_last_artifact", {
      id: 1,
      fileName: expect.anything(),
    });
  });

  it("后端拒绝未知构件时展示中文错误文案", async () => {
    invokeMock.mockImplementation(async (command: string) => {
      if (command === "list_deployment_targets") return targets;
      if (command === "list_servers") return servers;
      if (command === "list_build_artifacts") return artifacts;
      if (command === "set_target_last_artifact") return true;
      if (command === "start_deploy")
        return Promise.reject({ code: "artifact_not_found", message: "artifact missing.jar" });
      return null;
    });

    render(
      <DeployDialog projectId={5} projectName="演示项目" historyId={7} open onClose={vi.fn()} />,
    );
    await userEvent.click(await screen.findByTestId("deploy-target-1"));
    await userEvent.click(screen.getByTestId("deploy-submit"));

    expect(await screen.findByRole("alert")).toHaveTextContent("产物列表");
  });

  it("停止失败：不执行启动、错误与停止输出可见、可单独执行启动命令组", async () => {
    invokeMock.mockImplementation(async (command: string, args: Record<string, unknown>) => {
      if (command === "list_deployment_targets") return targets;
      if (command === "list_servers") return servers;
      if (command === "list_build_artifacts") return artifacts;
      if (command === "set_target_last_artifact") return true;
      if (command === "start_deploy") {
        const id = (args.assignments as { targetId: number }[])[0].targetId;
        push(id, { type: "phase", target_id: id, phase: "stop", status: "running" });
        push(id, { type: "output", target_id: id, phase: "stop", output: "stop.sh: permission denied", exit_code: 1 });
        push(id, {
          type: "target_finished",
          target_id: id,
          status: "failed",
          stop_failed: true,
          error: "stop command group failed",
        });
        push(0, { type: "finished", cancelled: false });
        return { cancelled: false, targets: [] };
      }
      if (command === "run_start_group") {
        const id = args.targetId as number;
        push(id, { type: "phase", target_id: id, phase: "start", status: "running" });
        push(id, { type: "output", target_id: id, phase: "start", output: "manual start ok", exit_code: 0 });
        push(id, {
          type: "target_finished",
          target_id: id,
          status: "success",
          stop_failed: false,
          error: null,
        });
        return { cancelled: false, targets: [] };
      }
      return null;
    });

    render(
      <DeployDialog projectId={5} projectName="演示项目" historyId={7} open onClose={vi.fn()} />,
    );
    await userEvent.click(await screen.findByTestId("deploy-target-1"));
    await userEvent.click(screen.getByTestId("deploy-submit"));

    const status = await screen.findByTestId("deploy-status-1");
    expect(status).toHaveTextContent("部署失败");
    // 停止阶段输出与中止重启提示可见。
    expect(screen.getByTestId("deploy-output-1-stop")).toHaveTextContent("permission denied");
    expect(screen.getByText(/停止命令组失败/)).toBeInTheDocument();
    // 启动阶段的输出快照不存在（启动未执行）。
    expect(screen.queryByTestId("deploy-output-1-start")).not.toBeInTheDocument();

    // 单独执行启动命令组。
    await userEvent.click(screen.getByTestId("manual-start-1"));
    await waitFor(() =>
      expect(screen.getByTestId("deploy-output-1-start")).toHaveTextContent("manual start ok"),
    );
    expect(invokeMock).toHaveBeenCalledWith(
      "run_start_group",
      expect.objectContaining({ targetId: 1 }),
    );
  });

  it("部署进行中可取消", async () => {
    invokeMock.mockImplementation(async (command: string) => {
      if (command === "list_deployment_targets") return targets;
      if (command === "list_servers") return servers;
      if (command === "list_build_artifacts") return artifacts;
      if (command === "set_target_last_artifact") return true;
      if (command === "start_deploy") {
        // 部署任务保持进行中（invoke 挂起），直到取消事件到达。
        return new Promise(() => {});
      }
      return null;
    });

    render(
      <DeployDialog projectId={5} projectName="演示项目" historyId={7} open onClose={vi.fn()} />,
    );
    await userEvent.click(await screen.findByTestId("deploy-target-1"));
    await userEvent.click(screen.getByTestId("deploy-submit"));

    // invoke(start_deploy) 尚未 resolve —— 部署进行中；Started 事件带来 deployId。
    push(1, { type: "started", target_ids: [1] });
    const cancel = await screen.findByTestId("deploy-cancel");
    await userEvent.click(cancel);
    expect(invokeMock).toHaveBeenCalledWith("cancel_deploy", { deployId: 1 });

    // 取消事件到达后目标显示“已取消”。
    push(1, {
      type: "target_finished",
      target_id: 1,
      status: "cancelled",
      stop_failed: false,
      error: null,
    });
    push(0, { type: "finished", cancelled: true });
    await waitFor(() =>
      expect(screen.getByTestId("deploy-status-1")).toHaveTextContent("已取消"),
    );
  });

  it("上传进度可见", async () => {
    invokeMock.mockImplementation(async (command: string, args: Record<string, unknown>) => {
      if (command === "list_deployment_targets") return targets;
      if (command === "list_servers") return servers;
      if (command === "list_build_artifacts") return artifacts;
      if (command === "set_target_last_artifact") return true;
      if (command === "start_deploy") {
        // 只推进到上传中途：任务保持进行中，进度条停留可见。
        const id = (args.assignments as { targetId: number }[])[0].targetId;
        push(id, { type: "phase", target_id: id, phase: "upload", status: "running" });
        push(id, { type: "upload_progress", target_id: id, transferred: 5, total: 10 });
        return new Promise(() => {});
      }
      return null;
    });

    render(
      <DeployDialog projectId={5} projectName="演示项目" historyId={7} open onClose={vi.fn()} />,
    );
    await userEvent.click(await screen.findByTestId("deploy-target-1"));
    await userEvent.click(screen.getByTestId("deploy-submit"));
    await waitFor(() =>
      expect(screen.getByTestId("deploy-progress-1")).toHaveTextContent("5/10"),
    );
  });
});
