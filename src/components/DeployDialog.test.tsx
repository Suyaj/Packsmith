// 部署对话框（ticket 10 / 23 / 26）行为测试：自由勾选——逐目标展示本次构建
// 的全部构件，可勾选/取消任意构件；映射行降级为预设（具名行对应的构件默认
// 勾选并沿用行目录，其他构件上传到目标默认上传目录）；空名行不产生预设、
// 不再单独展示（票 24/25 的补选 UI 移除）；payload 为每目标
// entries{fileName, uploadDir}；对话框级操作选择（票 23）与只读不回退；
// 跳过提示、状态/输出快照、停止失败中止重启、取消。

import { render, screen, waitFor, within } from "@testing-library/react";
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

// 目标 1：具名预设行 app-1.0.jar（产物在列）+ 空名行（不产生预设）+ 缺失
// 具名行（不在本次构建产物里，预设无效）；目标 2：具名预设行 other-1.0.jar。
const targets: Target[] = [
  {
    id: 1,
    projectId: 5,
    serverId: 11,
    uploadDir: "/srv/def-1",
    startCommands: "sh start.sh",
    stopCommands: "sh stop.sh",
    autoDeploy: false,
    files: [
      { id: 101, targetId: 1, fileName: "app-1.0.jar", uploadDir: "/srv/one", position: 0 },
      { id: 102, targetId: 1, fileName: "", uploadDir: "/srv/pick", position: 1 },
      { id: 103, targetId: 1, fileName: "missing-1.0.jar", uploadDir: "/srv/missing", position: 2 },
    ],
  },
  {
    id: 2,
    projectId: 5,
    serverId: 12,
    uploadDir: "/srv/def-2",
    startCommands: "sh start.sh",
    stopCommands: "sh stop.sh",
    autoDeploy: true,
    files: [
      { id: 201, targetId: 2, fileName: "other-1.0.jar", uploadDir: "/srv/two", position: 0 },
    ],
  },
];

const artifacts = [
  { file_name: "app-1.0.jar", size: 10 },
  { file_name: "other-1.0.jar", size: 20 },
];

type Assignment = { targetId: number; entries: { fileName: string; uploadDir: string }[] };

function push(targetId: number, event: DeployEvent) {
  for (const ch of channels) ch.onmessage?.({ deployId: 1, event: { ...event, target_id: targetId } as DeployEvent });
}

// 模拟后端 Channel 的真实投递：Rust 端 `Channel::send` 序列化后的 JSON 原样
// 到达 `onmessage`（不经任何包装）。用字面量 JSON 字符串锁线上的形状。
function pushRaw(...payloads: string[]) {
  for (const json of payloads) {
    const message = JSON.parse(json);
    for (const ch of channels) ch.onmessage?.(message);
  }
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

describe("DeployDialog（票 26 自由勾选）", () => {
  it("逐目标展示本次构建的全部构件：预设行默认勾选并沿用行目录，其他构件默认不勾选、目录为默认上传目录", async () => {
    render(
      <DeployDialog projectId={5} projectName="演示项目" historyId={7} open onClose={vi.fn()} />,
    );

    expect(await screen.findByText("web-1:22 — deploy")).toBeInTheDocument();

    // 计划区 = 全部构件（两条），不再是映射行集合：空名行与缺失具名行不展示。
    const plan = screen.getByTestId("deploy-plan-1");
    expect(within(plan).getAllByRole("checkbox")).toHaveLength(artifacts.length);
    expect(plan).toHaveTextContent("app-1.0.jar");
    expect(plan).toHaveTextContent("other-1.0.jar");
    expect(plan).not.toHaveTextContent("missing-1.0.jar");
    expect(plan).not.toHaveTextContent("（未指定文件）");

    // 预设：app-1.0.jar 勾选、目录沿用行目录 /srv/one；other-1.0.jar 不勾选、
    // 目录为目标默认上传目录 /srv/def-1。
    expect(screen.getByTestId("deploy-entry-check-1-0")).toBeChecked();
    expect(screen.getByTestId("deploy-entry-check-1-1")).not.toBeChecked();
    expect(screen.getByTestId("deploy-entry-dir-1-0")).toHaveTextContent("/srv/one");
    expect(screen.getByTestId("deploy-entry-dir-1-1")).toHaveTextContent("/srv/def-1");

    // 目标 2 的预设互相独立：other-1.0.jar（构件序 1）勾选，app（序 0）不勾选。
    expect(screen.getByTestId("deploy-entry-check-2-0")).not.toBeChecked();
    expect(screen.getByTestId("deploy-entry-check-2-1")).toBeChecked();
    expect(screen.getByTestId("deploy-entry-dir-2-0")).toHaveTextContent("/srv/def-2");
    expect(screen.getByTestId("deploy-entry-dir-2-1")).toHaveTextContent("/srv/two");

    // 未选目标时部署按钮禁用；选中目标（有预设勾选）后可用。
    const submit = screen.getByTestId("deploy-submit");
    expect(submit).toBeDisabled();
    await userEvent.click(screen.getByTestId("deploy-target-1"));
    expect(submit).toBeEnabled();
  });

  it("自由勾选：预设可取消、非预设可勾上，payload entries 携带文件名与目录（自由条目 = 默认目录）", async () => {
    render(
      <DeployDialog projectId={5} projectName="演示项目" historyId={7} open onClose={vi.fn()} />,
    );

    await userEvent.click(await screen.findByTestId("deploy-target-1"));
    // 取消预设 app-1.0.jar，勾上自由条目 other-1.0.jar。
    await userEvent.click(screen.getByTestId("deploy-entry-check-1-0"));
    await userEvent.click(screen.getByTestId("deploy-entry-check-1-1"));
    await userEvent.click(screen.getByTestId("deploy-submit"));

    await waitFor(() =>
      expect(screen.getByTestId("deploy-status-1")).toHaveTextContent("部署成功"),
    );
    const call = invokeMock.mock.calls.find(([cmd]) => cmd === "start_deploy");
    expect(call?.[1].assignments).toEqual([
      {
        targetId: 1,
        entries: [{ fileName: "other-1.0.jar", uploadDir: "/srv/def-1" }],
      },
    ]);
  });

  it("多目标：各目标的勾选互不影响，一起进入 payload", async () => {
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
    expect(call?.[1].operation).toBe("restart");
    expect(call?.[1].assignments).toEqual([
      {
        targetId: 1,
        entries: [{ fileName: "app-1.0.jar", uploadDir: "/srv/one" }],
      },
      {
        targetId: 2,
        entries: [{ fileName: "other-1.0.jar", uploadDir: "/srv/two" }],
      },
    ]);
  });

  it("预设只由具名映射行驱动：默认构件列表不再预选（票 21 回退在对话框中不生效）", async () => {
    mockBackend({
      targets: [
        { ...targets[0], files: [] },
      ],
    });
    render(
      <DeployDialog
        projectId={5}
        projectName="演示项目"
        historyId={7}
        open
        onClose={vi.fn()}
      />,
    );

    // 无映射行 = 无预设：所有构件都不勾选；勾选后目录为默认上传目录。
    expect(await screen.findByTestId("deploy-entry-check-1-0")).not.toBeChecked();
    expect(screen.getByTestId("deploy-entry-check-1-1")).not.toBeChecked();

    await userEvent.click(screen.getByTestId("deploy-target-1"));
    await userEvent.click(screen.getByTestId("deploy-entry-check-1-1"));
    await userEvent.click(screen.getByTestId("deploy-submit"));
    await waitFor(() =>
      expect(screen.getByTestId("deploy-status-1")).toHaveTextContent("部署成功"),
    );
    const call = invokeMock.mock.calls.find(([cmd]) => cmd === "start_deploy");
    expect(call?.[1].assignments).toEqual([
      { targetId: 1, entries: [{ fileName: "other-1.0.jar", uploadDir: "/srv/def-1" }] },
    ]);
  });

  it("空名行不再单独展示：无补选入口（deploy-batch 移除）、无「未指定」展示", async () => {
    render(
      <DeployDialog projectId={5} projectName="演示项目" historyId={7} open onClose={vi.fn()} />,
    );

    await screen.findByTestId("deploy-plan-1");
    expect(screen.queryByTestId("deploy-batch")).not.toBeInTheDocument();
    expect(screen.queryByTestId("deploy-batch-apply")).not.toBeInTheDocument();
    expect(screen.queryByText("（未指定文件）")).not.toBeInTheDocument();
    // 空名行的目录 /srv/pick 不出现在计划区（空名行不产生任何条目）。
    expect(screen.getByTestId("deploy-plan-1")).not.toHaveTextContent("/srv/pick");
  });

  it("重启时没有任何勾选：部署按钮禁用（仅停止/仅启动不依赖勾选）", async () => {
    render(
      <DeployDialog projectId={5} projectName="演示项目" historyId={7} open onClose={vi.fn()} />,
    );
    await userEvent.click(await screen.findByTestId("deploy-target-2"));

    // 目标 2 唯一预设 other-1.0.jar（构件序 1）：取消后重启不可发起。
    await userEvent.click(screen.getByTestId("deploy-entry-check-2-1"));
    expect(screen.getByTestId("deploy-submit")).toBeDisabled();

    // 仅停止/仅启动：命令组操作不依赖勾选，仍可发起。
    await userEvent.selectOptions(screen.getByTestId("deploy-operation"), "stop");
    expect(screen.getByTestId("deploy-submit")).toBeEnabled();
    await userEvent.selectOptions(screen.getByTestId("deploy-operation"), "start");
    expect(screen.getByTestId("deploy-submit")).toBeEnabled();
  });

  it("后端拒绝（empty_upload_dir / artifact_not_found）时展示中文错误文案", async () => {
    invokeMock.mockImplementation(async (command: string) => {
      if (command === "list_deployment_targets") return targets;
      if (command === "list_servers") return servers;
      if (command === "list_build_artifacts") return artifacts;
      if (command === "start_deploy")
        return Promise.reject({ code: "empty_upload_dir", message: "entry" });
      return null;
    });

    render(
      <DeployDialog projectId={5} projectName="演示项目" historyId={7} open onClose={vi.fn()} />,
    );
    await userEvent.click(await screen.findByTestId("deploy-target-1"));
    await userEvent.click(screen.getByTestId("deploy-submit"));

    expect(await screen.findByText(/上传目录不能为空/)).toBeInTheDocument();

    // artifact_not_found 的既有文案不回退。
    invokeMock.mockImplementation(async (command: string) => {
      if (command === "start_deploy")
        return Promise.reject({ code: "artifact_not_found", message: "missing.jar" });
      if (command === "list_deployment_targets") return targets;
      if (command === "list_servers") return servers;
      if (command === "list_build_artifacts") return artifacts;
      return null;
    });
    await userEvent.click(screen.getByTestId("deploy-submit"));
    expect(await screen.findByText(/产物列表/)).toBeInTheDocument();
  });

  it("仅停止：上传计划只读（勾选禁用），payload 只带目标（entries 空）", async () => {
    invokeMock.mockImplementation(async (command: string, args: Record<string, unknown>) => {
      if (command === "list_deployment_targets") return targets;
      if (command === "list_servers") return servers;
      if (command === "list_build_artifacts") return artifacts;
      if (command === "start_deploy") {
        const id = (args.assignments as Assignment[])[0].targetId;
        push(id, { type: "phase", target_id: id, phase: "stop", status: "running" });
        push(id, { type: "output", target_id: id, phase: "stop", output: "stopped ok", exit_code: 0 });
        push(id, {
          type: "target_finished",
          target_id: id,
          status: "success",
          stop_failed: false,
          error: null,
        });
        push(0, { type: "finished", cancelled: false });
        return { cancelled: false, targets: [] };
      }
      return null;
    });

    render(
      <DeployDialog projectId={5} projectName="演示项目" historyId={7} open onClose={vi.fn()} />,
    );
    await userEvent.click(await screen.findByTestId("deploy-target-1"));
    await userEvent.selectOptions(screen.getByTestId("deploy-operation"), "stop");

    // 上传计划区只读：条目勾选禁用、目录仍可见（不回退票 23 行为）。
    expect(screen.getByTestId("deploy-entry-check-1-0")).toBeDisabled();
    expect(screen.getByTestId("deploy-entry-check-1-1")).toBeDisabled();

    await userEvent.click(screen.getByTestId("deploy-submit"));
    await waitFor(() =>
      expect(screen.getByTestId("deploy-status-1")).toHaveTextContent("部署成功"),
    );

    const call = invokeMock.mock.calls.find(([cmd]) => cmd === "start_deploy");
    expect(call?.[1].operation).toBe("stop");
    expect(call?.[1].assignments).toEqual([{ targetId: 1, entries: [] }]);
    expect(screen.getByTestId("deploy-output-1-stop")).toHaveTextContent("stopped ok");
    expect(screen.queryByTestId("deploy-output-1-upload")).not.toBeInTheDocument();
  });

  it("跳过提示（skipped 事件）逐目标可见", async () => {
    invokeMock.mockImplementation(async (command: string, args: Record<string, unknown>) => {
      if (command === "list_deployment_targets") return targets;
      if (command === "list_servers") return servers;
      if (command === "list_build_artifacts") return artifacts;
      if (command === "start_deploy") {
        const id = (args.assignments as Assignment[])[0].targetId;
        push(id, { type: "skipped", target_id: id, message: "target skipped notice" });
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
      "target skipped notice",
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
    expect(screen.getByTestId("deploy-output-1-stop")).toHaveTextContent("permission denied");
    expect(screen.getByText(/停止命令组失败/)).toBeInTheDocument();
    expect(screen.queryByTestId("deploy-output-1-start")).not.toBeInTheDocument();

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

    push(1, { type: "started", target_ids: [1] });
    const cancel = await screen.findByTestId("deploy-cancel");
    await userEvent.click(cancel);
    expect(invokeMock).toHaveBeenCalledWith("cancel_deploy", { deployId: 1 });

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

  // 线上契约：后端 `start_deploy` / `run_start_group` 经 `tauri::ipc::Channel`
  // 推送的是 `Channel<AutoDeployEvent>::send` 序列化后的原始 JSON——信封
  // `{ deployId, event }`（camelCase）。事件本体不经任何再包装。golden 字符串与
  // src-tauri/src/deploy/commands.rs 的 event_envelope_serializes_to_the_frontend_contract
  // 互为镜像。
  it("后端 Channel 消息按线上真实 JSON 形状驱动状态机离开等待，deployId 可用于取消", async () => {
    invokeMock.mockImplementation(async (command: string) => {
      if (command === "list_deployment_targets") return targets;
      if (command === "list_servers") return servers;
      if (command === "list_build_artifacts") return artifacts;
      if (command === "start_deploy") {
        pushRaw('{"deployId":7,"event":{"type":"started","target_ids":[1]}}');
        return new Promise(() => {});
      }
      return null;
    });

    render(
      <DeployDialog projectId={5} projectName="演示项目" historyId={7} open onClose={vi.fn()} />,
    );
    await userEvent.click(await screen.findByTestId("deploy-target-1"));
    await userEvent.click(screen.getByTestId("deploy-submit"));

    await userEvent.click(await screen.findByTestId("deploy-cancel"));
    expect(invokeMock).toHaveBeenCalledWith("cancel_deploy", { deployId: 7 });

    pushRaw(
      '{"deployId":7,"event":{"type":"phase","target_id":1,"phase":"upload","status":"running"}}',
      '{"deployId":7,"event":{"type":"target_finished","target_id":1,"status":"success","stop_failed":false,"error":null}}',
      '{"deployId":7,"event":{"type":"finished","cancelled":false}}',
    );
    await waitFor(() =>
      expect(screen.getByTestId("deploy-status-1")).toHaveTextContent("部署成功"),
    );
  });

  it("操作选择器默认重启，四个操作可选", async () => {
    render(
      <DeployDialog projectId={5} projectName="演示项目" historyId={7} open onClose={vi.fn()} />,
    );

    const selector = await screen.findByTestId("deploy-operation") as HTMLSelectElement;
    expect(selector.value).toBe("restart");
    expect(
      Array.from(selector.options).map((o) => o.textContent),
    ).toEqual(["重启", "仅上传", "仅停止", "仅启动"]);
  });

  it("仅上传：payload 携带 operation=upload，后端只跑上传阶段", async () => {
    invokeMock.mockImplementation(async (command: string, args: Record<string, unknown>) => {
      if (command === "list_deployment_targets") return targets;
      if (command === "list_servers") return servers;
      if (command === "list_build_artifacts") return artifacts;
      if (command === "start_deploy") {
        const id = (args.assignments as Assignment[])[0].targetId;
        push(id, { type: "phase", target_id: id, phase: "upload", status: "running" });
        push(id, { type: "upload_progress", target_id: id, transferred: 10, total: 10 });
        push(id, { type: "output", target_id: id, phase: "upload", output: "uploaded 10 bytes", exit_code: 0 });
        push(id, {
          type: "target_finished",
          target_id: id,
          status: "success",
          stop_failed: false,
          error: null,
        });
        push(0, { type: "finished", cancelled: false });
        return { cancelled: false, targets: [] };
      }
      return null;
    });

    render(
      <DeployDialog projectId={5} projectName="演示项目" historyId={7} open onClose={vi.fn()} />,
    );
    await userEvent.click(await screen.findByTestId("deploy-target-1"));
    await userEvent.selectOptions(screen.getByTestId("deploy-operation"), "upload");
    await userEvent.click(screen.getByTestId("deploy-submit"));

    await waitFor(() =>
      expect(screen.getByTestId("deploy-status-1")).toHaveTextContent("部署成功"),
    );
    const call = invokeMock.mock.calls.find(([cmd]) => cmd === "start_deploy");
    expect(call?.[1].operation).toBe("upload");
    expect(screen.getByTestId("deploy-output-1-upload")).toHaveTextContent("uploaded 10 bytes");
    expect(screen.queryByTestId("deploy-output-1-stop")).not.toBeInTheDocument();
    expect(screen.queryByTestId("deploy-output-1-start")).not.toBeInTheDocument();
  });

  it("没有入库构件时计划区为空、重启按钮禁用", async () => {
    mockBackend({ artifacts: [] });
    render(
      <DeployDialog projectId={5} projectName="演示项目" historyId={7} open onClose={vi.fn()} />,
    );

    expect(await screen.findByText("该构建没有入库构件")).toBeInTheDocument();
    expect(screen.getByTestId("deploy-submit")).toBeDisabled();
  });
});
