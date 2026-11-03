import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import BuildPanel from "./BuildPanel";
import type { Project } from "@/lib/buildApi";

vi.mock("@tauri-apps/api/core", () => ({
  invoke: vi.fn(),
  Channel: vi.fn(function (this: unknown) {
    return { onmessage: null };
  }),
}));

import { Channel, invoke } from "@tauri-apps/api/core";
import "@/i18n";

const mockInvoke = vi.mocked(invoke);

const gitProject: Project = {
  id: 7,
  name: "demo-app",
  source: "git",
  url: "https://example.com/demo-app.git",
  path: "/Users/demo/Packsmith/workspace/demo-app",
  project_type: "maven",
  last_branch: "develop",
  created_at: "2026-11-02T00:00:00.000Z",
};

function renderPanel(project: Project = gitProject, onClose = () => {}) {
  return render(<BuildPanel project={project} onClose={onClose} />);
}

/** invoke mock dispatching by command name. */
function mockBackend(map: Record<string, (args: unknown) => unknown>) {
  mockInvoke.mockImplementation(async (cmd: string, args?: unknown) => {
    const handler = map[cmd];
    if (!handler) throw new Error(`unexpected command: ${cmd}`);
    return handler(args);
  });
}

beforeEach(() => {
  mockInvoke.mockReset();
});

describe("打包面板（分支选择与临时工作区）", () => {
  it("打开后实时拉取分支列表，并默认选中该项目上次使用的分支", async () => {
    mockBackend({ list_remote_branches: () => ["main", "develop", "feature/x"] });

    renderPanel();

    await waitFor(() => {
      expect(mockInvoke).toHaveBeenCalledWith("list_remote_branches", { projectId: 7 });
    });
    const select = (await screen.findByLabelText("打包分支")) as HTMLSelectElement;
    await waitFor(() => {
      expect(select.value).toBe("develop");
    });
    // 下拉里是实时拉取的远程分支
    const options = [...select.options].map((o) => o.value);
    expect(options).toEqual(["main", "develop", "feature/x"]);
  });

  it("没有上次分支时默认选中列表第一个分支", async () => {
    mockBackend({ list_remote_branches: () => ["main", "release/1.0"] });

    renderPanel({ ...gitProject, last_branch: undefined });

    const select = (await screen.findByLabelText("打包分支")) as HTMLSelectElement;
    await waitFor(() => {
      expect(select.value).toBe("main");
    });
  });

  it("确认后按所选分支调用 start_build_workspace 并展示克隆进度", async () => {
    let captured: { onmessage?: (msg: unknown) => void } = {};
    mockBackend({
      list_remote_branches: () => ["main", "develop"],
      // start_build_workspace stays pending like a real clone
      start_build_workspace: () => new Promise(() => {}),
    });
    vi.mocked(Channel).mockImplementation(function (this: unknown) {
      const ch: { onmessage?: (msg: unknown) => void } = {};
      captured = ch;
      return ch as never;
    });

    renderPanel();

    const select = (await screen.findByLabelText("打包分支")) as HTMLSelectElement;
    await waitFor(() => expect(select.value).toBe("develop"));
    await userEvent.click(screen.getByRole("button", { name: "开始打包" }));

    await waitFor(() => {
      expect(mockInvoke).toHaveBeenCalledWith("start_build_workspace", {
        projectId: 7,
        branch: "develop",
        onProgress: expect.anything(),
      });
    });
    expect(await screen.findByText(/正在准备/)).toBeInTheDocument();

    captured.onmessage?.({ stage: "transferring", received: 3, total: 10 });
    expect(await screen.findByText(/3\s*\/\s*10/)).toBeInTheDocument();
  });

  it("克隆成功后展示临时工作区就绪与路径", async () => {
    mockBackend({
      list_remote_branches: () => ["main"],
      start_build_workspace: () => ({ path: "/tmp/packsmith-build-demo-app-1" }),
    });

    renderPanel({ ...gitProject, last_branch: undefined });

    const select = (await screen.findByLabelText("打包分支")) as HTMLSelectElement;
    await waitFor(() => expect(select.value).toBe("main"));
    await userEvent.click(screen.getByRole("button", { name: "开始打包" }));

    expect(await screen.findByText("临时工作区已就绪")).toBeInTheDocument();
    expect(screen.getByText(/packsmith-build-demo-app-1/)).toBeInTheDocument();
  });

  it("克隆失败（如无效分支）展示明确错误", async () => {
    mockBackend({
      list_remote_branches: () => ["main"],
      start_build_workspace: () => {
        throw { code: "clone_failed", message: "reference 'refs/heads/nope' not found" };
      },
    });

    renderPanel({ ...gitProject, last_branch: undefined });

    const select = (await screen.findByLabelText("打包分支")) as HTMLSelectElement;
    await waitFor(() => expect(select.value).toBe("main"));
    await userEvent.click(screen.getByRole("button", { name: "开始打包" }));

    expect(await screen.findByRole("alert")).toHaveTextContent("克隆失败");
    expect(screen.queryByText("临时工作区已就绪")).not.toBeInTheDocument();
  });

  it("分支列表拉取失败展示错误", async () => {
    mockBackend({
      list_remote_branches: () => {
        throw { code: "branch_listing_failed", message: "connection refused" };
      },
    });

    renderPanel();

    expect(await screen.findByRole("alert")).toHaveTextContent("获取分支列表失败");
  });

  it("准备进行中可取消打包，向后端发送 cancel_build_workspace", async () => {
    mockBackend({
      list_remote_branches: () => ["main"],
      start_build_workspace: () => new Promise(() => {}),
      cancel_build_workspace: () => null,
    });

    renderPanel({ ...gitProject, last_branch: undefined });

    const select = (await screen.findByLabelText("打包分支")) as HTMLSelectElement;
    await waitFor(() => expect(select.value).toBe("main"));
    await userEvent.click(screen.getByRole("button", { name: "开始打包" }));

    await userEvent.click(await screen.findByRole("button", { name: "取消打包" }));

    await waitFor(() => {
      expect(mockInvoke).toHaveBeenCalledWith("cancel_build_workspace", { projectId: 7 });
    });
    // 面板保持打开（等待后端返回结果）
    expect(screen.getByRole("button", { name: "取消打包" })).toBeInTheDocument();
  });

  it("空闲时关闭按钮触发 onClose", async () => {
    mockBackend({ list_remote_branches: () => ["main"] });
    const onClose = vi.fn();

    renderPanel(gitProject, onClose);

    await screen.findByLabelText("打包分支");
    await userEvent.click(screen.getByRole("button", { name: "关闭" }));
    expect(onClose).toHaveBeenCalled();
  });
});
