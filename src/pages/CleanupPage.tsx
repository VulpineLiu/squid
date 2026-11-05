import { ScanSearch, Sparkles } from "lucide-react";
import { useI18n } from "../i18n";
import { formatBytes } from "../lib/format";
import type { DeleteMode } from "../types";
import type { ScanWorkspace } from "../hooks/useScanWorkspace";
import { CleanupInventory } from "../components/CleanupView";
import { Button, EmptyState, PageHeader } from "../components/ui";

export function CleanupPage({
  workspace,
  deleteMode,
  onHistory,
}: {
  workspace: ScanWorkspace;
  deleteMode: DeleteMode;
  onHistory: () => void;
}) {
  const { t } = useI18n();
  return (
    <div className="page">
      <PageHeader
        eyebrow={t("Cleanup", "清理")}
        title={t("Clutter, without the guesswork.", "清理垃圾，不靠猜。")}
        description={t(
          "Rebuildable caches, old logs and other safe-to-remove files — always reviewed before deletion.",
          "可重建缓存、旧日志与其他可安全删除的文件，删除前始终由您确认。",
        )}
        actions={
          workspace.result ? (
            <Button
              variant="secondary"
              icon={<ScanSearch size={16} />}
              onClick={() => void workspace.runScan()}
              disabled={workspace.scanning || workspace.deleting}
            >
              {t("Scan again", "重新扫描")}
            </Button>
          ) : null
        }
      />

      {workspace.error && !workspace.scanning ? (
        <div className="error-banner">{workspace.error}</div>
      ) : null}

      <CleanupInventory
        workspace={workspace}
        defaultMode={deleteMode}
        onHistory={onHistory}
      />

      {!workspace.result && !workspace.scanning && !workspace.summary ? (
        <EmptyState
          icon={<Sparkles size={28} />}
          title={t("Ready when you are", "随时可以开始")}
          description={t(
            "Squid checks known cache and log locations, measures what can be reclaimed, and locks anything it cannot remove safely.",
            "Squid 会检查已知缓存与日志位置，统计可释放空间，并锁定任何无法安全删除的项目。",
          )}
          action={
            <Button
              variant="primary"
              icon={<ScanSearch size={16} />}
              onClick={() => void workspace.runScan()}
            >
              {t("Scan for junk", "扫描系统垃圾")}
            </Button>
          }
        />
      ) : null}

      {workspace.result && workspace.selectedTotal > 0 ? (
        <p className="page-footnote">
          <span>
            {t(
              `${formatBytes(workspace.selectedTotal)} selected. Nothing is removed until you confirm.`,
              `已选择 ${formatBytes(workspace.selectedTotal)}，在您确认前不会删除任何内容。`,
            )}
          </span>
        </p>
      ) : null}
    </div>
  );
}
