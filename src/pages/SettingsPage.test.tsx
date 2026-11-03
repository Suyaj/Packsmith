import { render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import SettingsPage from "./SettingsPage";

vi.mock("@tauri-apps/api/core", () => {
  return {
    invoke: vi.fn(),
    Channel: class FakeChannel<T> {
      onmessage: ((msg: T) => void) | null = null;
    },
  };
});

const getVersionMock = vi.fn(async () => "1.2.3");
vi.mock("@tauri-apps/api/app", () => ({
  getVersion: () => getVersionMock(),
}));

import { invoke } from "@tauri-apps/api/core";
import "@/i18n";

const mockInvoke = vi.mocked(invoke);

import { ToolStatus } from "@/lib/toolchain";

const fixtureStatuses: ToolStatus[] = [
  { tool: "git", state: { state: "missing" }, canDownload: true },
  {
    tool: "jdk",
    state: { state: "detected", path: "/usr/bin/java" },
    canDownload: true,
  },
  {
    tool: "mvnd",
    state: { state: "managed", version: "1.0.6", path: "mvnd/bin/mvnd" },
    canDownload: false,
  },
];

function rowOf(toolText: string | RegExp): HTMLElement {
  const heading = screen.getByRole("heading", { name: toolText });
  return heading.closest("li") as HTMLElement;
}

beforeEach(() => {
  mockInvoke.mockReset();
  getVersionMock.mockClear();
});

describe("设置页：应用信息", () => {
  it("展示应用版本信息（tauri getVersion）", async () => {
    mockInvoke.mockImplementation(async (cmd: string) => {
      if (cmd === "toolchain_status") return fixtureStatuses;
      throw new Error(`unexpected command ${cmd}`);
    });
    render(<SettingsPage />);

    expect(await screen.findByText("应用版本：1.2.3")).toBeInTheDocument();
  });

  it("展示依赖仓库位置（<appdata>/m2-repo，持久保留）", async () => {
    mockInvoke.mockResolvedValue(fixtureStatuses);
    render(<SettingsPage />);

    expect(
      await screen.findByText(/依赖仓库：<appdata>\/m2-repo/),
    ).toBeInTheDocument();
    expect(screen.getByText(/持久保留/)).toBeInTheDocument();
  });
});

describe("设置页：托管工具链", () => {
  it("展示 git / JDK / mvnd 三行及各自状态", async () => {
    mockInvoke.mockResolvedValue(fixtureStatuses);
    render(<SettingsPage />);

    await waitFor(() => {
      expect(screen.getByRole("heading", { name: "Git" })).toBeInTheDocument();
    });
    expect(rowOf("Git")).toHaveTextContent("缺失");
    expect(rowOf(/^JDK/)).toHaveTextContent("已检测");
    expect(rowOf("mvnd")).toHaveTextContent("已托管");
    expect(rowOf("mvnd")).toHaveTextContent("1.0.6");
  });

  it("缺失与已检测的工具提供下载按钮，已托管不提供", async () => {
    mockInvoke.mockResolvedValue(fixtureStatuses);
    render(<SettingsPage />);

    await waitFor(() => {
      expect(
        within(rowOf("Git")).getByRole("button", { name: "下载托管" }),
      ).toBeInTheDocument();
    });
    expect(
      within(rowOf(/^JDK/)).getByRole("button", { name: "下载托管" }),
    ).toBeInTheDocument();
    expect(
      within(rowOf("mvnd")).queryByRole("button", { name: "下载托管" }),
    ).not.toBeInTheDocument();
  });

  it("下载中展示进度，完成后状态刷新为已托管", async () => {
    mockInvoke.mockImplementation(async (cmd: string, args?: unknown) => {
      if (cmd === "toolchain_status") return fixtureStatuses;
      if (cmd === "toolchain_download") {
        const channel = (args as { onProgress: { onmessage: (msg: unknown) => void } })?.onProgress;
        channel.onmessage({ tool: "git", phase: "downloading", received: 50, total: 100 });
        channel.onmessage({ tool: "git", phase: "extracting", received: 1, total: null });
        const updated: ToolStatus = {
          tool: "git",
          state: { state: "managed", version: "2.47.1", path: "git/bin/git" },
          canDownload: false,
        };
        return updated;
      }
      throw new Error(`unexpected command ${cmd}`);
    });
    render(<SettingsPage />);

    await waitFor(() => {
      expect(
        within(rowOf("Git")).getByRole("button", { name: "下载托管" }),
      ).toBeInTheDocument();
    });
    await userEvent.click(
      within(rowOf("Git")).getByRole("button", { name: "下载托管" }),
    );

    await waitFor(() => {
      expect(rowOf("Git")).toHaveTextContent("已托管");
    });
    expect(mockInvoke).toHaveBeenCalledWith("toolchain_download", expect.anything());
    // 下载结束后按钮消失（已托管）
    expect(
      within(rowOf("Git")).queryByRole("button", { name: "下载托管" }),
    ).not.toBeInTheDocument();
  });

  it("下载失败显示错误并可重试", async () => {
    mockInvoke.mockImplementation(async (cmd: string) => {
      if (cmd === "toolchain_status") return fixtureStatuses;
      if (cmd === "get_close_window_behavior") return "minimize";
      if (cmd === "get_git_account") return { username: null, hasToken: false };
      if (cmd === "get_mvnd_settings_xml")
        return { content: null, fileExists: false };
      throw new Error("网络下载失败：连接超时");
    });
    render(<SettingsPage />);

    await waitFor(() => {
      expect(
        within(rowOf("Git")).getByRole("button", { name: "下载托管" }),
      ).toBeInTheDocument();
    });
    await userEvent.click(
      within(rowOf("Git")).getByRole("button", { name: "下载托管" }),
    );

    expect(await screen.findByText(/网络下载失败：连接超时/)).toBeInTheDocument();
    expect(
      within(rowOf("Git")).getByRole("button", { name: "重试" }),
    ).toBeInTheDocument();
    expect(rowOf("Git")).not.toHaveTextContent("已托管");

    // 重试成功
    mockInvoke.mockImplementation(async (cmd: string, args?: unknown) => {
      if (cmd === "toolchain_status") return fixtureStatuses;
      if (cmd === "toolchain_download") {
        const channel = (args as { onProgress: { onmessage: (msg: unknown) => void } })?.onProgress;
        channel.onmessage({ tool: "git", phase: "installing", received: 1, total: 1 });
        return {
          tool: "git",
          state: { state: "managed", version: "2.47.1", path: "git/bin/git" },
          canDownload: false,
        } satisfies ToolStatus;
      }
      throw new Error(`unexpected command ${cmd}`);
    });
    await userEvent.click(within(rowOf("Git")).getByRole("button", { name: "重试" }));

    await waitFor(() => {
      expect(rowOf("Git")).toHaveTextContent("已托管");
    });
  });
});

describe("设置页：构件库保留策略", () => {
  function mockBackend(map: Record<string, (args: unknown) => unknown>) {
    mockInvoke.mockImplementation(async (cmd: string, args?: unknown) => {
      const handler = map[cmd];
      if (!handler) throw new Error(`unexpected command: ${cmd}`);
      return handler(args);
    });
  }

  it("进入设置页读取当前保留次数", async () => {
    mockBackend({
      toolchain_status: () => fixtureStatuses,
      get_build_retention: () => 10,
    });
    render(<SettingsPage />);

    const input = (await screen.findByLabelText("保留次数")) as HTMLInputElement;
    await waitFor(() => expect(input.value).toBe("10"));
  });

  it("修改保留次数并保存到后端", async () => {
    mockBackend({
      toolchain_status: () => fixtureStatuses,
      get_build_retention: () => 10,
      set_build_retention: (args) => {
        expect(args).toMatchObject({ retention: 3 });
        return null;
      },
    });
    render(<SettingsPage />);

    const input = await screen.findByLabelText("保留次数");
    const card = input.closest('[data-slot="card"]') as HTMLElement;
    await userEvent.clear(input);
    await userEvent.type(input, "3");
    await userEvent.click(within(card).getByRole("button", { name: "保存" }));

    expect(await within(card).findByText("已保存")).toBeInTheDocument();
    expect(mockInvoke).toHaveBeenCalledWith("set_build_retention", { retention: 3 });
  });

  it("非法保留次数被拒绝且不调用后端", async () => {
    mockBackend({
      toolchain_status: () => fixtureStatuses,
      get_build_retention: () => 10,
    });
    render(<SettingsPage />);

    const input = await screen.findByLabelText("保留次数");
    const card = input.closest('[data-slot="card"]') as HTMLElement;
    await userEvent.clear(input);
    await userEvent.type(input, "0");
    await userEvent.click(within(card).getByRole("button", { name: "保存" }));

    expect(await screen.findByText(/不小于 1/)).toBeInTheDocument();
    expect(mockInvoke).not.toHaveBeenCalledWith(
      "set_build_retention",
      expect.anything(),
    );
  });
});

describe("设置页：关闭窗口行为", () => {
  it("默认选中最小化到托盘", async () => {
    mockInvoke.mockImplementation(async (cmd: string) => {
      if (cmd === "toolchain_status") return fixtureStatuses;
      if (cmd === "get_close_window_behavior") return "minimize";
      throw new Error(`unexpected command ${cmd}`);
    });
    render(<SettingsPage />);

    const section = await screen.findByRole("group", { name: "关闭窗口行为" });
    expect(
      within(section).getByRole("radio", { name: "最小化到托盘" }),
    ).toBeChecked();
    expect(
      within(section).getByRole("radio", { name: "退出应用" }),
    ).not.toBeChecked();
    // 默认不写库
    expect(mockInvoke).not.toHaveBeenCalledWith(
      "set_close_window_behavior",
      expect.anything(),
    );
  });

  it("加载到 quit 时选中退出应用", async () => {
    mockInvoke.mockImplementation(async (cmd: string) => {
      if (cmd === "toolchain_status") return fixtureStatuses;
      if (cmd === "get_close_window_behavior") return "quit";
      throw new Error(`unexpected command ${cmd}`);
    });
    render(<SettingsPage />);

    const section = await screen.findByRole("group", { name: "关闭窗口行为" });
    await waitFor(() => {
      expect(
        within(section).getByRole("radio", { name: "退出应用" }),
      ).toBeChecked();
    });
  });

  it("切换选项立即持久化", async () => {
    mockInvoke.mockImplementation(async (cmd: string) => {
      if (cmd === "toolchain_status") return fixtureStatuses;
      if (cmd === "get_close_window_behavior") return "minimize";
      if (cmd === "set_close_window_behavior") return undefined;
      throw new Error(`unexpected command ${cmd}`);
    });
    render(<SettingsPage />);

    const section = await screen.findByRole("group", { name: "关闭窗口行为" });
    await userEvent.click(
      within(section).getByRole("radio", { name: "退出应用" }),
    );

    await waitFor(() => {
      expect(mockInvoke).toHaveBeenCalledWith("set_close_window_behavior", {
        behavior: "quit",
      });
    });
    expect(
      within(section).getByRole("radio", { name: "退出应用" }),
    ).toBeChecked();
  });

  it("保存失败时回显错误且不静默丢选项", async () => {
    mockInvoke.mockImplementation(async (cmd: string) => {
      if (cmd === "toolchain_status") return fixtureStatuses;
      if (cmd === "get_close_window_behavior") return "minimize";
      if (cmd === "set_close_window_behavior")
        throw new Error("数据保存失败");
      throw new Error(`unexpected command ${cmd}`);
    });
    render(<SettingsPage />);

    const section = await screen.findByRole("group", { name: "关闭窗口行为" });
    await userEvent.click(
      within(section).getByRole("radio", { name: "退出应用" }),
    );

    expect(await screen.findByText(/数据保存失败/)).toBeInTheDocument();
    // 保存失败：选项回退到已持久化的值
    expect(
      within(section).getByRole("radio", { name: "最小化到托盘" }),
    ).toBeChecked();
  });
});

describe("设置页：Git 账号（票 13）", () => {
  function mockAccount(account?: { username: string | null; hasToken: boolean }) {
    mockInvoke.mockImplementation(async (cmd: string, args?: unknown) => {
      switch (cmd) {
        case "toolchain_status":
          return fixtureStatuses;
        case "get_git_account":
          return account ?? { username: null, hasToken: false };
        case "set_git_account": {
          const a = args as { username: string; token?: string };
          return {
            username: a.username,
            hasToken: a.token !== undefined || Boolean(account?.hasToken),
          };
        }
        case "clear_git_account":
          return { username: null, hasToken: false };
        default:
          throw new Error(`unexpected command ${cmd}`);
      }
    });
  }

  it("未配置时显示未设置指示", async () => {
    mockAccount({ username: null, hasToken: false });
    render(<SettingsPage />);

    const card = screen
      .getByText("Git 账号")
      .closest('[data-slot="card"]') as HTMLElement;
    await within(card).findByText("未设置");
  });

  it("保存用户名与令牌：明文令牌走 set_git_account，保存后显示已设置", async () => {
    const user = userEvent.setup();
    mockAccount({ username: null, hasToken: false });
    render(<SettingsPage />);

    const card = screen
      .getByText("Git 账号")
      .closest('[data-slot="card"]') as HTMLElement;
    await within(card).findByText("未设置");

    await user.type(within(card).getByLabelText("用户名"), "octocat");
    await user.type(within(card).getByLabelText("密码 / 访问令牌"), "ghp_abc");
    await user.click(within(card).getByRole("button", { name: "保存" }));

    await waitFor(() => {
      expect(mockInvoke).toHaveBeenCalledWith("set_git_account", {
        username: "octocat",
        token: "ghp_abc",
      });
    });
    expect(await within(card).findByText("已设置")).toBeInTheDocument();
    // 保存成功后清空令牌输入框，明文不留在 DOM。
    expect(within(card).getByLabelText("密码 / 访问令牌")).toHaveValue("");
  });

  it("令牌输入框为密码型（不明文回显）", async () => {
    mockAccount({ username: null, hasToken: false });
    render(<SettingsPage />);

    const card = screen
      .getByText("Git 账号")
      .closest('[data-slot="card"]') as HTMLElement;
    await within(card).findByText("未设置");
    expect(within(card).getByLabelText("密码 / 访问令牌")).toHaveAttribute(
      "type",
      "password",
    );
  });

  it("编辑时令牌留空 = 保持不变（不传 token 字段）", async () => {
    const user = userEvent.setup();
    mockAccount({ username: "octocat", hasToken: true });
    render(<SettingsPage />);

    const card = screen
      .getByText("Git 账号")
      .closest('[data-slot="card"]') as HTMLElement;
    await within(card).findByText("已设置");

    await user.clear(within(card).getByLabelText("用户名"));
    await user.type(within(card).getByLabelText("用户名"), "renamed");
    await user.click(within(card).getByRole("button", { name: "保存" }));

    await waitFor(() => {
      expect(mockInvoke).toHaveBeenCalledWith("set_git_account", {
        username: "renamed",
      });
    });
  });

  it("编辑时回填用户名但不回填令牌（令牌不明文回传）", async () => {
    mockAccount({ username: "octocat", hasToken: true });
    render(<SettingsPage />);

    const card = screen
      .getByText("Git 账号")
      .closest('[data-slot="card"]') as HTMLElement;
    await within(card).findByText("已设置");
    expect(within(card).getByLabelText("用户名")).toHaveValue("octocat");
    expect(within(card).getByLabelText("密码 / 访问令牌")).toHaveValue("");
  });

  it("清除账号：调用 clear_git_account，恢复未设置", async () => {
    const user = userEvent.setup();
    mockAccount({ username: "octocat", hasToken: true });
    render(<SettingsPage />);

    const card = screen
      .getByText("Git 账号")
      .closest('[data-slot="card"]') as HTMLElement;
    await within(card).findByText("已设置");

    await user.click(within(card).getByRole("button", { name: "清除账号" }));

    await waitFor(() => {
      expect(mockInvoke).toHaveBeenCalledWith("clear_git_account");
    });
    expect(await within(card).findByText("未设置")).toBeInTheDocument();
  });

  it("未配置时不显示清除按钮", async () => {
    mockAccount({ username: null, hasToken: false });
    render(<SettingsPage />);

    const card = screen
      .getByText("Git 账号")
      .closest('[data-slot="card"]') as HTMLElement;
    await within(card).findByText("未设置");
    expect(
      within(card).queryByRole("button", { name: "清除账号" }),
    ).not.toBeInTheDocument();
  });

  it("保存失败：显示错误且不清空输入", async () => {
    const user = userEvent.setup();
    mockInvoke.mockImplementation(async (cmd: string) => {
      if (cmd === "toolchain_status") return fixtureStatuses;
      if (cmd === "get_git_account") return { username: null, hasToken: false };
      if (cmd === "set_git_account")
        throw { code: "git_username_required", message: "Git 用户名不能为空" };
      throw new Error(`unexpected command ${cmd}`);
    });
    render(<SettingsPage />);

    const card = screen
      .getByText("Git 账号")
      .closest('[data-slot="card"]') as HTMLElement;
    await within(card).findByText("未设置");

    await user.type(within(card).getByLabelText("用户名"), "   ");
    await user.click(within(card).getByRole("button", { name: "保存" }));

    expect(await within(card).findByText(/用户名不能为空/)).toBeInTheDocument();
  });
});

describe("设置页：mvnd settings.xml（票 18）", () => {
  type MvndState = { content: string | null; fileExists: boolean };

  function mockMvnd(state: MvndState) {
    let current = state;
    mockInvoke.mockImplementation(async (cmd: string, args?: unknown) => {
      switch (cmd) {
        case "toolchain_status":
          return fixtureStatuses;
        case "get_mvnd_settings_xml":
          return current;
        case "set_mvnd_settings_xml": {
          const a = args as { content: string };
          current = { content: a.content, fileExists: true };
          return current;
        }
        case "clear_mvnd_settings_xml":
          current = { content: null, fileExists: false };
          return current;
        default:
          throw new Error(`unexpected command ${cmd}`);
      }
    });
  }

  function card(): HTMLElement {
    return screen
      .getByText("mvnd settings.xml")
      .closest('[data-slot="card"]') as HTMLElement;
  }

  it("未设置时显示未设置指示、无清空按钮", async () => {
    mockMvnd({ content: null, fileExists: false });
    render(<SettingsPage />);

    await within(card()).findByText("未设置");
    expect(
      within(card()).queryByRole("button", { name: "清空" }),
    ).not.toBeInTheDocument();
  });

  it("加载时回填已保存的 XML 内容并显示已设置", async () => {
    mockMvnd({ content: "<settings><mirrors/></settings>", fileExists: true });
    render(<SettingsPage />);

    await within(card()).findByText("已设置");
    const textarea = within(card()).getByLabelText(
      "settings.xml 内容",
    ) as HTMLTextAreaElement;
    expect(textarea.value).toBe("<settings><mirrors/></settings>");
    // 内容是配置而非机密：回显为可读文本
    expect(textarea).not.toHaveAttribute("type", "password");
  });

  it("保存调用 set_mvnd_settings_xml 并显示已保存与已设置", async () => {
    const user = userEvent.setup();
    mockMvnd({ content: null, fileExists: false });
    render(<SettingsPage />);

    await within(card()).findByText("未设置");
    await user.type(
      within(card()).getByLabelText("settings.xml 内容"),
      "<settings/>",
    );
    await user.click(within(card()).getByRole("button", { name: "保存" }));

    await waitFor(() => {
      expect(mockInvoke).toHaveBeenCalledWith("set_mvnd_settings_xml", {
        content: "<settings/>",
      });
    });
    expect(await within(card()).findByText("已保存")).toBeInTheDocument();
    expect(await within(card()).findByText("已设置")).toBeInTheDocument();
    // 设置后出现清空按钮
    expect(
      within(card()).getByRole("button", { name: "清空" }),
    ).toBeInTheDocument();
  });

  it("清空调用 clear_mvnd_settings_xml、清空输入并恢复未设置", async () => {
    const user = userEvent.setup();
    mockMvnd({ content: "<settings/>", fileExists: true });
    render(<SettingsPage />);

    await within(card()).findByText("已设置");
    await user.click(within(card()).getByRole("button", { name: "清空" }));

    await waitFor(() => {
      expect(mockInvoke).toHaveBeenCalledWith("clear_mvnd_settings_xml");
    });
    expect(await within(card()).findByText("未设置")).toBeInTheDocument();
    expect(within(card()).getByLabelText("settings.xml 内容")).toHaveValue("");
  });

  it("编辑框为等宽多行文本域", async () => {
    mockMvnd({ content: null, fileExists: false });
    render(<SettingsPage />);

    const textarea = await within(card()).findByLabelText(
      "settings.xml 内容",
    );
    expect(textarea.tagName).toBe("TEXTAREA");
    expect(textarea).toHaveClass("font-mono");
  });

  it("保存失败时显示错误且不清空输入", async () => {
    const user = userEvent.setup();
    mockInvoke.mockImplementation(async (cmd: string) => {
      if (cmd === "toolchain_status") return fixtureStatuses;
      if (cmd === "get_mvnd_settings_xml")
        return { content: null, fileExists: false };
      if (cmd === "set_mvnd_settings_xml")
        throw { code: "io", message: "磁盘写入失败" };
      throw new Error(`unexpected command ${cmd}`);
    });
    render(<SettingsPage />);

    await within(card()).findByText("未设置");
    await user.type(
      within(card()).getByLabelText("settings.xml 内容"),
      "<settings/>",
    );
    await user.click(within(card()).getByRole("button", { name: "保存" }));

    expect(await within(card()).findByText(/磁盘写入失败/)).toBeInTheDocument();
    expect(within(card()).getByLabelText("settings.xml 内容")).toHaveValue(
      "<settings/>",
    );
  });
});
