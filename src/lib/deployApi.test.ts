// 部署执行 API（ticket 10）：typed boundary tests — every call is `invoke`
// with these exact payload shapes, and events flow through a Channel.

import { beforeEach, describe, expect, it, vi } from "vitest";

const invokeMock = vi.fn();

vi.mock("@tauri-apps/api/core", () => ({
  invoke: (...args: unknown[]) => invokeMock(...args),
  // Minimal Channel stand-in: the dialog/API just pass it through.
  Channel: class {
    onmessage: ((msg: unknown) => void) | null = null;
  },
}));

import {
  cancelDeploy,
  runStartGroup,
  setTargetAutoDeploy,
  startDeploy,
  type AutoDeployMessage,
} from "./deployApi";

beforeEach(() => {
  invokeMock.mockReset();
});

describe("deployApi", () => {
  it("start_deploy 携带历史 id 与逐目标构件 assignments（一一对应上传）", async () => {
    invokeMock.mockResolvedValue({ cancelled: false, targets: [] });
    const onEvent = { onmessage: null } as never;
    await startDeploy(
      7,
      [
        { targetId: 1, fileName: "app-1.0.jar" },
        { targetId: 2, fileName: "other-1.0.jar" },
      ],
      onEvent,
    );

    expect(invokeMock).toHaveBeenCalledWith("start_deploy", {
      historyId: 7,
      assignments: [
        { targetId: 1, fileName: "app-1.0.jar" },
        { targetId: 2, fileName: "other-1.0.jar" },
      ],
      onEvent,
    });
  });

  it("cancel_deploy 携带 deployId", async () => {
    invokeMock.mockResolvedValue(true);
    await cancelDeploy(42);
    expect(invokeMock).toHaveBeenCalledWith("cancel_deploy", { deployId: 42 });
  });

  it("run_start_group 单独执行启动命令组", async () => {
    invokeMock.mockResolvedValue({ cancelled: false, targets: [] });
    const onEvent = { onmessage: null } as never;
    await runStartGroup(3, onEvent);
    expect(invokeMock).toHaveBeenCalledWith("run_start_group", {
      targetId: 3,
      onEvent,
    });
  });

  it("set_target_auto_deploy 切换自动连锁开关", async () => {
    invokeMock.mockResolvedValue(true);
    await setTargetAutoDeploy(9, true);
    expect(invokeMock).toHaveBeenCalledWith("set_target_auto_deploy", {
      id: 9,
      enabled: true,
    });
  });

  it("自动连锁事件信封保留 deployId 与事件本体", () => {
    const message: AutoDeployMessage = {
      deployId: 5,
      event: {
        type: "target_finished",
        target_id: 1,
        status: "failed",
        stop_failed: true,
        error: null,
      },
    };
    expect(message.deployId).toBe(5);
    expect(message.event.type).toBe("target_finished");
  });
});
