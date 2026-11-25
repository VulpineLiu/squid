import {
  ChevronDown,
  Clock3,
  ExternalLink,
  FolderOpen,
  History,
  RotateCcw,
  Trash2,
} from "lucide-react";
import { useEffect, useMemo, useState } from "react";
import { useI18n } from "../i18n";
import { api } from "../lib/api";
import {
  formatBytes,
  formatCount,
  formatDate,
  messageFrom,
  shortPath,
} from "../lib/format";
import type { HistorySession } from "../types";
import {
  Badge,
  Button,
  ConfirmDialog,
  EmptyState,
  IconButton,
  InlineError,
  PageHeader,
  Row,
  Section,
  Stat,
  useToast,
} from "../components/ui";

export function HistoryPage() {
  const { t, lang, bin } = useI18n();
  const { push } = useToast();
  const [sessions, setSessions] = useState<HistorySession[]>([]);
  const [error, setError] = useState<string | null>(null);
  const [loading, setLoading] = useState(true);
  const [expanded, setExpanded] = useState<string | null>(null);
  const [restore, setRestore] = useState<HistorySession | null>(null);
  const [restoring, setRestoring] = useState(false);

  const load = async () => {
    setLoading(true);
    try {
      setSessions(await api.historyList(100));
      setError(null);
    } catch (reason) {
      setError(messageFrom(reason));
    } finally {
      setLoading(false);
    }
  };

  useEffect(() => {
    void load();
    // The history page has no live source; load once when it opens.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  const totalKept = useMemo(
    () =>
      sessions.reduce(
        (sum, session) =>
          sum + (session.restored ? 0 : session.allocated_freed || session.logical_freed),
        0,
      ),
    [sessions],
  );

  const undo = async () => {
    if (!restore) return;
    setRestoring(true);
    try {
      const result = await api.historyUndo(restore.id);
      push(
        t(
          `${result.restored} items restored${result.skipped ? `, ${result.skipped} skipped` : ""}`,
          `已还原 ${result.restored} 项${result.skipped ? `，${result.skipped} 项跳过` : ""}`,
        ),
        result.restored > 0 ? "success" : "danger",
      );
      setRestore(null);
      await load();
    } catch (reason) {
      push(messageFrom(reason), "danger");
    } finally {
      setRestoring(false);
    }
  };

  return (
    <div className="page history-page">
      <PageHeader
        eyebrow={t("History", "历史记录")}
        title={t("Every change, on the record", "每一次改动，都有记录")}
        actions={
          <Button
            variant="secondary"
            icon={<FolderOpen size={15} />}
            onClick={() => void api.openTrash().catch((reason) => push(messageFrom(reason), "danger"))}
          >
            {t(`Open ${bin}`, `打开${bin}`)}
          </Button>
        }
      />

      {error ? <InlineError message={error} /> : null}

      {sessions.length > 0 ? (
        <div className="stat-row">
          <Stat label={t("Space kept clean", "当前已释放")} value={formatBytes(totalKept)} />
          <Stat label={t("Cleanup sessions", "清理记录")} value={formatCount(sessions.length)} />
          <Stat
            label={t("Restorable sessions", "可还原记录")}
            value={formatCount(
              sessions.filter(
                (session) => session.mode === "trash" && !session.restored,
              ).length,
            )}
          />
        </div>
      ) : null}

      {loading ? (
        <div className="history-loading">
          <Clock3 size={22} />
          {t("Reading cleanup history…", "正在读取清理历史…")}
        </div>
      ) : sessions.length === 0 ? (
        <EmptyState
          icon={<History size={28} />}
          title={t("No cleanups yet", "还没有清理记录")}
          description={t(
            "Each confirmed cleanup will appear here with its size, result and restore option.",
            "每次确认的清理都会记录在这里，并提供大小、结果与还原选项。",
          )}
        />
      ) : (
        <div className="history-list">
          {sessions.map((session) => {
            const bytes = session.allocated_freed || session.logical_freed;
            const canRestore =
              session.mode === "trash" &&
              !session.restored &&
              session.entries.some((entry) => entry.trash_path);
            const isOpen = expanded === session.id;
            return (
              <article key={session.id} className="section history-card">
                <Row
                  className="history-head"
                  leading={
                    <span className={`history-mode ${session.restored ? "is-restored" : ""}`}>
                      {session.restored ? <RotateCcw size={18} /> : <Trash2 size={18} />}
                    </span>
                  }
                  title={
                    session.command.replace(/^Squid · /, "") || t("Cleanup", "清理")
                  }
                  subtitle={`${formatDate(session.started, lang)} · ${
                    session.mode === "trash"
                      ? t(`Moved to ${bin}`, `已移入${bin}`)
                      : t("Permanently removed", "已永久删除")
                  }`}
                  tags={
                    session.restored ? (
                      <Badge tone="neutral">
                        <RotateCcw size={11} />
                        {t("Restored", "已还原")}
                      </Badge>
                    ) : undefined
                  }
                  trailing={
                    <>
                      <span className="history-count">
                        <strong>{formatBytes(bytes)}</strong>
                        <small>
                          {formatCount(session.removed)} {t("items", "项")}
                          {session.failed > 0
                            ? ` · ${session.failed} ${t("failed", "失败")}`
                            : ""}
                        </small>
                      </span>
                      <span className="history-time">
                        {(session.duration_ms / 1000).toFixed(1)}s
                      </span>
                      <ChevronDown
                        size={17}
                        className={`history-chevron ${isOpen ? "is-open" : ""}`}
                      />
                    </>
                  }
                  onClick={() => setExpanded(isOpen ? null : session.id)}
                />

                {isOpen ? (
                  <div className="history-detail">
                    <div className="history-detail-head">
                      <span>
                        {t(
                          `${session.entries.length} ledger entries`,
                          `${session.entries.length} 条记录`,
                        )}
                      </span>
                      {canRestore ? (
                        <Button
                          variant="secondary"
                          size="sm"
                          icon={<RotateCcw size={14} />}
                          onClick={() => setRestore(session)}
                        >
                          {t("Restore all", "全部还原")}
                        </Button>
                      ) : null}
                    </div>
                    <div className="row-list history-entries">
                      {session.entries.slice(0, 40).map((entry, index) => (
                        <Row
                          key={`${entry.path}-${index}`}
                          className="history-entry"
                          title={shortPath(entry.path, 78)}
                          subtitle={`${entry.category || entry.kind}${
                            entry.trash_path ? ` · ${t(`in ${bin}`, `位于${bin}`)}` : ""
                          }`}
                          trailing={
                            <>
                              <span>{formatBytes(entry.allocated || entry.logical)}</span>
                              <IconButton
                                label={t("Show in Finder", "在访达中显示")}
                                onClick={() =>
                                  void api.revealPath(entry.path).catch(() => undefined)
                                }
                              >
                                <ExternalLink size={13} />
                              </IconButton>
                            </>
                          }
                        />
                      ))}
                    </div>
                  </div>
                ) : null}
              </article>
            );
          })}
        </div>
      )}

      <ConfirmDialog
        open={Boolean(restore)}
        title={t("Restore this cleanup?", "还原这次清理？")}
        message={
          restore
            ? t(
                `${formatCount(restore.removed)} items · ${formatBytes(
                  restore.allocated_freed || restore.logical_freed,
                )}`,
                `${formatCount(restore.removed)} 项 · ${formatBytes(
                  restore.allocated_freed || restore.logical_freed,
                )}`,
              )
            : ""
        }
        note={t(
          `Files still in the ${bin} are returned to their original folders. Anything already emptied is skipped.`,
          `仍在${bin}中的文件会回到原位置；已被清空的项目会被跳过。`,
        )}
        confirmLabel={t("Restore files", "还原文件")}
        cancelLabel={t("Cancel", "取消")}
        onClose={() => setRestore(null)}
        onConfirm={() => void undo()}
        busy={restoring}
        tone="default"
        icon={<RotateCcw size={22} />}
      />
    </div>
  );
}
