import { useCallback, useEffect, useState } from "react";
import { Channel } from "@tauri-apps/api/core";
import { getVersion } from "@tauri-apps/api/app";
import { useTranslation } from "react-i18next";
import { Button } from "@/components/ui/button";
import {
  Card,
  CardContent,
  CardDescription,
  CardHeader,
  CardTitle,
} from "@/components/ui/card";
import {
  downloadTool,
  fetchToolchainStatus,
  ProgressEvent,
  ToolKind,
  ToolStatus,
} from "@/lib/toolchain";
import { getBuildRetention, setBuildRetention } from "@/lib/historyApi";
import {
  fetchCloseWindowBehavior,
  normalizeCloseWindowBehavior,
  saveCloseWindowBehavior,
} from "@/lib/settings";
import {
  clearGitAccount,
  fetchGitAccount,
  saveGitAccount,
  type GitAccount,
} from "@/lib/gitAccountApi";
import {
  clearMvndSettingsXml,
  fetchMvndSettingsXml,
  saveMvndSettingsXml,
  type MvndSettingsXml,
} from "@/lib/mvndSettingsApi";

const TOOL_ORDER: ToolKind[] = ["git", "jdk", "mvnd"];

const TOOL_NAME_KEY: Record<ToolKind, string> = {
  git: "settings.toolchain.tools.git",
  jdk: "settings.toolchain.tools.jdk",
  mvnd: "settings.toolchain.tools.mvnd",
};

type RowUi = {
  status: ToolStatus;
  busy: boolean;
  progress: { phase: ProgressEvent["phase"]; percent: number | null } | null;
  error?: string;
};

type Rows = Record<ToolKind, RowUi>;

function rowsFromStatuses(statuses: ToolStatus[]): Rows {
  const byTool = new Map(statuses.map((s) => [s.tool, s]));
  return Object.fromEntries(
    TOOL_ORDER.map((tool) => [
      tool,
      {
        status: byTool.get(tool) ?? {
          tool,
          state: { state: "missing" },
          canDownload: false,
        },
        busy: false,
        progress: null,
      },
    ]),
  ) as Rows;
}

function StateBadge({ row }: { row: RowUi }) {
  const { t } = useTranslation();
  if (row.busy && row.progress) {
    const { phase, percent } = row.progress;
    const label =
      percent === null
        ? t(`settings.toolchain.phase.${phase}`)
        : `${t(`settings.toolchain.phase.${phase}`)} ${percent}%`;
    return <span className="text-sm text-muted-foreground">{label}</span>;
  }
  const state = row.status.state;
  if (state.state === "managed") {
    return (
      <span className="rounded-full bg-emerald-100 px-2 py-0.5 text-xs font-medium text-emerald-800 dark:bg-emerald-900/40 dark:text-emerald-300">
        {t("settings.toolchain.state.managed")}
      </span>
    );
  }
  if (state.state === "detected") {
    return (
      <span
        title={state.path}
        className="rounded-full bg-secondary px-2 py-0.5 text-xs font-medium text-secondary-foreground"
      >
        {t("settings.toolchain.state.detected")}
      </span>
    );
  }
  return (
    <span className="rounded-full bg-destructive/10 px-2 py-0.5 text-xs font-medium text-destructive">
      {t("settings.toolchain.state.missing")}
    </span>
  );
}

function ToolRow({
  tool,
  row,
  onDownload,
}: {
  tool: ToolKind;
  row: RowUi;
  onDownload: (tool: ToolKind) => void;
}) {
  const { t } = useTranslation();
  return (
    <li className="flex items-center justify-between gap-3 border-b py-3 last:border-b-0">
      <div className="min-w-0">
        <h3 className="text-sm font-medium">{t(TOOL_NAME_KEY[tool])}</h3>
        <p className="truncate text-xs text-muted-foreground">
          {row.status.state.state === "managed" &&
            t("settings.toolchain.managedVersion", {
              version: row.status.state.version,
            })}
          {row.status.state.state === "detected" && row.status.state.path}
          {row.status.state.state === "missing" && t("settings.toolchain.notOnPath")}
        </p>
        {row.error && (
          <p role="alert" className="mt-1 text-xs text-destructive">
            {row.error}
          </p>
        )}
      </div>
      <div className="flex shrink-0 items-center gap-2">
        <StateBadge row={row} />
        {row.status.canDownload && !row.busy && (
          <Button size="sm" variant="outline" onClick={() => onDownload(tool)}>
            {t(row.error ? "settings.toolchain.retry" : "settings.toolchain.download")}
          </Button>
        )}
        {row.busy && (
          <div
            role="progressbar"
            aria-label={t("settings.toolchain.download")}
            className="h-1.5 w-28 overflow-hidden rounded-full bg-muted"
          >
            <div
              className="h-full bg-primary transition-all"
              style={{
                width:
                  row.progress?.percent === null
                    ? "100%"
                    : `${row.progress?.percent ?? 0}%`,
              }}
            />
          </div>
        )}
      </div>
    </li>
  );
}

/**
 * 构件库保留策略（ticket 08）：每个项目在产物库中保留最近 N 次构建的构件
 * 与日志，超出在每次构建入库后自动清理。
 */
function RetentionCard() {
  const { t } = useTranslation();
  const [value, setValue] = useState<string>("");
  const [saved, setSaved] = useState(false);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    // 读取失败不阻塞页面：后端读取侧自带默认值回退。
    getBuildRetention()
      .then((n) => setValue(String(n)))
      .catch(() => {});
  }, []);

  async function handleSave() {
    setError(null);
    setSaved(false);
    const n = Number(value);
    if (!Number.isInteger(n) || n < 1) {
      setError(t("settings.retention.invalid"));
      return;
    }
    try {
      await setBuildRetention(n);
      setSaved(true);
    } catch (e) {
      setError(String(e));
    }
  }

  return (
    <Card className="mt-4">
      <CardHeader>
        <CardTitle>{t("settings.retention.title")}</CardTitle>
        <CardDescription>{t("settings.retention.description")}</CardDescription>
      </CardHeader>
      <CardContent>
        <div className="flex items-center gap-3">
          <label className="flex items-center gap-2 text-sm">
            {t("settings.retention.label")}
            <input
              type="number"
              min={1}
              aria-label={t("settings.retention.label")}
              value={value}
              onChange={(e) => {
                setValue(e.target.value);
                setSaved(false);
              }}
              className="w-24 rounded-md border bg-background px-2 py-1.5"
            />
          </label>
          <Button size="sm" variant="outline" onClick={handleSave}>
            {t("settings.retention.save")}
          </Button>
          {saved && (
            <span role="status" className="text-sm">
              {t("settings.retention.saved")}
            </span>
          )}
        </div>
        {error && (
          <p role="alert" className="mt-2 text-sm text-destructive">
            {error}
          </p>
        )}
      </CardContent>
    </Card>
  );
}

function CloseBehaviorCard() {
  const { t } = useTranslation();
  const [behavior, setBehavior] = useState<"minimize" | "quit">("minimize");
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    let disposed = false;
    fetchCloseWindowBehavior()
      .then((value) => {
        if (!disposed) setBehavior(normalizeCloseWindowBehavior(value));
      })
      .catch((e) => {
        if (!disposed) setError(String(e));
      });
    return () => {
      disposed = true;
    };
  }, []);

  const handleChange = async (value: "minimize" | "quit") => {
    const previous = behavior;
    setBehavior(value); // 乐观更新，失败回退
    setError(null);
    try {
      await saveCloseWindowBehavior(value);
    } catch (e) {
      setBehavior(previous);
      setError(String(e));
    }
  };

  return (
    <Card className="mt-4">
      <CardHeader>
        <CardTitle>{t("settings.closeBehavior.title")}</CardTitle>
        <CardDescription>
          {t("settings.closeBehavior.description")}
        </CardDescription>
      </CardHeader>
      <CardContent>
        <fieldset
          role="group"
          aria-label={t("settings.closeBehavior.title")}
          className="space-y-3"
        >
          <label className="flex cursor-pointer items-start gap-3">
            <input
              type="radio"
              name="close-window-behavior"
              aria-label={t("settings.closeBehavior.minimize")}
              checked={behavior === "minimize"}
              onChange={() => void handleChange("minimize")}
              className="mt-1"
            />
            <span>
              <span className="block text-sm font-medium">
                {t("settings.closeBehavior.minimize")}
              </span>
              <span className="block text-xs text-muted-foreground">
                {t("settings.closeBehavior.minimizeHint")}
              </span>
            </span>
          </label>
          <label className="flex cursor-pointer items-start gap-3">
            <input
              type="radio"
              name="close-window-behavior"
              aria-label={t("settings.closeBehavior.quit")}
              checked={behavior === "quit"}
              onChange={() => void handleChange("quit")}
              className="mt-1"
            />
            <span>
              <span className="block text-sm font-medium">
                {t("settings.closeBehavior.quit")}
              </span>
              <span className="block text-xs text-muted-foreground">
                {t("settings.closeBehavior.quitHint")}
              </span>
            </span>
          </label>
        </fieldset>
        {error && (
          <p role="alert" className="mt-2 text-xs text-destructive">
            {t("settings.closeBehavior.saveError", { error })}
          </p>
        )}
      </CardContent>
    </Card>
  );
}

/**
 * Git 账号（票 13）：clone / 分支拉取的 HTTPS 认证凭据，入加密库。
 * 掩码模式：用户名可展示，令牌只给"已设置"指示、明文永不过 IPC 回传；
 * 编辑令牌留空 = 保持不变，显式"清除账号"一起删用户名与令牌。
 */
function GitAccountCard() {
  const { t } = useTranslation();
  const [account, setAccount] = useState<GitAccount | null>(null);
  const [username, setUsername] = useState("");
  const [token, setToken] = useState("");
  const [saved, setSaved] = useState(false);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    let disposed = false;
    fetchGitAccount()
      .then((a) => {
        if (disposed) return;
        setAccount(a);
        setUsername(a.username ?? "");
      })
      .catch((e) => {
        if (!disposed) setError(String(e));
      });
    return () => {
      disposed = true;
    };
  }, []);

  const configured = account !== null && account.hasToken;

  async function handleSave() {
    setError(null);
    setSaved(false);
    if (username.trim() === "") {
      setError(t("settings.gitAccount.usernameRequired"));
      return;
    }
    try {
      // 令牌留空 = 保持不变（不传 token 字段，后端不覆盖）。
      const updated = await saveGitAccount(
        username,
        token === "" ? undefined : token,
      );
      setAccount(updated);
      setToken("");
      setSaved(true);
    } catch (e) {
      setError(extractErrorDetail(e) ?? String(e));
    }
  }

  async function handleClear() {
    setError(null);
    setSaved(false);
    try {
      const updated = await clearGitAccount();
      setAccount(updated);
      setUsername("");
      setToken("");
    } catch (e) {
      setError(extractErrorDetail(e) ?? String(e));
    }
  }

  return (
    <Card className="mt-4">
      <CardHeader>
        <CardTitle>{t("settings.gitAccount.title")}</CardTitle>
        <CardDescription>{t("settings.gitAccount.description")}</CardDescription>
      </CardHeader>
      <CardContent>
        <div className="flex items-center gap-3">
          {configured ? (
            <span className="rounded-full bg-emerald-100 px-2 py-0.5 text-xs font-medium text-emerald-800 dark:bg-emerald-900/40 dark:text-emerald-300">
              {t("settings.gitAccount.set")}
            </span>
          ) : (
            <span className="rounded-full bg-secondary px-2 py-0.5 text-xs font-medium text-secondary-foreground">
              {t("settings.gitAccount.unset")}
            </span>
          )}
        </div>
        <form
          className="mt-3 flex flex-col gap-3"
          onSubmit={(e) => {
            e.preventDefault();
            void handleSave();
          }}
        >
          <label className="flex items-center gap-2 text-sm">
            {t("settings.gitAccount.username")}
            <input
              type="text"
              aria-label={t("settings.gitAccount.username")}
              value={username}
              onChange={(e) => {
                setUsername(e.target.value);
                setSaved(false);
              }}
              className="w-56 rounded-md border bg-background px-2 py-1.5"
            />
          </label>
          <label className="flex items-center gap-2 text-sm">
            {t("settings.gitAccount.token")}
            <input
              type="password"
              aria-label={t("settings.gitAccount.token")}
              value={token}
              placeholder={
                configured ? t("settings.gitAccount.tokenKeepHint") : undefined
              }
              onChange={(e) => {
                setToken(e.target.value);
                setSaved(false);
              }}
              className="w-56 rounded-md border bg-background px-2 py-1.5"
            />
          </label>
          <div className="flex items-center gap-3">
            <Button size="sm" variant="outline" type="submit">
              {t("settings.gitAccount.save")}
            </Button>
            {configured && (
              <Button size="sm" variant="ghost" type="button" onClick={handleClear}>
                {t("settings.gitAccount.clear")}
              </Button>
            )}
            {saved && (
              <span role="status" className="text-sm">
                {t("settings.gitAccount.saved")}
              </span>
            )}
          </div>
        </form>
        {error && (
          <p role="alert" className="mt-2 text-sm text-destructive">
            {error}
          </p>
        )}
      </CardContent>
    </Card>
  );
}

/** 后端 ApiError 是 { code, message } 对象；取 message 作为展示细节。 */
function extractErrorDetail(e: unknown): string | null {
  if (typeof e === "object" && e !== null && "message" in e) {
    const m = (e as { message: unknown }).message;
    if (typeof m === "string") return m;
  }
  return null;
}

/**
 * mvnd settings.xml（票 18）：私服依赖的全局 Maven settings.xml。内容是配置
 * 而非机密，可明文回传做编辑回填；保存由后端写入加密库并物化到
 * `<appdata>/mvnd-settings.xml`（构建时自动追加 -s），前端从不直接写文件。
 */
function MvndSettingsCard() {
  const { t } = useTranslation();
  const [state, setState] = useState<MvndSettingsXml | null>(null);
  const [content, setContent] = useState("");
  const [saved, setSaved] = useState(false);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    let disposed = false;
    fetchMvndSettingsXml()
      .then((s) => {
        if (disposed) return;
        setState(s);
        setContent(s.content ?? "");
      })
      .catch((e) => {
        if (!disposed) setError(String(e));
      });
    return () => {
      disposed = true;
    };
  }, []);

  const configured = state !== null && state.content !== null;

  async function handleSave() {
    setError(null);
    setSaved(false);
    try {
      const updated = await saveMvndSettingsXml(content);
      setState(updated);
      setSaved(true);
    } catch (e) {
      setError(extractErrorDetail(e) ?? String(e));
    }
  }

  async function handleClear() {
    setError(null);
    setSaved(false);
    try {
      const updated = await clearMvndSettingsXml();
      setState(updated);
      setContent("");
    } catch (e) {
      setError(extractErrorDetail(e) ?? String(e));
    }
  }

  return (
    <Card className="mt-4">
      <CardHeader>
        <CardTitle>{t("settings.mvndSettings.title")}</CardTitle>
        <CardDescription>
          {t("settings.mvndSettings.description")}
        </CardDescription>
      </CardHeader>
      <CardContent>
        <div className="flex items-center gap-3">
          {configured ? (
            <span className="rounded-full bg-emerald-100 px-2 py-0.5 text-xs font-medium text-emerald-800 dark:bg-emerald-900/40 dark:text-emerald-300">
              {t("settings.mvndSettings.set")}
            </span>
          ) : (
            <span className="rounded-full bg-secondary px-2 py-0.5 text-xs font-medium text-secondary-foreground">
              {t("settings.mvndSettings.unset")}
            </span>
          )}
        </div>
        <label className="mt-3 block text-sm">
          {t("settings.mvndSettings.label")}
          <textarea
            rows={10}
            aria-label={t("settings.mvndSettings.label")}
            value={content}
            spellCheck={false}
            onChange={(e) => {
              setContent(e.target.value);
              setSaved(false);
            }}
            className="mt-1 w-full rounded-md border bg-background px-2 py-1.5 font-mono text-xs"
          />
        </label>
        <div className="mt-3 flex items-center gap-3">
          <Button size="sm" variant="outline" onClick={handleSave}>
            {t("settings.mvndSettings.save")}
          </Button>
          {configured && (
            <Button size="sm" variant="ghost" onClick={handleClear}>
              {t("settings.mvndSettings.clear")}
            </Button>
          )}
          {saved && (
            <span role="status" className="text-sm">
              {t("settings.mvndSettings.saved")}
            </span>
          )}
        </div>
        {error && (
          <p role="alert" className="mt-2 text-sm text-destructive">
            {t("settings.mvndSettings.saveError", { error })}
          </p>
        )}
      </CardContent>
    </Card>
  );
}

function SettingsPage() {
  const { t } = useTranslation();
  const [rows, setRows] = useState<Rows | null>(null);
  const [loadError, setLoadError] = useState<string | null>(null);
  const [appVersion, setAppVersion] = useState<string | null>(null);

  useEffect(() => {
    // 应用版本信息（spec：设置页展示 tauri 版本）。读取失败不阻塞页面。
    getVersion()
      .then(setAppVersion)
      .catch(() => {});
  }, []);

  const refresh = useCallback(async () => {
    try {
      const statuses = await fetchToolchainStatus();
      setRows(rowsFromStatuses(statuses));
      setLoadError(null);
    } catch (e) {
      setLoadError(String(e));
    }
  }, []);

  useEffect(() => {
    void refresh();
  }, [refresh]);

  const handleDownload = useCallback(async (tool: ToolKind) => {
    setRows((prev) => {
      if (!prev) return prev;
      return {
        ...prev,
        [tool]: { ...prev[tool], busy: true, progress: null, error: undefined },
      };
    });
    const channel = new Channel<ProgressEvent>();
    channel.onmessage = (event) => {
      setRows((prev) => {
        if (!prev) return prev;
        const percent =
          event.total && event.total > 0
            ? Math.min(100, Math.round((event.received / event.total) * 100))
            : null;
        return {
          ...prev,
          [event.tool]: {
            ...prev[event.tool],
            progress: { phase: event.phase, percent },
          },
        };
      });
    };
    try {
      const updated = await downloadTool(tool, channel);
      setRows((prev) => {
        if (!prev) return prev;
        return {
          ...prev,
          [tool]: { status: updated, busy: false, progress: null },
        };
      });
    } catch (e) {
      setRows((prev) => {
        if (!prev) return prev;
        return {
          ...prev,
          [tool]: {
            ...prev[tool],
            busy: false,
            progress: null,
            error: String(e),
          },
        };
      });
    }
  }, []);

  return (
    <section>
      <div className="flex items-baseline justify-between">
        <h1 className="text-2xl font-semibold">{t("settings.title")}</h1>
        {appVersion !== null && (
          <p className="text-sm text-muted-foreground" data-testid="app-version">
            {t("settings.appVersion", { version: appVersion })}
          </p>
        )}
      </div>
      <Card className="mt-4">
        <CardHeader>
          <CardTitle>{t("settings.toolchain.title")}</CardTitle>
          <CardDescription>{t("settings.toolchain.description")}</CardDescription>
        </CardHeader>
        <CardContent>
          {loadError ? (
            <p role="alert" className="text-sm text-destructive">
              {t("settings.toolchain.loadError", { error: loadError })}
            </p>
          ) : rows === null ? (
            <p className="text-sm text-muted-foreground">
              {t("settings.toolchain.loading")}
            </p>
          ) : (
            <ul>
              {TOOL_ORDER.map((tool) => (
                <ToolRow
                  key={tool}
                  tool={tool}
                  row={rows[tool]}
                  onDownload={handleDownload}
                />
              ))}
            </ul>
          )}
          <p className="mt-3 text-xs text-muted-foreground" data-testid="m2-repo-path">
            {t("settings.toolchain.dependencyRepo")}
          </p>
        </CardContent>
      </Card>
      <RetentionCard />
      <CloseBehaviorCard />
      <GitAccountCard />
      <MvndSettingsCard />
    </section>
  );
}

export default SettingsPage;
