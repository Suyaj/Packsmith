import { render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";

import type { Server, ServerInput, TestOutcome } from "@/lib/serversApi";

const invokeMock = vi.fn();

vi.mock("@tauri-apps/api/core", () => ({
  invoke: (...args: unknown[]) => invokeMock(...args),
}));

import ServersPage from "./ServersPage";
import "@/i18n";

function passwordServer(overrides: Partial<Server> = {}): Server {
  return {
    id: 1,
    host: "web-1.example.com",
    port: 22,
    username: "deploy",
    // 列表接口只返回凭据摘要，不含明文密码
    credential: { type: "password" },
    ...overrides,
  };
}

function keyServer(
  overrides: Partial<Omit<Server, "credential">> & {
    credential?: Server["credential"];
  } = {},
): Server {
  return {
    id: 2,
    host: "db-1.example.com",
    port: 2222,
    username: "root",
    credential: {
      type: "private_key",
      path: "/home/root/.ssh/id_ed25519",
      has_passphrase: false,
    },
    ...overrides,
  };
}

function expectInvoke(command: string) {
  const call = invokeMock.mock.calls.find(([c]) => c === command);
  expect(call, `expected invoke("${command}") to have been called`).toBeDefined();
  return call![1] as Record<string, unknown>;
}

beforeEach(() => {
  invokeMock.mockReset();
});

describe("服务器管理页", () => {
  it("加载并展示已有服务器列表（主机:端口 — 用户名）", async () => {
    invokeMock.mockResolvedValue([passwordServer(), keyServer()]);

    render(<ServersPage />);

    await waitFor(() => {
      expect(screen.getByText("web-1.example.com:22 — deploy")).toBeInTheDocument();
    });
    expect(screen.getByText("db-1.example.com:2222 — root")).toBeInTheDocument();
    expectInvoke("list_servers");
  });

  it("列表加载失败时展示具体错误", async () => {
    invokeMock.mockRejectedValue(new Error("db locked"));

    render(<ServersPage />);

    const alert = await screen.findByRole("alert");
    expect(alert).toHaveTextContent("加载服务器列表失败：db locked");
  });

  it("新建服务器：填写表单后保存，create_server 收到完整凭据", async () => {
    invokeMock.mockImplementation(async (command: string) => {
      if (command === "list_servers") return [];
      if (command === "create_server") return passwordServer();
      throw new Error(`unexpected command ${command}`);
    });

    render(<ServersPage />);
    await userEvent.click(await screen.findByRole("button", { name: "新建服务器" }));

    await userEvent.type(screen.getByLabelText("主机"), "web-1.example.com");
    await userEvent.clear(screen.getByLabelText("端口"));
    await userEvent.type(screen.getByLabelText("端口"), "22");
    await userEvent.type(screen.getByLabelText("用户名"), "deploy");
    await userEvent.type(screen.getByLabelText("密码"), "s3cret");
    await userEvent.click(screen.getByRole("button", { name: "保存" }));

    await waitFor(() => {
      expectInvoke("create_server");
    });
    const payload = expectInvoke("create_server");
    expect(payload).toEqual({
      host: "web-1.example.com",
      port: 22,
      username: "deploy",
      credential: { type: "password", password: "s3cret" },
    });
  });

  it("新建私钥凭据服务器：passphrase 为空时传 null", async () => {
    invokeMock.mockImplementation(async (command: string) => {
      if (command === "list_servers") return [];
      if (command === "create_server") return keyServer();
      throw new Error(`unexpected command ${command}`);
    });

    render(<ServersPage />);
    await userEvent.click(await screen.findByRole("button", { name: "新建服务器" }));

    await userEvent.type(screen.getByLabelText("主机"), "db-1.example.com");
    await userEvent.type(screen.getByLabelText("用户名"), "root");
    await userEvent.selectOptions(screen.getByLabelText("认证方式"), "private_key");
    await userEvent.type(screen.getByLabelText("私钥路径"), "/home/root/.ssh/id_ed25519");
    await userEvent.click(screen.getByRole("button", { name: "保存" }));

    await waitFor(() => {
      expectInvoke("create_server");
    });
    const payload = expectInvoke("create_server");
    expect(payload.credential).toEqual({
      type: "private_key",
      path: "/home/root/.ssh/id_ed25519",
      passphrase: null,
    });
  });

  it("编辑：凭据不回传明文，留空保存 = 保持现状（credential: null）", async () => {
    invokeMock.mockImplementation(async (command: string) => {
      if (command === "list_servers") return [passwordServer()];
      if (command === "update_server") return passwordServer();
      throw new Error(`unexpected command ${command}`);
    });

    render(<ServersPage />);
    await userEvent.click(await screen.findByRole("button", { name: "编辑" }));

    // 凭据字段留空，带"保持不变"占位与"已设置"指示
    expect(screen.getByLabelText("主机")).toHaveValue("web-1.example.com");
    expect(screen.getByLabelText("用户名")).toHaveValue("deploy");
    expect(screen.getByLabelText("密码")).toHaveValue("");
    expect(screen.getByLabelText("密码")).toHaveAttribute("placeholder", "留空保持不变");
    expect(screen.getByText("已设置密码")).toBeInTheDocument();

    await userEvent.clear(screen.getByLabelText("主机"));
    await userEvent.type(screen.getByLabelText("主机"), "web-9.example.com");
    await userEvent.click(screen.getByRole("button", { name: "保存" }));

    await waitFor(() => {
      expectInvoke("update_server");
    });
    const payload = expectInvoke("update_server");
    expect(payload.id).toBe(1);
    expect(payload.host).toBe("web-9.example.com");
    // 密码留空 = 保持现有密码，整体传 null
    expect(payload.credential).toBeNull();
  });

  it("编辑：输入新密码时按新值保存", async () => {
    invokeMock.mockImplementation(async (command: string) => {
      if (command === "list_servers") return [passwordServer()];
      if (command === "update_server") return passwordServer();
      throw new Error(`unexpected command ${command}`);
    });

    render(<ServersPage />);
    await userEvent.click(await screen.findByRole("button", { name: "编辑" }));

    await userEvent.type(screen.getByLabelText("密码"), "new-pass");
    await userEvent.click(screen.getByRole("button", { name: "保存" }));

    await waitFor(() => {
      expectInvoke("update_server");
    });
    const payload = expectInvoke("update_server");
    expect(payload.credential).toEqual({ type: "password", password: "new-pass" });
  });

  it("编辑：私钥口令已设置时留空保存 = 保持口令（省略字段）", async () => {
    invokeMock.mockImplementation(async (command: string) => {
      if (command === "list_servers") return [keyServer({ credential: { type: "private_key", path: "/home/root/.ssh/id_ed25519", has_passphrase: true } })];
      if (command === "update_server") return keyServer({ credential: { type: "private_key", path: "/home/root/.ssh/id_ed25519", has_passphrase: true } });
      throw new Error(`unexpected command ${command}`);
    });

    render(<ServersPage />);
    await userEvent.click(await screen.findByRole("button", { name: "编辑" }));

    // 私钥路径可见，口令留空且带"已设置"指示
    expect(screen.getByLabelText("私钥路径")).toHaveValue("/home/root/.ssh/id_ed25519");
    expect(screen.getByLabelText("私钥口令（可选）")).toHaveValue("");
    expect(screen.getByText("已设置私钥口令")).toBeInTheDocument();

    await userEvent.click(screen.getByRole("button", { name: "保存" }));

    await waitFor(() => {
      expectInvoke("update_server");
    });
    const payload = expectInvoke("update_server");
    // 口令字段整体缺席 = 保持现有口令（序列化后该字段被丢弃）
    expect(JSON.stringify(payload.credential)).not.toContain("passphrase");
    expect(payload.credential).toEqual({
      type: "private_key",
      path: "/home/root/.ssh/id_ed25519",
    });
  });

  it("编辑：私钥口令未设置时留空保存 = 显式无口令", async () => {
    invokeMock.mockImplementation(async (command: string) => {
      if (command === "list_servers") return [keyServer()];
      if (command === "update_server") return keyServer();
      throw new Error(`unexpected command ${command}`);
    });

    render(<ServersPage />);
    await userEvent.click(await screen.findByRole("button", { name: "编辑" }));
    await userEvent.click(screen.getByRole("button", { name: "保存" }));

    await waitFor(() => {
      expectInvoke("update_server");
    });
    const payload = expectInvoke("update_server");
    expect(payload.credential).toEqual({
      type: "private_key",
      path: "/home/root/.ssh/id_ed25519",
      passphrase: null,
    });
  });

  it("删除：确认后调用 delete_server 并刷新列表", async () => {
    invokeMock.mockImplementation(async (command: string) => {
      if (command === "list_servers") return [passwordServer()];
      if (command === "delete_server") return true;
      throw new Error(`unexpected command ${command}`);
    });

    render(<ServersPage />);
    await userEvent.click(await screen.findByRole("button", { name: "删除" }));

    // 确认对话框（Tauri WebView 不支持 window.confirm）出现后点确认
    await userEvent.click(await screen.findByTestId("confirm-dialog"));
    await userEvent.click(await screen.findByRole("button", { name: "确认" }));

    await waitFor(() => {
      expect(expectInvoke("delete_server").id).toBe(1);
    });
  });

  it("连接测试成功时展示成功", async () => {
    invokeMock.mockImplementation(async (command: string) => {
      if (command === "list_servers") return [passwordServer()];
      if (command === "test_saved_server_connection") {
        return { status: "success", new_fingerprint: null };
      }
      throw new Error(`unexpected command ${command}`);
    });

    render(<ServersPage />);
    await userEvent.click(
      (await screen.findAllByRole("button", { name: "连接测试" }))[0],
    );

    expect(await screen.findByText("连接成功")).toBeInTheDocument();
    // 列表行连接测试用存储凭据（后端侧），凭据不回传前端
    const payload = expectInvoke("test_saved_server_connection");
    expect(payload).toEqual({ id: 1, acceptUnknownHostKey: false });
  });

  it("密码被拒绝时展示认证失败原因", async () => {
    invokeMock.mockImplementation(async (command: string) => {
      if (command === "list_servers") return [passwordServer()];
      if (command === "test_saved_server_connection") {
        return { status: "auth_failed", method: "password" };
      }
      throw new Error(`unexpected command ${command}`);
    });

    render(<ServersPage />);
    await userEvent.click(
      (await screen.findAllByRole("button", { name: "连接测试" }))[0],
    );

    expect(
      await screen.findByText("认证失败：密码被服务器拒绝"),
    ).toBeInTheDocument();
  });

  it("私钥被拒绝时展示私钥认证失败原因", async () => {
    invokeMock.mockImplementation(async (command: string) => {
      if (command === "list_servers") return [keyServer()];
      if (command === "test_saved_server_connection") {
        return { status: "auth_failed", method: "private_key" };
      }
      throw new Error(`unexpected command ${command}`);
    });

    render(<ServersPage />);
    await userEvent.click(
      (await screen.findAllByRole("button", { name: "连接测试" }))[0],
    );

    expect(
      await screen.findByText("认证失败：私钥被服务器拒绝"),
    ).toBeInTheDocument();
  });

  it("无法连接时展示具体错误", async () => {
    invokeMock.mockImplementation(async (command: string) => {
      if (command === "list_servers") return [passwordServer()];
      if (command === "test_saved_server_connection") {
        return {
          status: "unreachable",
          detail: "Connection refused (os error 61)",
        };
      }
      throw new Error(`unexpected command ${command}`);
    });

    render(<ServersPage />);
    await userEvent.click(
      (await screen.findAllByRole("button", { name: "连接测试" }))[0],
    );

    expect(
      await screen.findByText(/无法连接服务器：Connection refused/),
    ).toBeInTheDocument();
  });

  it("首次连接未存储指纹：展示指纹，接受后携带 acceptUnknownHostKey 重试", async () => {
    const FINGERPRINT = "SHA256:AAAA1111aaaaBBBBccccDDDDeeeeFFFF00001111";
    let callCount = 0;
    invokeMock.mockImplementation(async (command: string) => {
      if (command === "list_servers") return [passwordServer()];
      if (command === "test_saved_server_connection") {
        callCount += 1;
        if (callCount === 1) {
          return { status: "host_key_unknown", fingerprint: FINGERPRINT };
        }
        return { status: "success", new_fingerprint: FINGERPRINT };
      }
      throw new Error(`unexpected command ${command}`);
    });

    render(<ServersPage />);
    await userEvent.click(
      (await screen.findAllByRole("button", { name: "连接测试" }))[0],
    );

    expect(
      await screen.findByText(/指纹尚未存储/),
    ).toBeInTheDocument();
    expect(screen.getByText(`指纹：${FINGERPRINT}`)).toBeInTheDocument();

    await userEvent.click(
      screen.getByRole("button", { name: "接受指纹并重试" }),
    );

    expect(await screen.findByText("连接成功")).toBeInTheDocument();
    const retryPayload = invokeMock.mock.calls.filter(
      ([c]) => c === "test_saved_server_connection",
    )[1]![1] as Record<string, unknown>;
    expect(retryPayload.acceptUnknownHostKey).toBe(true);
  });

  it("指纹变更：警告中间人风险，更新指纹后重试成功", async () => {
    const FINGERPRINT = "SHA256:BBBB2222bbbbCCCCddddEEEEffff000011112222";
    invokeMock.mockImplementation(async (command: string) => {
      if (command === "list_servers") return [passwordServer()];
      if (command === "test_saved_server_connection") {
        return { status: "host_key_changed", fingerprint: FINGERPRINT };
      }
      if (command === "accept_host_key") return null;
      throw new Error(`unexpected command ${command}`);
    });

    render(<ServersPage />);
    await userEvent.click(
      (await screen.findAllByRole("button", { name: "连接测试" }))[0],
    );

    expect(
      await screen.findByText(/可能存在中间人攻击/),
    ).toBeInTheDocument();

    await userEvent.click(
      screen.getByRole("button", { name: "更新指纹并重试" }),
    );

    expect(expectInvoke("accept_host_key")).toEqual({
      host: "web-1.example.com",
      port: 22,
      fingerprint: FINGERPRINT,
    });
  });

  it("私钥不可读时展示具体原因", async () => {
    invokeMock.mockImplementation(async (command: string) => {
      if (command === "list_servers") return [keyServer()];
      if (command === "test_saved_server_connection") {
        return {
          status: "key_unreadable",
          detail: "wrong passphrase",
        };
      }
      throw new Error(`unexpected command ${command}`);
    });

    render(<ServersPage />);
    await userEvent.click(
      (await screen.findAllByRole("button", { name: "连接测试" }))[0],
    );

    expect(
      await screen.findByText(/私钥无法读取：wrong passphrase/),
    ).toBeInTheDocument();
  });

  it("表单里的连接测试按当前表单值发起", async () => {
    invokeMock.mockImplementation(async (command: string) => {
      if (command === "list_servers") return [];
      if (command === "test_server_connection") {
        return { status: "success", new_fingerprint: null };
      }
      throw new Error(`unexpected command ${command}`);
    });

    render(<ServersPage />);
    await userEvent.click(await screen.findByRole("button", { name: "新建服务器" }));
    await userEvent.type(screen.getByLabelText("主机"), "web-1.example.com");
    await userEvent.type(screen.getByLabelText("用户名"), "deploy");
    await userEvent.type(screen.getByLabelText("密码"), "s3cret");

    await userEvent.click(screen.getByRole("button", { name: "连接测试" }));

    expect(await screen.findByText("连接成功")).toBeInTheDocument();
    const payload = expectInvoke("test_server_connection") as unknown as ServerInput;
    expect(payload.host).toBe("web-1.example.com");
    expect(payload.credential).toEqual({ type: "password", password: "s3cret" });
  });

  it("必填项未填时不允许保存", async () => {
    invokeMock.mockResolvedValue([]);
    render(<ServersPage />);
    await userEvent.click(await screen.findByRole("button", { name: "新建服务器" }));

    expect(screen.getByRole("button", { name: "保存" })).toBeDisabled();
    expect(screen.queryByRole("button", { name: "连接测试" })).not.toBeInTheDocument();

    await userEvent.type(screen.getByLabelText("主机"), "web-1");
    expect(screen.getByRole("button", { name: "保存" })).toBeDisabled();
  });

  it("编辑密码服务器且密码留空时，表单连接测试走存储凭据（test_saved_server_connection）", async () => {
    invokeMock.mockImplementation(async (command: string) => {
      if (command === "list_servers") return [passwordServer()];
      if (command === "test_saved_server_connection") {
        return { status: "success", new_fingerprint: null };
      }
      throw new Error(`unexpected command ${command}`);
    });

    render(<ServersPage />);
    await userEvent.click(await screen.findByRole("button", { name: "编辑" }));

    await userEvent.click(
      within(screen.getByTestId("server-form")).getByRole("button", {
        name: "连接测试",
      }),
    );

    expect(await screen.findByText("连接成功")).toBeInTheDocument();
    const payload = expectInvoke("test_saved_server_connection");
    expect(payload).toEqual({ id: 1, acceptUnknownHostKey: false });
  });
});

// Type-level guard: TestOutcome stays a closed union.
type AssertNever<T extends never> = T;
type _Closed = AssertNever<
  Exclude<
    TestOutcome["status"],
    | "success"
    | "host_key_unknown"
    | "host_key_changed"
    | "unreachable"
    | "auth_failed"
    | "key_unreadable"
  >
>;
void (undefined as unknown as _Closed | undefined);
