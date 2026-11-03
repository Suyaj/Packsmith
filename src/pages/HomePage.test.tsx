import { render, screen, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { MemoryRouter } from "react-router-dom";
import HomePage from "./HomePage";

vi.mock("@tauri-apps/api/core", () => ({
  invoke: vi.fn(),
}));

import { invoke } from "@tauri-apps/api/core";
import "@/i18n";

const mockInvoke = vi.mocked(invoke);

const entries = [
  {
    id: 2,
    build_id: "build-2-200",
    project_id: 2,
    project_name: "beta",
    branch: "release/1.0",
    success: true,
    exit_code: 0,
    log_path: "/log",
    artifacts_dir: "/appdata/artifacts/beta/x",
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
    log_path: "/log",
    artifacts_dir: null,
    created_at: "2026-11-03T12:00:00.000Z",
  },
];

beforeEach(() => {
  mockInvoke.mockReset();
});

describe("首页：最近打包记录", () => {
  it("展示最近打包记录（项目/分支/时间/结果，最近在前）", async () => {
    mockInvoke.mockImplementation(async (cmd: string) => {
      expect(cmd).toBe("list_build_history");
      return entries;
    });

    render(
      <MemoryRouter>
        <HomePage />
      </MemoryRouter>,
    );

    const rows = await screen.findAllByTestId("recent-build-row");
    expect(rows).toHaveLength(2);
    expect(rows[0]).toHaveTextContent("beta");
    expect(rows[0]).toHaveTextContent("成功");
    expect(rows[1]).toHaveTextContent("alpha");
    expect(rows[1]).toHaveTextContent("失败");
  });

  it("无打包记录时展示空状态", async () => {
    mockInvoke.mockResolvedValue([]);

    render(
      <MemoryRouter>
        <HomePage />
      </MemoryRouter>,
    );

    await waitFor(() => expect(screen.getByText("暂无打包记录")).toBeInTheDocument());
  });
});
