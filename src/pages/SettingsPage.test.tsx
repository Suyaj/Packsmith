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
    await userEvent.clear(input);
    await userEvent.type(input, "3");
    await userEvent.click(screen.getByRole("button", { name: "保存" }));

    expect(await screen.findByText("已保存")).toBeInTheDocument();
    expect(mockInvoke).toHaveBeenCalledWith("set_build_retention", { retention: 3 });
  });

  it("非法保留次数被拒绝且不调用后端", async () => {
    mockBackend({
      toolchain_status: () => fixtureStatuses,
      get_build_retention: () => 10,
    });
    render(<SettingsPage />);

    const input = await screen.findByLabelText("保留次数");
    await userEvent.clear(input);
    await userEvent.type(input, "0");
    await userEvent.click(screen.getByRole("button", { name: "保存" }));

    expect(await screen.findByRole("alert")).toHaveTextContent("不小于 1");
    expect(mockInvoke).not.toHaveBeenCalledWith(
      "set_build_retention",
      expect.anything(),
    );
  });
});
