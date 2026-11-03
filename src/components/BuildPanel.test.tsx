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
  build_command: "mvnd package -DskipTests",
  last_branch: "develop",
  created_at: "2026-11-02T00:00:00.000Z",
};

const outcomeOk = {
  success: true,
  exit_code: 0,
  build_id: "build-7-1000",
  log_path: "/appdata/build-logs/build-7-1000.log",
  artifacts_dir: "/appdata/artifacts/demo-app/20261103-120000",
  artifacts: [{ relative_path: "target/app-1.0.jar", file_name: "app-1.0.jar", size: 1024 }],
  history_id: 1,
};

const outcomeOkMulti = {
  success: true,
  exit_code: 0,
  build_id: "build-7-1002",
  log_path: "/appdata/build-logs/build-7-1002.log",
  artifacts_dir: "/appdata/artifacts/demo-app/20261103-130000",
  artifacts: [
    { relative_path: "module-a/target/a-api-1.0.jar", file_name: "a-api-1.0.jar", size: 100 },
    { relative_path: "module-b/target/b-web-1.0.war", file_name: "b-web-1.0.war", size: 200 },
  ],
  history_id: 2,
};

const outcomeFailed = {
  success: false,
  exit_code: 1,
  build_id: "build-7-1001",
  log_path: "/appdata/build-logs/build-7-1001.log",
  artifacts_dir: null,
  artifacts: [],
  history_id: 3,
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

/** 打开面板并等分支预选完成（返回分支选择触发按钮）。 */
async function openPanelWithBranches(
  map: Record<string, (args: unknown) => unknown>,
  project: Project = gitProject,
) {
  mockBackend({ list_remote_branches: () => ["main", "develop"], ...map });
  renderPanel(project);
  const trigger = await screen.findByLabelText("打包分支");
  await waitFor(() => expect(trigger).toHaveTextContent("develop"));
  return trigger;
}

/** 打开面板后展开分支搜索下拉。 */
async function openBranchPopover(
  map: Record<string, (args: unknown) => unknown> = {},
  project: Project = gitProject,
) {
  const trigger = await openPanelWithBranches(map, project);
  await userEvent.click(trigger);
  const searchInput = await screen.findByPlaceholderText("搜索分支…");
  return { trigger, searchInput };
}

beforeEach(() => {
  mockInvoke.mockReset();
  vi.mocked(Channel).mockImplementation(function (this: unknown) {
    return { onmessage: null };
  });
});

describe("打包面板（分支选择）", () => {
  it("打开后实时拉取分支列表，并默认选中该项目上次使用的分支", async () => {
    mockBackend({ list_remote_branches: () => ["main", "develop", "feature/x"] });

    renderPanel();

    await waitFor(() => {
      expect(mockInvoke).toHaveBeenCalledWith("list_remote_branches", { projectId: 7 });
    });
    const trigger = await screen.findByLabelText("打包分支");
    await waitFor(() => {
      expect(trigger).toHaveTextContent("develop");
    });

    await userEvent.click(trigger);
    const options = await screen.findAllByRole("option");
    expect(options.map((o) => o.textContent)).toEqual([
      "main",
      "develop",
      "feature/x",
    ]);
  });

  it("没有上次分支时默认选中列表第一个分支", async () => {
    mockBackend({ list_remote_branches: () => ["main", "release/1.0"] });

    renderPanel({ ...gitProject, last_branch: undefined });

    const trigger = await screen.findByLabelText("打包分支");
    await waitFor(() => {
      expect(trigger).toHaveTextContent("main");
    });
  });
});

describe("打包面板（分支模糊搜索）", () => {
  it("输入关键字实时模糊过滤分支列表并高亮命中片段", async () => {
    const { searchInput } = await openBranchPopover({
      list_remote_branches: () => ["main", "release/2.1", "hotfix-release", "develop"],
    });

    await userEvent.type(searchInput, "rel");

    const options = await screen.findAllByRole("option");
    expect(options.map((o) => o.textContent)).toEqual(["release/2.1", "hotfix-release"]);
    // 命中片段包裹在高亮 span 中：release/2.1 的前三个字符 r-e-l。
    expect(options[0].querySelectorAll("[data-highlight]")).toHaveLength(3);
    expect(options[0].querySelectorAll("[data-highlight]")[0]).toHaveTextContent("r");
    expect(options[0].querySelectorAll("[data-highlight]")[2]).toHaveTextContent("l");
  });

  it("没有匹配分支时展示空状态提示", async () => {
    const { searchInput } = await openBranchPopover();

    await userEvent.type(searchInput, "zzz");

    expect(await screen.findByText("没有匹配的分支")).toBeInTheDocument();
    expect(screen.queryByRole("option")).not.toBeInTheDocument();
  });

  it("点击选中分支后回填选择框，打包流程使用所选分支", async () => {
    await openPanelWithBranches({
      list_remote_branches: () => ["main", "develop", "feature/x"],
      start_build: (args) => {
        expect(args).toMatchObject({ projectId: 7, branch: "feature/x" });
        return outcomeOk;
      },
    });

    await userEvent.click(screen.getByLabelText("打包分支"));
    await userEvent.click(await screen.findByRole("option", { name: "feature/x" }));

    const trigger = screen.getByLabelText("打包分支");
    expect(trigger).toHaveTextContent("feature/x");
    // 选择后下拉收起。
    expect(trigger).toHaveAttribute("aria-expanded", "false");

    await userEvent.click(screen.getByRole("button", { name: "开始打包" }));
    expect(await screen.findByText("构建成功")).toBeInTheDocument();
  });

  it("键盘可用：上下移动高亮、回车确认选中、Esc 关闭下拉", async () => {
    const { trigger } = await openBranchPopover({
      list_remote_branches: () => ["main", "develop"],
      start_build: (args) => {
        expect(args).toMatchObject({ projectId: 7, branch: "develop" });
        return outcomeOk;
      },
    });

    // 初始高亮第一项，下移到第二项后回车选中（焦点在搜索输入框上）。
    await userEvent.keyboard("{ArrowDown}");
    await userEvent.keyboard("{Enter}");

    expect(trigger).toHaveTextContent("develop");
    expect(trigger).toHaveAttribute("aria-expanded", "false");

    // Esc 关闭：再次打开后按 Esc，下拉应收起。
    await userEvent.click(trigger);
    expect(screen.getByLabelText("打包分支")).toHaveAttribute("aria-expanded", "true");
    await userEvent.type(screen.getByPlaceholderText("搜索分支…"), "{Escape}");
    expect(screen.getByLabelText("打包分支")).toHaveAttribute("aria-expanded", "false");
  });

  it("关闭下拉后重新打开，搜索关键字已清空", async () => {
    const { trigger } = await openBranchPopover({
      list_remote_branches: () => ["main", "develop"],
    });

    await userEvent.type(screen.getByPlaceholderText("搜索分支…"), "dev");
    await userEvent.keyboard("{Escape}");
    await userEvent.click(trigger);

    const searchInput = await screen.findByPlaceholderText("搜索分支…");
    expect(searchInput).toHaveValue("");
    expect(screen.getAllByRole("option")).toHaveLength(2);
  });
});

describe("打包面板（构建命令编辑）", () => {
  it("展示项目当前构建命令，可编辑并保存到后端", async () => {
    await openPanelWithBranches({
      update_build_command: (args) => {
        expect(args).toMatchObject({
          projectId: 7,
          command: "mvnd package -DskipTests -Pfast",
        });
        return { ...gitProject, build_command: "mvnd package -DskipTests -Pfast" };
      },
    });

    const input = screen.getByLabelText("构建命令") as HTMLInputElement;
    expect(input.value).toBe("mvnd package -DskipTests");
    await userEvent.clear(input);
    await userEvent.type(input, "mvnd package -DskipTests -Pfast");
    await userEvent.click(screen.getByRole("button", { name: "保存命令" }));

    await waitFor(() => {
      expect(mockInvoke).toHaveBeenCalledWith("update_build_command", {
        projectId: 7,
        command: "mvnd package -DskipTests -Pfast",
      });
    });
    expect(await screen.findByText("构建命令已保存")).toBeInTheDocument();
  });

  it("保存非法构建命令时展示后端错误", async () => {
    await openPanelWithBranches({
      update_build_command: () => {
        throw { code: "unsupported_build_command", message: "unsupported build command: gradle" };
      },
    });

    const input = screen.getByLabelText("构建命令") as HTMLInputElement;
    await userEvent.clear(input);
    await userEvent.type(input, "gradle build");
    await userEvent.click(screen.getByRole("button", { name: "保存命令" }));

    expect(await screen.findByRole("alert")).toHaveTextContent("仅支持 mvnd");
  });
});

describe("打包面板（构件选择）", () => {
  it("单构件构建自动选中：展示构件名，不再弹出勾选", async () => {
    await openPanelWithBranches({ start_build: () => outcomeOk });

    await userEvent.click(screen.getByRole("button", { name: "开始打包" }));

    expect(await screen.findByText(/已自动选中构件/)).toBeInTheDocument();
    expect(screen.queryByRole("radiogroup")).not.toBeInTheDocument();
    expect(mockInvoke).not.toHaveBeenCalledWith(
      "set_default_artifact",
      expect.anything(),
    );
  });

  it("多构件构建弹出勾选：项目默认构件预选中，确认后保存为默认", async () => {
    await openPanelWithBranches({
      start_build: () => outcomeOkMulti,
      set_default_artifact: (args) => {
        expect(args).toMatchObject({ projectId: 7, artifact: "b-web-1.0.war" });
        return { ...gitProject, default_artifact: "b-web-1.0.war" };
      },
    });

    await userEvent.click(screen.getByRole("button", { name: "开始打包" }));

    const group = await screen.findByRole("radiogroup", { name: "构件选择" });
    const a = screen.getByRole("radio", { name: /a-api-1\.0\.jar/ }) as HTMLInputElement;
    const b = screen.getByRole("radio", { name: /b-web-1\.0\.war/ }) as HTMLInputElement;
    expect(group).toBeInTheDocument();
    // 无默认时预选第一个构件。
    expect(a.checked).toBe(true);
    expect(b.checked).toBe(false);

    await userEvent.click(b);
    await userEvent.click(screen.getByRole("button", { name: "保存默认构件" }));

    await waitFor(() => {
      expect(mockInvoke).toHaveBeenCalledWith("set_default_artifact", {
        projectId: 7,
        artifact: "b-web-1.0.war",
      });
    });
    expect(await screen.findByText("默认构件已保存")).toBeInTheDocument();
  });

  it("项目已有默认构件时勾选默认预选中", async () => {
    await openPanelWithBranches(
      { start_build: () => outcomeOkMulti },
      { ...gitProject, default_artifact: "b-web-1.0.war" },
    );

    await userEvent.click(screen.getByRole("button", { name: "开始打包" }));

    const b = (await screen.findByRole("radio", {
      name: /b-web-1\.0\.war/,
    })) as HTMLInputElement;
    await waitFor(() => expect(b.checked).toBe(true));
  });

  it("保存默认构件失败时展示后端错误", async () => {
    await openPanelWithBranches({
      start_build: () => outcomeOkMulti,
      set_default_artifact: () => {
        throw { code: "store", message: "store error" };
      },
    });

    await userEvent.click(screen.getByRole("button", { name: "开始打包" }));
    await userEvent.click(await screen.findByRole("button", { name: "保存默认构件" }));

    expect(await screen.findByRole("alert")).toBeInTheDocument();
  });
});

describe("打包面板（构建执行）", () => {
  it("开始打包后调用 start_build，克隆与构建阶段状态实时可见", async () => {
    let captured: { onmessage?: (msg: unknown) => void } = {};
    await openPanelWithBranches({
      // start_build stays pending like a real build
      start_build: () => new Promise(() => {}),
    });
    vi.mocked(Channel).mockImplementation(function (this: unknown) {
      const ch: { onmessage?: (msg: unknown) => void } = {};
      captured = ch;
      return ch as never;
    });

    await userEvent.click(screen.getByRole("button", { name: "开始打包" }));

    await waitFor(() => {
      expect(mockInvoke).toHaveBeenCalledWith("start_build", {
        projectId: 7,
        branch: "develop",
        onEvent: expect.anything(),
      });
    });

    // 克隆进度事件
    captured.onmessage?.({ type: "clone", stage: "transferring", received: 3, total: 10 });
    expect(await screen.findByText(/3\s*\/\s*10/)).toBeInTheDocument();

    // 构建阶段：实时日志行
    captured.onmessage?.({ type: "log", line: "[INFO] Scanning for projects..." });
    expect(await screen.findByText(/Scanning for projects/)).toBeInTheDocument();
    expect(await screen.findByText("正在构建…")).toBeInTheDocument();
  });

  it("构建成功展示成功状态", async () => {
    await openPanelWithBranches({ start_build: () => outcomeOk });

    await userEvent.click(screen.getByRole("button", { name: "开始打包" }));

    expect(await screen.findByText("构建成功")).toBeInTheDocument();
    expect(screen.queryByText("正在构建…")).not.toBeInTheDocument();
  });

  it("构建失败展示失败状态，并可通过按钮读取持久化日志", async () => {
    await openPanelWithBranches({
      start_build: () => outcomeFailed,
      read_build_log: (args) => {
        expect(args).toMatchObject({ buildId: "build-7-1001" });
        return "[INFO] ... \n[ERROR] BUILD FAILURE";
      },
    });

    await userEvent.click(screen.getByRole("button", { name: "开始打包" }));

    expect(await screen.findByText(/构建失败/)).toBeInTheDocument();
    await userEvent.click(screen.getByRole("button", { name: "查看完整日志" }));

    expect(await screen.findByText(/BUILD FAILURE/)).toBeInTheDocument();
  });

  it("构建失败（start_build 报错，如克隆失败）展示明确错误", async () => {
    await openPanelWithBranches({
      start_build: () => {
        throw { code: "clone_failed", message: "reference 'refs/heads/nope' not found" };
      },
    });

    await userEvent.click(screen.getByRole("button", { name: "开始打包" }));

    expect(await screen.findByRole("alert")).toHaveTextContent("克隆失败");
  });

  it("工具链缺失时报错并提示先托管下载", async () => {
    await openPanelWithBranches({
      start_build: () => {
        throw { code: "toolchain_missing", message: "managed toolchain missing: mvnd" };
      },
    });

    await userEvent.click(screen.getByRole("button", { name: "开始打包" }));

    expect(await screen.findByRole("alert")).toHaveTextContent("托管工具链缺失");
  });

  it("打包进行中可取消，向后端发送 cancel_build_workspace", async () => {
    await openPanelWithBranches({
      start_build: () => new Promise(() => {}),
      cancel_build_workspace: () => null,
    });

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
