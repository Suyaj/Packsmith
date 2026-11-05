// 部署对话框（ticket 10 / 19 / 23 / 24 / 25）行为测试：确认解析后的上传计划——
// 逐目标展示文件名→目录映射行（默认勾选）、产物缺失行禁用+警告、空名行只读
// 展示补选结果（票 25：逐行下拉移除，批量补选为唯一填充入口，重复执行按序
// 覆盖重填）、上传集合 = 勾选集合、跳过提示、目标多选、状态/输出快照展示、
// 停止失败中止重启 + 单独执行启动命令组、取消。

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

  // 票 21：空名行预选按项目默认构件列表**依序**取第一个可用项；列表用尽
  // （全部缺失）回退首个可用产物。票 25：预选结果只读展示，无逐行下拉。
  it("空名行只读展示预选结果：默认构件列表依序取第一个可用项，列表用尽回退首个产物", async () => {
    render(
      <DeployDialog
        projectId={5}
        projectName="演示项目"
        historyId={7}
        open
        onClose={vi.fn()}
        defaultArtifacts={["missing.jar", "other-1.0.jar", "app-1.0.jar"]}
      />,
    );

    await userEvent.click(await screen.findByTestId("deploy-target-1"));

    // 逐行下拉已移除；只读展示取第一个可用项 other-1.0.jar（而非首个产物 app）。
    expect(screen.queryByTestId("deploy-row-select-1-102")).not.toBeInTheDocument();
    const pick = screen.getByTestId("deploy-row-pick-1-102");
    expect(pick).toHaveTextContent("other-1.0.jar");
    expect(pick).not.toHaveTextContent("app-1.0.jar");
  });

  // 票 25：空名行只读展示补选结果，逐行下拉移除、批量补选为唯一填充入口；
  // 行勾选（是否上传）仍可用。
  it("空名行只读展示补选结果，逐行下拉移除、行勾选仍可用", async () => {
    render(
      <DeployDialog projectId={5} projectName="演示项目" historyId={7} open onClose={vi.fn()} />,
    );

    await userEvent.click(await screen.findByTestId("deploy-target-1"));

    // 逐行下拉已移除；空名行只读展示补选结果（默认预选首个产物）。
    expect(screen.queryByTestId("deploy-row-select-1-102")).not.toBeInTheDocument();
    expect(screen.getByTestId("deploy-row-pick-1-102")).toHaveTextContent("app-1.0.jar");

    // 行勾选仍可用：取消勾选再勾回。
    const check = screen.getByTestId("deploy-row-check-1-102");
    await userEvent.click(check);
    expect(check).not.toBeChecked();
    await userEvent.click(check);
    expect(check).toBeChecked();
  });

  // 票 25：无入库构件时空名行没有补选结果，只读展示「未指定」。
  it("没有入库构件时空名行补选结果显示「未指定」", async () => {
    mockBackend({ artifacts: [] });
    render(
      <DeployDialog projectId={5} projectName="演示项目" historyId={7} open onClose={vi.fn()} />,
    );

    expect(await screen.findByTestId("deploy-row-pick-1-102")).toHaveTextContent("未指定");
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
    expect(call?.[1].operation).toBe("restart");
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

  // 线上契约：后端 `start_deploy` / `run_start_group` 经 `tauri::ipc::Channel`
  // 推送的是 `Channel<AutoDeployEvent>::send` 序列化后的原始 JSON——信封
  // `{ deployId, event }`（camelCase）。事件本体不经任何再包装。若形状漂移
  // （如裸事件或缺 deployId），状态机永远停在“等待”（用户症状：点部署后
  // 目标一直 pending），取消也拿不到 id。golden 字符串与
  // src-tauri/src/deploy/commands.rs 的 event_envelope_serializes_to_the_frontend_contract
  // 互为镜像。
  it("后端 Channel 消息按线上真实 JSON 形状驱动状态机离开等待，deployId 可用于取消", async () => {
    invokeMock.mockImplementation(async (command: string) => {
      if (command === "list_deployment_targets") return targets;
      if (command === "list_servers") return servers;
      if (command === "list_build_artifacts") return artifacts;
      if (command === "start_deploy") {
        // Started 事件先到（带信封 deployId），部署保持进行中（invoke 挂起）。
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

    // 信封里的 deployId 驱动取消。
    await userEvent.click(await screen.findByTestId("deploy-cancel"));
    expect(invokeMock).toHaveBeenCalledWith("cancel_deploy", { deployId: 7 });

    // 之后的事件按同一形状继续驱动状态机：等待 → 成功。
    pushRaw(
      '{"deployId":7,"event":{"type":"phase","target_id":1,"phase":"upload","status":"running"}}',
      '{"deployId":7,"event":{"type":"target_finished","target_id":1,"status":"success","stop_failed":false,"error":null}}',
      '{"deployId":7,"event":{"type":"finished","cancelled":false}}',
    );
    await waitFor(() =>
      expect(screen.getByTestId("deploy-status-1")).toHaveTextContent("部署成功"),
    );
  });

  // 票 23：对话框级操作选择——重启（默认）/ 仅上传 / 仅停止 / 仅启动。
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

  it("仅上传：payload 携带 operation=upload，后端只跑上传阶段（无停止/启动输出）", async () => {
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
    // 上传快照可见；停止/启动阶段从未执行。
    expect(screen.getByTestId("deploy-output-1-upload")).toHaveTextContent("uploaded 10 bytes");
    expect(screen.queryByTestId("deploy-output-1-stop")).not.toBeInTheDocument();
    expect(screen.queryByTestId("deploy-output-1-start")).not.toBeInTheDocument();
  });

  it("仅停止：上传计划只读（行禁用），payload 只带目标不带文件行", async () => {
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

    // 上传计划区只读：行勾选禁用；空名行补选结果仍只读展示（票 25）。
    expect(screen.getByTestId("deploy-row-check-1-101")).toBeDisabled();
    expect(screen.getByTestId("deploy-row-check-1-102")).toBeDisabled();
    expect(screen.getByTestId("deploy-row-pick-1-102")).toHaveTextContent("app-1.0.jar");

    await userEvent.click(screen.getByTestId("deploy-submit"));
    await waitFor(() =>
      expect(screen.getByTestId("deploy-status-1")).toHaveTextContent("部署成功"),
    );

    const call = invokeMock.mock.calls.find(([cmd]) => cmd === "start_deploy");
    expect(call?.[1].operation).toBe("stop");
    expect(call?.[1].assignments).toEqual([{ targetId: 1, files: [] }]);
    // 停止输出可见；无上传阶段产物。
    expect(screen.getByTestId("deploy-output-1-stop")).toHaveTextContent("stopped ok");
    expect(screen.queryByTestId("deploy-output-1-upload")).not.toBeInTheDocument();
  });

  it("仅停止/仅启动无需可上传行也能发起（重启时同一状态按钮禁用）", async () => {
    render(
      <DeployDialog projectId={5} projectName="演示项目" historyId={7} open onClose={vi.fn()} />,
    );
    await userEvent.click(await screen.findByTestId("deploy-target-1"));

    // 重启：取消所有可上传行后无法发起。
    await userEvent.click(screen.getByTestId("deploy-row-check-1-101"));
    await userEvent.click(screen.getByTestId("deploy-row-check-1-102"));
    expect(screen.getByTestId("deploy-submit")).toBeDisabled();

    // 仅停止：命令组操作不依赖上传计划，仍可发起。
    await userEvent.selectOptions(screen.getByTestId("deploy-operation"), "stop");
    expect(screen.getByTestId("deploy-submit")).toBeEnabled();

    // 仅启动同理。
    await userEvent.selectOptions(screen.getByTestId("deploy-operation"), "start");
    expect(screen.getByTestId("deploy-submit")).toBeEnabled();
  });

  // ---- 票 24 / 25：批量补选可模糊搜索（逐行下拉已移除，搜索能力留在批量入口）----

  it("批量补选支持模糊搜索：关键字实时过滤并高亮命中片段", async () => {
    render(
      <DeployDialog projectId={5} projectName="演示项目" historyId={7} open onClose={vi.fn()} />,
    );

    await userEvent.click(await screen.findByTestId("deploy-target-1"));
    await userEvent.click(screen.getByTestId("deploy-batch"));

    // 操作选择器是原生 select（其 option 也是 role=option），option 断言
    // 限定在补选下拉的列表框内（搜索输入框在列表框外）。
    const list = await screen.findByRole("listbox");
    const search = await screen.findByPlaceholderText("搜索构件…");
    await userEvent.type(search, "app");
    // 本构建产物里只有 app-1.0.jar 命中子序列 a-p-p（other-1.0.jar 无 p）。
    expect(within(list).getAllByRole("option").map((o) => o.textContent)).toEqual([
      "app-1.0.jar",
    ]);
    // 命中片段高亮：app-1.0.jar 中 a-p-p 三个字符。
    const hits = within(list).getAllByRole("option")[0].querySelectorAll("[data-highlight]");
    expect(hits).toHaveLength(3);
    expect(hits[0]).toHaveTextContent("a");
    expect(hits[2]).toHaveTextContent("p");

    await userEvent.clear(search);
    await userEvent.type(search, "zzz");
    expect(await screen.findByText("没有匹配的构件")).toBeInTheDocument();
    expect(within(list).queryByRole("option")).not.toBeInTheDocument();
  });

  // 下拉不再钉死为触发器宽度（触发器窄，长构件名会被截断——用户反馈）：
  // 最小宽度=触发器，选项不换行可撑开，上限为视口可用宽度。票 25：断言
  // 目标改为批量补选的 Combobox（逐行下拉已移除）。
  it("补选下拉宽度随内容撑开：最小触发器宽、选项不换行、上限可用宽", async () => {
    render(
      <DeployDialog projectId={5} projectName="演示项目" historyId={7} open onClose={vi.fn()} />,
    );

    await userEvent.click(await screen.findByTestId("deploy-target-1"));
    await userEvent.click(screen.getByTestId("deploy-batch"));

    const list = await screen.findByRole("listbox");
    const content = list.closest("[data-radix-popper-content-wrapper]")?.firstElementChild ??
      list.parentElement!;
    expect(content.className).toContain("min-w-(--radix-popover-trigger-width)");
    expect(content.className).toContain("max-w-(--radix-popover-content-available-width)");
    expect(within(list).getAllByRole("option")[0].querySelector("span.whitespace-nowrap"))
      .not.toBeNull();
  });

  // 批量补选目标：目标 1 一行具名 + 一行空名；目标 2 两行空名。
  const batchTargets: Target[] = [
    {
      id: 1,
      projectId: 5,
      serverId: 11,
      startCommands: "sh start.sh",
      stopCommands: "sh stop.sh",
      autoDeploy: false,
      files: [
        { id: 101, targetId: 1, fileName: "c-app.jar", uploadDir: "/srv/one", position: 0 },
        { id: 102, targetId: 1, fileName: "", uploadDir: "/srv/pick", position: 1 },
      ],
    },
    {
      id: 2,
      projectId: 5,
      serverId: 12,
      startCommands: "sh start.sh",
      stopCommands: "sh stop.sh",
      autoDeploy: false,
      files: [
        { id: 201, targetId: 2, fileName: "", uploadDir: "/srv/two-a", position: 0 },
        { id: 202, targetId: 2, fileName: "", uploadDir: "/srv/two-b", position: 1 },
      ],
    },
  ];

  const batchArtifacts = [
    { file_name: "a-api.jar", size: 1 },
    { file_name: "b-web.war", size: 2 },
    { file_name: "c-app.jar", size: 3 },
  ];

  it("批量补选按目标顺序→行顺序填入空名行，填入行默认勾选，具名行不受影响", async () => {
    mockBackend({ targets: batchTargets, artifacts: batchArtifacts });

    render(
      <DeployDialog projectId={5} projectName="演示项目" historyId={7} open onClose={vi.fn()} />,
    );

    await userEvent.click(await screen.findByTestId("deploy-target-1"));
    await userEvent.click(await screen.findByTestId("deploy-target-2"));

    // 勾选顺序即填入顺序：先 b-web.war 后 a-api.jar。
    await userEvent.click(screen.getByTestId("deploy-batch"));
    await userEvent.click(await screen.findByRole("option", { name: /b-web\.war/ }));
    await userEvent.click(screen.getByRole("option", { name: /a-api\.jar/ }));
    await userEvent.click(screen.getByTestId("deploy-batch-apply"));

    // 目标顺序 → 目标内行顺序：102（目标 1）→ 201 → 202（目标 2）。
    expect(screen.getByTestId("deploy-row-pick-1-102")).toHaveTextContent("b-web.war");
    expect(screen.getByTestId("deploy-row-pick-2-201")).toHaveTextContent("a-api.jar");
    // 202 无人可填：保持既有空行预选行为（首个产物 a-api.jar，勾选）。
    expect(screen.getByTestId("deploy-row-pick-2-202")).toHaveTextContent("a-api.jar");
    expect(screen.getByTestId("deploy-row-check-1-102")).toBeChecked();
    expect(screen.getByTestId("deploy-row-check-2-201")).toBeChecked();
    expect(screen.getByTestId("deploy-row-check-2-202")).toBeChecked();
    // 具名行不受影响。
    expect(screen.getByTestId("deploy-row-check-1-101")).toBeChecked();

    await userEvent.click(screen.getByTestId("deploy-submit"));
    await waitFor(() =>
      expect(screen.getByTestId("deploy-status-2")).toHaveTextContent("部署成功"),
    );
    const call = invokeMock.mock.calls.find(([cmd]) => cmd === "start_deploy");
    expect(call?.[1].assignments).toEqual([
      {
        targetId: 1,
        files: [
          { rowId: 101, fileName: "" },
          { rowId: 102, fileName: "b-web.war" },
        ],
      },
      {
        targetId: 2,
        files: [
          { rowId: 201, fileName: "a-api.jar" },
          { rowId: 202, fileName: "a-api.jar" },
        ],
      },
    ]);
  });

  it("批量补选构件多于空名行：多出构件忽略并在入口旁提示", async () => {
    mockBackend({
      targets: [
        {
          id: 1,
          projectId: 5,
          serverId: 11,
          startCommands: "sh start.sh",
          stopCommands: "sh stop.sh",
          autoDeploy: false,
          files: [{ id: 102, targetId: 1, fileName: "", uploadDir: "/srv/pick", position: 0 }],
        },
      ],
      artifacts: batchArtifacts,
    });

    render(
      <DeployDialog projectId={5} projectName="演示项目" historyId={7} open onClose={vi.fn()} />,
    );

    await userEvent.click(await screen.findByTestId("deploy-target-1"));
    await userEvent.click(screen.getByTestId("deploy-batch"));
    await userEvent.click(await screen.findByRole("option", { name: /b-web\.war/ }));
    await userEvent.click(screen.getByRole("option", { name: /a-api\.jar/ }));
    await userEvent.click(screen.getByTestId("deploy-batch-apply"));

    expect(screen.getByTestId("deploy-batch-notice")).toHaveTextContent(
      "已填入 1 行，1 个构件无处可填",
    );
    // 只填入勾选顺序里的第一个构件。
    expect(screen.getByTestId("deploy-row-pick-1-102")).toHaveTextContent("b-web.war");
  });

  // 票 25：批量补选是唯一填充入口，重复执行按序覆盖重填（取代票 24 的逐行
  // 覆盖）：再次填入时按勾选顺序覆盖既有补选值，payload 取覆盖后的值。
  it("批量补选重复执行按序覆盖重填，payload 取覆盖后的值", async () => {
    mockBackend({ targets: batchTargets, artifacts: batchArtifacts });

    render(
      <DeployDialog projectId={5} projectName="演示项目" historyId={7} open onClose={vi.fn()} />,
    );

    await userEvent.click(await screen.findByTestId("deploy-target-1"));
    await userEvent.click(await screen.findByTestId("deploy-target-2"));

    // 第一次填入：b-web.war → 102，a-api.jar → 201（202 预选 a-api.jar）。
    await userEvent.click(screen.getByTestId("deploy-batch"));
    await userEvent.click(await screen.findByRole("option", { name: /b-web\.war/ }));
    await userEvent.click(screen.getByRole("option", { name: /a-api\.jar/ }));
    await userEvent.click(screen.getByTestId("deploy-batch-apply"));
    expect(screen.getByTestId("deploy-row-pick-1-102")).toHaveTextContent("b-web.war");

    // 重复执行：按新的勾选顺序覆盖重填。
    await userEvent.click(screen.getByTestId("deploy-batch"));
    await userEvent.click(await screen.findByRole("option", { name: /a-api\.jar/ }));
    await userEvent.click(screen.getByRole("option", { name: /b-web\.war/ }));
    await userEvent.click(screen.getByRole("option", { name: /c-app\.jar/ }));
    await userEvent.click(screen.getByTestId("deploy-batch-apply"));

    expect(screen.getByTestId("deploy-row-pick-1-102")).toHaveTextContent("a-api.jar");
    expect(screen.getByTestId("deploy-row-pick-2-201")).toHaveTextContent("b-web.war");
    expect(screen.getByTestId("deploy-row-pick-2-202")).toHaveTextContent("c-app.jar");

    await userEvent.click(screen.getByTestId("deploy-submit"));
    await waitFor(() =>
      expect(screen.getByTestId("deploy-status-2")).toHaveTextContent("部署成功"),
    );
    const call = invokeMock.mock.calls.find(([cmd]) => cmd === "start_deploy");
    expect(call?.[1].assignments).toEqual([
      {
        targetId: 1,
        files: [
          { rowId: 101, fileName: "" },
          { rowId: 102, fileName: "a-api.jar" },
        ],
      },
      {
        targetId: 2,
        files: [
          { rowId: 201, fileName: "b-web.war" },
          { rowId: 202, fileName: "c-app.jar" },
        ],
      },
    ]);
  });

  it("批量补选入口仅在存在空名行且操作传文件时可见", async () => {
    const first = render(
      <DeployDialog projectId={5} projectName="演示项目" historyId={7} open onClose={vi.fn()} />,
    );

    // 默认目标里有空名行 + 重启（传文件）：入口可见。
    expect(await screen.findByTestId("deploy-batch")).toBeInTheDocument();

    // 仅停止（不传文件）：入口隐藏。
    await userEvent.selectOptions(screen.getByTestId("deploy-operation"), "stop");
    expect(screen.queryByTestId("deploy-batch")).not.toBeInTheDocument();

    await userEvent.selectOptions(screen.getByTestId("deploy-operation"), "upload");
    expect(screen.getByTestId("deploy-batch")).toBeInTheDocument();

    // 没有空名行的项目：入口隐藏。
    first.unmount();
    mockBackend({
      targets: [
        {
          id: 2,
          projectId: 5,
          serverId: 12,
          startCommands: "sh start.sh",
          stopCommands: "sh stop.sh",
          autoDeploy: false,
          files: [
            { id: 201, targetId: 2, fileName: "gone-1.0.jar", uploadDir: "/srv/two", position: 0 },
            { id: 202, targetId: 2, fileName: "other-1.0.jar", uploadDir: "/srv/two-b", position: 1 },
          ],
        },
      ],
    });
    render(
      <DeployDialog projectId={5} projectName="演示项目" historyId={8} open onClose={vi.fn()} />,
    );
    expect(await screen.findByTestId("deploy-plan-2")).toBeInTheDocument();
    expect(screen.queryByTestId("deploy-batch")).not.toBeInTheDocument();
  });
});
