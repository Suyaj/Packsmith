import { useCallback, useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import {
  acceptHostKey,
  createServer,
  deleteServer,
  listServers,
  testServerConnection,
  updateServer,
  type Credential,
  type Server,
  type ServerInput,
  type TestOutcome,
} from "@/lib/serversApi";
import { Button } from "@/components/ui/button";
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

function formToInput(form: FormState): ServerInput {
  const credential: Credential =
    form.authMethod === "password"
      ? { type: "password", password: form.password }
      : {
          type: "private_key",
          path: form.privateKeyPath,
          passphrase: form.privateKeyPassphrase === "" ? null : form.privateKeyPassphrase,
        };
  return {
    host: form.host.trim(),
    port: Number.parseInt(form.port, 10) || 22,
    username: form.username.trim(),
    credential,
  };
}

function serverToInput(server: Server): ServerInput {
  return {
    host: server.host,
    port: server.port,
    username: server.username,
    credential: server.credential,
  };
}

/// A stable key under which each test result is tracked (one per list row,
/// one for the unsaved form).
type TestKey = string;

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
  const [loadError, setLoadError] = useState<string | null>(null);
  const [form, setForm] = useState<FormState | null>(null);
  const [saveError, setSaveError] = useState<string | null>(null);
  const [testResults, setTestResults] = useState<
    Record<TestKey, TestOutcome | "loading">
  >({});

  const refresh = useCallback(async () => {
    try {
      const list = await listServers();
      setServers(list);
      setLoadError(null);
    } catch (e) {
      setLoadError(e instanceof Error ? e.message : String(e));
    }
  }, []);

  useEffect(() => {
    void refresh();
  }, [refresh]);

  const runTest = useCallback(
    async (key: TestKey, input: ServerInput, acceptUnknownHostKey: boolean) => {
      setTestResults((prev) => ({ ...prev, [key]: "loading" }));
      try {
        const outcome = await testServerConnection(input, acceptUnknownHostKey);
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
      input: ServerInput,
      fingerprint: string,
      updateStored: boolean,
    ) => {
      if (updateStored) {
        try {
          await acceptHostKey(input.host, input.port, fingerprint);
        } catch {
          // fall through to the retry, which will re-report the problem
        }
      }
      void runTest(key, input, !updateStored);
    },
    [runTest],
  );

  const openCreate = () => {
    setSaveError(null);
    setForm({ ...EMPTY_FORM });
  };

  const openEdit = (server: Server) => {
    setSaveError(null);
    setForm({
      editingId: server.id,
      host: server.host,
      port: String(server.port),
      username: server.username,
      authMethod: server.credential.type === "password" ? "password" : "private_key",
      password: server.credential.type === "password" ? server.credential.password : "",
      privateKeyPath:
        server.credential.type === "private_key" ? server.credential.path : "",
      privateKeyPassphrase:
        server.credential.type === "private_key" ? server.credential.passphrase ?? "" : "",
    });
  };

  const handleSave = async () => {
    if (!form) return;
    const input = formToInput(form);
    try {
      if (form.editingId === null) {
        await createServer(input);
      } else {
        await updateServer(form.editingId, input);
      }
      setForm(null);
      setSaveError(null);
      await refresh();
    } catch (e) {
      setSaveError(e instanceof Error ? e.message : String(e));
    }
  };

  const handleDelete = async (server: Server) => {
    if (!window.confirm(t("servers.confirmDelete", { host: server.host }))) return;
    await deleteServer(server.id);
    await refresh();
  };

  const inputValid = (f: FormState) =>
    f.host.trim() !== "" &&
    f.username.trim() !== "" &&
    (f.authMethod === "password"
      ? f.password !== ""
      : f.privateKeyPath.trim() !== "");

  const renderTestSection = (
    key: TestKey,
    input: ServerInput,
  ) => {
    const outcome = testResults[key];
    return (
      <div className="mt-2" data-testid={`test-section-${key}`}>
        <Button
          variant="outline"
          size="sm"
          disabled={outcome === "loading"}
          onClick={() => void runTest(key, input, false)}
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
                      void acceptAndRetry(key, input, outcome.fingerprint, false)
                    }
                  >
                    {t("servers.result.hostKeyUnknownAccept")}
                  </Button>
                ) : (
                  <Button
                    size="sm"
                    variant="destructive"
                    onClick={() =>
                      void acceptAndRetry(key, input, outcome.fingerprint, true)
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
                  <span>{t("servers.password")}</span>
                  <input
                    className="rounded border px-2 py-1"
                    aria-label={t("servers.password")}
                    type="password"
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
                    <span>{t("servers.privateKeyPassphrase")}</span>
                    <input
                      className="rounded border px-2 py-1"
                      aria-label={t("servers.privateKeyPassphrase")}
                      type="password"
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
              renderTestSection("form", formToInput(form))}
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
                <Button size="sm" variant="outline" onClick={() => void handleDelete(server)}>
                  {t("servers.delete")}
                </Button>
              </div>
              {renderTestSection(`server-${server.id}`, serverToInput(server))}
            </CardContent>
          </Card>
        ))}
      </div>
    </section>
  );
}

export default ServersPage;
