import { useCallback, useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { invoke, Channel } from "@tauri-apps/api/core";
import { RefreshCw } from "lucide-react";
import { Button } from "@/components/ui/button";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import BuildPanel from "@/components/BuildPanel";
import type { Project } from "@/lib/buildApi";

type ProjectType = "maven" | "gradle";

interface BackendError {
  code: string;
  message: string;
}

interface CloneProgress {
  stage: string;
  received?: number;
  total?: number;
}

/** v1 默认工作区根目录（spec: 项目管理）。 */
export const DEFAULT_WORKSPACE_ROOT = "~/Packsmith/workspace";

function isBackendError(e: unknown): e is BackendError {
  return typeof e === "object" && e !== null && "code" in e;
}

function CreateProjectForm({
  onCreated,
  onCancel,
}: {
  onCreated: () => void;
  onCancel: () => void;
}) {
  const { t } = useTranslation();
  const [mode, setMode] = useState<"clone" | "import">("clone");
  const [name, setName] = useState("");
  const [url, setUrl] = useState("");
  const [path, setPath] = useState("");
  const [workspaceRoot, setWorkspaceRoot] = useState(DEFAULT_WORKSPACE_ROOT);
  const [projectType, setProjectType] = useState<ProjectType>("maven");
  const [error, setError] = useState<string | null>(null);
  const [progress, setProgress] = useState<CloneProgress | null>(null);
  const [busy, setBusy] = useState(false);

  /** 克隆进行中：取消按钮改为请求后端取消当前 clone。 */
  async function handleCancel() {
    if (busy && mode === "clone") {
      await invoke("cancel_clone", { name }).catch(() => {});
      return;
    }
    onCancel();
  }

  async function handleSubmit(e: React.FormEvent) {
    e.preventDefault();
    setError(null);
    setBusy(true);
    try {
      if (mode === "clone") {
        const onProgress = new Channel<CloneProgress>();
        onProgress.onmessage = (msg) => setProgress(msg);
        await invoke("clone_project", {
          name,
          url,
          workspaceRoot,
          projectType,
          onProgress,
        });
      } else {
        await invoke("import_project", { name, path, projectType });
      }
      onCreated();
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
    <Card className="mt-4">
      <CardHeader>
        <CardTitle>{t("projects.create.title")}</CardTitle>
      </CardHeader>
      <CardContent>
        <form onSubmit={handleSubmit} className="flex flex-col gap-3">
          <fieldset className="flex gap-4">
            <label className="flex items-center gap-1.5">
              <input
                type="radio"
                name="create-mode"
                value="clone"
                checked={mode === "clone"}
                onChange={() => setMode("clone")}
              />
              {t("projects.create.modeClone")}
            </label>
            <label className="flex items-center gap-1.5">
              <input
                type="radio"
                name="create-mode"
                value="import"
                checked={mode === "import"}
                onChange={() => setMode("import")}
              />
              {t("projects.create.modeImport")}
            </label>
          </fieldset>

          <label className="flex flex-col gap-1">
            {t("projects.create.name")}
            <input
              value={name}
              onChange={(e) => setName(e.target.value)}
              required
              className="rounded-md border bg-background px-2 py-1.5"
            />
          </label>

          <label className="flex flex-col gap-1">
            {t("projects.create.type")}
            <select
              value={projectType}
              onChange={(e) => setProjectType(e.target.value as ProjectType)}
              className="rounded-md border bg-background px-2 py-1.5"
            >
              <option value="maven">{t("projects.create.typeMaven")}</option>
              <option value="gradle" disabled>
                {t("projects.create.typeGradle")}
              </option>
            </select>
          </label>

          {mode === "clone" ? (
            <>
              <label className="flex flex-col gap-1">
                {t("projects.create.gitUrl")}
                <input
                  value={url}
                  onChange={(e) => setUrl(e.target.value)}
                  required
                  className="rounded-md border bg-background px-2 py-1.5"
                />
              </label>
              <label className="flex flex-col gap-1">
                {t("projects.create.workspaceRoot")}
                <input
                  value={workspaceRoot}
                  onChange={(e) => setWorkspaceRoot(e.target.value)}
                  className="rounded-md border bg-background px-2 py-1.5"
                />
              </label>
            </>
          ) : (
            <label className="flex flex-col gap-1">
              {t("projects.create.localPath")}
              <input
                value={path}
                onChange={(e) => setPath(e.target.value)}
                required
                className="rounded-md border bg-background px-2 py-1.5"
              />
            </label>
          )}

          {busy && mode === "clone" && (
            <p role="status" aria-live="polite">
              {progress?.total
                ? t("projects.create.cloningProgress", {
                    received: progress.received ?? 0,
                    total: progress.total,
                  })
                : t("projects.create.cloning")}
            </p>
          )}

          {error && (
            <p role="alert" className="text-destructive">
              {error}
            </p>
          )}

          <div className="flex gap-2">
            <Button type="submit" disabled={busy}>
              {mode === "clone"
                ? t("projects.create.submitClone")
                : t("projects.create.submitImport")}
            </Button>
            <Button type="button" variant="outline" onClick={handleCancel}>
              {busy && mode === "clone"
                ? t("projects.create.cancelClone")
                : t("projects.create.cancel")}
            </Button>
          </div>
        </form>
      </CardContent>
    </Card>
  );
}

function ProjectsPage() {
  const { t } = useTranslation();
  const [projects, setProjects] = useState<Project[] | null>(null);
  const [creating, setCreating] = useState(false);
  const [buildTarget, setBuildTarget] = useState<Project | null>(null);

  const refresh = useCallback(async () => {
    setProjects(await invoke<Project[]>("list_projects"));
  }, []);

  useEffect(() => {
    void refresh();
  }, [refresh]);

  return (
    <section>
      <div className="flex items-center justify-between">
        <h1 className="text-2xl font-semibold">{t("projects.title")}</h1>
        <div className="flex gap-2">
          {/* v1 预留：不做 pull / 远程刷新逻辑（spec: 明确不做） */}
          <Button
            variant="outline"
            size="icon"
            aria-label={t("projects.refresh")}
            title={t("projects.refreshPlaceholder")}
            onClick={() => {}}
          >
            <RefreshCw />
          </Button>
          <Button onClick={() => setCreating(true)}>
            {t("projects.create.button")}
          </Button>
        </div>
      </div>

      {buildTarget && (
        <BuildPanel project={buildTarget} onClose={() => setBuildTarget(null)} />
      )}

      {creating && (
        <CreateProjectForm
          onCreated={async () => {
            setCreating(false);
            await refresh();
          }}
          onCancel={() => setCreating(false)}
        />
      )}

      <Card className="mt-4">
        <CardHeader>
          <CardTitle>{t("projects.listTitle")}</CardTitle>
        </CardHeader>
        <CardContent>
          {projects && projects.length === 0 ? (
            <p>{t("projects.empty")}</p>
          ) : (
            <table className="w-full text-left">
              <thead>
                <tr className="text-muted-foreground">
                  <th className="py-1.5 pr-4 font-medium">{t("projects.columns.name")}</th>
                  <th className="py-1.5 pr-4 font-medium">{t("projects.columns.source")}</th>
                  <th className="py-1.5 pr-4 font-medium">{t("projects.columns.type")}</th>
                  <th className="py-1.5 pr-4 font-medium"></th>
                </tr>
              </thead>
              <tbody>
                {(projects ?? []).map((p) => (
                  <tr key={p.id} data-testid={`project-row-${p.name}`}>
                    <td className="py-1.5 pr-4">{p.name}</td>
                    <td className="py-1.5 pr-4">
                      {p.source === "git"
                        ? t("projects.source.git")
                        : t("projects.source.local")}
                    </td>
                    <td className="py-1.5 pr-4">
                      {t(`projects.type.${p.project_type}`)}
                    </td>
                    <td className="py-1.5 pr-4">
                      <Button
                        variant="outline"
                        size="sm"
                        onClick={() => setBuildTarget(p)}
                      >
                        {t("projects.build.button")}
                      </Button>
                    </td>
                  </tr>
                ))}
              </tbody>
            </table>
          )}
        </CardContent>
      </Card>
    </section>
  );
}

export default ProjectsPage;
