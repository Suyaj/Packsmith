import { render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { MemoryRouter } from "react-router-dom";
import { beforeEach, describe, expect, it, vi } from "vitest";
import ProjectsPage from "./ProjectsPage";

vi.mock("@tauri-apps/api/core", () => ({
  invoke: vi.fn(),
  Channel: vi.fn(function (this: unknown) {
    return { onmessage: null };
  }),
}));

import { Channel, invoke } from "@tauri-apps/api/core";
import "@/i18n";

const mockInvoke = vi.mocked(invoke);

const demoProjects = [
  {
    id: 1,
    name: "demo-app",
    source: "git",
    url: "https://example.com/demo-app.git",
    path: "/Users/demo/Packsmith/workspace/demo-app",
    project_type: "maven",
    created_at: "2026-11-02T00:00:00.000Z",
  },
  {
    id: 2,
    name: "legacy-app",
    source: "local",
    path: "/Users/demo/code/legacy-app",
    project_type: "maven",
    created_at: "2026-11-01T00:00:00.000Z",
  },
];

function renderPage() {
  return render(
    <MemoryRouter initialEntries={["/projects"]}>
      <ProjectsPage />
    </MemoryRouter>,
  );
}

/** invoke mock dispatching by command name; `list` is the current project list,
 *  grown by successful create/import results like the real backend would. */
function mockBackend(list: unknown[], result: { ok?: unknown; err?: unknown }) {
  mockInvoke.mockImplementation(async (cmd: string, _args?: unknown) => {
    if (cmd === "list_projects") return list;
    if (result.err !== undefined) throw result.err;
    list.push(result.ok);
    return result.ok;
  });
}

beforeEach(() => {
  mockInvoke.mockReset();
});

describe("项目管理页", () => {
  it("加载后展示项目列表：名称、来源、类型", async () => {
    mockBackend(demoProjects, {});

    renderPage();

    await waitFor(() => {
      expect(mockInvoke).toHaveBeenCalledWith("list_projects");
    });
    expect(await screen.findByText("demo-app")).toBeInTheDocument();
    expect(screen.getByText("legacy-app")).toBeInTheDocument();
    // 来源列：Git / 本地；类型列：Maven（Java）
    expect(screen.getByText("Git")).toBeInTheDocument();
    expect(screen.getByText("本地")).toBeInTheDocument();
    expect(screen.getAllByText("Maven（Java）").length).toBe(2);
  });

  it("类型下拉仅 Maven 可选，Gradle 为 disabled 占位", async () => {
    mockBackend([], {});

    renderPage();
    await userEvent.click(await screen.findByRole("button", { name: "新建项目" }));

    const select = screen.getByLabelText("项目类型") as HTMLSelectElement;
    const maven = [...select.options].find((o) => o.value === "maven");
    const gradle = [...select.options].find((o) => o.value === "gradle");
    expect(maven?.disabled).toBe(false);
    expect(gradle?.disabled).toBe(true);
  });

  it("新建：填 git URL 提交后调用 clone_project 并刷新列表", async () => {
    mockBackend([], {
      ok: {
        id: 1,
        name: "cloned-app",
        source: "git",
        url: "https://example.com/cloned-app.git",
        path: "/Users/demo/Packsmith/workspace/cloned-app",
        project_type: "maven",
        created_at: "2026-11-02T00:00:00.000Z",
      },
    });

    renderPage();
    await userEvent.click(await screen.findByRole("button", { name: "新建项目" }));

    await userEvent.type(screen.getByLabelText("项目名称"), "cloned-app");
    await userEvent.type(
      screen.getByLabelText("Git 仓库地址"),
      "https://example.com/cloned-app.git",
    );
    await userEvent.click(screen.getByRole("button", { name: "克隆并创建" }));

    await waitFor(() => {
      expect(mockInvoke).toHaveBeenCalledWith("clone_project", {
        name: "cloned-app",
        url: "https://example.com/cloned-app.git",
        workspaceRoot: "~/Packsmith/workspace",
        projectType: "maven",
        onProgress: expect.anything(),
      });
    });
    // 列表刷新后出现新项目
    expect(await screen.findByText("cloned-app")).toBeInTheDocument();
  });

  it("导入：填本地路径提交后调用 import_project 并刷新列表", async () => {
    mockBackend([], {
      ok: {
        id: 2,
        name: "legacy-app",
        source: "local",
        path: "/Users/demo/code/legacy-app",
        project_type: "maven",
        created_at: "2026-11-02T00:00:00.000Z",
      },
    });

    renderPage();
    await userEvent.click(await screen.findByRole("button", { name: "新建项目" }));
    await userEvent.click(screen.getByRole("radio", { name: /导入/ }));

    await userEvent.type(screen.getByLabelText("项目名称"), "legacy-app");
    await userEvent.type(screen.getByLabelText("本地路径"), "/Users/demo/code/legacy-app");
    await userEvent.click(screen.getByRole("button", { name: "导入" }));

    await waitFor(() => {
      expect(mockInvoke).toHaveBeenCalledWith("import_project", {
        name: "legacy-app",
        path: "/Users/demo/code/legacy-app",
        projectType: "maven",
      });
    });
    expect(await screen.findByText("legacy-app")).toBeInTheDocument();
  });

  it("后端错误（同名项目）以中文提示展示", async () => {
    mockBackend([], { err: { code: "duplicate_name", message: "already exists" } });

    renderPage();
    await userEvent.click(await screen.findByRole("button", { name: "新建项目" }));

    await userEvent.type(screen.getByLabelText("项目名称"), "demo-app");
    await userEvent.type(
      screen.getByLabelText("Git 仓库地址"),
      "https://example.com/demo-app.git",
    );
    await userEvent.click(screen.getByRole("button", { name: "克隆并创建" }));

    expect(await screen.findByText("已存在同名项目")).toBeInTheDocument();
  });

  it("克隆进行中展示进度状态", async () => {
    let captured: { onmessage?: (msg: unknown) => void } = {};
    mockInvoke.mockImplementation(async (cmd: string) => {
      if (cmd === "list_projects") return [];
      // clone_project stays pending until the test resolves it via captured channel
      return new Promise(() => {});
    });
    vi.mocked(Channel).mockImplementation(function (this: unknown) {
      const ch: { onmessage?: (msg: unknown) => void } = {};
      captured = ch;
      return ch as never;
    });

    renderPage();
    await userEvent.click(await screen.findByRole("button", { name: "新建项目" }));
    await userEvent.type(screen.getByLabelText("项目名称"), "slow-app");
    await userEvent.type(
      screen.getByLabelText("Git 仓库地址"),
      "https://example.com/slow-app.git",
    );
    await userEvent.click(screen.getByRole("button", { name: "克隆并创建" }));

    expect(await screen.findByText(/正在克隆/)).toBeInTheDocument();

    // 进度事件更新展示
    captured.onmessage?.({ stage: "transferring", received: 3, total: 10 });
    expect(await screen.findByText(/3\s*\/\s*10/)).toBeInTheDocument();
  });

  it("克隆进行中可点击取消克隆，向后端发送 cancel_clone", async () => {
    mockInvoke.mockImplementation(async (cmd: string) => {
      if (cmd === "list_projects") return [];
      if (cmd === "cancel_clone") return null;
      // clone_project stays pending until backend notices the cancel flag
      return new Promise(() => {});
    });

    renderPage();
    await userEvent.click(await screen.findByRole("button", { name: "新建项目" }));
    await userEvent.type(screen.getByLabelText("项目名称"), "slow-app");
    await userEvent.type(
      screen.getByLabelText("Git 仓库地址"),
      "https://example.com/slow-app.git",
    );
    await userEvent.click(screen.getByRole("button", { name: "克隆并创建" }));

    const cancel = await screen.findByRole("button", { name: "取消克隆" });
    await userEvent.click(cancel);

    await waitFor(() => {
      expect(mockInvoke).toHaveBeenCalledWith("cancel_clone", { name: "slow-app" });
    });
    // 表单在取消克隆期间保持打开（等待后端返回结果）
    expect(screen.getByRole("button", { name: "取消克隆" })).toBeInTheDocument();
  });

  it("点击项目行的打包按钮打开打包面板，并按该项目实时拉取分支", async () => {
    mockInvoke.mockImplementation(async (cmd: string, args?: unknown) => {
      if (cmd === "list_projects") return demoProjects;
      if (cmd === "list_remote_branches") {
        // 打包的是 legacy-app（id 2，本地导入项目）
        expect(args).toEqual({ projectId: 2 });
        return ["main", "release/1.0"];
      }
      throw new Error(`unexpected command: ${cmd}`);
    });

    renderPage();

    const row = await screen.findByTestId("project-row-legacy-app");
    await userEvent.click(within(row).getByRole("button", { name: "打包" }));

    expect(await screen.findByLabelText("打包分支")).toBeInTheDocument();
  });

  it("点击项目行的“部署目标”按钮展开该项目的部署目标面板", async () => {
    mockInvoke.mockImplementation(async (cmd: string) => {
      if (cmd === "list_projects") return demoProjects;
      if (cmd === "list_servers") {
        return [
          {
            id: 11,
            host: "web-1.example.com",
            port: 22,
            username: "deploy",
            credential: { type: "password", password: "s3cret" },
          },
        ];
      }
      if (cmd === "list_deployment_targets") {
        return [
          {
            id: 1,
            projectId: 1,
            serverId: 11,
            uploadDir: "/home/app",
            startCommands: "cd /home/app\nsh start.sh start",
            stopCommands: "sh start.sh stop",
          },
        ];
      }
      throw new Error(`unexpected command ${cmd}`);
    });

    renderPage();

    await userEvent.click(
      (await screen.findAllByRole("button", { name: /部署目标/ }))[0],
    );

    expect(
      await screen.findByText("demo-app 的部署目标"),
    ).toBeInTheDocument();
    expect(await screen.findByText("web-1.example.com:22 — deploy")).toBeInTheDocument();
  });
});
