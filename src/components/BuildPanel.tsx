import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { Channel } from "@tauri-apps/api/core";
import { Button } from "@/components/ui/button";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import {
  cancelBuildWorkspace,
  listRemoteBranches,
  startBuildWorkspace,
  type BuildProgress,
  type Project,
} from "@/lib/buildApi";

type BackendError = { code: string; message: string };

function isBackendError(e: unknown): e is BackendError {
  return typeof e === "object" && e !== null && "code" in e;
}

/**
 * 打包面板（ticket 06）：打开时实时拉取分支列表（默认选中上次使用的分支），
 * 确认后创建 Packsmith 临时工作区并 clone 所选分支。构建执行属于 ticket 07，
 * 本面板结束时只是"代码在临时工作区就绪"。
 */
function BuildPanel({ project, onClose }: { project: Project; onClose: () => void }) {
  const { t } = useTranslation();
  const [branches, setBranches] = useState<string[] | null>(null);
  const [branch, setBranch] = useState("");
  const [branchError, setBranchError] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [progress, setProgress] = useState<BuildProgress | null>(null);
  const [readyPath, setReadyPath] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);

  // 打开面板即实时拉取分支列表（远程新建分支可见）。
  useEffect(() => {
    let cancelled = false;
    listRemoteBranches(project.id)
      .then((list) => {
        if (cancelled) return;
        setBranches(list);
        // 默认选中上次使用的分支；没有则选列表第一个。
        setBranch(
          project.last_branch && list.includes(project.last_branch)
            ? project.last_branch
            : (list[0] ?? ""),
        );
      })
      .catch((err) => {
        if (cancelled) return;
        const code = isBackendError(err) ? err.code : "unknown";
        const message = isBackendError(err) ? err.message : String(err);
        setBranchError(t(`projects.errors.${code}`, { message }));
      });
    return () => {
      cancelled = true;
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [project.id, t]);

  /** 准备进行中：取消按钮改为请求后端取消当前任务。 */
  async function handleCancel() {
    if (busy) {
      await cancelBuildWorkspace(project.id).catch(() => {});
      return;
    }
    onClose();
  }

  async function handleStart() {
    setError(null);
    setReadyPath(null);
    setBusy(true);
    setProgress(null);
    try {
      const onProgress = new Channel<BuildProgress>();
      onProgress.onmessage = (msg) => setProgress(msg);
      const ws = await startBuildWorkspace(project.id, branch, onProgress);
      setReadyPath(ws.path);
    } catch (err) {
      const code = isBackendError(err) ? err.code : "unknown";
      const message = isBackendError(err) ? err.message : String(err);
      setError(t(`projects.errors.${code}`, { message }));
    } finally {
      setBusy(false);
      setProgress(null);
    }
  }

  return (
    <Card className="mt-4" data-testid={`build-panel-${project.name}`}>
      <CardHeader>
        <CardTitle>
          {t("projects.build.title", { name: project.name })}
        </CardTitle>
      </CardHeader>
      <CardContent>
        <div className="flex flex-col gap-3">
          {branchError && (
            <p role="alert" className="text-destructive">
              {branchError}
            </p>
          )}

          {branches === null && !branchError && (
            <p role="status" aria-live="polite">
              {t("projects.build.loadingBranches")}
            </p>
          )}

          {branches !== null && branches.length === 0 && !branchError && (
            <p>{t("projects.build.noBranches")}</p>
          )}

          {branches !== null && branches.length > 0 && (
            <label className="flex flex-col gap-1">
              {t("projects.build.branch")}
              <select
                value={branch}
                onChange={(e) => setBranch(e.target.value)}
                disabled={busy}
                className="rounded-md border bg-background px-2 py-1.5"
              >
                {branches.map((b) => (
                  <option key={b} value={b}>
                    {b}
                  </option>
                ))}
              </select>
            </label>
          )}

          {busy && (
            <p role="status" aria-live="polite">
              {progress?.total
                ? t("projects.build.startingProgress", {
                    branch,
                    received: progress.received ?? 0,
                    total: progress.total,
                  })
                : t("projects.build.starting")}
            </p>
          )}

          {readyPath && (
            <div>
              <p className="font-medium">{t("projects.build.ready")}</p>
              <p className="text-muted-foreground">
                {t("projects.build.readyPath", { path: readyPath })}
              </p>
            </div>
          )}

          {error && (
            <p role="alert" className="text-destructive">
              {error}
            </p>
          )}

          <div className="flex gap-2">
            <Button onClick={handleStart} disabled={busy || !branch}>
              {t("projects.build.start")}
            </Button>
            <Button type="button" variant="outline" onClick={handleCancel}>
              {busy ? t("projects.build.cancelWork") : t("projects.build.close")}
            </Button>
          </div>
        </div>
      </CardContent>
    </Card>
  );
}

export default BuildPanel;
