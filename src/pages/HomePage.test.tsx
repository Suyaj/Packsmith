import { render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { MemoryRouter } from "react-router-dom";
import HomePage from "./HomePage";

vi.mock("@tauri-apps/api/core", () => ({
  invoke: vi.fn(),
  Channel: vi.fn(function (this: unknown) {
    return { onmessage: null };
  }),
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

const projects = [
  {
    id: 1,
    name: "alpha",
    source: "git",
    url: "https://example.com/alpha.git",
    path: "/w/alpha",
    project_type: "maven",
    build_command: "mvnd package -DskipTests",
    created_at: "2026-11-01T00:00:00.000Z",
  },
  {
    id: 2,
    name: "beta",
    source: "local",
    path: "/w/beta",
    project_type: "maven",
    build_command: "mvnd package -DskipTests",
    created_at: "2026-11-02T00:00:00.000Z",
  },
];

const servers = [
  { id: 1, host: "web-1", port: 22, username: "deploy" },
  { id: 2, host: "web-2", port: 2222, username: "root" },
];

beforeEach(() => {
  mockInvoke.mockReset();
});

function renderHome() {
  return render(
    <MemoryRouter>
      <HomePage />
    </MemoryRouter>,
  );
}

describe("首页：最近打包记录", () => {
  it("展示最近打包记录（项目/分支/时间/结果，最近在前）", async () => {
    mockBackend({
      list_build_history: (args) => {
        expect(args).toMatchObject({ limit: 5 });
        return entries;
      },
      list_projects: () => projects,
      list_servers: () => [],
      list_server_last_deploys: () => [],
    });

    renderHome();

    const rows = await screen.findAllByTestId("recent-build-row");
    expect(rows).toHaveLength(2);
    expect(rows[0]).toHaveTextContent("beta");
    expect(rows[0]).toHaveTextContent("成功");
    expect(rows[1]).toHaveTextContent("alpha");
    expect(rows[1]).toHaveTextContent("失败");
  });

  // 详情 = 构建历史页里展开该条目的日志（HistoryPage 支持 ?entry= 深链）。
  it("每条记录可点击跳转到构建历史详情", async () => {
    mockBackend({
      list_build_history: () => entries,
      list_projects: () => projects,
      list_servers: () => [],
      list_server_last_deploys: () => [],
    });

    renderHome();

    const rows = await screen.findAllByTestId("recent-build-row");
    const links = within(rows[0]).getAllByRole("link");
    expect(links[0]).toHaveAttribute("href", "/history?entry=2");
    expect(within(rows[1]).getAllByRole("link")[0]).toHaveAttribute("href", "/history?entry=1");
  });

  it("无打包记录时展示空状态", async () => {
    mockBackend({
      list_build_history: () => [],
      list_projects: () => projects,
      list_servers: () => [],
      list_server_last_deploys: () => [],
    });

    renderHome();

    await waitFor(() => expect(screen.getByText("暂无打包记录")).toBeInTheDocument());
  });
});

describe("首页：快速发起打包", () => {
  it("项目快选后点击打包，直达该项目的打包面板", async () => {
    mockBackend({
      list_build_history: () => [],
      list_projects: () => projects,
      list_servers: () => [],
      list_server_last_deploys: () => [],
      list_remote_branches: (args) => {
        expect(args).toMatchObject({ projectId: 2 });
        return ["main", "release/1.0"];
      },
    });

    renderHome();

    await userEvent.selectOptions(await screen.findByLabelText("选择项目"), "2");
    await userEvent.click(screen.getByRole("button", { name: "打包" }));

    // 打包面板（ticket 06/07 组件复用）出现，并已为所选项目拉取分支。
    expect(await screen.findByTestId("build-panel-beta")).toBeInTheDocument();
    await waitFor(() => {
      expect(mockInvoke).toHaveBeenCalledWith("list_remote_branches", { projectId: 2 });
    });
  });

  it("没有项目时给出引导入口", async () => {
    mockBackend({
      list_build_history: () => [],
      list_projects: () => [],
      list_servers: () => [],
      list_server_last_deploys: () => [],
    });

    renderHome();

    expect(await screen.findByText("还没有项目，请先在项目管理页新建")).toBeInTheDocument();
    expect(screen.getByRole("link", { name: "去项目管理" })).toHaveAttribute("href", "/projects");
  });
});
