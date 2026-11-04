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

describe("打包面板（构件选择·搜索多选）", () => {
  it("单构件构建自动选中：展示构件名，构件选择器同样可用", async () => {
    await openPanelWithBranches({ start_build: () => outcomeOk });

    await userEvent.click(screen.getByRole("button", { name: "开始打包" }));

    expect(await screen.findByText(/已自动选中构件/)).toBeInTheDocument();
    // 票 21：单/多产物场景下选择器均可用（单构件已预选，后端已自动记住）。
    expect(await screen.findByRole("combobox", { name: "构件选择" })).toBeInTheDocument();
    await userEvent.click(screen.getByRole("combobox", { name: "构件选择" }));
    expect(await screen.findByRole("option", { name: /app-1\.0\.jar/ })).toBeInTheDocument();
    expect(mockInvoke).not.toHaveBeenCalledWith(
      "set_default_artifacts",
      expect.anything(),
    );
  });

  it("多构件构建：搜索关键字实时过滤构件列表，无匹配展示空状态", async () => {
    await openPanelWithBranches({ start_build: () => outcomeOkMulti });

    await userEvent.click(screen.getByRole("button", { name: "开始打包" }));
    await userEvent.click(await screen.findByRole("combobox", { name: "构件选择" }));

    const search = await screen.findByPlaceholderText("搜索构件…");
    await userEvent.type(search, "web");
    expect(screen.getAllByRole("option").map((o) => o.textContent)).toEqual([
      "b-web-1.0.war",
    ]);

    await userEvent.clear(search);
    await userEvent.type(search, "zzz");
    expect(await screen.findByText("没有匹配的构件")).toBeInTheDocument();
    expect(screen.queryByRole("option")).not.toBeInTheDocument();
  });

  it("多选构件并按勾选顺序保存为默认构件列表", async () => {
    await openPanelWithBranches({
      start_build: () => outcomeOkMulti,
      set_default_artifacts: (args) => {
        expect(args).toMatchObject({
          projectId: 7,
          artifacts: ["b-web-1.0.war", "a-api-1.0.jar"],
        });
        return { ...gitProject, default_artifacts: ["b-web-1.0.war", "a-api-1.0.jar"] };
      },
    });

    await userEvent.click(screen.getByRole("button", { name: "开始打包" }));
    const trigger = await screen.findByRole("combobox", { name: "构件选择" });
    await userEvent.click(trigger);

    // 初始预选第一个构件：先取消，再按目标顺序勾选（先 b 后 a）。
    await userEvent.click(await screen.findByRole("option", { name: /a-api-1\.0\.jar/ }));
    await userEvent.click(screen.getByRole("option", { name: /b-web-1\.0\.war/ }));
    expect(screen.getByRole("checkbox", { name: /b-web-1\.0\.war/ })).toBeChecked();
    await userEvent.click(screen.getByRole("option", { name: /a-api-1\.0\.jar/ }));
    expect(screen.getByRole("checkbox", { name: /a-api-1\.0\.jar/ })).toBeChecked();

    // 勾选不关闭下拉（可继续搜索/调整），Esc 收起后触发按钮回显已勾选集。
    expect(trigger).toHaveAttribute("aria-expanded", "true");
    await userEvent.type(screen.getByPlaceholderText("搜索构件…"), "{Escape}");
    expect(trigger).toHaveAttribute("aria-expanded", "false");
    expect(trigger).toHaveTextContent("b-web-1.0.war");
    expect(trigger).toHaveTextContent("a-api-1.0.jar");

    await userEvent.click(screen.getByRole("button", { name: "保存默认构件" }));
    await waitFor(() => {
      expect(mockInvoke).toHaveBeenCalledWith("set_default_artifacts", {
        projectId: 7,
        artifacts: ["b-web-1.0.war", "a-api-1.0.jar"],
      });
    });
    expect(await screen.findByText("默认构件已保存")).toBeInTheDocument();
  });

  it("项目默认构件列表预选已保存的项；再次勾选可取消", async () => {
    await openPanelWithBranches(
      { start_build: () => outcomeOkMulti },
      { ...gitProject, default_artifacts: ["b-web-1.0.war"] },
    );

    await userEvent.click(screen.getByRole("button", { name: "开始打包" }));
    await userEvent.click(await screen.findByRole("combobox", { name: "构件选择" }));

    const b = screen.getByRole("checkbox", { name: /b-web-1\.0\.war/ }) as HTMLInputElement;
    const a = screen.getByRole("checkbox", { name: /a-api-1\.0\.jar/ }) as HTMLInputElement;
    await waitFor(() => expect(b.checked).toBe(true));
    expect(a.checked).toBe(false);

    await userEvent.click(screen.getByRole("option", { name: /b-web-1\.0\.war/ }));
    expect(screen.getByRole("checkbox", { name: /b-web-1\.0\.war/ })).not.toBeChecked();
  });

  it("无默认构件时预选第一个构件（单选行为与原来一致）", async () => {
    await openPanelWithBranches({ start_build: () => outcomeOkMulti });

    await userEvent.click(screen.getByRole("button", { name: "开始打包" }));
    await userEvent.click(await screen.findByRole("combobox", { name: "构件选择" }));

    expect(
      (screen.getByRole("checkbox", { name: /a-api-1\.0\.jar/ }) as HTMLInputElement).checked,
    ).toBe(true);
    expect(
      (screen.getByRole("checkbox", { name: /b-web-1\.0\.war/ }) as HTMLInputElement).checked,
    ).toBe(false);
  });

  it("保存默认构件失败时展示后端错误", async () => {
    await openPanelWithBranches({
      start_build: () => outcomeOkMulti,
      set_default_artifacts: () => {
        throw { code: "invalid_artifact", message: "invalid artifact name" };
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

// 最新提交（票 22）：克隆完成后经事件流推送所选分支最新 3 条提交，构建
// 结束后列表保持可见，再次打包时刷新。
describe("打包面板（最新提交）", () => {
  const commitsEvent = {
    type: "commits" as const,
    commits: [
      { sha: "cafeb01", author: "卡罗尔", time: 1730600000, message: "feature: 流式导出" },
      { sha: "cafeb02", author: "bob", time: 1730599000, message: "fix: 修复空指针" },
      { sha: "cafeb03", author: "alice", time: 1730598000, message: "add tests" },
    ],
  };

  /** 手动结算的 promise（模拟一次真实打包的生命周期）。 */
  function deferredStart() {
    let resolve!: (v: unknown) => void;
    const promise = new Promise((res) => {
      resolve = res;
    });
    return { promise, resolve };
  }

  /** 发起打包并捕获事件通道（start_build 挂起，返回手动结算函数）。 */
  async function startBuildCapturingChannel() {
    const pending = deferredStart();
    let captured: { onmessage?: (msg: unknown) => void } = {};
    await openPanelWithBranches({
      start_build: () => pending.promise,
    });
    vi.mocked(Channel).mockImplementation(function (this: unknown) {
      const ch: { onmessage?: (msg: unknown) => void } = {};
      captured = ch;
      return ch as never;
    });
    await userEvent.click(screen.getByRole("button", { name: "开始打包" }));
    await waitFor(() => {
      expect(mockInvoke).toHaveBeenCalledWith("start_build", expect.anything());
    });
    return { captured, resolveBuild: pending.resolve };
  }

  it("克隆完成后展示最新 3 条提交（短 SHA/作者/说明首行），构建结束后仍可见", async () => {
    const { captured, resolveBuild } = await startBuildCapturingChannel();

    captured.onmessage?.({ type: "clone", stage: "done" });
    captured.onmessage?.(commitsEvent);
    captured.onmessage?.({ type: "log", line: "[INFO] Scanning for projects..." });

    expect(await screen.findByTestId("build-commits")).toBeInTheDocument();
    expect(screen.getByText("最新提交")).toBeInTheDocument();
    expect(screen.getByText("cafeb01")).toBeInTheDocument();
    expect(screen.getByText("卡罗尔")).toBeInTheDocument();
    expect(screen.getByText("feature: 流式导出")).toBeInTheDocument();
    expect(screen.getByText("cafeb03")).toBeInTheDocument();
    expect(screen.getByText("add tests")).toBeInTheDocument();
    // 截断由样式处理：说明首行元素带 truncate 类。
    expect(screen.getByText("feature: 流式导出")).toHaveClass("truncate");

    // 构建结束后提交列表保持可见（紧挨构建结果区）。
    resolveBuild(outcomeOk);
    expect(await screen.findByText("构建成功")).toBeInTheDocument();
    expect(screen.getByTestId("build-commits")).toBeInTheDocument();
    expect(screen.getByText("cafeb01")).toBeInTheDocument();
  });

  it("再次打包后提交列表随新事件刷新", async () => {
    let captured: { onmessage?: (msg: unknown) => void } = {};
    await openPanelWithBranches({ start_build: () => outcomeOk });
    vi.mocked(Channel).mockImplementation(function (this: unknown) {
      const ch: { onmessage?: (msg: unknown) => void } = {};
      captured = ch;
      return ch as never;
    });

    // 第一次打包：main 的提交（构建结束后列表可见）。
    await userEvent.click(screen.getByRole("button", { name: "开始打包" }));
    captured.onmessage?.(commitsEvent);
    await screen.findByText("构建成功");
    expect(screen.getByText("cafeb01")).toBeInTheDocument();

    // 第二次打包：换分支后新提交列表替换旧列表。
    const second = deferredStart();
    mockBackend({
      list_remote_branches: () => ["main", "develop"],
      start_build: () => second.promise,
    });
    vi.mocked(Channel).mockImplementation(function (this: unknown) {
      const ch: { onmessage?: (msg: unknown) => void } = {};
      captured = ch;
      return ch as never;
    });
    await userEvent.click(screen.getByRole("button", { name: "开始打包" }));
    await waitFor(() => {
      expect(mockInvoke).toHaveBeenCalledWith("start_build", expect.anything());
    });
    captured.onmessage?.({
      type: "commits",
      commits: [
        { sha: "dead001", author: "dave", time: 1730700000, message: "release: v2" },
        { sha: "dead002", author: "erin", time: 1730690000, message: "chore: bump" },
        { sha: "dead003", author: "fred", time: 1730680000, message: "docs" },
      ],
    });

    expect(await screen.findByText("dead001")).toBeInTheDocument();
    expect(screen.queryByText("cafeb01")).not.toBeInTheDocument();
  });

  it("后端未推送提交事件时（读取失败静默）不渲染提交列表", async () => {
    await openPanelWithBranches({ start_build: () => outcomeOk });

    await userEvent.click(screen.getByRole("button", { name: "开始打包" }));
    expect(await screen.findByText("构建成功")).toBeInTheDocument();

    expect(screen.queryByText("最新提交")).not.toBeInTheDocument();
  });
});

// 部署入口（bug 回归）：构建成功后点击「部署」按钮必须渲染 DeployDialog。
describe("打包面板（部署入口）", () => {
  const deployServers = [
    { id: 11, host: "web-1", port: 22, username: "deploy", credential: { type: "password" as const, password: "x" } },
  ];
  const deployTargets = [
    {
      id: 1,
      projectId: 7,
      serverId: 11,
      startCommands: "sh start.sh",
      stopCommands: "sh stop.sh",
      autoDeploy: false,
      files: [{ id: 101, targetId: 1, fileName: "app-1.0.jar", uploadDir: "/srv/one", position: 0 }],
    },
  ];

  it("构建成功后点击部署按钮，DeployDialog 可见", async () => {
    await openPanelWithBranches({
      start_build: () => outcomeOk,
      list_deployment_targets: () => deployTargets,
      list_servers: () => deployServers,
      list_build_artifacts: () => [{ file_name: "app-1.0.jar", size: 1024 }],
    });

    await userEvent.click(screen.getByRole("button", { name: "开始打包" }));
    await screen.findByText("构建成功");

    await userEvent.click(screen.getByTestId("build-deploy"));

    expect(await screen.findByTestId("deploy-dialog")).toBeInTheDocument();
  });

  // 默认窗口 800×600，构建成功后面板里日志块已占据视口，DeployDialog 追加在
  // 折叠线以下——若不滚动进可视区域，用户看到的就是「点击没反应」。
  it("点击部署后对话框滚动进可视区域", async () => {
    await openPanelWithBranches({
      start_build: () => outcomeOk,
      list_deployment_targets: () => deployTargets,
      list_servers: () => deployServers,
      list_build_artifacts: () => [{ file_name: "app-1.0.jar", size: 1024 }],
    });

    await userEvent.click(screen.getByRole("button", { name: "开始打包" }));
    await screen.findByText("构建成功");

    const scrolled: Element[] = [];
    const spy = vi
      .spyOn(Element.prototype, "scrollIntoView")
      .mockImplementation(function (this: Element) {
        scrolled.push(this);
      });
    await userEvent.click(screen.getByTestId("build-deploy"));
    await screen.findByTestId("deploy-dialog");
    spy.mockRestore();

    expect(
      scrolled.some((el) => (el as HTMLElement).dataset.testid === "deploy-dialog"),
    ).toBe(true);
  });
});
