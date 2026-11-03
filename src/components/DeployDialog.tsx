// 部署对话框（ticket 10）：从一条构建历史选择构件与部署目标（多选），
// 依次部署；展示各目标阶段状态与输出快照；停止失败中止重启并提供
// “单独执行启动命令组”；部署进行中可取消。

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
  type TargetStatus,
} from "@/lib/deployApi";
import { listBuildArtifacts, type ArtifactFile } from "@/lib/historyApi";
import { listServers } from "@/lib/serversApi";
import { listDeploymentTargets, type Target } from "@/lib/targetsApi";
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
  const [selectedArtifact, setSelectedArtifact] = useState<string | null>(null);
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
      const preselect =
        defaultArtifact && names.includes(defaultArtifact)
          ? defaultArtifact
          : (names[0] ?? null);
      setSelectedArtifact(preselect);
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
    if (!selectedArtifact || selectedIds.length === 0) return;
    setStartError(null);
    setRunning(true);
    const channel = new Channel<AutoDeployMessage>();
    channel.onmessage = handleMessage;
    try {
      await startDeploy(historyId, selectedArtifact, selectedIds, channel);
    } catch (e) {
      setStartError(t("deploy.deployFailed", { detail: backendMessage(e) }));
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
        {artifacts !== null && artifacts.length > 0 && (
          <fieldset>
            <legend className="text-sm font-medium">{t("deploy.artifactTitle")}</legend>
            <div role="radiogroup" aria-label={t("deploy.artifactTitle")} className="mt-1 flex flex-col gap-1">
              {artifacts.map((artifact) => (
                <label key={artifact.file_name} className="flex items-center gap-2 text-sm">
                  <input
                    type="radio"
                    name="deploy-artifact"
                    value={artifact.file_name}
                    checked={selectedArtifact === artifact.file_name}
                    disabled={running}
                    onChange={() => setSelectedArtifact(artifact.file_name)}
                  />
                  <span className="font-mono">{artifact.file_name}</span>
                </label>
              ))}
            </div>
          </fieldset>
        )}

        <fieldset>
          <legend className="text-sm font-medium">{t("deploy.targetTitle")}</legend>
          {targets !== null && targets.length === 0 && (
            <p className="text-sm text-muted-foreground">{t("deploy.noTargets")}</p>
          )}
          <div className="mt-1 flex flex-col gap-2">
            {(targets ?? []).map((target) => {
              return (
                <label
                  key={target.id}
                  className="flex items-center gap-2 text-sm"
                >
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
              );
            })}
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
            disabled={running || !selectedArtifact || selectedIds.length === 0}
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
