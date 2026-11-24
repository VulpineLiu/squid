import { ClipboardList, EyeOff, FileClock, History, Lock, ShieldQuestion } from "lucide-react";
import { useCallback, useEffect, useState } from "react";
import { api } from "../lib/api";
import { useI18n } from "../i18n";
import { formatBytes, formatCount, messageFrom } from "../lib/format";
import {
  Button,
  Checkbox,
  EmptyState,
  InlineError,
  PageHeader,
  useToast,
} from "../components/ui";
import type { CleanupItem } from "../types";

function jobId(): string {
  return crypto.randomUUID?.() ?? `${Date.now()}-${Math.random().toString(36).slice(2)}`;
}

const SPEC_ICONS: Record<string, typeof History> = {
  "privacy-edge": History,
  "privacy-chrome": History,
  "privacy-recent": FileClock,
  "privacy-clipboard": ClipboardList,
  "privacy-activity": EyeOff,
};

const WARN_COPY: Record<string, [string, string]> = {
  "privacy-edge": ["影响「恢复上次会话」", "affects session restore"],
  "privacy-chrome": ["影响「恢复上次会话」", "affects session restore"],
  "privacy-clipboard": ["Win+V 历史会被清空", "clears the Win+V list"],
  "privacy-activity": ["影响「恢复上次会话」", "affects session restore"],
};

interface SpecGroup {
  ruleId: string;
  label: string;
  detail: string | null;
  indexes: number[];
  bytes: number;
  locked: string | null;
}

export function PrivacyPage({
  embedded = false,
  onBadge,
}: {
  /** Rendered inside 系统维护 hub: page title hides, trace count flows up. */
  embedded?: boolean;
  onBadge?: (value: string | null) => void;
} = {}) {
  const { t, lang } = useI18n();
  const { push } = useToast();
  const [items, setItems] = useState<CleanupItem[]>([]);
  const [scanId, setScanId] = useState<string | null>(null);
  const [picked, setPicked] = useState<Set<number>>(new Set());
  const [scanning, setScanning] = useState(false);
  const [cleaning, setCleaning] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [loaded, setLoaded] = useState(false);

  const scan = useCallback(async () => {
    setScanning(true);
    setError(null);
    const id = jobId();
    try {
      const view = await api.privacyScan(id);
      setItems(view.items);
      setScanId(view.scanId);
      setPicked(new Set());
    } catch (reason) {
      setError(messageFrom(reason));
    } finally {
      setScanning(false);
      setLoaded(true);
    }
  }, []);

  useEffect(() => {
    void scan();
  }, [scan]);

  const groups: SpecGroup[] = [];
  items.forEach((item, index) => {
    const last = groups[groups.length - 1];
    if (last && last.ruleId === item.ruleId) {
      last.indexes.push(index);
      last.bytes += item.onDisk;
      if (item.lock && !last.locked) {
        last.locked = item.note ?? item.lock;
      }
    } else {
      groups.push({
        ruleId: item.ruleId,
        label: item.label,
        detail: item.note,
        indexes: [index],
        bytes: item.onDisk,
        locked: item.lock ? item.note ?? item.lock : null,
      });
    }
  });

  const toggleGroup = (group: SpecGroup) => {
    if (group.locked) return;
    setPicked((current) => {
      const next = new Set(current);
      const allOn = group.indexes.every((index) => next.has(index));
      for (const index of group.indexes) {
        if (allOn) next.delete(index);
        else next.add(index);
      }
      return next;
    });
  };

  const pickedBytes = [...picked].reduce((sum, index) => sum + (items[index]?.onDisk ?? 0), 0);

  // Hub badge: trace groups found by the auto-scan, or null when clean.
  useEffect(() => {
    if (!embedded || !onBadge) return;
    onBadge(groups.length > 0 ? `${groups.length} 类痕迹` : null);
  }, [embedded, groups.length, onBadge]);

  const clean = async () => {
    if (!scanId || picked.size === 0) return;
    setCleaning(true);
    try {
      const outcome = await api.privacyDelete(jobId(), scanId, [...picked]);
      push(
        lang === "zh"
          ? `已清除 ${outcome.removed} 项隐私痕迹（${formatBytes(outcome.freed)}）；这些删除不进回收站，明细见历史记录`
          : `Cleared ${outcome.removed} traces (${formatBytes(outcome.freed)}); these deletions skip the Recycle Bin — see History`,
        "success",
      );
      await scan();
    } catch (reason) {
      push(messageFrom(reason), "danger");
    } finally {
      setCleaning(false);
    }
  };

  return (
    <div className={`page privacy-page ${embedded ? "is-embedded" : ""}`}>
      {embedded ? null : (
        <PageHeader
          title={t("Privacy cleaning", "隐私清理")}
          description={t(
            "History and traces leave the machine for good — this page never uses the Recycle Bin, but every run is written to History.",
            "历史与痕迹「删了就回不来」：本页删除不进回收站（浏览器 profile 半恢复反而更糟），但同样逐条写入历史记录审计。",
          )}
        />
      )}
      {error ? <InlineError message={error} onDismiss={() => setError(null)} /> : null}

      <section className="privacy-grid">
        {groups.map((group) => {
          const Icon = SPEC_ICONS[group.ruleId] ?? ShieldQuestion;
          const allOn = group.indexes.every((index) => picked.has(index));
          const warn = WARN_COPY[group.ruleId];
          return (
            <label
              key={group.ruleId}
              className={`privacy-card ${group.locked ? "is-locked" : ""} ${allOn ? "is-picked" : ""}`}
              onClick={() => toggleGroup(group)}
            >
              <Checkbox
                checked={allOn}
                disabled={Boolean(group.locked)}
                onChange={() => toggleGroup(group)}
                label={group.label}
              />
              <span className="privacy-icon">
                <Icon size={17} />
              </span>
              <span className="privacy-copy">
                <b>{group.label}</b>
                <small>{group.detail}</small>
                {warn && !group.locked ? (
                  <small className="privacy-warn">{lang === "zh" ? warn[0] : warn[1]}</small>
                ) : null}
              </span>
              {group.locked ? (
                <span className="privacy-lock">
                  <Lock size={12} /> {group.locked}
                </span>
              ) : (
                <span className="privacy-size">{formatBytes(group.bytes)}</span>
              )}
            </label>
          );
        })}
        {loaded && groups.length === 0 && !scanning ? (
          <EmptyState
            icon={<EyeOff size={26} />}
            title={t("Nothing to clean", "没有可清理的痕迹")}
            description={t(
              "No browser profiles or trace stores were found on this machine.",
              "本机没有找到浏览器 profile 或痕迹存储。",
            )}
          />
        ) : null}
      </section>

      <section className="privacy-footer">
        <span className="privacy-footer-sum">
          {picked.size > 0
            ? t(
                `${formatCount(picked.size)} traces selected · ${formatBytes(pickedBytes)} · permanent`,
                `已选 ${formatCount(picked.size)} 项 · ${formatBytes(pickedBytes)} · 永久删除`,
              )
            : t(
                "These deletions skip the Recycle Bin on purpose; History still lists every path.",
                "这些删除有意绕过回收站；历史记录仍会逐条登记路径与大小。",
              )}
        </span>
        <Button variant="ghost" onClick={() => void scan()} disabled={scanning || cleaning}>
          {scanning ? t("Refreshing…", "正在刷新…") : t("Re-scan", "重新扫描")}
        </Button>
        <Button
          variant="danger"
          disabled={picked.size === 0 || cleaning || scanning}
          onClick={() => void clean()}
        >
          {cleaning ? t("Cleaning…", "清理中…") : t("Clean now", "立即清理")}
        </Button>
      </section>
    </div>
  );
}
