import {
  ArrowUpRight,
  FolderOpen,
  LockKeyhole,
  RotateCcw,
  ShieldCheck,
  Trash2,
} from "lucide-react";
import { useEffect, useMemo, useState } from "react";
import { useI18n } from "../i18n";
import { api } from "../lib/api";
import { formatBytes, formatCount, shortPath } from "../lib/format";
import type { Category, CleanupItem, DeleteMode, ScanResult } from "../types";
import type { ScanWorkspace } from "../hooks/useScanWorkspace";
import {
  Button,
  CATEGORY_META,
  CATEGORY_ORDER,
  CategoryCard,
  Checkbox,
  ConfirmDialog,
  InlineError,
  ProgressCard,
  ResultSummary,
  SectionTitle,
  categoryLabel,
  lockLabel,
  useToast,
} from "./ui";

type CategoryGroup = { total: number; count: number; items: number[]; selected: number };

function grouped(result: ScanResult) {
  const groups = new Map<Category, CategoryGroup>();
  result.items.forEach((item, index) => {
    const group = groups.get(item.category) ?? {
      total: 0,
      count: 0,
      items: [],
      selected: 0,
    };
    group.total += item.onDisk;
    group.count += 1;
    group.items.push(index);
    group.selected += 1;
    groups.set(item.category, group);
  });
  return groups;
}

export function CategorySummary({
  result,
  selected,
  onToggle,
  onInspect,
}: {
  result: ScanResult;
  selected: Set<number>;
  onToggle: (category: Category) => void;
  onInspect: (category: Category) => void;
}) {
  const groups = useMemo(() => grouped(result), [result]);
  return (
    <div className="category-grid">
      {CATEGORY_ORDER.filter((category) => groups.has(category)).map((category) => {
        const group = groups.get(category)!;
        const indexes = group.items.filter((index) => !result.items[index].lock);
        const allSelected = indexes.length > 0 && indexes.every((index) => selected.has(index));
        return (
          <CategoryCard
            key={category}
            category={category}
            total={group.total}
            count={group.count}
            selected={allSelected}
            disabled={indexes.length === 0}
            onToggle={() => onToggle(category)}
            onInspect={() => onInspect(category)}
          />
        );
      })}
    </div>
  );
}

function ItemRow({
  item,
  checked,
  onToggle,
}: {
  item: CleanupItem;
  checked: boolean;
  onToggle: () => void;
}) {
  const { lang, t } = useI18n();
  const Icon = CATEGORY_META[item.category].icon;
  return (
    <div
      className={`cleanup-row ${item.lock ? "is-locked" : ""} ${
        checked ? "is-selected" : ""
      }`}
      onClick={item.lock ? undefined : onToggle}
    >
      <Checkbox
        checked={checked}
        disabled={Boolean(item.lock)}
        onChange={onToggle}
        label={item.label}
      />
      <span className={`row-icon tone-${CATEGORY_META[item.category].tone}`}>
        <Icon size={17} />
      </span>
      <span className="cleanup-row-copy">
        <span className="cleanup-row-title">
          <strong>{item.label}</strong>
          <span className="row-category">{categoryLabel(item.category, lang)}</span>
        </span>
        <span className="cleanup-row-path" title={item.displayPath}>
          {shortPath(item.displayPath, 90)}
        </span>
        {item.lock || item.note ? (
          <span className={`cleanup-row-note ${item.lock ? "is-locked" : ""}`}>
            {item.lock ? <LockKeyhole size={12} /> : null}
            {item.lock ? lockLabel(item.lock, t) : item.note}
          </span>
        ) : null}
      </span>
      <span className="cleanup-row-size">{formatBytes(item.onDisk)}</span>
      <button
        className="row-reveal"
        title={t("Show in Finder", "在访达中显示")}
        aria-label={t("Show in Finder", "Show in Finder")}
        onClick={(event) => {
          event.stopPropagation();
          void api.revealPath(item.path).catch(() => undefined);
        }}
      >
        <ArrowUpRight size={15} />
      </button>
    </div>
  );
}

export function CleanupInventory({
  workspace,
  defaultMode,
  onHistory,
}: {
  workspace: ScanWorkspace;
  defaultMode: DeleteMode;
  onHistory: () => void;
}) {
  const { t, lang, bin } = useI18n();
  const { push } = useToast();
  const [focus, setFocus] = useState<Category | null>(null);
  const [confirming, setConfirming] = useState(false);
  const [mode, setMode] = useState<DeleteMode>(defaultMode);

  useEffect(() => setMode(defaultMode), [defaultMode]);

  const result = workspace.result;
  const groups = useMemo(
    () => (result ? grouped(result) : new Map<Category, CategoryGroup>()),
    [result],
  );
  const visibleItems = useMemo(() => {
    if (!result) return [];
    return result.items
      .map((item, index) => ({ item, index }))
      .filter(({ item }) => !focus || item.category === focus);
  }, [focus, result]);
  const removables = result?.items.filter((item) => !item.lock).length ?? 0;

  if (workspace.scanning) {
    return (
      <ProgressCard
        progress={workspace.progress}
        title={t("Scanning your computer safely…", "正在安全扫描您的电脑…")}
        onCancel={() => void workspace.cancel()}
      />
    );
  }

  if (workspace.summary) {
    return (
      <ResultSummary
        removed={workspace.summary.removed}
        freed={workspace.summary.freedAllocated || workspace.summary.freed}
        failed={workspace.summary.failed}
        mode={workspace.summary.mode}
        onAgain={() => void workspace.runScan()}
        onHistory={onHistory}
      />
    );
  }

  if (!result) return null;

  const confirm = async () => {
    const summary = await workspace.clean(mode);
    setConfirming(false);
    if (summary) {
      push(
        t(
          `${formatBytes(summary.freedAllocated || summary.freed)} cleaned`,
          `已清理 ${formatBytes(summary.freedAllocated || summary.freed)}`,
        ),
        "success",
      );
    }
  };

  return (
    <>
      <div className="scan-result-hero">
        <div>
          <span className="result-kicker">
            <ShieldCheck size={14} />
            {t("Scan complete", "扫描完成")}
          </span>
          <h2>
            {formatBytes(workspace.selectedTotal)}
            <small>{t(" ready to clean", " 可安全清理")}</small>
          </h2>
          <p>
            {t(
              `${formatCount(workspace.selectedCount)} of ${formatCount(
                removables,
              )} available items selected`,
              `已选择 ${formatCount(workspace.selectedCount)} / ${formatCount(
                removables,
              )} 个可清理项目`,
            )}
            {result.unreadable > 0
              ? t(
                  ` · ${formatCount(result.unreadable)} could not be read`,
                  ` · ${formatCount(result.unreadable)} 项无法读取`,
                )
              : ""}
          </p>
        </div>
        <div className="scan-result-actions">
          <Button
            variant="ghost"
            onClick={workspace.clearSelection}
            disabled={workspace.selectedCount === 0}
          >
            {t("Deselect all", "取消全选")}
          </Button>
          <Button
            variant="primary"
            icon={<Trash2 size={16} />}
            disabled={workspace.selectedCount === 0}
            onClick={() => setConfirming(true)}
          >
            {t("Review & clean", "检查并清理")}
          </Button>
        </div>
      </div>

      {workspace.error ? (
        <InlineError message={workspace.error} onDismiss={workspace.clearError} />
      ) : null}

      <div className="category-grid">
        {CATEGORY_ORDER.filter((category) => groups.has(category)).map((category) => {
          const group = groups.get(category);
          if (!group) return null;
          const indexes = group.items.filter((index) => !result.items[index].lock);
          const allSelected =
            indexes.length > 0 && indexes.every((index) => workspace.selected.has(index));
          return (
            <CategoryCard
              key={category}
              category={category}
              total={group.total}
              count={group.count}
              selected={allSelected}
              focused={focus === category}
              disabled={indexes.length === 0}
              onToggle={() => workspace.toggleCategory(category)}
              onInspect={() => setFocus(focus === category ? null : category)}
            />
          );
        })}
      </div>

      <section className="cleanup-list-panel">
        <SectionTitle
          title={
            focus
              ? categoryLabel(focus, lang)
              : t("Everything found in this scan", "本次扫描的全部项目")
          }
          action={
            focus ? (
              <Button variant="ghost" size="sm" onClick={() => setFocus(null)}>
                {t("Show all", "显示全部")}
              </Button>
            ) : (
              <Button variant="ghost" size="sm" onClick={workspace.selectAll}>
                {workspace.selectedCount === removables
                  ? t("Deselect all", "取消全选")
                  : t("Select all", "全部选择")}
              </Button>
            )
          }
        />
        <div className="cleanup-rows">
          {visibleItems.map(({ item, index }) => (
            <ItemRow
              key={`${item.path}-${index}`}
              item={item}
              checked={workspace.selected.has(index)}
              onToggle={() => workspace.toggleItem(index)}
            />
          ))}
        </div>
      </section>

      <div className="floating-action">
        <div>
          <span>{t("Selected for cleanup", "已选择清理")}</span>
          <strong>{formatBytes(workspace.selectedTotal)}</strong>
        </div>
        <div className="floating-actions">
          <Button
            variant="ghost"
            size="sm"
            icon={<FolderOpen size={15} />}
            onClick={() => void api.openTrash().catch(() => undefined)}
          >
            {bin}
          </Button>
          <Button
            variant="primary"
            icon={<Trash2 size={15} />}
            disabled={workspace.selectedCount === 0}
            onClick={() => setConfirming(true)}
          >
            {t("Clean now", "立即清理")}
          </Button>
        </div>
      </div>

      <ConfirmDialog
        open={confirming}
        title={t("Ready to make room?", "准备释放空间？")}
        message={
          <span>
            {t(
              `${formatCount(workspace.selectedCount)} items · ${formatBytes(
                workspace.selectedTotal,
              )}`,
              `${formatCount(workspace.selectedCount)} 项 · ${formatBytes(
                workspace.selectedTotal,
              )}`,
            )}
          </span>
        }
        note={
          mode === "trash"
            ? t(
                `Items will move to the ${bin}. You can restore them from History.`,
                `文件将移入${bin}，之后可从历史记录中还原。`,
              )
            : t(
                "Permanent removal cannot be undone.",
                "永久删除无法撤销。",
              )
        }
        confirmLabel={
          mode === "trash"
            ? t(`Move to ${bin}`, `移入${bin}`)
            : t("Remove permanently", "永久删除")
        }
        cancelLabel={t("Keep files", "保留文件")}
        onConfirm={() => void confirm()}
        onClose={() => setConfirming(false)}
        busy={workspace.deleting}
        icon={<RotateCcw size={23} />}
      >
        <div className="mode-picker">
          <button
            className={mode === "trash" ? "is-active" : ""}
            onClick={() => setMode("trash")}
          >
            <FolderOpen size={17} />
            <span>{bin}</span>
          </button>
          <button
            className={mode === "permanent" ? "is-active" : ""}
            onClick={() => setMode("permanent")}
          >
            <Trash2 size={17} />
            <span>{t("Permanent", "永久")}</span>
          </button>
        </div>
      </ConfirmDialog>
    </>
  );
}
