// 部署对话框（ticket 10 / 17）：从一条构建历史逐个部署目标选择要上传的
// 构件（一一对应上传），依次部署；展示各目标阶段状态与输出快照；停止失败
// 中止重启并提供“单独执行启动命令组”；部署进行中可取消。

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
  type DeployPhase,
  type TargetAssignment,
  type TargetStatus,
} from "@/lib/deployApi";
import { listBuildArtifacts, type ArtifactFile } from "@/lib/historyApi";
import { listServers } from "@/lib/serversApi";
import {
  listDeploymentTargets,
  setTargetLastArtifact,
  type Target,
} from "@/lib/targetsApi";
import { serverLabel } from "./DeploymentTargetsPanel";

/** 每个目标的实时展示状态：状态 + 各阶段输出快照 + 上传进度。 */
type TargetView = {
  status: TargetStatus;
  outputs: Partial<Record<DeployPhase, { output: string; exit_code: number }>>;
  stopFailed: boolean;
  error: string | null;
  progress: { transferred: number; total: number } | null;
};

const INITIAL_VIEW: TargetView = {
  status: "pending",
  outputs: {},
  stopFailed: false,
  error: null,
  progress: null,
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
  defaultArtifact,
}: {
  projectId: number;
  projectName: string;
  historyId: number;
  open: boolean;
  onClose: () => void;
  defaultArtifact?: string | null;
}) {
  const { t } = useTranslation();
  const [targets, setTargets] = useState<Target[] | null>(null);
  const [serverLabels, setServerLabels] = useState<Map<number, string>>(new Map());
  const [artifacts, setArtifacts] = useState<ArtifactFile[] | null>(null);
  // 每个目标各自选中的构件文件名（ticket 17 一一对应上传）。
  const [selectedFiles, setSelectedFiles] = useState<Map<number, string>>(new Map());
  const [selectedIds, setSelectedIds] = useState<number[]>([]);
  const [views, setViews] = useState<Map<number, TargetView>>(new Map());
  const [running, setRunning] = useState(false);
  const [loadError, setLoadError] = useState<string | null>(null);
  const [startError, setStartError] = useState<string | null>(null);
  const deployIdRef = useRef<number | null>(null);

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
      // 每个目标的默认构件：单产物自动选中；多产物优先记住的上次选择
      // （ticket 17 per-target last choice），否则退回项目默认构件。
      const defaults = new Map<number, string>();
      for (const target of targetList) {
        const preferred =
          (target.lastArtifact && names.includes(target.lastArtifact)
            ? target.lastArtifact
            : null) ??
          (defaultArtifact && names.includes(defaultArtifact)
            ? defaultArtifact
            : null) ??
          names[0] ??
          null;
        if (preferred) defaults.set(target.id, preferred);
      }
      setSelectedFiles(defaults);
      setViews(new Map(targetList.map((target) => [target.id, { ...INITIAL_VIEW, outputs: {} }])));
      setLoadError(null);
    } catch (e) {
      setLoadError(t("deploy.loadFailed", { detail: backendMessage(e) }));
    }
  }, [open, projectId, historyId, defaultArtifact, t]);

  useEffect(() => {
    void refresh();
  }, [refresh]);

  if (!open) return null;

  const patchView = (targetId: number, patch: Partial<TargetView>) => {
    setViews((prev) => {
      const next = new Map(prev);
      const current = next.get(targetId) ?? { ...INITIAL_VIEW, outputs: {} };
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
          const current = next.get(event.target_id) ?? { ...INITIAL_VIEW, outputs: {} };
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

  const handleStart = async () => {
    // 每个选中的目标必须有自己的构件选择；提交一一对应的 assignments。
    const assignments: TargetAssignment[] = selectedIds
      .map((id) => ({ targetId: id, fileName: selectedFiles.get(id) ?? "" }))
      .filter((a) => a.fileName !== "");
    if (assignments.length !== selectedIds.length) return;
    setStartError(null);
    setRunning(true);
    // 记住每个目标本次的选择（ticket 17），失败不阻塞部署。
    await Promise.allSettled(
      assignments.map((a) => setTargetLastArtifact(a.targetId, a.fileName)),
    );
    const channel = new Channel<AutoDeployMessage>();
    channel.onmessage = handleMessage;
    try {
      await startDeploy(historyId, assignments, channel);
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

  return (
    <Card data-testid="deploy-dialog" className="mt-4">
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
          <legend className="text-sm font-medium">{t("deploy.targetTitle")}</legend>
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
                    <span className="text-muted-foreground"> · {target.uploadDir}</span>
                  </span>
                </label>
                {artifacts !== null && artifacts.length > 0 && (
                  <label className="flex items-center gap-2 text-sm">
                    <span className="text-muted-foreground">{t("deploy.artifactLabel")}</span>
                    <select
                      data-testid={`deploy-artifact-select-${target.id}`}
                      value={selectedFiles.get(target.id) ?? ""}
                      disabled={running}
                      onChange={(e) => {
                        const fileName = e.target.value;
                        setSelectedFiles((prev) => new Map(prev).set(target.id, fileName));
                      }}
                    >
                      {artifacts.map((artifact) => (
                        <option key={artifact.file_name} value={artifact.file_name}>
                          {artifact.file_name}
                        </option>
                      ))}
                    </select>
                  </label>
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
              artifacts.length === 0 ||
              selectedIds.length === 0 ||
              selectedIds.some((id) => !selectedFiles.get(id))
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
