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
