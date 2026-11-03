// 部署目标 API（ticket 09 / 17）：typed boundary tests — every call is
// `invoke` with these exact payload shapes.

import { beforeEach, describe, expect, it, vi } from "vitest";

const invokeMock = vi.fn();

vi.mock("@tauri-apps/api/core", () => ({
  invoke: (...args: unknown[]) => invokeMock(...args),
}));

import { setTargetLastArtifact } from "./targetsApi";

beforeEach(() => {
  invokeMock.mockReset();
});

describe("targetsApi", () => {
  it("set_target_last_artifact 记住该目标上次部署选择的构件（ticket 17）", async () => {
    invokeMock.mockResolvedValue(true);
    await setTargetLastArtifact(9, "app-1.0.jar");
    expect(invokeMock).toHaveBeenCalledWith("set_target_last_artifact", {
      id: 9,
      fileName: "app-1.0.jar",
    });
  });
});
