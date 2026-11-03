import { useCallback, useEffect, useState } from "react";
import { Channel } from "@tauri-apps/api/core";
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

function SettingsPage() {
  const { t } = useTranslation();
  const [rows, setRows] = useState<Rows | null>(null);
  const [loadError, setLoadError] = useState<string | null>(null);

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
      <h1 className="text-2xl font-semibold">{t("settings.title")}</h1>
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
        </CardContent>
      </Card>
      <RetentionCard />
    </section>
  );
}

export default SettingsPage;
