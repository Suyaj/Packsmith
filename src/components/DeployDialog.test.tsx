// 部署对话框（ticket 10 / 19）行为测试：确认解析后的上传计划——逐目标展示
// 文件名→目录映射行（默认勾选）、产物缺失行禁用+警告、空名行内联补选、
// 上传集合 = 勾选集合、跳过提示、目标多选、状态/输出快照展示、停止失败
// 中止重启 + 单独执行启动命令组、取消。

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

// 目标 1：具名行（产物在列）+ 空名行（部署时补选）；目标 2：一个缺失行 +
// 一个可解析行。
const targets: Target[] = [
  {
    id: 1,
    projectId: 5,
    serverId: 11,
    startCommands: "sh start.sh",
    stopCommands: "sh stop.sh",
    autoDeploy: false,
    files: [
      { id: 101, targetId: 1, fileName: "app-1.0.jar", uploadDir: "/srv/one", position: 0 },
      { id: 102, targetId: 1, fileName: "", uploadDir: "/srv/pick", position: 1 },
    ],
  },
  {
    id: 2,
    projectId: 5,
    serverId: 12,
    startCommands: "sh start.sh",
    stopCommands: "sh stop.sh",
    autoDeploy: true,
    files: [
      { id: 201, targetId: 2, fileName: "gone-1.0.jar", uploadDir: "/srv/two", position: 0 },
      { id: 202, targetId: 2, fileName: "other-1.0.jar", uploadDir: "/srv/two-b", position: 1 },
    ],
  },
];

const artifacts = [
  { file_name: "app-1.0.jar", size: 10 },
  { file_name: "other-1.0.jar", size: 20 },
];

type Assignment = { targetId: number; files: { rowId: number; fileName: string }[] };

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
        const assignments = args.assignments as Assignment[];
        for (const a of assignments) {
          push(a.targetId, { type: "started", target_ids: assignments.map((x) => x.targetId) });
          pushSuccess(a.targetId);
        }
        push(0, { type: "finished", cancelled: false });
        return { cancelled: false, targets: [] };
      }
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
  it("逐目标展示解析后的上传计划：行默认勾选、目录可见；未选目标时部署按钮禁用", async () => {
    render(
      <DeployDialog projectId={5} projectName="演示项目" historyId={7} open onClose={vi.fn()} />,
    );

    expect(await screen.findByText("web-1:22 — deploy")).toBeInTheDocument();
    expect(screen.getByText("web-2:22 — deploy")).toBeInTheDocument();

    // 目标 1 的两行都默认勾选，目录可见。
    expect(screen.getByTestId("deploy-row-check-1-101")).toBeChecked();
    expect(screen.getByTestId("deploy-row-check-1-102")).toBeChecked();
    expect(screen.getByTestId("deploy-plan-1")).toHaveTextContent("/srv/one");
    expect(screen.getByTestId("deploy-plan-1")).toHaveTextContent("/srv/pick");

    const submit = screen.getByTestId("deploy-submit");
    expect(submit).toBeDisabled();

    await userEvent.click(screen.getByTestId("deploy-target-1"));
    expect(submit).toBeEnabled();
  });

  it("产物缺失行禁用 + 标红警告，不进入 payload；同目标可解析行照常上传", async () => {
    render(
      <DeployDialog projectId={5} projectName="演示项目" historyId={7} open onClose={vi.fn()} />,
    );

    await userEvent.click(await screen.findByTestId("deploy-target-2"));

    // 缺失行：禁用 + 警告可见。
    const missing = screen.getByTestId("deploy-row-check-2-201");
    expect(missing).toBeDisabled();
    expect(missing).not.toBeChecked();
    expect(screen.getByTestId("deploy-row-warning-2-201")).toHaveTextContent("产物缺失");
    // 可解析行默认勾选。
    expect(screen.getByTestId("deploy-row-check-2-202")).toBeChecked();

    await userEvent.click(screen.getByTestId("deploy-submit"));
    await waitFor(() =>
      expect(screen.getByTestId("deploy-status-2")).toHaveTextContent("部署成功"),
    );

    const call = invokeMock.mock.calls.find(([cmd]) => cmd === "start_deploy");
    expect(call?.[1].assignments).toEqual([
      { targetId: 2, files: [{ rowId: 202, fileName: "" }] },
    ]);
  });

  it("空名行内联补选：默认预选首个产物，改动随 payload 提交", async () => {
    render(
      <DeployDialog projectId={5} projectName="演示项目" historyId={7} open onClose={vi.fn()} />,
    );

    await userEvent.click(await screen.findByTestId("deploy-target-1"));

    // 空名行显示「（未指定文件）」+ 内联选择器，默认预选首个产物。
    expect(screen.getByTestId("deploy-plan-1")).toHaveTextContent("（未指定文件）");
    const pick = screen.getByTestId("deploy-row-select-1-102") as HTMLSelectElement;
    expect(pick.value).toBe("app-1.0.jar");

    await userEvent.selectOptions(pick, "other-1.0.jar");
    await userEvent.click(screen.getByTestId("deploy-submit"));

    await waitFor(() =>
      expect(screen.getByTestId("deploy-status-1")).toHaveTextContent("部署成功"),
    );
    const call = invokeMock.mock.calls.find(([cmd]) => cmd === "start_deploy");
    expect(call?.[1].assignments).toEqual([
      {
        targetId: 1,
        files: [
          { rowId: 101, fileName: "" },
          { rowId: 102, fileName: "other-1.0.jar" },
        ],
      },
    ]);
  });

  it("取消勾选的行不进入 payload（上传集合 = 勾选集合）", async () => {
    render(
      <DeployDialog projectId={5} projectName="演示项目" historyId={7} open onClose={vi.fn()} />,
    );

    await userEvent.click(await screen.findByTestId("deploy-target-1"));
    await userEvent.click(screen.getByTestId("deploy-row-check-1-101"));
    await userEvent.click(screen.getByTestId("deploy-submit"));

    await waitFor(() =>
      expect(screen.getByTestId("deploy-status-1")).toHaveTextContent("部署成功"),
    );
    const call = invokeMock.mock.calls.find(([cmd]) => cmd === "start_deploy");
    expect(call?.[1].assignments).toEqual([
      { targetId: 1, files: [{ rowId: 102, fileName: "app-1.0.jar" }] },
    ]);
  });

  it("多目标：各目标的勾选行互不影响，一起进入 payload", async () => {
    render(
      <DeployDialog projectId={5} projectName="演示项目" historyId={7} open onClose={vi.fn()} />,
    );

    await userEvent.click(await screen.findByTestId("deploy-target-1"));
    await userEvent.click(await screen.findByTestId("deploy-target-2"));
    await userEvent.click(screen.getByTestId("deploy-submit"));

    await waitFor(() =>
      expect(screen.getByTestId("deploy-status-2")).toHaveTextContent("部署成功"),
    );
    const call = invokeMock.mock.calls.find(([cmd]) => cmd === "start_deploy");
    expect(call?.[1].historyId).toBe(7);
    expect(call?.[1].assignments).toEqual([
      {
        targetId: 1,
        files: [
          { rowId: 101, fileName: "" },
          { rowId: 102, fileName: "app-1.0.jar" },
        ],
      },
      { targetId: 2, files: [{ rowId: 202, fileName: "" }] },
    ]);
  });

  it("后端拒绝（artifact_not_found）时展示中文错误文案", async () => {
    invokeMock.mockImplementation(async (command: string) => {
      if (command === "list_deployment_targets") return targets;
      if (command === "list_servers") return servers;
      if (command === "list_build_artifacts") return artifacts;
      if (command === "start_deploy")
        return Promise.reject({ code: "artifact_not_found", message: "artifact missing.jar" });
      return null;
    });

    render(
      <DeployDialog projectId={5} projectName="演示项目" historyId={7} open onClose={vi.fn()} />,
    );
    await userEvent.click(await screen.findByTestId("deploy-target-1"));
    await userEvent.click(screen.getByTestId("deploy-submit"));

    expect(await screen.findByText(/产物列表/)).toBeInTheDocument();
  });

  it("跳过提示（skipped 事件）逐目标可见", async () => {
    invokeMock.mockImplementation(async (command: string, args: Record<string, unknown>) => {
      if (command === "list_deployment_targets") return targets;
      if (command === "list_servers") return servers;
      if (command === "list_build_artifacts") return artifacts;
      if (command === "start_deploy") {
        const id = (args.assignments as Assignment[])[0].targetId;
        push(id, { type: "skipped", target_id: id, message: "row 102 (部署时补选): 未选择构件，已跳过" });
        pushSuccess(id);
        push(0, { type: "finished", cancelled: false });
        return { cancelled: false, targets: [] };
      }
      return null;
    });

    render(
      <DeployDialog projectId={5} projectName="演示项目" historyId={7} open onClose={vi.fn()} />,
    );
    await userEvent.click(await screen.findByTestId("deploy-target-1"));
    await userEvent.click(screen.getByTestId("deploy-submit"));

    expect(await screen.findByTestId("deploy-skipped-1")).toHaveTextContent(
      "row 102 (部署时补选): 未选择构件，已跳过",
    );
    expect(screen.getByTestId("deploy-status-1")).toHaveTextContent("部署成功");
  });

  it("停止失败：不执行启动、错误与停止输出可见、可单独执行启动命令组", async () => {
    invokeMock.mockImplementation(async (command: string, args: Record<string, unknown>) => {
      if (command === "list_deployment_targets") return targets;
      if (command === "list_servers") return servers;
      if (command === "list_build_artifacts") return artifacts;
      if (command === "start_deploy") {
        const id = (args.assignments as Assignment[])[0].targetId;
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

    // 取消事件到达后目标显示"已取消"。
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
      if (command === "start_deploy") {
        // 只推进到上传中途：任务保持进行中，进度条停留可见。
        const id = (args.assignments as Assignment[])[0].targetId;
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
