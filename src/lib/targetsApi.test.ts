// 部署目标 API（ticket 09 / 19）：typed boundary tests — every call is
// `invoke` with these exact payload shapes. 映射行 (files) 随 create/update
// 整组提交（ticket 19）；set_target_last_artifact 已废弃，UI 不再调用。

import { beforeEach, describe, expect, it, vi } from "vitest";

const invokeMock = vi.fn();

vi.mock("@tauri-apps/api/core", () => ({
  invoke: (...args: unknown[]) => invokeMock(...args),
}));

import { createDeploymentTarget, updateDeploymentTarget } from "./targetsApi";

beforeEach(() => {
  invokeMock.mockReset();
});

describe("targetsApi", () => {
  it("create_deployment_target 携带完整的 文件名→目录 映射行", async () => {
    invokeMock.mockResolvedValue({});
    await createDeploymentTarget(5, {
      serverId: 11,
      files: [
        { fileName: "app-1.0.jar", uploadDir: "/srv/app" },
        { fileName: "", uploadDir: "/srv/pick-later" },
      ],
      startCommands: "sh start.sh",
      stopCommands: "sh stop.sh",
    });
    expect(invokeMock).toHaveBeenCalledWith("create_deployment_target", {
      projectId: 5,
      serverId: 11,
      files: [
        { fileName: "app-1.0.jar", uploadDir: "/srv/app" },
        { fileName: "", uploadDir: "/srv/pick-later" },
      ],
      startCommands: "sh start.sh",
      stopCommands: "sh stop.sh",
    });
  });

  it("update_deployment_target 携带 id 与整组替换的映射行", async () => {
    invokeMock.mockResolvedValue({});
    await updateDeploymentTarget(1, {
      serverId: 12,
      files: [{ fileName: "only.jar", uploadDir: "/srv/x" }],
      startCommands: "",
      stopCommands: "",
      autoDeploy: true,
    });
    expect(invokeMock).toHaveBeenCalledWith("update_deployment_target", {
      id: 1,
      serverId: 12,
      files: [{ fileName: "only.jar", uploadDir: "/srv/x" }],
      startCommands: "",
      stopCommands: "",
      autoDeploy: true,
    });
  });
});
