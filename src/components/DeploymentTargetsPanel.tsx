import { useCallback, useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { Button } from "@/components/ui/button";
import ConfirmDialog from "@/components/ConfirmDialog";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { listServers, type Server } from "@/lib/serversApi";
import { setTargetAutoDeploy } from "@/lib/deployApi";
import {
  createDeploymentTarget,
  deleteDeploymentTarget,
  listDeploymentTargets,
  updateDeploymentTarget,
  type Target,
  type TargetFileInput,
  type TargetInput,
} from "@/lib/targetsApi";

/** 服务器在界面上的统一展示：主机:端口 — 用户名。 */
export function serverLabel(server: Server): string {
  return `${server.host}:${server.port} — ${server.username}`;
}

/** 表单里的一行 文件名→目录 映射（尚未持久化）。 */
type RowDraft = {
  fileName: string;
  uploadDir: string;
};

type FormState = {
  editingId: number | null;
  serverId: string;
  rows: RowDraft[];
  startCommands: string;
  stopCommands: string;
};

const EMPTY_ROW: RowDraft = { fileName: "", uploadDir: "" };

const EMPTY_FORM: FormState = {
  editingId: null,
  serverId: "",
  rows: [{ ...EMPTY_ROW }],
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

/** 单行上传目录校验：仅一行非空路径（ticket 19 沿用原单目录规则）。 */
function uploadDirProblem(uploadDir: string): string | null {
  const lines = uploadDir
    .split("\n")
    .map((line) => line.trim())
    .filter((line) => line !== "");
  if (lines.length === 0) return "uploadDirRequired";
  if (lines.length > 1) return "uploadDirMultiline";
  return null;
}

/** 整表校验：每行目录合法、非空文件名不得重复。返回首个问题的 i18n key。 */
function rowsProblem(rows: RowDraft[]): string | null {
  if (rows.length === 0) return "rowsRequired";
  const seen = new Set<string>();
  for (const row of rows) {
    const problem = uploadDirProblem(row.uploadDir);
    if (problem) return problem;
    const name = row.fileName.trim();
    if (name !== "") {
      if (seen.has(name)) return "duplicateFileName";
      seen.add(name);
    }
  }
  return null;
}

/**
 * 部署目标面板：一个项目的 项目 × 服务器绑定 CRUD（ticket 09）。
 * ticket 19：上传目录为 文件名→目录 映射表（可增删行）；部署时按行匹配
 * 本次构建的产物并上传到该行的目录。
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
  // 待确认删除的部署目标（Tauri WebView 不支持 window.confirm，改用对话框）
  const [confirmDelete, setConfirmDelete] = useState<Target | null>(null);

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
    setForm({ ...EMPTY_FORM, rows: [{ ...EMPTY_ROW }] });
  };

  const openEdit = (target: Target) => {
    setSaveError(null);
    setForm({
      editingId: target.id,
      serverId: String(target.serverId),
      rows: target.files.map((f) => ({ fileName: f.fileName, uploadDir: f.uploadDir })),
      startCommands: target.startCommands,
      stopCommands: target.stopCommands,
    });
  };

  const patchRow = (index: number, patch: Partial<RowDraft>) => {
    if (!form) return;
    setForm({
      ...form,
      rows: form.rows.map((row, i) => (i === index ? { ...row, ...patch } : row)),
    });
  };

  const addRow = () => {
    if (!form) return;
    setForm({ ...form, rows: [...form.rows, { ...EMPTY_ROW }] });
  };

  const removeRow = (index: number) => {
    if (!form) return;
    setForm({ ...form, rows: form.rows.filter((_, i) => i !== index) });
  };

  const formInput = (f: FormState): TargetInput => ({
    serverId: Number.parseInt(f.serverId, 10),
    files: f.rows.map(
      (row): TargetFileInput => ({
        fileName: row.fileName.trim(),
        uploadDir: row.uploadDir.trim(),
      }),
    ),
    startCommands: f.startCommands,
    stopCommands: f.stopCommands,
    autoDeploy: targets.find((t) => t.id === f.editingId)?.autoDeploy ?? false,
  });

  const formValid = (f: FormState) =>
    f.serverId !== "" && rowsProblem(f.rows) === null;

  const handleSave = async () => {
    if (!form) return;
    const input = formInput(form);
    try {
      if (form.editingId === null) {
        await createDeploymentTarget(projectId, input);
      } else {
        await updateDeploymentTarget(form.editingId, input, projectId);
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
    setConfirmDelete(null);
    await deleteDeploymentTarget(target.id);
    await refresh();
  };

  const renderRowInput = (row: RowDraft, index: number) => (
    <tr key={index} data-testid={`target-file-row-${index}`}>
      <td className="pr-2">
        {/* 空文件名 = 部署时补选（旧数据迁移行），用占位提示 */}
        <input
          aria-label={t("targets.files.file")}
          data-testid={`file-name-${index}`}
          value={row.fileName}
          placeholder={t("targets.files.filePlaceholder")}
          onChange={(e) => patchRow(index, { fileName: e.target.value })}
          className="w-full rounded-md border bg-background px-2 py-1.5 font-mono"
        />
      </td>
      <td className="pr-2">
        <input
          aria-label={t("targets.files.dir")}
          data-testid={`upload-dir-${index}`}
          value={row.uploadDir}
          placeholder="/srv/app"
          onChange={(e) => patchRow(index, { uploadDir: e.target.value })}
          className="w-full rounded-md border bg-background px-2 py-1.5 font-mono"
        />
      </td>
      <td>
        <Button
          size="sm"
          variant="outline"
          data-testid={`remove-row-${index}`}
          aria-label={t("targets.files.removeRow")}
          onClick={() => removeRow(index)}
        >
          {t("targets.files.removeRow")}
        </Button>
      </td>
    </tr>
  );

  const renderForm = (f: FormState) => {
    const problem = rowsProblem(f.rows);
    return (
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

            {/* 文件名→目录 映射表（ticket 19）：每行 = 构建产物文件名 → 服务器上传目录 */}
            <div className="flex flex-col gap-1">
              <span className="text-sm font-medium">{t("targets.files.title")}</span>
              <table className="w-full">
                <thead>
                  <tr className="text-left text-xs text-muted-foreground">
                    <th className="pr-2 font-normal">{t("targets.files.file")}</th>
                    <th className="pr-2 font-normal">{t("targets.files.dir")}</th>
                    <th />
                  </tr>
                </thead>
                <tbody>{f.rows.map((row, index) => renderRowInput(row, index))}</tbody>
              </table>
              <span className="text-xs text-muted-foreground">{t("targets.files.hint")}</span>
              <Button
                size="sm"
                variant="outline"
                className="self-start"
                data-testid="add-row"
                onClick={addRow}
              >
                {t("targets.files.addRow")}
              </Button>
            </div>

            {problem !== null && (
              <p role="alert" className="text-sm text-red-600 dark:text-red-400">
                {t(`targets.errors.${problem}`)}
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
  };

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
                        onClick={() => setConfirmDelete(target)}
                      >
                        {t("targets.delete")}
                      </Button>
                    </div>
                  </div>
                  {/* 映射行列表：文件名 → 目录；空名行显示（部署时补选） */}
                  <table className="mt-2 w-full text-sm" data-testid={`target-files-${target.id}`}>
                    <tbody>
                      {target.files.map((file) => (
                        <tr key={file.id}>
                          <td className="pr-3 font-mono">
                            {file.fileName === ""
                              ? t("targets.files.unnamed")
                              : file.fileName}
                          </td>
                          <td className="font-mono text-muted-foreground">{file.uploadDir}</td>
                        </tr>
                      ))}
                    </tbody>
                  </table>
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
      <ConfirmDialog
        open={confirmDelete !== null}
        onOpenChange={(open) => {
          if (!open) setConfirmDelete(null);
        }}
        title={t("targets.confirmDelete")}
        confirmLabel={t("common.confirm")}
        cancelLabel={t("common.cancel")}
        onConfirm={() => {
          if (confirmDelete !== null) void handleDelete(confirmDelete);
        }}
      />
      </CardContent>
    </Card>
  );
}

export default DeploymentTargetsPanel;
