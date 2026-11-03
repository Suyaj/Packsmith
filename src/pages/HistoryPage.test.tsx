import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { MemoryRouter } from "react-router-dom";
import HistoryPage from "./HistoryPage";

vi.mock("@tauri-apps/api/core", () => ({
  invoke: vi.fn(),
}));

import { invoke } from "@tauri-apps/api/core";
import "@/i18n";

const mockInvoke = vi.mocked(invoke);

function mockBackend(map: Record<string, (args: unknown) => unknown>) {
  mockInvoke.mockImplementation(async (cmd: string, args?: unknown) => {
    const handler = map[cmd];
    if (!handler) throw new Error(`unexpected command: ${cmd}`);
    return handler(args);
  });
}

const entries = [
  {
    id: 3,
    build_id: "build-2-300",
    project_id: 2,
    project_name: "beta",
    branch: "release/1.0",
    success: true,
    exit_code: 0,
    log_path: "/appdata/build-logs/build-2-300.log",
    artifacts_dir: "/appdata/artifacts/beta/20261103-130000",
    created_at: "2026-11-03T13:00:00.000Z",
  },
  {
    id: 1,
    build_id: "build-1-100",
    project_id: 1,
    project_name: "alpha",
    branch: "main",
    success: false,
    exit_code: 3,
    log_path: "/appdata/build-logs/build-1-100.log",
    artifacts_dir: null,
    created_at: "2026-11-03T12:00:00.000Z",
  },
];

const projects = [
  { id: 1, name: "alpha" },
  { id: 2, name: "beta" },
];

function renderPage() {
  return render(
    <MemoryRouter>
      <HistoryPage />
    </MemoryRouter>,
  );
}

beforeEach(() => {
  mockInvoke.mockReset();
});

describe("构建历史页", () => {
  it("跨项目按最近在前展示：项目/分支/时间/结果", async () => {
    mockBackend({
      list_build_history: (args) => {
        expect(args).toMatchObject({ limit: null });
        return entries;
      },
      list_projects: () => projects,
    });

    renderPage();

    // 最近的条目在最上面；成功/失败结果可见。
    const rows = await screen.findAllByTestId("history-row");
    expect(rows).toHaveLength(2);
    expect(rows[0]).toHaveTextContent("beta");
    expect(rows[0]).toHaveTextContent("release/1.0");
    expect(rows[0]).toHaveTextContent("成功");
    expect(rows[1]).toHaveTextContent("alpha");
    expect(rows[1]).toHaveTextContent("失败");
    expect(rows[1]).toHaveTextContent("3");
  });

  it("无记录时展示空状态", async () => {
    mockBackend({ list_build_history: () => [], list_projects: () => projects });
    renderPage();
    expect(await screen.findByText("暂无构建记录")).toBeInTheDocument();
  });

  it("可查看对应构建的持久化日志", async () => {
    mockBackend({
      list_build_history: () => entries,
      list_projects: () => projects,
      read_build_log: (args) => {
        expect(args).toMatchObject({ buildId: "build-2-300" });
        return "[INFO] BUILD SUCCESS";
      },
    });

    renderPage();
    await screen.findAllByTestId("history-row");
    await userEvent.click(screen.getAllByRole("button", { name: "查看日志" })[0]);

    expect(await screen.findByText(/BUILD SUCCESS/)).toBeInTheDocument();
  });

  it("删除一条历史：构件与日志一并删除并刷新列表", async () => {
    const confirm = vi.spyOn(window, "confirm").mockReturnValue(true);
    mockBackend({
      list_build_history: () => entries,
      list_projects: () => projects,
      delete_build_history: (args) => {
        expect(args).toMatchObject({ historyId: 3 });
        return null;
      },
    });

    renderPage();
    await screen.findAllByTestId("history-row");
    await userEvent.click(screen.getAllByRole("button", { name: "删除" })[0]);

    await waitFor(() => {
      expect(mockInvoke).toHaveBeenCalledWith("delete_build_history", { historyId: 3 });
    });
    confirm.mockRestore();
  });

  it("按项目筛选后可手动清理该项目的全部历史", async () => {
    const confirm = vi.spyOn(window, "confirm").mockReturnValue(true);
    mockBackend({
      list_build_history: () => entries,
      list_projects: () => projects,
      list_project_build_history: (args) => {
        expect(args).toMatchObject({ projectId: 1 });
        return [entries[1]];
      },
      cleanup_project_history: (args) => {
        expect(args).toMatchObject({ projectId: 1 });
        return null;
      },
    });

    renderPage();
    await screen.findAllByTestId("history-row");

    await userEvent.selectOptions(screen.getByLabelText("按项目筛选"), "1");
    await userEvent.click(await screen.findByRole("button", { name: "清理该项目历史" }));

    await waitFor(() => {
      expect(mockInvoke).toHaveBeenCalledWith("cleanup_project_history", { projectId: 1 });
    });
    confirm.mockRestore();
  });
});
