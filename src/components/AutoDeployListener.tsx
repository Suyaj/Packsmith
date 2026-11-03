// 自动连锁部署通知（ticket 10）：监听 `deploy-event` 应用事件，为每个后台
// 自动部署显示一条轻量状态行（进行中/完成），用户可关闭。

import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { listenAutoDeploy, type AutoDeployMessage, type TargetStatus } from "@/lib/deployApi";

type AutoDeployState = {
  deployId: number;
  statuses: Map<number, TargetStatus>;
  done: boolean;
};

function AutoDeployListener() {
  const { t } = useTranslation();
  const [deploys, setDeploys] = useState<Map<number, AutoDeployState>>(new Map());
  const [dismissed, setDismissed] = useState<Set<number>>(new Set());

  useEffect(() => {
    let unlisten: (() => void) | undefined;
    let disposed = false;
    void listenAutoDeploy((message: AutoDeployMessage) => {
      setDeploys((prev) => {
        const next = new Map(prev);
        const current =
          next.get(message.deployId) ??
          { deployId: message.deployId, statuses: new Map<number, TargetStatus>(), done: false };
        const statuses = new Map(current.statuses);
        if (message.event.type === "target_finished") {
          statuses.set(message.event.target_id, message.event.status);
        }
        const done = message.event.type === "finished" ? true : current.done;
        next.set(message.deployId, { ...current, statuses, done });
        return next;
      });
    }).then((fn) => {
      if (disposed) fn();
      else unlisten = fn;
    });
    return () => {
      disposed = true;
      unlisten?.();
    };
  }, []);

  const visible = [...deploys.values()].filter((d) => !dismissed.has(d.deployId));
  if (visible.length === 0) return null;

  return (
    <div
      data-testid="auto-deploy-listener"
      className="fixed bottom-4 right-4 z-50 flex flex-col gap-2"
    >
      {visible.map((deploy) => {
        const statuses = [...deploy.statuses.values()];
        const summary = statuses.length
          ? `${statuses.filter((s) => s === "success").length}/${statuses.length}`
          : t("deploy.autoRunning");
        return (
          <div
            key={deploy.deployId}
            className="rounded-md border bg-background p-3 text-sm shadow-md"
          >
            <div className="flex items-center gap-2">
              <span>
                {deploy.done
                  ? t("deploy.autoFinished", { summary })
                  : t("deploy.autoRunningTitle", { summary })}
              </span>
              {deploy.done && (
                <button
                  type="button"
                  className="ml-2 text-xs text-muted-foreground underline"
                  data-testid={`auto-deploy-dismiss-${deploy.deployId}`}
                  onClick={() =>
                    setDismissed((prev) => new Set(prev).add(deploy.deployId))
                  }
                >
                  {t("deploy.autoDismiss")}
                </button>
              )}
            </div>
          </div>
        );
      })}
    </div>
  );
}

export default AutoDeployListener;
