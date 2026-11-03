import { useEffect, useState } from "react";
import { Link } from "react-router-dom";
import { useTranslation } from "react-i18next";
import {
  Card,
  CardContent,
  CardHeader,
  CardTitle,
} from "@/components/ui/card";
import { listBuildHistory, type HistoryEntry } from "@/lib/historyApi";

/** 首页最近打包记录条数（spec: 首页总览最近打包记录列表）。 */
const RECENT_LIMIT = 5;

function formatTime(iso: string): string {
  const d = new Date(iso);
  return Number.isNaN(d.getTime()) ? iso : d.toLocaleString();
}

/**
 * 首页（spec）：总览 + 快捷入口。展示最近打包记录（项目/分支/时间/结果，
 * 最近在前），完整历史见构建历史页。
 */
function HomePage() {
  const { t } = useTranslation();
  const [entries, setEntries] = useState<HistoryEntry[] | null>(null);

  useEffect(() => {
    let cancelled = false;
    listBuildHistory(RECENT_LIMIT)
      .then((rows) => {
        if (!cancelled) setEntries(rows);
      })
      .catch(() => {
        if (!cancelled) setEntries([]);
      });
    return () => {
      cancelled = true;
    };
  }, []);

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
    </section>
  );
}

export default HomePage;
