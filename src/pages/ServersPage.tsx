import { useCallback, useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import {
  acceptHostKey,
  createServer,
  deleteServer,
  listServers,
  testSavedServerConnection,
  testServerConnection,
  updateServer,
  type CredentialInput,
  type Server,
  type ServerInput,
  type ServerUpdateInput,
  type TestOutcome,
} from "@/lib/serversApi";
import { Button } from "@/components/ui/button";
import ConfirmDialog from "@/components/ConfirmDialog";
import {
  Card,
  CardContent,
  CardHeader,
  CardTitle,
} from "@/components/ui/card";

type AuthMethod = "password" | "private_key";

type FormState = {
  editingId: number | null;
  host: string;
  port: string;
  username: string;
  authMethod: AuthMethod;
  password: string;
  privateKeyPath: string;
  privateKeyPassphrase: string;
};

const EMPTY_FORM: FormState = {
  editingId: null,
  host: "",
  port: "22",
  username: "",
  authMethod: "password",
  password: "",
  privateKeyPath: "",
  privateKeyPassphrase: "",
};

function formToInput(
  form: FormState,
  editing: Server | null,
): ServerInput | ServerUpdateInput {
  const base = {
    host: form.host.trim(),
    port: Number.parseInt(form.port, 10) || 22,
    username: form.username.trim(),
  };
  if (form.authMethod === "password") {
    if (form.password === "") {
      // 编辑态密码留空 = 保持现有密码（后端整体保持凭据）。校验层保证只有
      // 原凭据就是密码时才可能走到这里。
      return { ...base, credential: null };
    }
    return { ...base, credential: { type: "password", password: form.password } };
  }
  // 私钥：路径可见可编辑；口令是机密——留空且原口令存在时省略字段
  // （undefined 序列化后被丢弃），即"保持现有口令"。
  const keepPassphrase =
    editing !== null &&
    editing.credential.type === "private_key" &&
    editing.credential.has_passphrase &&
    form.privateKeyPassphrase === "";
  const credential: CredentialInput = {
    type: "private_key",
    path: form.privateKeyPath,
    passphrase: keepPassphrase
      ? undefined
      : form.privateKeyPassphrase === ""
        ? null
        : form.privateKeyPassphrase,
  };
  return { ...base, credential };
}

/// A stable key under which each test result is tracked (one per list row,
/// one for the unsaved form).
type TestKey = string;

/**
 * 连接测试目标：表单值（新建 / 编辑中改了凭据）或已保存服务器（用后端
 * 存储的凭据测试——凭据不回传前端，编辑留空时只能（也应该）测存储值）。
 */
type TestTarget =
  | { kind: "form"; input: ServerInput }
  | { kind: "saved"; server: Server };

function outcomeText(
  outcome: TestOutcome,
  t: (key: string, opts?: Record<string, unknown>) => string,
): string {
  switch (outcome.status) {
    case "success":
      return t("servers.result.success");
    case "auth_failed":
      return outcome.method === "password"
        ? t("servers.result.authFailedPassword")
        : t("servers.result.authFailedPrivateKey");
    case "key_unreadable":
      return t("servers.result.keyUnreadable", { detail: outcome.detail });
    case "unreachable":
      return t("servers.result.unreachable", { detail: outcome.detail });
    case "host_key_unknown":
      return t("servers.result.hostKeyUnknown");
    case "host_key_changed":
      return t("servers.result.hostKeyChanged");
  }
}

function ServersPage() {
  const { t } = useTranslation();
  const [servers, setServers] = useState<Server[]>([]);
  // 待确认删除的服务器（Tauri WebView 不支持 window.confirm，改用对话框）
  const [confirmDelete, setConfirmDelete] = useState<Server | null>(null);
  const [loadError, setLoadError] = useState<string | null>(null);
  const [form, setForm] = useState<FormState | null>(null);
  const [editingServer, setEditingServer] = useState<Server | null>(null);
  const [saveError, setSaveError] = useState<string | null>(null);
  const [testResults, setTestResults] = useState<
    Record<TestKey, TestOutcome | "loading">
  >({});

  const refresh = useCallback(async () => {
    try {
      const list = await listServers();
      // Outside a real Tauri runtime (tests, plain browser) invoke resolves
      // null instead of rejecting — treat that as "no servers".
      setServers(Array.isArray(list) ? list : []);
      setLoadError(null);
    } catch (e) {
      setLoadError(e instanceof Error ? e.message : String(e));
    }
  }, []);

  useEffect(() => {
    void refresh();
  }, [refresh]);

  const runTest = useCallback(
    async (key: TestKey, target: TestTarget, acceptUnknownHostKey: boolean) => {
      setTestResults((prev) => ({ ...prev, [key]: "loading" }));
      try {
        const outcome =
          target.kind === "form"
            ? await testServerConnection(target.input, acceptUnknownHostKey)
            : await testSavedServerConnection(target.server.id, acceptUnknownHostKey);
        setTestResults((prev) => ({ ...prev, [key]: outcome }));
      } catch (e) {
        setTestResults((prev) => ({
          ...prev,
          [key]: {
            status: "unreachable",
            detail: e instanceof Error ? e.message : String(e),
          },
        }));
      }
    },
    [],
  );

  const acceptAndRetry = useCallback(
    async (
      key: TestKey,
      target: TestTarget,
      fingerprint: string,
      updateStored: boolean,
    ) => {
      if (updateStored) {
        const host = target.kind === "form" ? target.input.host : target.server.host;
        const port = target.kind === "form" ? target.input.port : target.server.port;
        try {
          await acceptHostKey(host, port, fingerprint);
        } catch {
          // fall through to the retry, which will re-report the problem
        }
      }
      void runTest(key, target, !updateStored);
    },
    [runTest],
  );

  const openCreate = () => {
    setSaveError(null);
    setEditingServer(null);
    setForm({ ...EMPTY_FORM });
  };

  const openEdit = (server: Server) => {
    setSaveError(null);
    setEditingServer(server);
    // 凭据不回传：密码 / 口令一律留空（留空 = 保持不变）；私钥路径不是
    // 机密，直接展示。
    setForm({
      editingId: server.id,
      host: server.host,
      port: String(server.port),
      username: server.username,
      authMethod: server.credential.type === "password" ? "password" : "private_key",
      password: "",
      privateKeyPath:
        server.credential.type === "private_key" ? server.credential.path : "",
      privateKeyPassphrase: "",
    });
  };

  const handleSave = async () => {
    if (!form) return;
    const input = formToInput(form, editingServer);
    try {
      if (form.editingId === null) {
        await createServer(input as ServerInput);
      } else {
        await updateServer(form.editingId, input as ServerUpdateInput);
      }
      setForm(null);
      setEditingServer(null);
      setSaveError(null);
      await refresh();
    } catch (e) {
      setSaveError(e instanceof Error ? e.message : String(e));
    }
  };

  const handleDelete = async (server: Server) => {
    setConfirmDelete(null);
    await deleteServer(server.id);
    await refresh();
  };

  // 密码仅在新建（或编辑时更换了新值）必填；编辑态留空 = 保持现有密码。
  const inputValid = (f: FormState) =>
    f.host.trim() !== "" &&
    f.username.trim() !== "" &&
    (f.authMethod === "password"
      ? f.password !== "" ||
        (editingServer !== null && editingServer.credential.type === "password")
      : f.privateKeyPath !== "");

  /**
   * 编辑表单的连接测试目标：机密字段留空（= 保持现状）时只能测已保存
   * 凭据（走后端）；用户输入了完整新凭据时按表单值测试。
   */
  const formTestTarget = (f: FormState): TestTarget => {
    if (editingServer !== null) {
      const keepsSecret =
        (f.authMethod === "password" && f.password === "") ||
        (f.authMethod === "private_key" && f.privateKeyPassphrase === "");
      if (keepsSecret) return { kind: "saved", server: editingServer };
    }
    return { kind: "form", input: formToInput(f, editingServer) as ServerInput };
  };

  const renderTestSection = (key: TestKey, target: TestTarget) => {
    const outcome = testResults[key];
    return (
      <div className="mt-2" data-testid={`test-section-${key}`}>
        <Button
          variant="outline"
          size="sm"
          disabled={outcome === "loading"}
          onClick={() => void runTest(key, target, false)}
        >
          {outcome === "loading" ? t("servers.testing") : t("servers.test")}
        </Button>
        {outcome && outcome !== "loading" && (
          <div role="status" className="mt-1 text-sm">
            <p
              className={
                outcome.status === "success"
                  ? "text-green-600 dark:text-green-400"
                  : "text-red-600 dark:text-red-400"
              }
            >
              {outcomeText(outcome, t)}
            </p>
            {(outcome.status === "host_key_unknown" ||
              outcome.status === "host_key_changed") && (
              <>
                <p className="break-all text-muted-foreground">
                  {t("servers.result.fingerprint", { fingerprint: outcome.fingerprint })}
                </p>
                {outcome.status === "host_key_unknown" ? (
                  <Button
                    size="sm"
                    onClick={() =>
                      void acceptAndRetry(key, target, outcome.fingerprint, false)
                    }
                  >
                    {t("servers.result.hostKeyUnknownAccept")}
                  </Button>
                ) : (
                  <Button
                    size="sm"
                    variant="destructive"
                    onClick={() =>
                      void acceptAndRetry(key, target, outcome.fingerprint, true)
                    }
                  >
                    {t("servers.result.hostKeyUpdate")}
                  </Button>
                )}
              </>
            )}
          </div>
        )}
      </div>
    );
  };

  return (
    <section>
      <div className="flex items-center justify-between">
        <h1 className="text-2xl font-semibold">{t("servers.title")}</h1>
        <Button onClick={openCreate}>{t("servers.add")}</Button>
      </div>

      {loadError && (
        <p role="alert" className="mt-2 text-sm text-red-600 dark:text-red-400">
          {t("servers.loadFailed", { detail: loadError })}
        </p>
      )}

      {form && (
        <Card className="mt-4" data-testid="server-form">
          <CardHeader>
            <CardTitle>
              {form.editingId === null ? t("servers.add") : t("servers.title")}
            </CardTitle>
          </CardHeader>
          <CardContent>
            <div className="grid grid-cols-1 gap-3 md:grid-cols-3">
              <label className="flex flex-col gap-1">
                <span>{t("servers.host")}</span>
                <input
                  className="rounded border px-2 py-1"
                  aria-label={t("servers.host")}
                  value={form.host}
                  onChange={(e) => setForm({ ...form, host: e.target.value })}
                />
              </label>
              <label className="flex flex-col gap-1">
                <span>{t("servers.port")}</span>
                <input
                  className="rounded border px-2 py-1"
                  aria-label={t("servers.port")}
                  type="number"
                  value={form.port}
                  onChange={(e) => setForm({ ...form, port: e.target.value })}
                />
              </label>
              <label className="flex flex-col gap-1">
                <span>{t("servers.username")}</span>
                <input
                  className="rounded border px-2 py-1"
                  aria-label={t("servers.username")}
                  value={form.username}
                  onChange={(e) => setForm({ ...form, username: e.target.value })}
                />
              </label>
              <label className="flex flex-col gap-1">
                <span>{t("servers.authMethod")}</span>
                <select
                  className="rounded border px-2 py-1"
                  aria-label={t("servers.authMethod")}
                  value={form.authMethod}
                  onChange={(e) =>
                    setForm({ ...form, authMethod: e.target.value as AuthMethod })
                  }
                >
                  <option value="password">{t("servers.authMethodPassword")}</option>
                  <option value="private_key">{t("servers.authMethodPrivateKey")}</option>
                </select>
              </label>
              {form.authMethod === "password" ? (
                <label className="flex flex-col gap-1 md:col-span-2">
                  <span>
                    {t("servers.password")}
                    {editingServer !== null &&
                      editingServer.credential.type === "password" && (
                        <span className="ml-2 text-xs text-muted-foreground">
                          {t("servers.credentialSetPassword")}
                        </span>
                      )}
                  </span>
                  <input
                    className="rounded border px-2 py-1"
                    aria-label={t("servers.password")}
                    type="password"
                    placeholder={
                      editingServer !== null &&
                      editingServer.credential.type === "password"
                        ? t("servers.keepUnchanged")
                        : undefined
                    }
                    value={form.password}
                    onChange={(e) => setForm({ ...form, password: e.target.value })}
                  />
                </label>
              ) : (
                <>
                  <label className="flex flex-col gap-1">
                    <span>{t("servers.privateKeyPath")}</span>
                    <input
                      className="rounded border px-2 py-1"
                      aria-label={t("servers.privateKeyPath")}
                      value={form.privateKeyPath}
                      onChange={(e) =>
                        setForm({ ...form, privateKeyPath: e.target.value })
                      }
                    />
                  </label>
                  <label className="flex flex-col gap-1">
                    <span>
                      {t("servers.privateKeyPassphrase")}
                      {editingServer !== null &&
                        editingServer.credential.type === "private_key" &&
                        editingServer.credential.has_passphrase && (
                          <span className="ml-2 text-xs text-muted-foreground">
                            {t("servers.credentialSetPassphrase")}
                          </span>
                        )}
                    </span>
                    <input
                      className="rounded border px-2 py-1"
                      aria-label={t("servers.privateKeyPassphrase")}
                      type="password"
                      placeholder={
                        editingServer !== null &&
                        editingServer.credential.type === "private_key" &&
                        editingServer.credential.has_passphrase
                          ? t("servers.keepUnchanged")
                          : undefined
                      }
                      value={form.privateKeyPassphrase}
                      onChange={(e) =>
                        setForm({ ...form, privateKeyPassphrase: e.target.value })
                      }
                    />
                  </label>
                </>
              )}
            </div>

            {saveError && (
              <p role="alert" className="mt-2 text-sm text-red-600 dark:text-red-400">
                {t("servers.saveFailed", { detail: saveError })}
              </p>
            )}

            <div className="mt-3 flex gap-2">
              <Button disabled={!inputValid(form)} onClick={() => void handleSave()}>
                {t("servers.save")}
              </Button>
              <Button variant="outline" onClick={() => setForm(null)}>
                {t("servers.cancel")}
              </Button>
            </div>

            {inputValid(form) &&
              renderTestSection("form", formTestTarget(form))}
          </CardContent>
        </Card>
      )}

      <div className="mt-4 space-y-3" data-testid="server-list">
        {servers.length === 0 && !form && (
          <Card>
            <CardHeader>
              <CardTitle>{t("servers.empty")}</CardTitle>
            </CardHeader>
            <CardContent />
          </Card>
        )}
        {servers.map((server) => (
          <Card key={server.id} data-testid={`server-row-${server.id}`}>
            <CardHeader>
              <CardTitle className="text-base">
                {server.host}:{server.port} — {server.username}
              </CardTitle>
            </CardHeader>
            <CardContent>
              <div className="flex flex-wrap items-center gap-2">
                <Button size="sm" variant="outline" onClick={() => openEdit(server)}>
                  {t("servers.edit")}
                </Button>
                <Button size="sm" variant="outline" onClick={() => setConfirmDelete(server)}>
                  {t("servers.delete")}
                </Button>
              </div>
              {/* 已保存服务器用存储凭据测试：凭据不出后端。 */}
              {renderTestSection(`server-${server.id}`, {
                kind: "saved",
                server,
              })}
            </CardContent>
          </Card>
        ))}
      </div>
      <ConfirmDialog
        open={confirmDelete !== null}
        onOpenChange={(open) => {
          if (!open) setConfirmDelete(null);
        }}
        title={t("servers.confirmDelete", { host: confirmDelete?.host ?? "" })}
        confirmLabel={t("common.confirm")}
        cancelLabel={t("common.cancel")}
        onConfirm={() => {
          if (confirmDelete !== null) void handleDelete(confirmDelete);
        }}
      />
    </section>
  );
}

export default ServersPage;
