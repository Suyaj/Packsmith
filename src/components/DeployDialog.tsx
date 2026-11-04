// 部署对话框（ticket 10 / 19 / 23 / 24）：确认解析后的上传计划——每个部署目标
// 逐行展示其 文件名→目录 映射行（可勾选；产物缺失的行禁用+标红警告；空文件名行
// 内联补选，cmdk 可搜索 Combobox），上传集合 = 勾选集合。批量补选（票 24）：
// 一次勾选多个构件按 目标顺序→行顺序 填入空名行，多出构件提示。对话框级操作
// 选择（重启/仅上传/仅停止/仅启动，票 23）：仅停止/仅启动时上传计划区只读展示、
// payload 只带目标；依次部署；展示各目标阶段状态、跳过提示与输出快照；重启时
// 停止失败中止重启并提供"单独执行启动命令组"；部署进行中可取消。

import { useCallback, useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { Channel } from "@tauri-apps/api/core";
import { Button } from "@/components/ui/button";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import FuzzyCombobox from "@/components/FuzzyCombobox";
import {
  cancelDeploy,
  runStartGroup,
  startDeploy,
  type AutoDeployMessage,
  type DeployOperation,
  type DeployPhase,
  type RowAssignment,
  type TargetAssignment,
  type TargetStatus,
} from "@/lib/deployApi";
import { listBuildArtifacts, type ArtifactFile } from "@/lib/historyApi";
import { listServers } from "@/lib/serversApi";
import { listDeploymentTargets, type Target, type TargetFile } from "@/lib/targetsApi";
import { serverLabel } from "./DeploymentTargetsPanel";

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

/** 单行是否可直接解析上传：具名行须在本次构建产物里；空名行依赖补选。 */
function rowMissing(row: TargetFile, names: string[]): boolean {
  return row.fileName.trim() !== "" && !names.includes(row.fileName.trim());
}

function DeployDialog({
  projectId,
  projectName,
  historyId,
  open,
  onClose,
  defaultArtifacts,
}: {
  projectId: number;
  projectName: string;
  historyId: number;
  open: boolean;
  onClose: () => void;
  /** 项目默认构件列表（票 21）：空名行补选预选按序取第一个可用项。 */
  defaultArtifacts?: string[];
}) {
  const { t } = useTranslation();
  const [targets, setTargets] = useState<Target[] | null>(null);
  const [serverLabels, setServerLabels] = useState<Map<number, string>>(new Map());
  const [artifacts, setArtifacts] = useState<ArtifactFile[] | null>(null);
  const [selectedIds, setSelectedIds] = useState<number[]>([]);
  // 每行上传计划的勾选状态；空名行另存补选的文件名（ticket 19）。
  const [rowState, setRowState] = useState<Map<number, { checked: boolean; pick: string }>>(
    new Map(),
  );
  const [views, setViews] = useState<Map<number, TargetView>>(new Map());
  // 本次操作（票 23，对话框级）：重启（默认）/ 仅上传 / 仅停止 / 仅启动。
  const [operation, setOperation] = useState<DeployOperation>("restart");
  // 批量补选（票 24）：多选勾选集（按勾选顺序）+ 多出构件的提示。
  const [batchPicks, setBatchPicks] = useState<string[]>([]);
  const [batchNotice, setBatchNotice] = useState<string | null>(null);
  const [running, setRunning] = useState(false);
  const [loadError, setLoadError] = useState<string | null>(null);
  const [startError, setStartError] = useState<string | null>(null);
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
      // 解析后的上传计划：具名行默认勾选（缺失行禁用+警告）；空名行
      // （部署时补选）默认勾选并预选默认构件列表中第一个可用项（票 21，
      // 依序取用）；列表为空/用尽回退首个可用产物。
      const plan = new Map<number, { checked: boolean; pick: string }>();
      const fallback =
        defaultArtifacts?.find((n) => names.includes(n)) ?? names[0] ?? "";
      for (const target of targetList) {
        for (const row of target.files) {
          if (row.fileName.trim() === "") {
            plan.set(row.id, { checked: fallback !== "", pick: fallback });
          } else {
            plan.set(row.id, { checked: names.includes(row.fileName.trim()), pick: "" });
          }
        }
      }
      setRowState(plan);
      setSelectedIds([]);
      setOperation("restart");
      setBatchPicks([]);
      setBatchNotice(null);
      setViews(new Map(targetList.map((target) => [target.id, { ...INITIAL_VIEW, outputs: {}, notices: [] }])));
      setLoadError(null);
    } catch (e) {
      setLoadError(t("deploy.loadFailed", { detail: backendMessage(e) }));
    }
  }, [open, projectId, historyId, defaultArtifacts, t]);

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
        // 跳过提示（空名行未补选 / 缺失产物行）不改目标状态，仅通知。
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

  /** 一行当前是否可上传：具名且产物在列，或空名行已补选。 */
  const rowUploadable = (row: TargetFile, names: string[]): boolean => {
    if (rowMissing(row, names)) return false;
    if (row.fileName.trim() !== "") return true;
    return (rowState.get(row.id)?.pick ?? "").trim() !== "";
  };

  /** 本次操作是否传文件（重启/仅上传）：仅停止/仅启动只执行命令组（票 23）。 */
  const uploadsFiles = operation === "restart" || operation === "upload";

  // 空名行（部署时补选）数量：批量补选入口的可见条件之一（票 24）。
  const emptyRowCount = (targets ?? []).reduce(
    (count, target) =>
      count + target.files.filter((row) => row.fileName.trim() === "").length,
    0,
  );

  /** 批量补选确认（票 24）：按勾选顺序填入各目标空名行（目标顺序 → 目标内
   * 行顺序），填入行默认勾选；构件多于空名行时多出的忽略并提示。只改本次
   * 部署的补选值，不改映射行集合；具名行不参与。 */
  const applyBatchFill = () => {
    if (batchPicks.length === 0) return;
    const emptyRowIds: number[] = [];
    for (const target of targets ?? []) {
      for (const row of target.files) {
        if (row.fileName.trim() === "") emptyRowIds.push(row.id);
      }
    }
    const filled = Math.min(batchPicks.length, emptyRowIds.length);
    const extra = batchPicks.length - filled;
    setRowState((prev) => {
      const next = new Map(prev);
      for (let i = 0; i < filled; i++) {
        next.set(emptyRowIds[i], { checked: true, pick: batchPicks[i] });
      }
      return next;
    });
    setBatchNotice(extra > 0 ? t("deploy.batch.filledNotice", { filled, extra }) : null);
    setBatchPicks([]);
  };

  const handleStart = async () => {
    if (artifacts === null) return;
    const names = artifacts.map((a) => a.file_name);
    let assignments: TargetAssignment[];
    if (uploadsFiles) {
      // 上传集合 = 勾选集合：只提交选中目标里勾选且可解析的行（ticket 19）。
      assignments = (targets ?? [])
        .filter((target) => selectedIds.includes(target.id))
        .map((target) => ({
          targetId: target.id,
          files: target.files
            .filter((row) => rowState.get(row.id)?.checked && rowUploadable(row, names))
            .map(
              (row): RowAssignment => ({
                rowId: row.id,
                // 具名行以后端库内文件名为准；空名行带补选结果。
                fileName:
                  row.fileName.trim() === "" ? (rowState.get(row.id)?.pick ?? "").trim() : "",
              }),
            ),
        }))
        .filter((a) => a.files.length > 0);
      if (assignments.length === 0) return;
    } else {
      // 仅停止/仅启动（票 23）：不传文件，payload 只带选中的目标。
      assignments = (targets ?? [])
        .filter((target) => selectedIds.includes(target.id))
        .map((target): TargetAssignment => ({ targetId: target.id, files: [] }));
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

  const names = (artifacts ?? []).map((a) => a.file_name);
  /** 选中目标里至少有一行可上传，才允许发起部署。 */
  const hasUploadableRows = (targets ?? [])
    .filter((target) => selectedIds.includes(target.id))
    .some((target) =>
      target.files.some(
        (row) => rowState.get(row.id)?.checked && rowUploadable(row, names),
      ),
    );

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
          {/* 批量补选（票 24）：仅当存在空名行且本次操作传文件时可见。 */}
          {uploadsFiles && emptyRowCount > 0 && (
            <div className="flex flex-col gap-1">
              <div className="flex items-center gap-2">
                <FuzzyCombobox
                  multiple
                  items={names}
                  selected={batchPicks}
                  onSelect={(name) =>
                    setBatchPicks((prev) =>
                      prev.includes(name)
                        ? prev.filter((n) => n !== name)
                        : [...prev, name],
                    )
                  }
                  disabled={running}
                  ariaLabel={t("deploy.batch.title")}
                  searchPlaceholder={t("deploy.plan.searchArtifacts")}
                  emptyText={t("deploy.plan.noMatchingArtifacts")}
                  renderValue={(sel) =>
                    sel.length > 0 ? sel.join("、") : t("deploy.batch.placeholder")
                  }
                  className="max-w-72"
                  testId="deploy-batch"
                />
                <Button
                  variant="outline"
                  size="sm"
                  data-testid="deploy-batch-apply"
                  disabled={running || batchPicks.length === 0}
                  onClick={applyBatchFill}
                >
                  {t("deploy.batch.apply")}
                </Button>
              </div>
              {batchNotice && (
                <p role="status" data-testid="deploy-batch-notice" className="text-xs text-amber-600 dark:text-amber-400">
                  {batchNotice}
                </p>
              )}
            </div>
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
                {target.files.length > 0 && (
                  <table className="ml-6 text-sm" data-testid={`deploy-plan-${target.id}`}>
                    <thead>
                      <tr className="text-left text-xs text-muted-foreground">
                        <th className="pr-3 font-normal">{t("deploy.plan.file")}</th>
                        <th className="font-normal">{t("deploy.plan.dir")}</th>
                      </tr>
                    </thead>
                    <tbody>
                      {target.files.map((row) => {
                        const missing = rowMissing(row, names);
                        const unnamed = row.fileName.trim() === "";
                        const checked = rowState.get(row.id)?.checked ?? false;
                        const pick = rowState.get(row.id)?.pick ?? "";
                        return (
                          <tr
                            key={row.id}
                            data-testid={`deploy-row-${target.id}-${row.id}`}
                            className={missing ? "text-destructive" : ""}
                          >
                            <td className="pr-3">
                              <label className="flex items-center gap-2">
                                <input
                                  type="checkbox"
                                  data-testid={`deploy-row-check-${target.id}-${row.id}`}
                                  checked={checked}
                                  disabled={running || missing || !uploadsFiles}
                                  onChange={() =>
                                    setRowState((prev) => {
                                      const next = new Map(prev);
                                      const current = prev.get(row.id) ?? { checked, pick: "" };
                                      next.set(row.id, { ...current, checked: !current.checked });
                                      return next;
                                    })
                                  }
                                />
                                <span className="font-mono">
                                  {unnamed ? t("deploy.plan.unnamedFile") : row.fileName}
                                </span>
                              </label>
                              {missing && (
                                <span
                                  role="alert"
                                  data-testid={`deploy-row-warning-${target.id}-${row.id}`}
                                  className="text-xs text-destructive"
                                >
                                  {t("deploy.plan.missingWarning")}
                                </span>
                              )}
                              {unnamed && !missing && (
                                // 票 24：补选升级为可搜索 Combobox（与打包面板
                                // 选择器同款模糊过滤 + 命中高亮），单选选中即收起。
                                <FuzzyCombobox
                                  items={names}
                                  selected={pick ? [pick] : []}
                                  onSelect={(name) =>
                                    setRowState((prev) => {
                                      const next = new Map(prev);
                                      const current = prev.get(row.id) ?? { checked, pick: "" };
                                      next.set(row.id, { ...current, pick: name });
                                      return next;
                                    })
                                  }
                                  disabled={running || !uploadsFiles}
                                  ariaLabel={t("deploy.plan.pickLabel")}
                                  searchPlaceholder={t("deploy.plan.searchArtifacts")}
                                  emptyText={t("deploy.plan.noMatchingArtifacts")}
                                  renderValue={() =>
                                    pick || t("deploy.plan.pickPlaceholder")
                                  }
                                  className="ml-2 h-7 max-w-56 px-2 text-xs"
                                  testId={`deploy-row-select-${target.id}-${row.id}`}
                                />
                              )}
                            </td>
                            <td className="font-mono text-muted-foreground">{row.uploadDir}</td>
                          </tr>
                        );
                      })}
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
              (uploadsFiles && (artifacts.length === 0 || !hasUploadableRows))
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
