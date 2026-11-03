import { useCallback, useEffect, useState } from "react";
import { Link } from "react-router-dom";
import { useTranslation } from "react-i18next";
import { Button } from "@/components/ui/button";
import {
  Card,
  CardContent,
  CardHeader,
  CardTitle,
} from "@/components/ui/card";
import BuildPanel from "@/components/BuildPanel";
import { listProjects, type Project } from "@/lib/buildApi";
import { listBuildHistory, type HistoryEntry } from "@/lib/historyApi";
import {
  checkServerReachable,
  listServerLastDeploys,
  listServers,
  type Server,
  type ServerReachability,
} from "@/lib/serversApi";

/** 首页最近打包记录条数（spec: 首页总览最近打包记录列表）。 */
const RECENT_LIMIT = 5;

function formatTime(iso: string): string {
  const d = new Date(iso);
  return Number.isNaN(d.getTime()) ? iso : d.toLocaleString();
}

/** 服务器卡片的连接状态：探测中，或一次探测结果。 */
type ReachState = "checking" | ServerReachability;

/**
 * 首页（spec）：总览 + 快捷入口，无独有功能。
 * - 最近打包记录（项目/分支/时间/结果，最近在前），点击直达构建历史详情；
 * - 快速发起打包：项目快选后复用 ticket 06/07 的打包面板；
 * - 服务器概览卡片（连接状态、最近部署时间），完整管理在服务器页。
 */
function HomePage() {
  const { t } = useTranslation();
  const [entries, setEntries] = useState<HistoryEntry[] | null>(null);
  const [projects, setProjects] = useState<Project[] | null>(null);
  const [quickProjectId, setQuickProjectId] = useState<string>("");
  const [buildProject, setBuildProject] = useState<Project | null>(null);
  const [servers, setServers] = useState<Server[] | null>(null);
  const [lastDeploys, setLastDeploys] = useState<Record<number, string | null>>({});
  const [reach, setReach] = useState<Record<number, ReachState>>({});

  useEffect(() => {
    let cancelled = false;
    listBuildHistory(RECENT_LIMIT)
      .then((rows) => {
        if (!cancelled) setEntries(rows);
      })
      .catch(() => {
        if (!cancelled) setEntries([]);
      });
    listProjects()
      .then((rows) => {
        if (!cancelled) setProjects(rows);
      })
      .catch(() => {
        if (!cancelled) setProjects([]);
      });
    listServers()
      .then((rows) => {
        if (!cancelled) setServers(Array.isArray(rows) ? rows : []);
      })
      .catch(() => {
        if (!cancelled) setServers([]);
      });
    listServerLastDeploys()
      .then((rows) => {
        if (cancelled || !Array.isArray(rows)) return;
        setLastDeploys(Object.fromEntries(rows.map((r) => [r.serverId, r.lastDeployAt])));
      })
      .catch(() => {});
    return () => {
      cancelled = true;
    };
  }, []);

  /** 真实 TCP 可达性探测（页面加载时逐台触发，可手动重测）。 */
  const probe = useCallback((server: Server) => {
    setReach((prev) => ({ ...prev, [server.id]: "checking" }));
    checkServerReachable(server.host, server.port)
      .then((result) => {
        setReach((prev) => ({ ...prev, [server.id]: result }));
      })
      .catch((err) => {
        setReach((prev) => ({
          ...prev,
          [server.id]: {
            reachable: false,
            detail: err instanceof Error ? err.message : String(err),
          },
        }));
      });
  }, []);

  useEffect(() => {
    if (servers === null) return;
    for (const server of servers) probe(server);
  }, [servers, probe]);

  function startQuickBuild() {
    const project = (projects ?? []).find((p) => String(p.id) === quickProjectId) ?? null;
    setBuildProject(project);
  }

  return (
    <section>
      <h1 className="text-2xl font-semibold">{t("home.title")}</h1>

      <Card className="mt-4">
        <CardHeader className="flex flex-row items-center justify-between">
          <CardTitle>{t("home.recentBuilds")}</CardTitle>
          <Link to="/history" className="text-sm text-muted-foreground hover:text-foreground">
            {t("home.viewAll")}
          </Link>
        </CardHeader>
        <CardContent>
          {entries === null ? (
            <p className="text-sm text-muted-foreground">{t("history.loading")}</p>
          ) : entries.length === 0 ? (
            <p className="text-sm text-muted-foreground">{t("home.empty")}</p>
          ) : (
            <ul className="flex flex-col divide-y">
              {entries.map((entry) => (
                <li
                  key={entry.id}
                  data-testid="recent-build-row"
                  className="flex flex-wrap items-center gap-x-4 gap-y-1 py-2 text-sm"
                >
                  <Link
                    to={`/history?entry=${entry.id}`}
                    className="font-medium hover:underline"
                  >
                    {entry.project_name}
                  </Link>
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
                  <Link
                    to={`/history?entry=${entry.id}`}
                    className="ml-auto text-xs text-muted-foreground hover:text-foreground"
                  >
                    {t("home.viewDetail")}
                  </Link>
                </li>
              ))}
            </ul>
          )}
        </CardContent>
      </Card>

      <Card className="mt-4" data-testid="quick-build-card">
        <CardHeader>
          <CardTitle>{t("home.quickBuild")}</CardTitle>
        </CardHeader>
        <CardContent>
          {projects !== null && projects.length === 0 ? (
            <div className="flex items-center gap-3">
              <p className="text-sm text-muted-foreground">{t("home.quickBuildEmpty")}</p>
              <Link
                to="/projects"
                className="text-sm text-muted-foreground underline hover:text-foreground"
              >
                {t("home.quickBuildGoProjects")}
              </Link>
            </div>
          ) : (
            <div className="flex flex-wrap items-center gap-3">
              <label className="flex items-center gap-2 text-sm">
                {t("home.quickBuildPick")}
                <select
                  aria-label={t("home.quickBuildPick")}
                  value={quickProjectId}
                  onChange={(e) => setQuickProjectId(e.target.value)}
                  className="rounded-md border bg-background px-2 py-1.5"
                >
                  <option value="" disabled>
                    {t("home.quickBuildPick")}
                  </option>
                  {(projects ?? []).map((p) => (
                    <option key={p.id} value={p.id}>
                      {p.name}
                    </option>
                  ))}
                </select>
              </label>
              <Button
                size="sm"
                data-testid="quick-build-start"
                disabled={quickProjectId === ""}
                onClick={startQuickBuild}
              >
                {t("home.quickBuildStart")}
              </Button>
            </div>
          )}
          {buildProject && (
            <div className="mt-3">
              <BuildPanel
                project={buildProject}
                onClose={() => setBuildProject(null)}
              />
            </div>
          )}
        </CardContent>
      </Card>

      <Card className="mt-4" data-testid="servers-overview-card">
        <CardHeader className="flex flex-row items-center justify-between">
          <CardTitle>{t("home.serversOverview")}</CardTitle>
          <Link to="/servers" className="text-sm text-muted-foreground hover:text-foreground">
            {t("home.serversGoManage")}
          </Link>
        </CardHeader>
        <CardContent>
          {servers !== null && servers.length === 0 ? (
            <p className="text-sm text-muted-foreground">{t("home.serversEmpty")}</p>
          ) : (
            <div className="grid grid-cols-1 gap-3 md:grid-cols-2 xl:grid-cols-3">
              {(servers ?? []).map((server) => {
                const state = reach[server.id] ?? "checking";
                const lastDeployAt = lastDeploys[server.id] ?? null;
                return (
                  <div
                    key={server.id}
                    data-testid={`server-card-${server.id}`}
                    className="rounded-md border p-3"
                  >
                    <div className="flex items-center justify-between gap-2">
                      <p className="font-medium">
                        {server.host}:{server.port} — {server.username}
                      </p>
                      {state === "checking" ? (
                        <span
                          data-testid={`server-reach-${server.id}`}
                          role="status"
                          className="text-xs text-muted-foreground"
                        >
                          {t("home.serverChecking")}
                        </span>
                      ) : state.reachable ? (
                        <span
                          data-testid={`server-reach-${server.id}`}
                          className="rounded-full bg-emerald-100 px-2 py-0.5 text-xs font-medium text-emerald-800 dark:bg-emerald-900/40 dark:text-emerald-300"
                        >
                          {t("home.serverReachable")}
                        </span>
                      ) : (
                        <span
                          data-testid={`server-reach-${server.id}`}
                          title={state.detail ?? undefined}
                          className="rounded-full bg-destructive/10 px-2 py-0.5 text-xs font-medium text-destructive"
                        >
                          {t("home.serverUnreachable")}
                        </span>
                      )}
                    </div>
                    <p className="mt-1 text-sm text-muted-foreground">
                      {t("home.serverLastDeploy")}
                      {lastDeployAt === null
                        ? t("home.serverNeverDeployed")
                        : `：${formatTime(lastDeployAt)}`}
                    </p>
                    <Button
                      variant="outline"
                      size="sm"
                      className="mt-2"
                      onClick={() => probe(server)}
                    >
                      {t("home.serverCheck")}
                    </Button>
                  </div>
                );
              })}
            </div>
          )}
        </CardContent>
      </Card>
    </section>
  );
}

export default HomePage;
