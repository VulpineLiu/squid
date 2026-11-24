import {
  CopyX,
  FileQuestion,
  FolderPlus,
  Lock,
  MapPin,
  ShieldAlert,
  Sparkles,
  Trash2,
  X,
} from "lucide-react";
import { useEffect, useState } from "react";
import { open } from "@tauri-apps/plugin-dialog";
import * as apiClient from "../lib/api";
import { useI18n } from "../i18n";
import { agoLabel, formatBytes, formatCount } from "../lib/format";
import { keepIndexOf, useDuplicatesWorkspace } from "../hooks/useDuplicatesWorkspace";
import {
  Button,
  Checkbox,
  EmptyState,
  InlineError,
  PageHeader,
  ProgressCard,
  useToast,
} from "../components/ui";
import type { DupGroup } from "../types";

const MIN_SIZE_OPTIONS = [
  { value: 1024 * 1024, label: "≥1MB" },
  { value: 10 * 1024 * 1024, label: "≥10MB" },
  { value: 100 * 1024 * 1024, label: "≥100MB" },
];

function FunnelStage({
  value,
  label,
  hot,
}: {
  value: number;
  label: string;
  hot?: boolean;
}) {
  return (
    <div className={`dup-stage ${hot ? "is-hot" : ""}`}>
      <b>{formatCount(value)}</b>
      <span>{label}</span>
    </div>
  );
}

function DupGroupCard({
  group,
  index,
  keep,
  picked,
  lang,
  onToggle,
  onDismiss,
}: {
  group: DupGroup;
  index: number;
  keep: number;
  picked: Set<number> | undefined;
  lang: "zh" | "en";
  onToggle: (groupIndex: number, copyIndex: number) => void;
  onDismiss: (groupIndex: number) => void;
}) {
  const { t } = useI18n();
  const wasted = group.size * (group.copies.length - 1);
  return (
    <section className={`dup-group ${group.personal ? "is-personal" : ""}`}>
      <header className="dup-group-head">
        <div className="dup-group-title">
          <span className="dup-group-count">
            {group.copies.length} {t("copies", "份相同")}
          </span>
          <b>{group.copies[keep]?.path.split(/[\\/]/).pop()}</b>
          <span className="dup-waste-chip">
            {t("wastes", "重复浪费")} {formatBytes(wasted)}
          </span>
          {group.personal ? (
            <span className="dup-personal-chip">
              <Lock size={11} />
              {t("personal files", "个人文件")}
            </span>
          ) : null}
        </div>
        <div className="dup-group-size">
          {formatBytes(group.size)} {t("each", "/ 份")}
        </div>
        <button
          type="button"
          className="dup-group-dismiss"
          title={t("Ignore this group", "忽略本组")}
          onClick={() => onDismiss(index)}
        >
          <X size={13} />
        </button>
      </header>
      <ul className="dup-copies">
        {group.copies.map((copy, copyIndex) => {
          const isKeep = copyIndex === keep;
          const checked = picked?.has(copyIndex) ?? false;
          return (
            <li
              key={copy.path}
              className={`dup-copy ${checked ? "is-picked" : ""} ${isKeep ? "is-keep" : ""}`}
            >
              <Checkbox
                checked={checked}
                disabled={Boolean(copy.locked) || isKeep}
                onChange={() => onToggle(index, copyIndex)}
                label={copy.path}
              />
              {isKeep ? <span className="dup-keep-chip">{t("keep", "保留")}</span> : null}
              {copy.locked ? (
                <span className="dup-lock-chip" title={copy.locked}>
                  <Lock size={11} />
                </span>
              ) : null}
              <span className="dup-copy-path" title={copy.path}>
                {copy.path}
              </span>
              <span className="dup-copy-age">
                {copy.mtimeMs ? agoLabel(copy.mtimeMs, lang) : ""}
              </span>
              <button
                type="button"
                className="dup-copy-reveal"
                onClick={() => void apiClient.api.revealPath(copy.path).catch(() => undefined)}
                title={t("Reveal in Explorer", "在文件夹中显示")}
              >
                <MapPin size={12} />
                {t("Reveal", "定位")}
              </button>
            </li>
          );
        })}
      </ul>
    </section>
  );
}

export function DuplicateFilesPage({
  embedded = false,
  onBadge,
}: {
  /** Rendered inside 空间清理 hub: page title hides, stats flow to onBadge. */
  embedded?: boolean;
  onBadge?: (value: string | null) => void;
} = {}) {
  const { t, lang } = useI18n();
  const { push } = useToast();
  const workspace = useDuplicatesWorkspace();
  const [minSize, setMinSize] = useState(1024 * 1024);
  const [extraRoots, setExtraRoots] = useState<string[]>([]);
  const [confirming, setConfirming] = useState(false);

  const entries = workspace.visible;
  const groups = entries.map((entry) => entry.group);
  const report = workspace.result?.report;

  const addRoot = async () => {
    const chosen = await open({
      directory: true,
      multiple: true,
      title: t("Add folders to the duplicate scan", "选择要一起扫描的文件夹"),
    });
    if (Array.isArray(chosen)) setExtraRoots((current) => [...current, ...chosen]);
    else if (typeof chosen === "string") setExtraRoots((current) => [...current, chosen]);
  };

  const start = () => {
    setConfirming(false);
    void workspace.runScan(extraRoots, minSize);
  };

  const confirmClean = async () => {
    const outcome = await workspace.clean("trash");
    setConfirming(false);
    if (outcome) {
      push(
        lang === "zh"
          ? `已将 ${outcome.removed} 个副本移入回收站（${formatBytes(outcome.freed)}），可在「历史记录」撤销`
          : `Moved ${outcome.removed} copies to the Recycle Bin (${formatBytes(outcome.freed)}) — undo from History`,
        "success",
      );
    }
  };

  // Totals next to the toolbar follow the *visible* groups: an ignored group
  // stops counting toward what the page offers to reclaim.
  const visibleWasted = groups.reduce((sum, g) => sum + g.size * (g.copies.length - 1), 0);

  // Hub badge: confirmed groups after ignores, or null before a scan.
  useEffect(() => {
    if (!embedded || !onBadge) return;
    onBadge(groups.length > 0 ? `${groups.length} 组 · ${formatBytes(visibleWasted)}` : null);
  }, [embedded, groups.length, onBadge, visibleWasted]);

  return (
    <div className={`page dup-page ${embedded ? "is-embedded" : ""}`}>
      {embedded ? null : (
        <PageHeader
          title={t("Duplicate files", "重复文件")}
          description={t(
            "Byte-identical copies proven by a three-stage hash funnel. Deletions land in the Recycle Bin.",
            "大小分组 → 头中尾采样 → 全量校验，三级漏斗逐字节确认。删除全部走回收站，可撤销。",
          )}
        />
      )}
      {workspace.error ? <InlineError message={workspace.error} onDismiss={workspace.clearError} /> : null}

      <section className="dup-config">
        <span className="dup-config-label">{t("Scan scope", "扫描范围")}</span>
        <span className="dup-root-chip is-static">
          {t(
            "Home folders + all drives (system folders skipped)",
            "用户目录 + 各磁盘（自动跳过系统与软件数据目录）",
          )}
        </span>
        {extraRoots.map((root) => (
          <span className="dup-root-chip" key={root} title={root}>
            {root}
            <button
              type="button"
              onClick={() => setExtraRoots((current) => current.filter((r) => r !== root))}
              aria-label={t("Remove", "移除")}
            >
              <X size={11} />
            </button>
          </span>
        ))}
        <button type="button" className="dup-root-add" onClick={() => void addRoot()}>
          <FolderPlus size={13} /> {t("Add folder", "添加文件夹")}
        </button>
        <span className="dup-config-spacer" />
        <span className="dup-config-label">{t("Min size", "最小体积")}</span>
        <div className="dup-seg" role="group">
          {MIN_SIZE_OPTIONS.map((option) => (
            <button
              key={option.value}
              type="button"
              className={minSize === option.value ? "is-active" : ""}
              onClick={() => setMinSize(option.value)}
            >
              {option.label}
            </button>
          ))}
        </div>
        <Button variant="primary" onClick={start} disabled={workspace.scanning || workspace.deleting}>
          <span className="dup-start-icon">
            {workspace.scanning ? t("Scanning…", "扫描中…") : t("Scan duplicates", "开始查找")}
          </span>
        </Button>
      </section>

      {workspace.scanning || (workspace.funnel.walked > 0 && groups.length > 0) ? (
        <section className="dup-funnel" aria-live="polite">
          <div className="dup-funnel-row">
            <FunnelStage value={workspace.funnel.walked} label={t("files walked", "已遍历文件")} />
            <i>→</i>
            <FunnelStage
              value={workspace.funnel.sameSize}
              label={t("same-size candidates", "同大小候选")}
            />
            <i>→</i>
            <FunnelStage value={workspace.funnel.sampleHits} label={t("sample hits", "采样哈希命中")} />
            <i>→</i>
            <FunnelStage
              value={workspace.funnel.confirmed}
              label={t("full-hash confirmed", "全量校验确认")}
              hot
            />
            <div className="dup-funnel-total">
              {t("duplicates found", "已发现重复")}{" "}
              <b>{formatCount(groups.length)}</b> {t("groups", "组")} ·{" "}
              {t("recoverable", "可回收")} <b>{formatBytes(visibleWasted)}</b>
            </div>
          </div>
          {workspace.progress?.subject ? (
            <div className="dup-funnel-current">
              {t("Checking", "正在校验")} {workspace.progress.subject}
            </div>
          ) : null}
        </section>
      ) : null}

      {workspace.scanning ? (
        <ProgressCard
          progress={workspace.progress}
          title={t("Scanning for duplicates", "正在查找重复文件")}
          onCancel={() => void workspace.cancel()}
          capAt99
        />
      ) : null}

      {!workspace.scanning && groups.length === 0 && !workspace.result && !workspace.error ? (
        <EmptyState
          icon={<FileQuestion size={26} />}
          title={t("No duplicate scan yet", "还没有扫描过重复文件")}
          description={t(
            "The first scan walks your files once; later scans reuse the hash cache and only re-check what changed.",
            "首次扫描遍历一遍文件；之后的扫描复用哈希缓存，只重算发生变化的文件。",
          )}
        />
      ) : null}

      {!workspace.scanning && groups.length === 0 && workspace.result ? (
        <EmptyState
          icon={<CopyX size={26} />}
          title={t("No duplicate files found", "没有发现重复文件")}
          description={
            lang === "zh"
              ? `已检查 ${formatCount(workspace.result.report.walked)} 个达到体积门槛的文件，无重复。`
              : `${formatCount(workspace.result.report.walked)} files checked above the size floor.`
          }
        />
      ) : null}

      {!workspace.scanning && groups.length > 0 ? (
        <>
          <section className="dup-toolbar">
            <button type="button" className="dup-smart" onClick={() => workspace.smartSelect()}>
              <Sparkles size={13} />{" "}
              {t("Smart pick (keep one per group)", "智能选择（每组保留一份）")}
            </button>
            <button
              type="button"
              className="dup-strategy"
              onClick={() =>
                workspace.setKeepStrategy(
                  workspace.keepStrategy === "shortest" ? "oldest" : "shortest",
                )
              }
              title={t("Which copy survives when picking smartly", "智能选择保留哪一份")}
            >
              {workspace.keepStrategy === "shortest"
                ? t("keep: shortest path", "保留：路径最短")
                : t("keep: oldest copy", "保留：最早的一份")}
            </button>
            <button type="button" className="dup-invert" onClick={workspace.invertSelection}>
              {t("Invert", "反选")}
            </button>
            <span className="dup-toolbar-sum">
              {formatCount(groups.length)} {t("groups · duplicate copies", "组 · 重复副本")}{" "}
              {formatCount(groups.reduce((sum, g) => sum + g.copies.length - 1, 0))} ·{" "}
              {formatBytes(visibleWasted)}
            </span>
            <Button
              variant="danger"
              className="dup-clean"
              disabled={workspace.selectedCount === 0 || workspace.deleting}
              onClick={() => setConfirming(true)}
            >
              <Trash2 size={14} />
              <span>
                {workspace.deleting
                  ? t("Cleaning…", "清理中…")
                  : `${t("Clean", "清理")} ${formatCount(workspace.selectedCount)} ${t(
                      "copies",
                      "个副本",
                    )} · ${formatBytes(workspace.selectedBytes)}`}
              </span>
            </Button>
          </section>
          {confirming ? (
            <section className="dup-confirm">
              <ShieldAlert size={15} />
              <div>
                <b>
                  {lang === "zh"
                    ? `将 ${workspace.selectedCount} 个副本（${formatBytes(workspace.selectedBytes)}）移入回收站？`
                    : `Move ${workspace.selectedCount} copies (${formatBytes(workspace.selectedBytes)}) to the Recycle Bin?`}
                </b>
                <small>
                  {t(
                    "Every group keeps at least one copy; a mis-click is one Undo away from History.",
                    "每组至少保留一份；误删可从「历史记录 → 撤销」找回。",
                  )}
                </small>
              </div>
              <Button onClick={() => setConfirming(false)}>{t("Cancel", "取消")}</Button>
              <Button variant="danger" onClick={() => void confirmClean()}>
                {t("Confirm", "确认清理")}
              </Button>
            </section>
          ) : null}
          <div className="dup-groups">
            {entries.map(({ group, index }) => (
              <DupGroupCard
                key={`${group.size}-${group.copies[0]?.path}-${index}`}
                group={group}
                index={index}
                keep={keepIndexOf(group, workspace.keepStrategy)}
                picked={workspace.picks.get(index)}
                lang={lang}
                onToggle={workspace.toggleCopy}
                onDismiss={workspace.removeGroup}
              />
            ))}
          </div>
        </>
      ) : null}
    </div>
  );
}
