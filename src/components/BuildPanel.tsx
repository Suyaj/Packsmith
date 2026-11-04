import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { Channel } from "@tauri-apps/api/core";
import { Button } from "@/components/ui/button";
import FuzzyCombobox from "@/components/FuzzyCombobox";
import DeployDialog from "@/components/DeployDialog";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import {
  cancelBuildWorkspace,
  listRemoteBranches,
  readBuildLog,
  setDefaultArtifacts,
  startBuild,
  updateBuildCommand,
  type BuildEvent,
  type BuildOutcome,
  type CommitSummary,
  type Project,
} from "@/lib/buildApi";

type BackendError = { code: string; message: string };

function isBackendError(e: unknown): e is BackendError {
  return typeof e === "object" && e !== null && "code" in e;
}

/** 实时日志视图保留的最多行数（避免超长构建撑爆 DOM）。 */
const MAX_LOG_LINES = 500;

/** 提交时间（UNIX 秒）按本地时区格式化展示。 */
function formatCommitTime(time: number): string {
  return new Date(time * 1000).toLocaleString();
}

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
  const [commits, setCommits] = useState<CommitSummary[]>([]);
  const [outcome, setOutcome] = useState<BuildOutcome | null>(null);
  const [fullLog, setFullLog] = useState<string | null>(null);
  const [buildCommand, setBuildCommand] = useState(project.build_command);
  const [commandSaved, setCommandSaved] = useState(false);
  const [selectedArtifacts, setSelectedArtifacts] = useState<string[]>([]);
  const [defaultArtifactSaved, setDefaultArtifactSaved] = useState(false);
  const [deploying, setDeploying] = useState(false);

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
    } else if (msg.type === "commits") {
      // 所选分支最新提交（票 22）：随每次打包刷新，读取失败后端不发事件。
      setCommits(msg.commits);
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
    setCommits([]);
    setDefaultArtifactSaved(false);
    setSelectedArtifacts([]);
    setBusy(true);
    try {
      const onEvent = new Channel<BuildEvent>();
      onEvent.onmessage = handleEvent;
      const result = await startBuild(project.id, branch, onEvent);
      setOutcome(result);
      // 构件选择器（票 21）：预选项目默认构件列表中可用的项（依序保留在
      // 列表中的项）；无默认时预选第一个构件（单选行为与原来一致；单构件
      // 后端已自动选中并记住）。
      const names = result.artifacts.map((a) => a.file_name);
      const remembered = (project.default_artifacts ?? []).filter((n) => names.includes(n));
      setSelectedArtifacts(remembered.length > 0 ? remembered : (names[0] ? [names[0]] : []));
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

  /** 把勾选结果（按勾选顺序）保存为该项目默认构件列表（票 21）。 */
  async function handleSaveDefaultArtifact() {
    if (selectedArtifacts.length === 0) return;
    setError(null);
    setDefaultArtifactSaved(false);
    try {
      await setDefaultArtifacts(project.id, selectedArtifacts);
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
              {/* 可搜索 Combobox（票 14）：模糊过滤 + 命中高亮在
                  FuzzyCombobox 内实现。 */}
              <FuzzyCombobox
                items={branches}
                selected={branch ? [branch] : []}
                onSelect={setBranch}
                disabled={busy}
                ariaLabel={t("projects.build.branch")}
                searchPlaceholder={t("projects.build.searchBranch")}
                emptyText={t("projects.build.noMatchingBranches")}
                renderValue={() => branch || t("projects.build.selectBranch")}
                className="w-full"
              />
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

          {/* 最新提交（票 22）：克隆完成后展示所选分支最新 3 条，构建结束后
              保持可见（紧挨构建结果区），再次打包时随新事件刷新。 */}
          {commits.length > 0 && (
            <div>
              <p className="font-medium">{t("projects.build.commitsTitle")}</p>
              <ul data-testid="build-commits" className="flex flex-col gap-2 text-sm">
                {commits.map((c) => (
                  <li key={c.sha} className="flex flex-col">
                    <span className="flex flex-wrap items-baseline gap-x-2 text-xs text-muted-foreground">
                      <code className="font-mono text-foreground">{c.sha}</code>
                      <span>{c.author}</span>
                      <time>{formatCommitTime(c.time)}</time>
                    </span>
                    <span className="min-w-0 truncate" title={c.message}>
                      {c.message}
                    </span>
                  </li>
                ))}
              </ul>
            </div>
          )}

          {outcome && outcome.success && outcome.artifacts.length > 0 && (
            <fieldset>
              <legend className="text-sm font-medium">
                {t("projects.build.artifactPickerTitle")}
              </legend>
              {/* 可搜索多选 Combobox（票 21）：勾选后下拉保持展开，可继续
                  调整；模糊过滤 + 命中高亮在 FuzzyCombobox 内实现。 */}
              <FuzzyCombobox
                multiple
                items={outcome.artifacts.map((a) => a.file_name)}
                selected={selectedArtifacts}
                onSelect={(name) =>
                  setSelectedArtifacts((prev) =>
                    prev.includes(name)
                      ? prev.filter((n) => n !== name)
                      : [...prev, name],
                  )
                }
                disabled={busy}
                ariaLabel={t("projects.build.artifactPickerTitle")}
                searchPlaceholder={t("projects.build.searchArtifacts")}
                emptyText={t("projects.build.noMatchingArtifacts")}
                renderValue={(sel) =>
                  sel.length > 0 ? sel.join("、") : t("projects.build.selectArtifacts")
                }
                className="mt-1 w-full"
              />
              <div className="mt-2 flex items-center gap-2">
                <Button
                  variant="outline"
                  size="sm"
                  onClick={handleSaveDefaultArtifact}
                  disabled={busy || selectedArtifacts.length === 0}
                >
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
            <div className="flex gap-2">
              <Button variant="outline" size="sm" onClick={handleViewLog}>
                {t("projects.build.viewLog")}
              </Button>
              {outcome.success && outcome.history_id !== null && (
                <Button
                  size="sm"
                  data-testid="build-deploy"
                  onClick={() => setDeploying(true)}
                >
                  {t("deploy.submit")}
                </Button>
              )}
            </div>
          )}

          {deploying && outcome && outcome.history_id !== null && (
            <DeployDialog
              projectId={project.id}
              projectName={project.name}
              historyId={outcome.history_id}
              open
              onClose={() => setDeploying(false)}
              defaultArtifacts={project.default_artifacts ?? []}
            />
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
