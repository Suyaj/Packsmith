import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { Channel } from "@tauri-apps/api/core";
import { Button } from "@/components/ui/button";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import {
  cancelBuildWorkspace,
  listRemoteBranches,
  readBuildLog,
  setDefaultArtifact,
  startBuild,
  updateBuildCommand,
  type BuildEvent,
  type BuildOutcome,
  type Project,
} from "@/lib/buildApi";

type BackendError = { code: string; message: string };

function isBackendError(e: unknown): e is BackendError {
  return typeof e === "object" && e !== null && "code" in e;
}

/** 实时日志视图保留的最多行数（避免超长构建撑爆 DOM）。 */
const MAX_LOG_LINES = 500;

/**
 * 打包面板（ticket 06/07）：打开时实时拉取分支列表（默认选中上次使用的
 * 分支），构建命令随项目可编辑；确认后走完整打包流程——创建临时工作区并
 * clone 所选分支，再用托管工具链执行构建命令，结束后临时工作区自动清理，
 * 构建日志实时展示并持久化（失败后仍可读取）。
 */
function BuildPanel({ project, onClose }: { project: Project; onClose: () => void }) {
  const { t } = useTranslation();
  const [branches, setBranches] = useState<string[] | null>(null);
  const [branch, setBranch] = useState("");
  const [branchError, setBranchError] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const [cloneProgress, setCloneProgress] = useState<Extract<BuildEvent, { type: "clone" }> | null>(
    null,
  );
  const [logLines, setLogLines] = useState<string[]>([]);
  const [outcome, setOutcome] = useState<BuildOutcome | null>(null);
  const [fullLog, setFullLog] = useState<string | null>(null);
  const [buildCommand, setBuildCommand] = useState(project.build_command);
  const [commandSaved, setCommandSaved] = useState(false);
  const [selectedArtifact, setSelectedArtifact] = useState<string | null>(null);
  const [defaultArtifactSaved, setDefaultArtifactSaved] = useState(false);

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

  function handleEvent(msg: BuildEvent) {
    if (msg.type === "clone") {
      setCloneProgress(msg);
    } else {
      setLogLines((prev) => [...prev, msg.line].slice(-MAX_LOG_LINES));
    }
  }

  /** 打包进行中：取消按钮改为请求后端取消当前任务（克隆中断/子进程被杀）。 */
  async function handleCancel() {
    if (busy) {
      await cancelBuildWorkspace(project.id).catch(() => {});
      return;
    }
    onClose();
  }

  async function handleStart() {
    setError(null);
    setOutcome(null);
    setFullLog(null);
    setLogLines([]);
    setCloneProgress(null);
    setDefaultArtifactSaved(false);
    setSelectedArtifact(null);
    setBusy(true);
    try {
      const onEvent = new Channel<BuildEvent>();
      onEvent.onmessage = handleEvent;
      const result = await startBuild(project.id, branch, onEvent);
      setOutcome(result);
      // 多构件时弹出勾选：默认构件预选中（单构件后端已自动选中并记住）。
      if (result.artifacts.length > 1) {
        const names = result.artifacts.map((a) => a.file_name);
        const fallback = project.default_artifact;
        setSelectedArtifact(fallback && names.includes(fallback) ? fallback : names[0]);
      }
    } catch (err) {
      const code = isBackendError(err) ? err.code : "unknown";
      const message = isBackendError(err) ? err.message : String(err);
      setError(t(`projects.errors.${code}`, { message }));
    } finally {
      setBusy(false);
    }
  }

  async function handleSaveCommand() {
    setError(null);
    setCommandSaved(false);
    try {
      const updated = await updateBuildCommand(project.id, buildCommand.trim());
      setBuildCommand(updated.build_command);
      setCommandSaved(true);
    } catch (err) {
      const code = isBackendError(err) ? err.code : "unknown";
      const message = isBackendError(err) ? err.message : String(err);
      setError(t(`projects.errors.${code}`, { message }));
    }
  }

  /** 多构件构建：把勾选结果保存为该项目默认构件。 */
  async function handleSaveDefaultArtifact() {
    if (!selectedArtifact) return;
    setError(null);
    setDefaultArtifactSaved(false);
    try {
      await setDefaultArtifact(project.id, selectedArtifact);
      setDefaultArtifactSaved(true);
    } catch (err) {
      const code = isBackendError(err) ? err.code : "unknown";
      const message = isBackendError(err) ? err.message : String(err);
      setError(t(`projects.errors.${code}`, { message }));
    }
  }

  async function handleViewLog() {
    if (!outcome) return;
    try {
      setFullLog(await readBuildLog(outcome.build_id));
    } catch (err) {
      const code = isBackendError(err) ? err.code : "unknown";
      const message = isBackendError(err) ? err.message : String(err);
      setError(t(`projects.errors.${code}`, { message }));
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

          <div className="flex flex-col gap-1">
            <label className="flex flex-col gap-1">
              {t("projects.build.buildCommand")}
              <input
                value={buildCommand}
                onChange={(e) => {
                  setBuildCommand(e.target.value);
                  setCommandSaved(false);
                }}
                disabled={busy}
                className="rounded-md border bg-background px-2 py-1.5"
              />
            </label>
            <span className="text-muted-foreground text-xs">
              {t("projects.build.buildCommandHint")}
            </span>
          </div>

          <div>
            <Button variant="outline" size="sm" onClick={handleSaveCommand} disabled={busy}>
              {t("projects.build.saveCommand")}
            </Button>
            {commandSaved && (
              <span role="status" className="ml-2 text-sm">
                {t("projects.build.commandSaved")}
              </span>
            )}
          </div>

          {busy && (
            <p role="status" aria-live="polite">
              {logLines.length > 0
                ? t("projects.build.building")
                : cloneProgress?.total
                  ? t("projects.build.cloningProgress", {
                      received: cloneProgress.received ?? 0,
                      total: cloneProgress.total,
                    })
                  : t("projects.build.cloning")}
            </p>
          )}

          {outcome && (
            <div>
              {outcome.success ? (
                <p className="font-medium">{t("projects.build.buildSuccess")}</p>
              ) : (
                <p className="font-medium text-destructive">
                  {t("projects.build.buildFailed", { code: outcome.exit_code ?? "-" })}
                </p>
              )}
              {outcome.success && outcome.artifacts.length === 1 && (
                <p role="status" className="text-sm">
                  {t("projects.build.artifactAutoSelected", {
                    name: outcome.artifacts[0].file_name,
                  })}
                </p>
              )}
              <p className="text-muted-foreground text-xs">{outcome.log_path}</p>
            </div>
          )}

          {outcome && outcome.success && outcome.artifacts.length > 1 && (
            <fieldset>
              <legend className="text-sm font-medium">
                {t("projects.build.artifactPickerTitle")}
              </legend>
              <div role="radiogroup" aria-label={t("projects.build.artifactPickerTitle")} className="mt-1 flex flex-col gap-1">
                {outcome.artifacts.map((a) => (
                  <label key={a.relative_path} className="flex items-center gap-2 text-sm">
                    <input
                      type="radio"
                      name="default-artifact"
                      value={a.file_name}
                      checked={selectedArtifact === a.file_name}
                      onChange={() => setSelectedArtifact(a.file_name)}
                    />
                    {a.file_name}
                  </label>
                ))}
              </div>
              <div className="mt-2 flex items-center gap-2">
                <Button variant="outline" size="sm" onClick={handleSaveDefaultArtifact}>
                  {t("projects.build.saveDefaultArtifact")}
                </Button>
                {defaultArtifactSaved && (
                  <span role="status" className="text-sm">
                    {t("projects.build.defaultArtifactSaved")}
                  </span>
                )}
              </div>
            </fieldset>
          )}

          {logLines.length > 0 && (
            <div>
              <p className="font-medium">{t("projects.build.logTitle")}</p>
              <pre
                data-testid="build-live-log"
                className="max-h-64 overflow-auto rounded-md border bg-background p-2 text-xs"
              >
                {logLines.join("\n")}
              </pre>
            </div>
          )}

          {fullLog !== null && (
            <pre
              data-testid="build-full-log"
              className="max-h-64 overflow-auto rounded-md border bg-background p-2 text-xs"
            >
              {fullLog}
            </pre>
          )}

          {outcome && !busy && (
            <div>
              <Button variant="outline" size="sm" onClick={handleViewLog}>
                {t("projects.build.viewLog")}
              </Button>
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
