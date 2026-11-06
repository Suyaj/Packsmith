// 部署对话框（ticket 10 / 23 / 26）：自由勾选上传计划——每个部署目标展示
// 本次构建的全部构件，可勾选/取消任意构件。映射行降级为预设（票 26）：具名
// 映射行对应的构件默认勾选并沿用其行目录；其他构件默认不勾选，上传到目标
// 默认上传目录。空名行不产生预设、不再单独展示（票 24/25 的补选/批量补选
// UI 随之移除——预设由映射行驱动）。payload 为每目标
// entries: [{fileName, uploadDir}]。对话框级操作选择（重启/仅上传/仅停止/
// 仅启动，票 23）：仅停止/仅启动时上传计划区只读展示、payload 只带目标；
// 依次部署；展示各目标阶段状态、跳过提示与输出快照；重启时停止失败中止
// 重启并提供"单独执行启动命令组"；部署进行中可取消。

import { useCallback, useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { Channel } from "@tauri-apps/api/core";
import { Button } from "@/components/ui/button";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import {
  cancelDeploy,
  runStartGroup,
  startDeploy,
  type AutoDeployMessage,
  type DeployOperation,
  type DeployPhase,
  type TargetAssignment,
  type TargetStatus,
} from "@/lib/deployApi";
import { listBuildArtifacts, type ArtifactFile } from "@/lib/historyApi";
import { listServers } from "@/lib/serversApi";
import { listDeploymentTargets, type Target } from "@/lib/targetsApi";
import { serverLabel } from "./DeploymentTargetsPanel";
import { HighlightedText } from "./FuzzyCombobox";
import { filterBranches } from "@/lib/branchFilter";

/** 每个目标的实时展示状态：状态 + 各阶段输出快照 + 上传进度 + 跳过提示。 */
type TargetView = {
  status: TargetStatus;
  outputs: Partial<Record<DeployPhase, { output: string; exit_code: number }>>;
  stopFailed: boolean;
  error: string | null;
  progress: { transferred: number; total: number } | null;
  notices: string[];
};

const INITIAL_VIEW: TargetView = {
  status: "pending",
  outputs: {},
  stopFailed: false,
  error: null,
  progress: null,
  notices: [],
};

function backendMessage(e: unknown): string {
  if (typeof e === "object" && e !== null && "message" in e) {
    return String((e as { message: unknown }).message);
  }
  return String(e);
}

/** 后端错误 → 中文文案：按稳定 code 选 i18n，无对应文案时回退原始 detail。 */
function startErrorMessage(e: unknown, t: (key: string, opts?: Record<string, unknown>) => string): string {
  const detail = backendMessage(e);
  const code =
    typeof e === "object" && e !== null && "code" in e
      ? String((e as { code: unknown }).code)
      : null;
  return code
    ? t(`deploy.errors.${code}`, { detail, defaultValue: t("deploy.deployFailed", { detail }) })
    : t("deploy.deployFailed", { detail });
}

function DeployDialog({
  projectId,
  projectName,
  historyId,
  open,
  onClose,
}: {
  projectId: number;
  projectName: string;
  historyId: number;
  open: boolean;
  onClose: () => void;
}) {
  const { t } = useTranslation();
  const [targets, setTargets] = useState<Target[] | null>(null);
  const [serverLabels, setServerLabels] = useState<Map<number, string>>(new Map());
  const [artifacts, setArtifacts] = useState<ArtifactFile[] | null>(null);
  const [selectedIds, setSelectedIds] = useState<number[]>([]);
  // 每个目标勾选的构件文件名集合（票 26 自由勾选；具名映射行作预设）。
  const [entryState, setEntryState] = useState<Map<number, Set<string>>>(new Map());
  const [views, setViews] = useState<Map<number, TargetView>>(new Map());
  // 本次操作（票 23，对话框级）：重启（默认）/ 仅上传 / 仅停止 / 仅启动。
  const [operation, setOperation] = useState<DeployOperation>("restart");
  const [running, setRunning] = useState(false);
  const [loadError, setLoadError] = useState<string | null>(null);
  const [startError, setStartError] = useState<string | null>(null);
  // 构件列表模糊搜索关键字（票 26 自由勾选列表的过滤，复用分支选择器匹配）
  const [entryQuery, setEntryQuery] = useState("");
  const deployIdRef = useRef<number | null>(null);
  const dialogRef = useRef<HTMLDivElement | null>(null);

  const refresh = useCallback(async () => {
    if (!open) return;
    try {
      const [targetList, servers, artifactList] = await Promise.all([
        listDeploymentTargets(projectId),
        listServers(),
        listBuildArtifacts(historyId),
      ]);
      setTargets(targetList);
      setServerLabels(new Map(servers.map((s) => [s.id, serverLabel(s)])));
      setArtifacts(artifactList);
      const names = artifactList.map((a) => a.file_name);
      // 预设（票 26）：具名映射行对应的构件（须在本次构建产物里）默认勾选；
      // 空名行不产生预设；其余构件默认不勾选（目标默认上传目录兜底）。
      const presets = new Map<number, Set<string>>();
      for (const target of targetList) {
        const preset = new Set<string>();
        for (const row of target.files) {
          const name = row.fileName.trim();
          if (name !== "" && names.includes(name)) preset.add(name);
        }
        presets.set(target.id, preset);
      }
      setEntryState(presets);
      setSelectedIds([]);
      setOperation("restart");
      setEntryQuery("");
      setViews(new Map(targetList.map((target) => [target.id, { ...INITIAL_VIEW, outputs: {}, notices: [] }])));
      setLoadError(null);
    } catch (e) {
      setLoadError(t("deploy.loadFailed", { detail: backendMessage(e) }));
    }
  }, [open, projectId, historyId, t]);

  useEffect(() => {
    void refresh();
  }, [refresh]);

  // 对话框是内联卡片，挂在打包面板/历史列表的末尾：默认 800×600 窗口里通常
  // 位于折叠线以下，不滚动进视口用户看到的就是「点击部署没反应」。
  useEffect(() => {
    if (open) dialogRef.current?.scrollIntoView({ behavior: "smooth", block: "nearest" });
  }, [open]);

  if (!open) return null;

  const patchView = (targetId: number, patch: Partial<TargetView>) => {
    setViews((prev) => {
      const next = new Map(prev);
      const current = next.get(targetId) ?? { ...INITIAL_VIEW, outputs: {}, notices: [] };
      next.set(targetId, { ...current, ...patch });
      return next;
    });
  };

  const handleMessage = (message: AutoDeployMessage) => {
    deployIdRef.current = message.deployId;
    const event = message.event;
    switch (event.type) {
      case "phase":
        patchView(event.target_id, {
          status:
            event.phase === "upload"
              ? "uploading"
              : event.phase === "stop"
                ? "stopping"
                : "starting",
        });
        break;
      case "upload_progress":
        patchView(event.target_id, {
          progress: { transferred: event.transferred, total: event.total },
        });
        break;
      case "output":
        setViews((prev) => {
          const next = new Map(prev);
          const current = next.get(event.target_id) ?? { ...INITIAL_VIEW, outputs: {}, notices: [] };
          next.set(event.target_id, {
            ...current,
            outputs: {
              ...current.outputs,
              [event.phase]: { output: event.output, exit_code: event.exit_code },
            },
          });
          return next;
        });
        break;
      case "skipped":
        // 跳过提示不改目标状态，仅通知。
        setViews((prev) => {
          const next = new Map(prev);
          const current = next.get(event.target_id) ?? { ...INITIAL_VIEW, outputs: {}, notices: [] };
          next.set(event.target_id, { ...current, notices: [...current.notices, event.message] });
          return next;
        });
        break;
      case "target_finished":
        patchView(event.target_id, {
          status: event.status,
          stopFailed: event.stop_failed,
          error: event.error,
        });
        break;
      case "finished":
        setRunning(false);
        break;
    }
  };

  /** 本次操作是否传文件（重启/仅上传）：仅停止/仅启动只执行命令组（票 23）。 */
  const uploadsFiles = operation === "restart" || operation === "upload";

  const names = (artifacts ?? []).map((a) => a.file_name);

  /** 一个构件在该目标的上传目录：具名映射行预设 = 行目录；自由条目 = 目标
   * 默认上传目录（票 26）。 */
  const dirFor = (target: Target, fileName: string): string =>
    target.files.find((row) => row.fileName.trim() === fileName)?.uploadDir ??
    target.uploadDir;

  const toggleEntry = (targetId: number, fileName: string) => {
    setEntryState((prev) => {
      const next = new Map(prev);
      const checked = new Set(next.get(targetId) ?? []);
      if (checked.has(fileName)) {
        checked.delete(fileName);
      } else {
        checked.add(fileName);
      }
      next.set(targetId, checked);
      return next;
    });
  };

  const handleStart = async () => {
    if (artifacts === null) return;
    let assignments: TargetAssignment[];
    if (uploadsFiles) {
      // 上传集合 = 勾选集合（票 26）：预设条目目录 = 行目录，自由条目目录 =
      // 目标默认上传目录。
      assignments = (targets ?? [])
        .filter((target) => selectedIds.includes(target.id))
        .map((target) => ({
          targetId: target.id,
          entries: names
            .filter((name) => entryState.get(target.id)?.has(name))
            .map((fileName) => ({ fileName, uploadDir: dirFor(target, fileName) })),
        }))
        .filter((a) => a.entries.length > 0);
      if (assignments.length === 0) return;
    } else {
      // 仅停止/仅启动（票 23）：不传文件，payload 只带选中的目标。
      assignments = (targets ?? [])
        .filter((target) => selectedIds.includes(target.id))
        .map((target): TargetAssignment => ({ targetId: target.id, entries: [] }));
      if (assignments.length === 0) return;
    }
    setStartError(null);
    setRunning(true);
    const channel = new Channel<AutoDeployMessage>();
    channel.onmessage = handleMessage;
    try {
      await startDeploy(historyId, operation, assignments, channel);
    } catch (e) {
      setStartError(startErrorMessage(e, t));
    } finally {
      setRunning(false);
    }
  };

  const handleCancel = async () => {
    if (deployIdRef.current !== null) {
      await cancelDeploy(deployIdRef.current);
    }
  };

  const handleManualStart = async (targetId: number) => {
    setStartError(null);
    const channel = new Channel<AutoDeployMessage>();
    channel.onmessage = handleMessage;
    try {
      await runStartGroup(targetId, channel);
    } catch (e) {
      setStartError(t("deploy.deployFailed", { detail: backendMessage(e) }));
    }
  };

  const statusText = (status: TargetStatus) => t(`deploy.status.${status}`);

  /** 选中目标里至少有一个勾选构件，才允许发起上传类部署（票 26）。 */
  const hasCheckedEntries = (targets ?? [])
    .filter((target) => selectedIds.includes(target.id))
    .some((target) => names.some((name) => entryState.get(target.id)?.has(name)));

  return (
    <Card ref={dialogRef} data-testid="deploy-dialog" className="mt-4">
      <CardHeader className="flex flex-row items-center justify-between space-y-0">
        <CardTitle>{t("deploy.title", { project: projectName })}</CardTitle>
        <Button
          variant="outline"
          size="sm"
          data-testid="deploy-close"
          onClick={onClose}
          disabled={running}
        >
          {t("deploy.close")}
        </Button>
      </CardHeader>
      <CardContent className="flex flex-col gap-4">
        {loadError && (
          <p role="alert" className="text-sm text-destructive">
            {loadError}
          </p>
        )}

        {artifacts !== null && artifacts.length === 0 && (
          <p className="text-sm text-muted-foreground">{t("deploy.noArtifacts")}</p>
        )}

        <fieldset>
          <legend className="text-sm font-medium">{t("deploy.operationLabel")}</legend>
          <select
            data-testid="deploy-operation"
            value={operation}
            disabled={running}
            aria-label={t("deploy.operationLabel")}
            className="mt-1 rounded-md border bg-background px-2 py-1 text-sm"
            onChange={(e) => setOperation(e.target.value as DeployOperation)}
          >
            {(["restart", "upload", "stop", "start"] as DeployOperation[]).map((op) => (
              <option key={op} value={op}>
                {t(`deploy.operations.${op}`)}
              </option>
            ))}
          </select>
        </fieldset>

        <fieldset>
          <legend className="text-sm font-medium">{t("deploy.targetTitle")}</legend>
          {!uploadsFiles && (
            <p className="text-xs text-muted-foreground">{t("deploy.planReadonlyHint")}</p>
          )}
          {uploadsFiles && (
            <p className="text-xs text-muted-foreground">{t("deploy.plan.freeHint")}</p>
          )}
          {uploadsFiles && names.length > 0 && (
            <input
              type="search"
              data-testid="deploy-entry-search"
              aria-label={t("deploy.plan.searchArtifacts")}
              placeholder={t("deploy.plan.searchArtifacts")}
              value={entryQuery}
              onChange={(e) => setEntryQuery(e.target.value)}
              className="mt-1 w-56 rounded-md border bg-background px-2 py-1 text-sm"
            />
          )}
          {targets !== null && targets.length === 0 && (
            <p className="text-sm text-muted-foreground">{t("deploy.noTargets")}</p>
          )}
          <div className="mt-1 flex flex-col gap-2">
            {(targets ?? []).map((target) => (
              <div key={target.id} className="flex flex-col gap-1 rounded-md border p-2">
                <label className="flex items-center gap-2 text-sm">
                  <input
                    type="checkbox"
                    data-testid={`deploy-target-${target.id}`}
                    checked={selectedIds.includes(target.id)}
                    disabled={running}
                    onChange={() =>
                      setSelectedIds((prev) =>
                        prev.includes(target.id)
                          ? prev.filter((id) => id !== target.id)
                          : [...prev, target.id],
                      )
                    }
                  />
                  <span>
                    {serverLabels.get(target.serverId) ??
                      t("targets.serverMissing", { serverId: target.serverId })}
                  </span>
                </label>
                {names.length > 0 && (
                  <table className="ml-6 text-sm" data-testid={`deploy-plan-${target.id}`}>
                    <thead>
                      <tr className="text-left text-xs text-muted-foreground">
                        <th className="pr-3 font-normal">{t("deploy.plan.file")}</th>
                        <th className="font-normal">{t("deploy.plan.dir")}</th>
                      </tr>
                    </thead>
                    <tbody>
                      {filterBranches(names, entryQuery).map(({ branch: fileName, indices }) => {
                        const index = names.indexOf(fileName);
                        return (
                          <tr key={fileName}>
                            <td className="pr-3">
                              <label className="flex items-center gap-2">
                                <input
                                  type="checkbox"
                                  data-testid={`deploy-entry-check-${target.id}-${index}`}
                                  checked={entryState.get(target.id)?.has(fileName) ?? false}
                                  disabled={running || !uploadsFiles}
                                  onChange={() => toggleEntry(target.id, fileName)}
                                />
                                <span className="font-mono">
                                  <HighlightedText text={fileName} indices={indices} />
                                </span>
                              </label>
                            </td>
                            <td
                              className="font-mono text-muted-foreground"
                              data-testid={`deploy-entry-dir-${target.id}-${index}`}
                            >
                              {dirFor(target, fileName)}
                            </td>
                          </tr>
                        );
                      })}
                      {entryQuery !== "" && filterBranches(names, entryQuery).length === 0 && (
                        <tr>
                          <td colSpan={2} className="py-2 text-muted-foreground">
                            {t("deploy.plan.noMatchingArtifacts")}
                          </td>
                        </tr>
                      )}
                    </tbody>
                  </table>
                )}
              </div>
            ))}
          </div>
        </fieldset>

        {startError && (
          <p role="alert" className="text-sm text-destructive">
            {startError}
          </p>
        )}

        <div className="flex gap-2">
          <Button
            data-testid="deploy-submit"
            onClick={() => void handleStart()}
            disabled={
              running ||
              artifacts === null ||
              selectedIds.length === 0 ||
              (uploadsFiles && (artifacts.length === 0 || !hasCheckedEntries))
            }
          >
            {t("deploy.submit")}
          </Button>
          {running && (
            <Button variant="outline" data-testid="deploy-cancel" onClick={() => void handleCancel()}>
              {t("deploy.cancel")}
            </Button>
          )}
        </div>

        {[...(views ?? new Map()).entries()].map(([targetId, view]) => (
          <div key={targetId} data-testid={`deploy-result-${targetId}`} className="rounded-md border p-3">
            <div className="flex items-center gap-2 text-sm">
              <span className="font-medium">{t("deploy.targetLabel", { id: targetId })}</span>
              <span
                data-testid={`deploy-status-${targetId}`}
                className={
                  view.status === "success"
                    ? "rounded-full bg-emerald-100 px-2 py-0.5 text-xs font-medium text-emerald-800 dark:bg-emerald-900/40 dark:text-emerald-300"
                    : view.status === "failed"
                      ? "rounded-full bg-destructive/10 px-2 py-0.5 text-xs font-medium text-destructive"
                      : "rounded-full bg-muted px-2 py-0.5 text-xs"
                }
              >
                {statusText(view.status)}
              </span>
              {view.progress && view.status === "uploading" && (
                <span data-testid={`deploy-progress-${targetId}`} className="text-xs text-muted-foreground">
                  {t("deploy.uploadProgress", view.progress)}
                </span>
              )}
            </div>

            {view.notices.length > 0 && (
              <ul data-testid={`deploy-skipped-${targetId}`} className="mt-1 list-inside list-disc text-xs text-amber-600 dark:text-amber-400">
                {view.notices.map((notice, i) => (
                  <li key={i}>{notice}</li>
                ))}
              </ul>
            )}

            {view.stopFailed && (
              <div className="mt-2">
                <p role="alert" className="text-sm text-destructive">
                  {t("deploy.stopFailedNotice")}
                </p>
                <Button
                  variant="outline"
                  size="sm"
                  data-testid={`manual-start-${targetId}`}
                  className="mt-1"
                  onClick={() => void handleManualStart(targetId)}
                >
                  {t("deploy.manualStart")}
                </Button>
              </div>
            )}
            {view.error && !view.stopFailed && (
              <p role="alert" className="mt-1 text-sm text-destructive">
                {view.error}
              </p>
            )}

            {(["upload", "stop", "start"] as DeployPhase[]).map((phase) => {
              const snapshot = view.outputs[phase];
              if (!snapshot) return null;
              return (
                <div key={phase} className="mt-2">
                  <p className="text-xs font-medium">{t(`deploy.phases.${phase}`)}</p>
                  <pre
                    data-testid={`deploy-output-${targetId}-${phase}`}
                    className={`mt-0.5 max-h-40 overflow-auto whitespace-pre-wrap rounded bg-muted px-2 py-1 text-xs ${
                      snapshot.exit_code !== 0 ? "text-destructive" : ""
                    }`}
                  >
                    {snapshot.output}
                  </pre>
                </div>
              );
            })}
          </div>
        ))}
      </CardContent>
    </Card>
  );
}

export default DeployDialog;
