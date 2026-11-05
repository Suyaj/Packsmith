// 部署目标 API（ticket 09 / 19 / 26）：typed boundary tests — every call is
// `invoke` with these exact payload shapes. 默认上传目录 (uploadDir) 随
// create/update 提交（票 26 恢复为业务字段）；映射行 (files) 可选、整组提交；
// set_target_last_artifact 已废弃，UI 不再调用。

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
  it("create_deployment_target 携带默认上传目录与映射行", async () => {
    invokeMock.mockResolvedValue({});
    await createDeploymentTarget(5, {
      serverId: 11,
      uploadDir: "/srv/app",
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
      uploadDir: "/srv/app",
      files: [
        { fileName: "app-1.0.jar", uploadDir: "/srv/app" },
        { fileName: "", uploadDir: "/srv/pick-later" },
      ],
      startCommands: "sh start.sh",
      stopCommands: "sh stop.sh",
    });
  });

  it("create_deployment_target 允许零映射行（映射行可选，票 26）", async () => {
    invokeMock.mockResolvedValue({});
    await createDeploymentTarget(5, {
      serverId: 11,
      uploadDir: "/srv/app",
      files: [],
      startCommands: "",
      stopCommands: "",
    });
    expect(invokeMock).toHaveBeenCalledWith(
      "create_deployment_target",
      expect.objectContaining({ uploadDir: "/srv/app", files: [] }),
    );
  });

  it("update_deployment_target 携带 id、默认上传目录与整组替换的映射行", async () => {
    invokeMock.mockResolvedValue({});
    await updateDeploymentTarget(1, {
      serverId: 12,
      uploadDir: "/srv/other",
      files: [{ fileName: "only.jar", uploadDir: "/srv/x" }],
      startCommands: "",
      stopCommands: "",
      autoDeploy: true,
    }, 5);
    expect(invokeMock).toHaveBeenCalledWith("update_deployment_target", {
      id: 1,
      projectId: 5,
      serverId: 12,
      uploadDir: "/srv/other",
      files: [{ fileName: "only.jar", uploadDir: "/srv/x" }],
      startCommands: "",
      stopCommands: "",
      autoDeploy: true,
    });
  });
});
