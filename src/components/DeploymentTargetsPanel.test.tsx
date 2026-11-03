import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";

import type { Server } from "@/lib/serversApi";
import type { Target } from "@/lib/targetsApi";

const invokeMock = vi.fn();

vi.mock("@tauri-apps/api/core", () => ({
  invoke: (...args: unknown[]) => invokeMock(...args),
}));

import DeploymentTargetsPanel from "./DeploymentTargetsPanel";
import "@/i18n";

const servers: Server[] = [
  {
    id: 11,
    host: "web-1.example.com",
    port: 22,
    username: "deploy",
    credential: { type: "password" },
  },
  {
    id: 12,
    host: "db-1.example.com",
    port: 2222,
    username: "root",
    credential: {
      type: "private_key",
      path: "/home/root/.ssh/id_ed25519",
      has_passphrase: false,
    },
  },
];

const target: Target = {
  id: 1,
  projectId: 5,
  serverId: 11,
  uploadDir: "/home/app",
  startCommands: "cd /home/app\nsh start.sh start",
  stopCommands: "sh start.sh stop",
  autoDeploy: false,
  lastArtifact: null,
};

function mockBackend(overrides: Record<string, unknown> = {}) {
  invokeMock.mockImplementation(async (command: string) => {
    if (command === "list_servers") return overrides.servers ?? servers;
    if (command === "list_deployment_targets") {
      return overrides.targets ?? [target];
    }
    if (command in overrides) return overrides[command];
    throw new Error(`unexpected command ${command}`);
  });
}

function expectInvoke(command: string) {
  const call = invokeMock.mock.calls.find(([c]) => c === command);
  expect(call, `expected invoke("${command}") to have been called`).toBeDefined();
  return call![1] as Record<string, unknown>;
}

beforeEach(() => {
  invokeMock.mockReset();
});

describe("部署目标面板", () => {
  it("列出项目的部署目标，服务器列回显为 主机:端口 — 用户名", async () => {
    mockBackend();

    render(<DeploymentTargetsPanel projectId={5} projectName="demo-app" />);

    await waitFor(() => {
      expect(expectInvoke("list_deployment_targets").projectId).toBe(5);
    });
    expect(await screen.findByText("web-1.example.com:22 — deploy")).toBeInTheDocument();
    expect(screen.getByText(/上传目录：\/home\/app/)).toBeInTheDocument();
    expect(screen.getByTestId("target-start-1")).toHaveTextContent(
      /cd \/home\/app\s+sh start\.sh start/,
    );
    expect(screen.getByTestId("target-stop-1")).toHaveTextContent(/sh start\.sh stop/);
  });

  it("没有部署目标时展示空态", async () => {
    mockBackend({ targets: [] });

    render(<DeploymentTargetsPanel projectId={5} projectName="demo-app" />);

    expect(await screen.findByText(/尚未配置部署目标/)).toBeInTheDocument();
  });

  it("新建：服务器下拉来自服务器列表，保存时收到完整字段（含多行命令组）", async () => {
    mockBackend({
      create_deployment_target: { ...target, id: 2 },
      targets: [],
    });

    render(<DeploymentTargetsPanel projectId={5} projectName="demo-app" />);
    await userEvent.click(await screen.findByRole("button", { name: "新建部署目标" }));

    const select = screen.getByLabelText("服务器") as HTMLSelectElement;
    const options = [...select.options].filter((o) => o.value !== "");
    expect(options.map((o) => o.textContent)).toEqual([
      "web-1.example.com:22 — deploy",
      "db-1.example.com:2222 — root",
    ]);

    await userEvent.selectOptions(select, "11");
    await userEvent.type(screen.getByLabelText("上传目录"), "/home/app");
    // 多行命令编辑器：一条命令一行
    await userEvent.type(screen.getByLabelText("启动命令组"), "cd /home/app\nsh start.sh start");
    await userEvent.type(screen.getByLabelText("停止命令组"), "sh start.sh stop");
    await userEvent.click(screen.getByRole("button", { name: "保存" }));

    await waitFor(() => {
      expectInvoke("create_deployment_target");
    });
    expect(expectInvoke("create_deployment_target")).toEqual({
      projectId: 5,
      serverId: 11,
      uploadDir: "/home/app",
      startCommands: "cd /home/app\nsh start.sh start",
      stopCommands: "sh start.sh stop",
    });
  });

  it("新建后刷新列表（保存结果出现在目标列表中）", async () => {
    const list: unknown[] = [];
    invokeMock.mockImplementation(async (command: string) => {
      if (command === "list_servers") return servers;
      if (command === "list_deployment_targets") return list;
      if (command === "create_deployment_target") {
        list.push({ ...target, id: 2 });
        return { ...target, id: 2 };
      }
      throw new Error(`unexpected command ${command}`);
    });

    render(<DeploymentTargetsPanel projectId={5} projectName="demo-app" />);
    await userEvent.click(await screen.findByRole("button", { name: "新建部署目标" }));
    await userEvent.selectOptions(screen.getByLabelText("服务器"), "11");
    await userEvent.type(screen.getByLabelText("上传目录"), "/home/app");
    await userEvent.click(screen.getByRole("button", { name: "保存" }));

    expect(await screen.findByText("web-1.example.com:22 — deploy")).toBeInTheDocument();
  });

  it("必填项未填时不允许保存", async () => {
    mockBackend({ targets: [] });

    render(<DeploymentTargetsPanel projectId={5} projectName="demo-app" />);
    await userEvent.click(await screen.findByRole("button", { name: "新建部署目标" }));

    expect(screen.getByRole("button", { name: "保存" })).toBeDisabled();

    await userEvent.selectOptions(screen.getByLabelText("服务器"), "11");
    expect(screen.getByRole("button", { name: "保存" })).toBeDisabled();
  });

  it("上传目录多行时展示中文校验错误且不允许保存", async () => {
    mockBackend({ targets: [] });

    render(<DeploymentTargetsPanel projectId={5} projectName="demo-app" />);
    await userEvent.click(await screen.findByRole("button", { name: "新建部署目标" }));

    await userEvent.selectOptions(screen.getByLabelText("服务器"), "11");
    await userEvent.type(screen.getByLabelText("上传目录"), "/home/app\n/backup");
    expect(screen.getByRole("alert")).toHaveTextContent("上传目录只能是一行路径");
    expect(screen.getByRole("button", { name: "保存" })).toBeDisabled();
  });

  it("后端错误（duplicate_binding）以中文提示展示", async () => {
    invokeMock.mockImplementation(async (command: string) => {
      if (command === "list_servers") return servers;
      if (command === "list_deployment_targets") return [];
      if (command === "create_deployment_target") {
        throw { code: "duplicate_binding", message: "already bound" };
      }
      throw new Error(`unexpected command ${command}`);
    });

    render(<DeploymentTargetsPanel projectId={5} projectName="demo-app" />);
    await userEvent.click(await screen.findByRole("button", { name: "新建部署目标" }));
    await userEvent.selectOptions(screen.getByLabelText("服务器"), "11");
    await userEvent.type(screen.getByLabelText("上传目录"), "/home/app");
    await userEvent.click(screen.getByRole("button", { name: "保存" }));

    expect(await screen.findByText("该项目已绑定此服务器")).toBeInTheDocument();
  });

  it("编辑：表单回填已有值（含多行命令），update 携带 id", async () => {
    mockBackend({ update_deployment_target: target });

    render(<DeploymentTargetsPanel projectId={5} projectName="demo-app" />);
    await userEvent.click(await screen.findByRole("button", { name: "编辑" }));

    expect(screen.getByLabelText("上传目录")).toHaveValue("/home/app");
    expect(screen.getByLabelText("启动命令组")).toHaveValue(
      "cd /home/app\nsh start.sh start",
    );
    expect(screen.getByLabelText("停止命令组")).toHaveValue("sh start.sh stop");
    expect(screen.getByLabelText("服务器")).toHaveValue("11");

    await userEvent.clear(screen.getByLabelText("上传目录"));
    await userEvent.type(screen.getByLabelText("上传目录"), "/srv/app");
    await userEvent.click(screen.getByRole("button", { name: "保存" }));

    await waitFor(() => {
      expect(expectInvoke("update_deployment_target").id).toBe(1);
    });
    expect(expectInvoke("update_deployment_target").uploadDir).toBe("/srv/app");
  });

  it("删除：确认后调用 delete_deployment_target 并刷新", async () => {
    const confirmSpy = vi.spyOn(window, "confirm").mockReturnValue(true);
    mockBackend({ delete_deployment_target: true });

    render(<DeploymentTargetsPanel projectId={5} projectName="demo-app" />);
    await userEvent.click(await screen.findByRole("button", { name: "删除" }));

    await waitFor(() => {
      expect(expectInvoke("delete_deployment_target").id).toBe(1);
    });
    expect(confirmSpy).toHaveBeenCalledWith("确定删除该部署目标吗？");
    confirmSpy.mockRestore();
  });
});

// ---------------------------------------------------------------------------
// 构建成功后自动连锁部署开关（ticket 10，默认关）
// ---------------------------------------------------------------------------

describe("部署目标自动连锁开关", () => {
  it("默认关闭，可打开并发送 set_target_auto_deploy", async () => {
    mockBackend();

    render(<DeploymentTargetsPanel projectId={5} projectName="demo-app" />);

    const toggle = await screen.findByTestId("auto-deploy-toggle-1");
    expect(toggle).not.toBeChecked();

    await userEvent.click(toggle);

    await waitFor(() => {
      expect(expectInvoke("set_target_auto_deploy")).toEqual({
        id: 1,
        enabled: true,
      });
    });
  });

  it("已开启的目标回显为选中", async () => {
    mockBackend({ targets: [{ ...target, autoDeploy: true }] });

    render(<DeploymentTargetsPanel projectId={5} projectName="demo-app" />);

    expect(await screen.findByTestId("auto-deploy-toggle-1")).toBeChecked();
  });
});
