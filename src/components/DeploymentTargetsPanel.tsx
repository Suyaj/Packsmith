import { useCallback, useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { Button } from "@/components/ui/button";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { listServers, type Server } from "@/lib/serversApi";
import { setTargetAutoDeploy } from "@/lib/deployApi";
import {
  createDeploymentTarget,
  deleteDeploymentTarget,
  listDeploymentTargets,
  updateDeploymentTarget,
  type Target,
  type TargetInput,
} from "@/lib/targetsApi";

/** 服务器在界面上的统一展示：主机:端口 — 用户名。 */
export function serverLabel(server: Server): string {
  return `${server.host}:${server.port} — ${server.username}`;
}

type FormState = {
  editingId: number | null;
  serverId: string;
  uploadDir: string;
  startCommands: string;
  stopCommands: string;
};

const EMPTY_FORM: FormState = {
  editingId: null,
  serverId: "",
  uploadDir: "",
  startCommands: "",
  stopCommands: "",
};

interface BackendError {
  code: string;
  message: string;
}

function isBackendError(e: unknown): e is BackendError {
  return typeof e === "object" && e !== null && "code" in e;
}

function uploadDirProblem(uploadDir: string): string | null {
  const lines = uploadDir
    .split("\n")
    .map((line) => line.trim())
    .filter((line) => line !== "");
  if (lines.length === 0) return "uploadDirRequired";
  if (lines.length > 1) return "uploadDirMultiline";
  return null;
}

/**
 * 部署目标面板：一个项目的 项目 × 服务器绑定 CRUD（ticket 09）。
 * 只做配置管理；上传与命令执行在打包/部署流程（ticket 10）中按目标操作。
 */
function DeploymentTargetsPanel({
  projectId,
  projectName,
}: {
  projectId: number;
  projectName: string;
}) {
  const { t } = useTranslation();
  const [targets, setTargets] = useState<Target[]>([]);
  const [servers, setServers] = useState<Server[]>([]);
  const [loadError, setLoadError] = useState<string | null>(null);
  const [form, setForm] = useState<FormState | null>(null);
  const [saveError, setSaveError] = useState<string | null>(null);

  const refresh = useCallback(async () => {
    try {
      const [targetList, serverList] = await Promise.all([
        listDeploymentTargets(projectId),
        listServers(),
      ]);
      setTargets(Array.isArray(targetList) ? targetList : []);
      setServers(Array.isArray(serverList) ? serverList : []);
      setLoadError(null);
    } catch (e) {
      setLoadError(e instanceof Error ? e.message : String(e));
    }
  }, [projectId]);

  useEffect(() => {
    void refresh();
  }, [refresh]);

  const serverById = (id: number) => servers.find((s) => s.id === id);

  const openCreate = () => {
    setSaveError(null);
    setForm({ ...EMPTY_FORM });
  };

  const openEdit = (target: Target) => {
    setSaveError(null);
    setForm({
      editingId: target.id,
      serverId: String(target.serverId),
      uploadDir: target.uploadDir,
      startCommands: target.startCommands,
      stopCommands: target.stopCommands,
    });
  };

  const formInput = (f: FormState): TargetInput => ({
    serverId: Number.parseInt(f.serverId, 10),
    uploadDir: f.uploadDir.trim(),
    startCommands: f.startCommands,
    stopCommands: f.stopCommands,
    autoDeploy: targets.find((t) => t.id === f.editingId)?.autoDeploy ?? false,
  });

  const formValid = (f: FormState) =>
    f.serverId !== "" && uploadDirProblem(f.uploadDir) === null;

  const handleSave = async () => {
    if (!form) return;
    const input = formInput(form);
    try {
      if (form.editingId === null) {
        await createDeploymentTarget(projectId, input);
      } else {
        await updateDeploymentTarget(form.editingId, input);
      }
      setForm(null);
      setSaveError(null);
      await refresh();
    } catch (e) {
      const code = isBackendError(e) ? e.code : "unknown";
      const message = isBackendError(e) ? e.message : String(e);
      setSaveError(t(`targets.errors.${code}`, { message }));
    }
  };

  const handleToggleAutoDeploy = async (target: Target, enabled: boolean) => {
    try {
      await setTargetAutoDeploy(target.id, enabled);
      setTargets((prev) =>
        prev.map((t) => (t.id === target.id ? { ...t, autoDeploy: enabled } : t)),
      );
    } catch (e) {
      setLoadError(e instanceof Error ? e.message : String(e));
    }
  };

  const handleDelete = async (target: Target) => {
    if (!window.confirm(t("targets.confirmDelete"))) return;
    await deleteDeploymentTarget(target.id);
    await refresh();
  };

  const renderForm = (f: FormState) => (
    <Card className="mt-3" data-testid="target-form">
      <CardHeader>
        <CardTitle>
          {f.editingId === null ? t("targets.formTitleNew") : t("targets.formTitleEdit")}
        </CardTitle>
      </CardHeader>
      <CardContent>
        <div className="flex flex-col gap-3">
          <label className="flex flex-col gap-1">
            {t("targets.server")}
            <select
              aria-label={t("targets.server")}
              value={f.serverId}
              onChange={(e) => setForm({ ...f, serverId: e.target.value })}
              className="rounded-md border bg-background px-2 py-1.5"
            >
              <option value="">{t("targets.serverPlaceholder")}</option>
              {servers.map((server) => (
                <option key={server.id} value={server.id}>
                  {serverLabel(server)}
                </option>
              ))}
            </select>
          </label>

          <label className="flex flex-col gap-1">
            {t("targets.uploadDir")}
            {/* 验收要求三个字段均可多行编辑；上传目录另校验“仅一行” */}
            <textarea
              aria-label={t("targets.uploadDir")}
              rows={1}
              value={f.uploadDir}
              onChange={(e) => setForm({ ...f, uploadDir: e.target.value })}
              className="rounded-md border bg-background px-2 py-1.5"
            />
            <span className="text-xs text-muted-foreground">{t("targets.uploadDirHint")}</span>
          </label>

          {uploadDirProblem(f.uploadDir) === "uploadDirMultiline" && (
            <p role="alert" className="text-sm text-red-600 dark:text-red-400">
              {t("targets.errors.uploadDirMultiline")}
            </p>
          )}

          <label className="flex flex-col gap-1">
            {t("targets.startCommands")}
            {/* 命令组：多行编辑器，一条命令一行，按序执行 */}
            <textarea
              aria-label={t("targets.startCommands")}
              rows={3}
              value={f.startCommands}
              placeholder={"cd /home/app\nsh start.sh start"}
              onChange={(e) => setForm({ ...f, startCommands: e.target.value })}
              className="rounded-md border bg-background px-2 py-1.5 font-mono"
            />
            <span className="text-xs text-muted-foreground">{t("targets.commandsHint")}</span>
          </label>

          <label className="flex flex-col gap-1">
            {t("targets.stopCommands")}
            <textarea
              aria-label={t("targets.stopCommands")}
              rows={3}
              value={f.stopCommands}
              placeholder="sh start.sh stop"
              onChange={(e) => setForm({ ...f, stopCommands: e.target.value })}
              className="rounded-md border bg-background px-2 py-1.5 font-mono"
            />
          </label>
        </div>

        {saveError && (
          <p role="alert" className="mt-2 text-sm text-red-600 dark:text-red-400">
            {saveError}
          </p>
        )}

        <div className="mt-3 flex gap-2">
          <Button disabled={!formValid(f)} onClick={() => void handleSave()}>
            {t("targets.save")}
          </Button>
          <Button variant="outline" onClick={() => setForm(null)}>
            {t("targets.cancel")}
          </Button>
        </div>
      </CardContent>
    </Card>
  );

  return (
    <Card data-testid="deployment-targets-panel">
      <CardHeader className="flex flex-row items-center justify-between space-y-0">
        <CardTitle>{t("targets.title", { project: projectName })}</CardTitle>
        {!form && (
          <Button onClick={openCreate}>{t("targets.add")}</Button>
        )}
      </CardHeader>
      <CardContent>
        {loadError && (
          <p role="alert" className="text-sm text-red-600 dark:text-red-400">
            {t("targets.loadFailed", { detail: loadError })}
          </p>
        )}

        {targets.length === 0 && !loadError && !form && (
          <p>{t("targets.empty")}</p>
        )}

        {form && renderForm(form)}

        <div className="space-y-3" data-testid="target-list">
          {targets.map((target) => {
            const server = serverById(target.serverId);
            return (
              <Card key={target.id} data-testid={`target-row-${target.id}`}>
                <CardContent className="pt-4">
                  <div className="flex flex-wrap items-center justify-between gap-2">
                    <div className="text-base font-medium">
                      {server
                        ? serverLabel(server)
                        : t("targets.serverMissing", { serverId: target.serverId })}
                    </div>
                    <div className="flex gap-2">
                      <Button
                        size="sm"
                        variant="outline"
                        onClick={() => openEdit(target)}
                      >
                        {t("targets.edit")}
                      </Button>
                      <Button
                        size="sm"
                        variant="outline"
                        onClick={() => void handleDelete(target)}
                      >
                        {t("targets.delete")}
                      </Button>
                    </div>
                  </div>
                  <p className="mt-1 text-sm">
                    {t("targets.uploadDirValue", { value: target.uploadDir })}
                  </p>
                  <label className="mt-2 flex items-center gap-2 text-sm">
                    <input
                      type="checkbox"
                      data-testid={`auto-deploy-toggle-${target.id}`}
                      checked={target.autoDeploy}
                      onChange={(e) =>
                        void handleToggleAutoDeploy(target, e.target.checked)
                      }
                    />
                    {t("targets.autoDeploy")}
                  </label>
                  <div className="mt-1 text-sm">
                    <span>{t("targets.startCommands")}</span>
                    <pre
                      data-testid={`target-start-${target.id}`}
                      className="mt-0.5 whitespace-pre-wrap rounded bg-muted px-2 py-1"
                    >
                      {target.startCommands}
                    </pre>
                  </div>
                  <div className="mt-1 text-sm">
                    <span>{t("targets.stopCommands")}</span>
                    <pre
                      data-testid={`target-stop-${target.id}`}
                      className="mt-0.5 whitespace-pre-wrap rounded bg-muted px-2 py-1"
                    >
                      {target.stopCommands}
                    </pre>
                  </div>
                </CardContent>
              </Card>
            );
          })}
        </div>
      </CardContent>
    </Card>
  );
}

export default DeploymentTargetsPanel;
