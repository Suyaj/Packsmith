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

// ticket 19：目标携带 文件名→目录 映射行；含一行空文件名的"部署时补选"行。
const target: Target = {
  id: 1,
  projectId: 5,
  serverId: 11,
  startCommands: "cd /home/app\nsh start.sh start",
  stopCommands: "sh start.sh stop",
  autoDeploy: false,
  files: [
    { id: 101, targetId: 1, fileName: "app-1.0.jar", uploadDir: "/home/app", position: 0 },
    { id: 102, targetId: 1, fileName: "", uploadDir: "/srv/pick", position: 1 },
  ],
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
  it("列出项目的部署目标：服务器列回显 + 文件名→目录映射表（空名行显示 未指定文件）", async () => {
    mockBackend();

    render(<DeploymentTargetsPanel projectId={5} projectName="demo-app" />);

    await waitFor(() => {
      expect(expectInvoke("list_deployment_targets").projectId).toBe(5);
    });
    expect(await screen.findByText("web-1.example.com:22 — deploy")).toBeInTheDocument();
    const files = screen.getByTestId("target-files-1");
    expect(files).toHaveTextContent("app-1.0.jar");
    expect(files).toHaveTextContent("/home/app");
    expect(files).toHaveTextContent("（未指定文件）");
    expect(files).toHaveTextContent("/srv/pick");
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

  it("新建：映射表多行随保存整组提交（可增行）", async () => {
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
    // 第一行：文件名 → 目录
    await userEvent.type(screen.getByLabelText("构建产物文件名"), "app-1.0.jar");
    await userEvent.type(screen.getByLabelText("服务器上传目录"), "/home/app");
    // 添加第二行（空文件名 = 部署时补选），只填目录
    await userEvent.click(screen.getByTestId("add-row"));
    await userEvent.type(screen.getAllByLabelText("服务器上传目录")[1], "/srv/pick");
    // 多行命令编辑器：一条命令一行
    await userEvent.type(screen.getByLabelText("启动命令组"), "cd /home/app\nsh start.sh start");
    await userEvent.type(screen.getByLabelText("停止命令组"), "sh start.sh stop");
    await userEvent.click(screen.getByRole("button", { name: "保存" }));

    await waitFor(() => {
      expect(expectInvoke("create_deployment_target"));
    });
    expect(expectInvoke("create_deployment_target")).toEqual({
      projectId: 5,
      serverId: 11,
      files: [
        { fileName: "app-1.0.jar", uploadDir: "/home/app" },
        { fileName: "", uploadDir: "/srv/pick" },
      ],
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
    await userEvent.type(screen.getByLabelText("构建产物文件名"), "app-1.0.jar");
    await userEvent.type(screen.getByLabelText("服务器上传目录"), "/home/app");
    await userEvent.click(screen.getByRole("button", { name: "保存" }));

    expect(await screen.findByText("web-1.example.com:22 — deploy")).toBeInTheDocument();
  });

  it("必填项未填时不允许保存（映射行目录为空 = 不合法）", async () => {
    mockBackend({ targets: [] });

    render(<DeploymentTargetsPanel projectId={5} projectName="demo-app" />);
    await userEvent.click(await screen.findByRole("button", { name: "新建部署目标" }));

    expect(screen.getByRole("button", { name: "保存" })).toBeDisabled();

    await userEvent.selectOptions(screen.getByLabelText("服务器"), "11");
    expect(screen.getByRole("button", { name: "保存" })).toBeDisabled();
  });

  it("后端映射行校验错误（upload_dir_multiline）以中文提示展示", async () => {
    // 前端行输入框是单行 input（DOM 会清掉换行），多行目录只能由后端拒绝。
    invokeMock.mockImplementation(async (command: string) => {
      if (command === "list_servers") return servers;
      if (command === "list_deployment_targets") return [];
      if (command === "create_deployment_target") {
        throw { code: "upload_dir_multiline", message: "a mapping row's upload dir must be a single line" };
      }
      throw new Error(`unexpected command ${command}`);
    });

    render(<DeploymentTargetsPanel projectId={5} projectName="demo-app" />);
    await userEvent.click(await screen.findByRole("button", { name: "新建部署目标" }));
    await userEvent.selectOptions(screen.getByLabelText("服务器"), "11");
    await userEvent.type(screen.getByLabelText("构建产物文件名"), "a.jar");
    await userEvent.type(screen.getByLabelText("服务器上传目录"), "/srv/a");
    await userEvent.click(screen.getByRole("button", { name: "保存" }));

    expect(await screen.findByText("上传目录只能是一行路径")).toBeInTheDocument();
  });

  it("同一具名文件名重复时展示中文校验错误且不允许保存", async () => {
    mockBackend({ targets: [] });

    render(<DeploymentTargetsPanel projectId={5} projectName="demo-app" />);
    await userEvent.click(await screen.findByRole("button", { name: "新建部署目标" }));

    await userEvent.selectOptions(screen.getByLabelText("服务器"), "11");
    await userEvent.type(screen.getByLabelText("构建产物文件名"), "a.jar");
    await userEvent.type(screen.getByLabelText("服务器上传目录"), "/srv/a");
    await userEvent.click(screen.getByTestId("add-row"));
    await userEvent.type(screen.getByTestId("file-name-1"), "a.jar");
    await userEvent.type(screen.getByTestId("upload-dir-1"), "/srv/b");
    expect(screen.getByRole("alert")).toHaveTextContent("同一文件名不能在映射表中重复出现");
    expect(screen.getByRole("button", { name: "保存" })).toBeDisabled();
  });

  it("删光所有映射行时不允许保存（至少一行）", async () => {
    mockBackend({ targets: [] });

    render(<DeploymentTargetsPanel projectId={5} projectName="demo-app" />);
    await userEvent.click(await screen.findByRole("button", { name: "新建部署目标" }));

    await userEvent.selectOptions(screen.getByLabelText("服务器"), "11");
    await userEvent.click(screen.getByTestId("remove-row-0"));
    expect(screen.getByRole("alert")).toHaveTextContent("至少需要一行");
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
    await userEvent.type(screen.getByLabelText("构建产物文件名"), "app.jar");
    await userEvent.type(screen.getByLabelText("服务器上传目录"), "/home/app");
    await userEvent.click(screen.getByRole("button", { name: "保存" }));

    expect(await screen.findByText("该项目已绑定此服务器")).toBeInTheDocument();
  });

  it("后端映射行校验错误（duplicate_file_name）以中文提示展示", async () => {
    invokeMock.mockImplementation(async (command: string) => {
      if (command === "list_servers") return servers;
      if (command === "list_deployment_targets") return [];
      if (command === "create_deployment_target") {
        throw { code: "duplicate_file_name", message: "the file name \"a.jar\" appears twice" };
      }
      throw new Error(`unexpected command ${command}`);
    });

    render(<DeploymentTargetsPanel projectId={5} projectName="demo-app" />);
    await userEvent.click(await screen.findByRole("button", { name: "新建部署目标" }));
    await userEvent.selectOptions(screen.getByLabelText("服务器"), "11");
    await userEvent.type(screen.getByLabelText("构建产物文件名"), "a.jar");
    await userEvent.type(screen.getByLabelText("服务器上传目录"), "/srv/a");
    await userEvent.click(screen.getByRole("button", { name: "保存" }));

    expect(await screen.findByText(/同一文件名在该目标内重复出现/)).toBeInTheDocument();
  });

  it("编辑：表单回填映射行与命令组，update 携带 id 与整组替换的 files", async () => {
    mockBackend({ update_deployment_target: target });

    render(<DeploymentTargetsPanel projectId={5} projectName="demo-app" />);
    await userEvent.click(await screen.findByRole("button", { name: "编辑" }));

    expect(screen.getByTestId("file-name-0")).toHaveValue("app-1.0.jar");
    expect(screen.getByTestId("upload-dir-0")).toHaveValue("/home/app");
    expect(screen.getByTestId("file-name-1")).toHaveValue("");
    expect(screen.getByTestId("upload-dir-1")).toHaveValue("/srv/pick");
    expect(screen.getByLabelText("启动命令组")).toHaveValue(
      "cd /home/app\nsh start.sh start",
    );
    expect(screen.getByLabelText("停止命令组")).toHaveValue("sh start.sh stop");
    expect(screen.getByLabelText("服务器")).toHaveValue("11");

    await userEvent.clear(screen.getByTestId("upload-dir-0"));
    await userEvent.type(screen.getByTestId("upload-dir-0"), "/srv/app");
    await userEvent.click(screen.getByRole("button", { name: "保存" }));

    await waitFor(() => {
      expect(expectInvoke("update_deployment_target").id).toBe(1);
    });
    expect(expectInvoke("update_deployment_target").files).toEqual([
      { fileName: "app-1.0.jar", uploadDir: "/srv/app" },
      { fileName: "", uploadDir: "/srv/pick" },
    ]);
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
