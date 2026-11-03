import { useCallback, useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { Button } from "@/components/ui/button";
import {
  Card,
  CardContent,
  CardHeader,
  CardTitle,
} from "@/components/ui/card";
import {
  cleanupProjectHistory,
  deleteBuildHistory,
  listBuildArtifacts,
  listBuildHistory,
  listProjectBuildHistory,
  type ArtifactFile,
  type HistoryEntry,
} from "@/lib/historyApi";
import { listProjects, readBuildLog, type Project } from "@/lib/buildApi";
import DeployDialog from "@/components/DeployDialog";

type BackendError = { code: string; message: string };

function isBackendError(e: unknown): e is BackendError {
  return typeof e === "object" && e !== null && "code" in e;
}

function errorMessage(err: unknown): string {
  return isBackendError(err) ? err.message : String(err);
}

function formatTime(iso: string): string {
  const d = new Date(iso);
  return Number.isNaN(d.getTime()) ? iso : d.toLocaleString();
}

/**
 * 构建历史页（ticket 08）：跨项目的打包记录（项目/分支/时间/结果，最近在
 * 前），每条可查看持久化构建日志与入库构件；支持按项目筛选、删除单条记录
 * （构件 + 日志一并删除）与手动清理某项目的全部历史。保留次数 N 在设置页
 * 配置，自动清理在每次构建入库后执行。
 */
function HistoryPage() {
  const { t } = useTranslation();
  const [entries, setEntries] = useState<HistoryEntry[] | null>(null);
  const [projects, setProjects] = useState<Project[]>([]);
  const [projectFilter, setProjectFilter] = useState<string>("");
  const [error, setError] = useState<string | null>(null);
  const [openLogId, setOpenLogId] = useState<number | null>(null);
  const [logText, setLogText] = useState<string | null>(null);
  const [openArtifactsId, setOpenArtifactsId] = useState<number | null>(null);
  const [artifacts, setArtifacts] = useState<ArtifactFile[]>([]);
  const [deployEntry, setDeployEntry] = useState<HistoryEntry | null>(null);

  const load = useCallback(
    async (projectId: string) => {
      try {
        const rows =
          projectId === ""
            ? await listBuildHistory(null)
            : await listProjectBuildHistory(Number(projectId));
        setEntries(rows);
        setError(null);
      } catch (err) {
        setError(t("history.loadFailed", { detail: errorMessage(err) }));
      }
    },
    [t],
  );

  useEffect(() => {
    void load(projectFilter);
  }, [load, projectFilter]);

  useEffect(() => {
    listProjects()
      .then(setProjects)
      .catch(() => setProjects([]));
  }, []);

  async function handleViewLog(entry: HistoryEntry) {
    if (openLogId === entry.id) {
      setOpenLogId(null);
      setLogText(null);
      return;
    }
    try {
      setLogText(await readBuildLog(entry.build_id));
      setOpenLogId(entry.id);
    } catch (err) {
      setError(t("history.loadFailed", { detail: errorMessage(err) }));
    }
  }

  async function handleViewArtifacts(entry: HistoryEntry) {
    if (openArtifactsId === entry.id) {
      setOpenArtifactsId(null);
      setArtifacts([]);
      return;
    }
    try {
      setArtifacts(await listBuildArtifacts(entry.id));
      setOpenArtifactsId(entry.id);
    } catch (err) {
      setError(t("history.loadFailed", { detail: errorMessage(err) }));
    }
  }

  async function handleDelete(entry: HistoryEntry) {
    if (!window.confirm(t("history.confirmDelete"))) return;
    try {
      await deleteBuildHistory(entry.id);
      await load(projectFilter);
    } catch (err) {
      setError(t("history.loadFailed", { detail: errorMessage(err) }));
    }
  }

  async function handleCleanup() {
    if (projectFilter === "") return;
    if (!window.confirm(t("history.confirmCleanup"))) return;
    try {
      await cleanupProjectHistory(Number(projectFilter));
      await load(projectFilter);
    } catch (err) {
      setError(t("history.loadFailed", { detail: errorMessage(err) }));
    }
  }

  return (
    <section>
      <h1 className="text-2xl font-semibold">{t("history.title")}</h1>

      <div className="mt-4 flex items-center gap-3">
        <label className="flex items-center gap-2 text-sm">
          {t("history.filterProject")}
          <select
            aria-label={t("history.filterProject")}
            value={projectFilter}
            onChange={(e) => setProjectFilter(e.target.value)}
            className="rounded-md border bg-background px-2 py-1.5"
          >
            <option value="">{t("history.allProjects")}</option>
            {projects.map((p) => (
              <option key={p.id} value={p.id}>
                {p.name}
              </option>
            ))}
          </select>
        </label>
        {projectFilter !== "" && (
          <Button variant="outline" size="sm" onClick={handleCleanup}>
            {t("history.cleanup")}
          </Button>
        )}
      </div>

      {error && (
        <p role="alert" className="mt-3 text-sm text-destructive">
          {error}
        </p>
      )}

      {entries === null ? (
        <p className="mt-4 text-sm text-muted-foreground">{t("history.loading")}</p>
      ) : entries.length === 0 ? (
        <p className="mt-4 text-sm text-muted-foreground">{t("history.empty")}</p>
      ) : (
        <Card className="mt-4">
          <CardHeader>
            <CardTitle>{t("history.listTitle")}</CardTitle>
          </CardHeader>
          <CardContent>
            <ul className="flex flex-col divide-y">
              {entries.map((entry) => (
                <li key={entry.id} data-testid="history-row" className="py-3">
                  <div className="flex flex-wrap items-center gap-x-4 gap-y-1 text-sm">
                    <span className="font-medium">{entry.project_name}</span>
                    <span className="text-muted-foreground">{entry.branch}</span>
                    <span className="text-muted-foreground">
                      {formatTime(entry.created_at)}
                    </span>
                    {entry.success ? (
                      <span className="rounded-full bg-emerald-100 px-2 py-0.5 text-xs font-medium text-emerald-800 dark:bg-emerald-900/40 dark:text-emerald-300">
                        {t("history.resultSuccess")}
                      </span>
                    ) : (
                      <span className="rounded-full bg-destructive/10 px-2 py-0.5 text-xs font-medium text-destructive">
                        {t("history.resultFailed", { code: entry.exit_code ?? "-" })}
                      </span>
                    )}
                    <span className="ml-auto flex gap-2">
                      <Button variant="outline" size="sm" onClick={() => handleViewLog(entry)}>
                        {t("history.viewLog")}
                      </Button>
                      {entry.artifacts_dir !== null && (
                        <Button
                          variant="outline"
                          size="sm"
                          onClick={() => handleViewArtifacts(entry)}
                        >
                          {t("history.artifacts")}
                        </Button>
                      )}
                      {entry.success && entry.artifacts_dir !== null && (
                        <Button
                          size="sm"
                          data-testid={`deploy-history-${entry.id}`}
                          onClick={() => setDeployEntry(entry)}
                        >
                          {t("history.deploy")}
                        </Button>
                      )}
                      <Button variant="outline" size="sm" onClick={() => handleDelete(entry)}>
                        {t("history.delete")}
                      </Button>
                    </span>
                  </div>
                  {openLogId === entry.id && logText !== null && (
                    <pre
                      data-testid={`history-log-${entry.id}`}
                      className="mt-2 max-h-64 overflow-auto rounded-md border bg-background p-2 text-xs"
                    >
                      {logText}
                    </pre>
                  )}
                  {openArtifactsId === entry.id && (
                    <ul className="mt-2 flex flex-col gap-1 text-xs" data-testid={`history-artifacts-${entry.id}`}>
                      {artifacts.length === 0 ? (
                        <li className="text-muted-foreground">{t("history.artifactsEmpty")}</li>
                      ) : (
                        artifacts.map((f) => (
                          <li key={f.file_name} className="flex gap-3">
                            <span className="font-mono">{f.file_name}</span>
                            <span className="text-muted-foreground">
                              {t("history.artifactSize", { size: f.size })}
                            </span>
                          </li>
                        ))
                      )}
                    </ul>
                  )}
                </li>
              ))}
            </ul>
          </CardContent>
        </Card>
      )}

      {deployEntry && (
        <DeployDialog
          projectId={deployEntry.project_id}
          projectName={deployEntry.project_name}
          historyId={deployEntry.id}
          open
          onClose={() => setDeployEntry(null)}
        />
      )}
    </section>
  );
}

export default HistoryPage;
