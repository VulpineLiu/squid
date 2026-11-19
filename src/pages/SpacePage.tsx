/* 空间清理: one page, one scan, one delete pass.
 *
 * Merges the old 日常清理 (rule-driven caches/logs/…) and 深度清理 (build
 * artefacts under project folders) into a single flow: the scan runs the
 * quick pass first, then the deep pass, and both result sets share one
 * category grid and one item list. Daily categories start selected; the
 * three optional groups (升级安装包 / 项目产物 / 开发文件) start untouched
 * and join the same delete run when the user checks their card. */
import {
  ArrowUpRight,
  ChevronRight,
  DownloadCloud,
  FolderOpen,
  ScanSearch,
  Trash2,
} from "lucide-react";
import { open } from "@tauri-apps/plugin-dialog";
import {
  useCallback,
  useEffect,
  useMemo,
  useRef,
  useState,
} from "react";
import { useI18n } from "../i18n";
import { api } from "../lib/api";
import { formatBytes, formatCount, shortPath } from "../lib/format";
import type {
  Bootstrap,
  Category,
  CleanupItem,
  DeleteMode,
  DeleteResult,
} from "../types";
import type { ScanWorkspace } from "../hooks/useScanWorkspace";
import {
  Button,
  CATEGORY_META,
  CATEGORY_ORDER,
  Checkbox,
  ConfirmDialog,
  InlineError,
  ProgressCard,
  SectionTitle,
  Toggle,
  categoryLabel,
  lockLabel,
  useToast,
} from "../components/ui";

/* Groups the user opts into by hand; they arrive unchecked. */
export const OPTIONAL_CATEGORIES: Category[] = [
  "update-installers",
  "project-artifacts",
  "developer",
];

type Entry = {
  source: ScanWorkspace;
  index: number;
  item: CleanupItem;
};

const AGE_CHOICES = [3, 7, 30];

/* 清理点 names: one cluster per producing rule (电脑管家's "Windows系统日志
 * 文件" granularity). Unknown ids fall back to the prettified rule id. */
const RULE_NAMES: Record<string, [string, string]> = {
  "windows-temp": ["Temp files", "系统临时文件"],
  "recent-items": ["Recent items", "最近使用记录"],
  "windows-update-downloads": ["Windows Update downloads", "Windows 更新下载缓存"],
  "bits-transfer-cache": ["Delivery Optimization cache", "传递优化缓存"],
  prefetch: ["Prefetch files", "预读取文件 Prefetch"],
  "edge-caches": ["Microsoft Edge caches", "Edge 浏览器缓存"],
  "chrome-caches": ["Chrome caches", "Chrome 浏览器缓存"],
  "firefox-cache": ["Firefox cache", "Firefox 浏览器缓存"],
  "inet-cache": ["IE/legacy browser cache", "IE 遗留浏览器缓存"],
  "icon-thumbnail-cache": ["Icon & thumbnail caches", "缩略图与图标缓存"],
  "discord-cache": ["Discord cache", "Discord 缓存"],
  "slack-cache": ["Slack cache", "Slack 缓存"],
  "vscode-cache": ["VS Code caches", "VS Code 缓存"],
  "vscode-logs": ["VS Code logs", "VS Code 日志"],
  "user-error-reporting": ["User error reports", "用户错误报告"],
  "system-error-reporting": ["Windows error reports", "Windows 错误报告"],
  "crash-dumps": ["Crash dumps", "崩溃转储文件"],
  "kernel-dumps": ["Kernel memory dumps", "内存转储文件"],
  "connected-devices-platform": ["Connected Devices logs", "已连接设备平台日志"],
  "onedrive-logs": ["OneDrive logs", "OneDrive 日志"],
  "windows-setup-logs": ["Windows setup logs", "Windows 安装日志"],
  "npm-cache": ["npm cache", "npm 缓存"],
  "pip-cache": ["pip cache", "pip 缓存"],
  "cargo-cache": ["cargo registry cache", "cargo 缓存"],
  "go-build-cache": ["Go build cache", "Go 构建缓存"],
  "nuget-cache": ["NuGet cache", "NuGet 缓存"],
  "gradle-cache": ["Gradle cache", "Gradle 缓存"],
  "windows-old": ["Previous Windows install", "Windows.old 旧系统"],
  "package-cache": ["Package cache", "系统包缓存"],
  "nvidia-installer-archives": ["NVIDIA installer archives", "NVIDIA 安装程序缓存"],
  "orphan-leftovers": ["Leftovers of uninstalled apps", "已卸载应用残留"],
  "update-installers-known": ["Known app update packages", "已知应用的升级包"],
  "update-installers-scan": ["Other update packages", "其他升级安装包"],
  "purge-artifacts": ["Build artefacts", "项目构建产物"],
};

function ruleName(ruleId: string, lang: "zh" | "en"): string {
  const pair = RULE_NAMES[ruleId];
  if (pair) return lang === "zh" ? pair[1] : pair[0];
  if (!ruleId) return lang === "zh" ? "其他项目" : "Other items";
  return ruleId.replace(/[-_]/g, " ").replace(/\b\w/g, (c) => c.toUpperCase());
}

/* Flat rule id → a normalised cluster key. Update installers cluster per
 * app ("<app> · <file>" labels) so the list matches how people think. */
function clusterKey(item: CleanupItem): string {
  if (item.ruleId.startsWith("update-installers")) {
    return `update:${item.label.split(" · ")[0] ?? item.label}`;
  }
  return item.ruleId;
}

// Categories whose items are raw directories with one meaning each; they
// keep the flat list. Rule-driven categories group into 清理点 clusters.
const FLAT_CATEGORIES: Category[] = ["project-artifacts", "developer", "trash"];

/* Cluster label for update installers: items carry "<app> · <file>" labels. */
function clusterLabel(item: CleanupItem, lang: "zh" | "en"): string {
  if (item.ruleId.startsWith("update-installers")) {
    const app = item.label.split(" · ")[0] ?? item.label;
    return lang === "zh" ? `${app} 升级包` : `${app} installers`;
  }
  return ruleName(item.ruleId, lang);
}

type Cluster = {
  key: string;
  label: string;
  entries: Entry[];
  total: number;
  removable: number;
  selected: number;
};

/* A 清理点: one rule's worth of hits, aggregated like 电脑管家's sub-items.
 * Tick = whole cluster; hover = 忽略 / 查看路径; body = expand to paths. */
function ClusterRow({
  cluster,
  open,
  onOpen,
  onTick,
  onIgnore,
  lang,
}: {
  cluster: Cluster;
  open: boolean;
  onOpen: () => void;
  onTick: (on: boolean) => void;
  onIgnore: () => void;
  lang: "zh" | "en";
}) {
  const { t } = useI18n();
  const allOn = cluster.removable > 0 && cluster.selected >= cluster.removable;
  return (
    <>
      <div className="space-rule" onClick={onOpen}>
        <Checkbox
          checked={allOn}
          disabled={cluster.removable === 0}
          onChange={() => onTick(!allOn)}
          label={cluster.label}
        />
        <span className="space-rule-name">{cluster.label}</span>
        <span className="space-rule-cnt">
          {t(
            `${formatCount(cluster.entries.length)} location(s)`,
            `${formatCount(cluster.entries.length)} 个位置`,
          )}
        </span>
        <span className="space-rule-acts">
          <button
            type="button"
            onClick={(event) => {
              event.stopPropagation();
              onIgnore();
            }}
          >
            {t("Ignore", "忽略")}
          </button>
          <button
            type="button"
            onClick={(event) => {
              event.stopPropagation();
              onOpen();
            }}
          >
            {t("Paths", "查看路径")}
          </button>
        </span>
        <span className="space-rule-size">{formatBytes(cluster.total)}</span>
        <ChevronRight
          className={`space-rule-chv ${open ? "is-open" : ""}`}
          size={13}
          aria-hidden="true"
        />
      </div>
      {open ? (
        <div className="space-rule-paths">
          {cluster.entries.map((entry) => (
            <AccItem key={`${entry.item.path}-${entry.index}`} entry={entry} />
          ))}
        </div>
      ) : null}
    </>
  );
}

/* One candidate inside an expanded category. The whole row reveals the path
 * in Explorer (电脑管家 habit: 点击定位); only the tick box changes the
 * selection, so browsing never mis-toggles. */
function AccItem({ entry }: { entry: Entry }) {
  const { t } = useI18n();
  const { item, source, index } = entry;
  const checked = source.selected.has(index);
  return (
    <div
      className={`space-item ${item.lock ? "is-locked" : ""}`}
      onClick={() => void api.revealPath(item.path).catch(() => undefined)}
      title={t("Open the location in Explorer", "在资源管理器中打开所在位置")}
    >
      <Checkbox
        checked={checked}
        disabled={Boolean(item.lock)}
        onChange={() => source.toggleItem(index)}
        label={item.label}
      />
      <span className="space-item-name">{item.label}</span>
      <span className="space-item-path" title={item.displayPath}>
        {shortPath(item.displayPath, 96)}
      </span>
      {item.lock || item.note ? (
        <span className={`space-item-meta ${item.lock ? "is-locked" : ""}`}>
          {item.lock ? lockLabel(item.lock, t) : item.note}
        </span>
      ) : null}
      <span className="space-item-size">{formatBytes(item.onDisk)}</span>
      <ArrowUpRight className="space-item-go" size={14} />
    </div>
  );
}

export function SpacePage({
  cleanup,
  purge,
  bootstrap,
  deleteMode,
  onHistory,
}: {
  cleanup: ScanWorkspace;
  purge: ScanWorkspace;
  bootstrap: Bootstrap;
  deleteMode: DeleteMode;
  onHistory: () => void;
}) {
  const { t, lang, bin } = useI18n();
  const { push } = useToast();
  const [roots, setRoots] = useState(bootstrap.settings.purgeRoots.join("\n"));
  const [minAge, setMinAge] = useState(bootstrap.settings.purgeMinAgeDays || 7);
  const [force, setForce] = useState(false);
  const [confirming, setConfirming] = useState(false);
  // Open accordions. On first results we auto-expand the biggest category so
  // the list never reads as a dead panel; that initial choice is remembered
  // here (null until the first scan lands).
  const [expanded, setExpanded] = useState<Category[] | null>(null);
  const toggleExpanded = (category: Category) =>
    setExpanded((current) =>
      (current ?? []).includes(category)
        ? (current ?? []).filter((item) => item !== category)
        : [...(current ?? []), category],
    );
  const [mode, setMode] = useState<DeleteMode>(deleteMode);
  const [busy, setBusy] = useState(false);
  const [result, setResult] = useState<DeleteResult | null>(null);
  const scanning = cleanup.scanning || purge.scanning;
  const deleting = cleanup.deleting || purge.deleting;
  const inFlight = useRef(false);

  useEffect(() => setMode(deleteMode), [deleteMode]);

  const runScan = useCallback(async () => {
    if (scanning || deleting || inFlight.current) return;
    inFlight.current = true;
    setResult(null);
    const rootList = roots
      .split(/\r?\n/)
      .map((root) => root.trim())
      .filter(Boolean);
    try {
      await cleanup.runScan();
      await purge.runScan({
        roots: rootList,
        minAgeDays: minAge > 0 ? minAge : null,
        force,
      });
    } finally {
      inFlight.current = false;
    }
  }, [cleanup, deleting, force, minAge, purge, roots, scanning]);

  // Default checking is seeded at scan time by the workspaces' selection
  // policy (App.tsx spaceDefaults): daily groups on, opt-in groups off.

  const browse = async () => {
    const chosen = await open({
      directory: true,
      multiple: true,
      title: t("Choose project folders to inspect", "选择要检查的项目目录"),
    });
    if (!chosen) return;
    const picked = (Array.isArray(chosen) ? chosen : [chosen]).filter(
      (path) => !roots.split(/\r?\n/).some((line) => line.trim() === path),
    );
    if (picked.length > 0) {
      setRoots((current) =>
        current.trim().length > 0
          ? `${current.trimEnd()}\n${picked.join("\n")}`
          : picked.join("\n"),
      );
    }
  };

  const entries = useMemo<Entry[]>(() => {
    const all: Entry[] = [];
    for (const source of [cleanup, purge]) {
      source.result?.items.forEach((item, index) => {
        all.push({ source, index, item });
      });
    }
    return all.sort((a, b) => b.item.onDisk - a.item.onDisk);
    // Each Entry carries the workspace object (and with it the `selected`
    // Set), so a click must rebuild this list; without the sets in the deps
    // every derived card tick, hero number, and row checkbox stays frozen on
    // the scan-time snapshot — the cards look dead even though state changed.
  }, [cleanup.result, purge.result, cleanup.selected, purge.selected]);

  const groups = useMemo(() => {
    const map = new Map<
      Category,
      { total: number; count: number; selected: number; removable: number }
    >();
    for (const entry of entries) {
      const group = map.get(entry.item.category) ?? {
        total: 0,
        count: 0,
        selected: 0,
        removable: 0,
      };
      group.total += entry.item.onDisk;
      group.count += 1;
      if (!entry.item.lock) group.removable += 1;
      if (entry.source.selected.has(entry.index)) group.selected += 1;
      map.set(entry.item.category, group);
    }
    return map;
    // The selection sets are read through `entry.source`, so they must also
    // sit in the dependency list — otherwise toggling a card updates state
    // but the memo (and every card tick derived from it) stays frozen.
  }, [entries, cleanup.selected, purge.selected]);

  const selectedStats = useMemo(() => {
    let count = 0;
    let total = 0;
    for (const entry of entries) {
      if (entry.source.selected.has(entry.index) && !entry.item.lock) {
        count += 1;
        total += entry.item.onDisk;
      }
    }
    return { count, total };
  }, [entries, cleanup.selected, purge.selected]);

  // 升级安装包 keep the confirm-dialog escape: count them inside the current
  // selection so the banner can offer "skip installers, clean the rest".
  const selectedUpdaters = useMemo(() => {
    let total = 0;
    let count = 0;
    for (const entry of entries) {
      if (
        entry.item.category === "update-installers" &&
        entry.source.selected.has(entry.index) &&
        !entry.item.lock
      ) {
        total += entry.item.onDisk;
        count += 1;
      }
    }
    return { count, total };
  }, [entries, cleanup.selected, purge.selected]);

  // First results in: pre-open the single fattest category so the page never
  // looks inert; afterwards the user owns the open set entirely.
  useEffect(() => {
    if (expanded !== null || entries.length === 0) return;
    let biggest: Category | null = null;
    let best = 0;
    for (const [category, group] of groups) {
      if (group.removable > 0 && group.total > best) {
        best = group.total;
        biggest = category;
      }
    }
    setExpanded(biggest ? [biggest] : []);
  }, [entries, expanded, groups]);

  const toggleGroup = (category: Category) => {
    const allOn = groupSelected(category);
    for (const source of [cleanup, purge]) {
      if (source.result?.items.some((item) => item.category === category)) {
        source.setCategorySelection(category, !allOn);
      }
    }
  };

  // 受安全策略保护 items are the engine's own refusals — the user can fix
  // "app running" or "needs admin" rows, but never a protected path, so
  // those fold into a one-line footnote with a reveal switch per category.
  const [showProtected, setShowProtected] = useState<Partial<Record<Category, boolean>>>({});
  const visibleEntries = useMemo(
    () =>
      entries.filter(
        (entry) =>
          entry.item.lock !== "protected" || showProtected[entry.item.category],
      ),
    [entries, showProtected],
  );
  const hiddenProtected = useMemo(() => {
    const map = new Map<Category, number>();
    for (const entry of entries) {
      if (entry.item.lock === "protected" && !showProtected[entry.item.category]) {
        map.set(entry.item.category, (map.get(entry.item.category) ?? 0) + 1);
      }
    }
    return map;
  }, [entries, showProtected]);

  // 清理点 clusters for one category: same producing rule (update packages
  // cluster per app). Empty rule ids collapse into one "other" cluster.
  const clustersFor = (items: Entry[]): Cluster[] => {
    const map = new Map<string, Cluster>();
    for (const entry of items) {
      const key = clusterKey(entry.item);
      const cluster = map.get(key) ?? {
        key,
        label: clusterLabel(entry.item, lang),
        entries: [],
        total: 0,
        removable: 0,
        selected: 0,
      };
      cluster.entries.push(entry);
      cluster.total += entry.item.onDisk;
      if (!entry.item.lock) cluster.removable += 1;
      if (entry.source.selected.has(entry.index)) cluster.selected += 1;
      map.set(key, cluster);
    }
    return [...map.values()]
      .map((cluster) => ({
        ...cluster,
        entries: [...cluster.entries].sort((a, b) => b.item.onDisk - a.item.onDisk),
      }))
      .sort((a, b) => b.total - a.total);
  };

  // Open clusters inside expanded categories, keyed "category|ruleId".
  const [openClusters, setOpenClusters] = useState<Set<string>>(new Set());
  const toggleCluster = (key: string) =>
    setOpenClusters((current) => {
      const next = new Set(current);
      if (next.has(key)) next.delete(key);
      else next.add(key);
      return next;
    });

  // 忽略 = protect every path in the cluster through the whitelist — the
  // visible, reversible twin of 电脑管家's opaque "全部忽略".
  const ignoreCluster = async (cluster: Cluster) => {
    const unique = new Map<string, string>();
    for (const entry of cluster.entries) {
      unique.set(entry.item.path, entry.item.displayPath);
    }
    try {
      for (const path of unique.keys()) await api.whitelistAdd(path);
      const label =
        unique.size === 1
          ? [...unique.values()][0]
          : t(
              `${formatCount(unique.size)} paths of ${cluster.label}`,
              `${cluster.label} 的 ${formatCount(unique.size)} 个路径`,
            );
      push(
        t(`${label} moved to the protection list`, `已将 ${label} 加入保护名单`),
        "success",
      );
    } catch (reason) {
      push(
        t(`Ignore failed: ${String(reason)}`, `忽略失败：${String(reason)}`),
        "danger",
      );
    }
  };

  // A group is "on" when every removable item in it is checked; it may only
  // switch itself off when nothing can be removed at all (every row locked)
  // — an empty selection is a state to click back from, never a disability.
  const groupSelected = (category: Category) => {
    const group = groups.get(category);
    if (!group || group.removable === 0) return false;
    return group.selected >= group.removable;
  };
  const groupDisabled = (category: Category) => {
    const group = groups.get(category);
    return !group || group.removable === 0;
  };

  // One delete button, possibly two engines: run whichever halves have a
  // selection, in order, and merge the outcome into one summary.
  // `installerOnly` / `skipInstallers` drive the confirm-dialog escape so
  // the user can split the run: keep installers now, or clean only them.
  const cleanSelected = async (
    opts: { installerOnly?: boolean; skipInstallers?: boolean } = {},
  ) => {
    const plans: Array<{ source: ScanWorkspace; indexes: number[] }> = [];
    for (const source of [cleanup, purge]) {
      const indexes = [...source.selected].filter((index) => {
        const item = source.result?.items[index];
        if (!item || item.lock) return false;
        const isInstaller = item.category === "update-installers";
        if (opts.installerOnly) return isInstaller;
        if (opts.skipInstallers && isInstaller) return false;
        return true;
      });
      if (indexes.length > 0) plans.push({ source, indexes });
    }
    if (plans.length === 0) {
      setConfirming(false);
      return;
    }
    setBusy(true);
    const merged: DeleteResult = {
      freed: 0,
      freedAllocated: 0,
      removed: 0,
      failed: 0,
      errors: [],
      mode,
      sessionId: null,
      skipped: [],
    };
    try {
      for (const plan of plans) {
        const outcome = await plan.source.clean(mode, plan.indexes);
        if (outcome) {
          merged.freed += outcome.freed;
          merged.freedAllocated += outcome.freedAllocated;
          merged.removed += outcome.removed;
          merged.failed += outcome.failed;
          merged.errors.push(...outcome.errors);
          merged.skipped.push(...outcome.skipped);
          if (!merged.sessionId) merged.sessionId = outcome.sessionId;
        }
      }
    } finally {
      setBusy(false);
      setConfirming(false);
    }
    setResult(merged);
    push(
      t(
        `${formatBytes(merged.freedAllocated || merged.freed)} cleaned`,
        `已清理 ${formatBytes(merged.freedAllocated || merged.freed)}`,
      ),
      "success",
    );
  };

  const progress = cleanup.progress ?? purge.progress;
  const stage = cleanup.scanning
    ? t("Scanning caches and junk", "第一步 · 扫描缓存与垃圾")
    : purge.scanning
      ? t("Scanning project folders", "第二步 · 扫描项目目录")
      : t("Collecting results", "正在汇总结果");

  const error = cleanup.error ?? purge.error;

  if (result) {
    return (
      <div className="page">
        <SectionTitle
          title={t("Cleaning finished", "清理完成")}
          subtitle={t(
            "Everything else on this machine is untouched.",
            "机器上的其他内容未被改动。",
          )}
        />
        <div className="hero">
          <div>
            <div className="k">{t("Done", "完成")}</div>
            <div className="n">
              {formatBytes(result.freedAllocated || result.freed)}{" "}
              <small>
                {t("freed", "已释放")} · {formatCount(result.removed)}{" "}
                {t("items", "项")}
                {result.failed > 0
                  ? t(` · ${result.failed} failed`, ` · ${result.failed} 项失败`)
                  : ""}
              </small>
            </div>
          </div>
          <div className="acts">
            <Button variant="ghost" onClick={() => setResult(null)}>
              {t("Back", "返回")}
            </Button>
            <Button variant="secondary" onClick={onHistory}>
              {t("Open history", "查看历史")}
            </Button>
            <Button variant="primary" icon={<ScanSearch size={15} />} onClick={() => void runScan()}>
              {t("Scan again", "再扫一次")}
            </Button>
          </div>
        </div>
      </div>
    );
  }

  return (
    <div className="page">
      <div className="space-head">
        <div>
          <div className="eyebrow">{t("Space cleaning", "空间清理")}</div>
          <h1 className="page-title">
            {t("One scan. Every bit of space.", "一次扫描，清掉能清的空间。")}
          </h1>
          <p className="page-desc">
            {t(
              "Caches, logs, update installers and rebuildable project artefacts — always reviewed before deletion.",
              "缓存、日志、升级安装包与可重建的项目构建目录，删除前始终由您确认。",
            )}
          </p>
        </div>
        <Button
          variant="primary"
          icon={<ScanSearch size={16} />}
          onClick={() => void runScan()}
          disabled={scanning || deleting}
        >
          {cleanup.result || purge.result
            ? t("Rescan", "重新扫描")
            : t("Start scan", "开始扫描")}
        </Button>
      </div>

      <div className="space-config">
        <span className="space-config-label">
          {t("Deep-scan folders", "深度扫描目录")}
        </span>
        <button
          type="button"
          className="space-config-value"
          onClick={() => void browse()}
          title={roots.replace(/\r?\n/g, "\n")}
        >
          {roots.trim()
            ? t(
                `${roots.split(/\r?\n/).filter((line) => line.trim()).length} folder(s) · click to edit`,
                `共 ${roots.split(/\r?\n/).filter((line) => line.trim()).length} 处 · 点击编辑`,
              )
            : t("Home folder (default)", "主目录（默认）")}
          <FolderOpen size={13} />
        </button>
        <span className="space-config-label">
          {t("Older than", "仅处理早于")}
        </span>
        <div className="space-seg">
          {AGE_CHOICES.map((days) => (
            <button
              key={days}
              type="button"
              className={minAge === days ? "is-on" : ""}
              onClick={() => setMinAge(days)}
            >
              {t(`${days} days`, `${days}天`)}
            </button>
          ))}
        </div>
        <span className="space-config-label">
          {t("Git-protected", "Git 保护")}
        </span>
        <Toggle
          checked={!force}
          onChange={(on) => setForce(!on)}
          label={t("Git protection", "Git 保护")}
        />
        <input
          className="space-roots-hidden"
          value={roots}
          onChange={(event) => setRoots(event.target.value)}
          aria-label={t("Project folders to inspect", "要检查的项目目录")}
        />
      </div>

      {scanning ? (
        <div className="space-stage-steps">
          <span className={`space-step ${cleanup.scanning ? "is-now" : "is-done"}`}>
            {cleanup.scanning ? "◌" : "✓"} {t("Scan caches & junk", "扫描缓存与垃圾")}
          </span>
          <span
            className={`space-step ${
              purge.scanning ? "is-now" : !cleanup.scanning && cleanup.result ? "is-done" : ""
            }`}
          >
            {purge.scanning ? "◌" : !cleanup.scanning && cleanup.result ? "✓" : "·"}{" "}
            {t("Scan project folders", "扫描项目目录")}
          </span>
          <ProgressCard
            progress={progress}
            title={stage}
            onCancel={() => void (cleanup.scanning ? cleanup.cancel() : purge.cancel())}
          />
        </div>
      ) : null}

      {deleting ? (
        <div className="space-stage-steps">
          <span className="space-step is-done">
            ✓ {t("Scan complete", "扫描完成")}
          </span>
          <span className="space-step is-now">
            ◌ {t("Cleaning", "正在清理")}（{formatBytes(selectedStats.total)}）
          </span>
          <ProgressCard
            progress={progress}
            title={t("Removing selected items", "正在删除已选项目")}
            onCancel={() => void (cleanup.deleting ? cleanup.cancel() : purge.cancel())}
          />
        </div>
      ) : null}

      {error && !scanning ? (
        <InlineError
          message={error}
          onDismiss={() => {
            cleanup.clearError();
            purge.clearError();
          }}
        />
      ) : null}

      {!scanning && !deleting && !cleanup.result && !purge.result ? (
        <div className="empty-inline">
          <ScanSearch size={22} />
          <span>
            {t(
              "Not scanned yet. Start scan checks known cache spots and searches the project folders above.",
              "尚未扫描。点击「开始扫描」，Squid 会检查已知缓存位置并搜索上方项目目录里的构建产物。",
            )}
          </span>
        </div>
      ) : null}

      {!scanning && !deleting && (cleanup.result || purge.result) ? (
        <>
          <div className="space-sum" style={{ position: "sticky", top: 0, zIndex: 5 }}>
            <div className="grow">
              <div className="big">
                {t("Selected ", "已选中 ")}
                <em>{formatBytes(selectedStats.total)}</em>
                {t(" ready to clean", " 可清理文件")}
              </div>
              <div className="sub">
                {t(
                  `${formatBytes(
                    entries
                      .filter((entry) => !entry.item.lock)
                      .reduce((sum, entry) => sum + entry.item.onDisk, 0),
                  )} found in total · ${formatCount(selectedStats.count)} of ${formatCount(
                    entries.filter((entry) => !entry.item.lock).length,
                  )} items selected`,
                  `共发现 ${formatBytes(
                    entries
                      .filter((entry) => !entry.item.lock)
                      .reduce((sum, entry) => sum + entry.item.onDisk, 0),
                  )} 可清理文件 · 已选 ${formatCount(selectedStats.count)} / ${formatCount(
                    entries.filter((entry) => !entry.item.lock).length,
                  )} 项`,
                )}
              </div>
            </div>
            <Button
              variant="ghost"
              onClick={() => {
                cleanup.clearSelection();
                purge.clearSelection();
              }}
              disabled={selectedStats.count === 0}
            >
              {t("Deselect all", "取消全选")}
            </Button>
            <Button
              variant="primary"
              icon={<Trash2 size={16} />}
              disabled={selectedStats.count === 0}
              onClick={() => setConfirming(true)}
            >
              {t("Clean now", "立即清理")}
            </Button>
          </div>

          {(["daily", "optional"] as const).map((section) => {
            const inSection = (category: Category) =>
              section === "optional"
                ? OPTIONAL_CATEGORIES.includes(category)
                : !OPTIONAL_CATEGORIES.includes(category);
            const cats = CATEGORY_ORDER.filter(
              (category) => groups.has(category) && inSection(category),
            );
            if (cats.length === 0) return null;
            const inSectionEntry = (entry: Entry) => inSection(entry.item.category);
            const sectionCount = visibleEntries.filter(inSectionEntry).length;
            const sectionTotal = visibleEntries
              .filter(inSectionEntry)
              .reduce((sum, entry) => sum + entry.item.onDisk, 0);
            return (
              <div key={section}>
                <div className="space-sec">
                  <h2>
                    {section === "daily"
                      ? t("Pre-selected for you", "已为你默认勾选")
                      : t("Needs your tick", "需要你手动勾选")}
                  </h2>
                  <span className="tip">
                    {section === "daily"
                      ? t(
                          `${formatCount(sectionCount)} items, safe to clean`,
                          `共 ${formatCount(sectionCount)} 项，可以放心清理`,
                        )
                      : t(
                          `${formatCount(sectionCount)} items, review before cleaning`,
                          `共 ${formatCount(sectionCount)} 项，请确认后清理`,
                        )}
                  </span>
                  <span className={`r ${section === "optional" ? "is-opt" : ""}`}>
                    {section === "daily" ? t("selected", "已选") : t("optional", "可选")}{" "}
                    <b>{formatBytes(sectionTotal)}</b>
                  </span>
                </div>
                {cats.map((category) => {
                  const group = groups.get(category)!;
                  const open = (expanded ?? []).includes(category);
                  const meta = CATEGORY_META[category];
                  const Icon = meta.icon;
                  const optional = OPTIONAL_CATEGORIES.includes(category);
                  const shown = visibleEntries.filter(
                    (entry) => entry.item.category === category,
                  );
                  const flat = FLAT_CATEGORIES.includes(category);
                  const clusters = flat ? [] : clustersFor(shown);
                  const hidden = hiddenProtected.get(category) ?? 0;
                  return (
                    <div key={category} className={`space-cat ${open ? "is-open" : ""}`}>
                      <div
                        className="space-cat-head"
                        role="button"
                        tabIndex={0}
                        onClick={() => toggleExpanded(category)}
                        onKeyDown={(event) => {
                          if (event.key === "Enter" || event.key === " ") {
                            event.preventDefault();
                            toggleExpanded(category);
                          }
                        }}
                      >
                        <Checkbox
                          checked={groupSelected(category)}
                          disabled={groupDisabled(category)}
                          onChange={() => toggleGroup(category)}
                          label={categoryLabel(category, lang, bin)}
                        />
                        <span className={`space-cat-icon tone-${meta.tone}`}>
                          <Icon size={15} />
                        </span>
                        <span className="space-cat-name">
                          {categoryLabel(category, lang, bin)}
                        </span>
                        <span className="space-cat-desc">
                          {lang === "zh" ? meta.blurbZh : meta.blurbEn}
                        </span>
                        <span className="space-cat-n">
                          {flat
                            ? t(
                                `${formatCount(shown.length)} items`,
                                `${formatCount(shown.length)} 项`,
                              )
                            : t(
                                `${formatCount(clusters.length)} clean-up spots`,
                                `${formatCount(clusters.length)} 个清理点`,
                              )}
                        </span>
                        <span className={`space-cat-size ${optional ? "is-opt" : ""}`}>
                          {optional ? `${t("optional", "可选")} ` : ""}
                          {formatBytes(
                            shown.reduce((sum, entry) => sum + entry.item.onDisk, 0),
                          )}
                        </span>
                        <ChevronRight
                          className="space-cat-chv"
                          size={15}
                          aria-hidden="true"
                        />
                      </div>
                      {open ? (
                        <div className="space-cat-items">
                          {flat
                            ? shown.map((entry) => (
                                <AccItem
                                  key={`${entry.item.path}-${entry.index}`}
                                  entry={entry}
                                />
                              ))
                            : clusters.map((cluster) => {
                                const key = `${category}|${cluster.key}`;
                                return (
                                  <ClusterRow
                                    key={key}
                                    cluster={cluster}
                                    open={openClusters.has(key)}
                                    lang={lang}
                                    onOpen={() => toggleCluster(key)}
                                    onTick={(on) => {
                                      const bySource = new Map<ScanWorkspace, number[]>();
                                      for (const entry of cluster.entries) {
                                        if (entry.item.lock) continue;
                                        const list = bySource.get(entry.source) ?? [];
                                        list.push(entry.index);
                                        bySource.set(entry.source, list);
                                      }
                                      for (const [source, list] of bySource) {
                                        source.setIndexes(list, on);
                                      }
                                    }}
                                    onIgnore={() => void ignoreCluster(cluster)}
                                  />
                                );
                              })}
                          {hidden > 0 && !showProtected[category] ? (
                            <div className="space-hidden">
                              {t(
                                `${formatCount(hidden)} items in protected system locations are hidden (they cannot be removed).`,
                                `已自动隐藏 ${formatCount(hidden)} 个受安全策略保护、无法删除的项目。`,
                              )}
                              <button
                                type="button"
                                onClick={() =>
                                  setShowProtected((current) => ({
                                    ...current,
                                    [category]: true,
                                  }))
                                }
                              >
                                {t("Show anyway", "仍然显示")}
                              </button>
                            </div>
                          ) : null}
                        </div>
                      ) : null}
                    </div>
                  );
                })}
              </div>
            );
          })}
        </>
      ) : null}

      <ConfirmDialog
        open={confirming}
        title={t("Ready to make room?", "准备释放空间？")}
        message={
          <span>
            {t(
              `${formatCount(selectedStats.count)} items · ${formatBytes(selectedStats.total)}`,
              `${formatCount(selectedStats.count)} 项 · ${formatBytes(selectedStats.total)}`,
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
                "Permanent removal cannot be undone. Squid re-checks every path before deleting it.",
                "永久删除无法撤销。Squid 会在删除前重新检查每一个路径。",
              )
        }
        confirmLabel={
          mode === "trash"
            ? t(`Move to ${bin}`, `移入${bin}`)
            : t("Remove permanently", "永久删除")
        }
        cancelLabel={t("Keep files", "保留文件")}
        onConfirm={() => void cleanSelected()}
        onClose={() => setConfirming(false)}
        busy={busy}
        icon={<Trash2 size={23} />}
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
        {selectedUpdaters.count > 0 ? (
          <div className="confirm-updater-banner">
            <DownloadCloud size={18} />
            <div>
              <strong>
                {t(
                  `Includes ${formatCount(selectedUpdaters.count)} update installers (${formatBytes(selectedUpdaters.total)})`,
                  `其中包含 ${formatCount(selectedUpdaters.count)} 个升级安装包（${formatBytes(selectedUpdaters.total)}）`,
                )}
              </strong>
              <span>
                {t(
                  "Apps re-download them when needed. Delete them too?",
                  "应用需要时会自动重新下载。要一并清理吗？",
                )}
              </span>
            </div>
            <div className="confirm-updater-actions">
              <Button
                variant="ghost"
                size="sm"
                disabled={busy}
                onClick={() => void cleanSelected({ installerOnly: true })}
              >
                {t("Only installers", "只清升级包")}
              </Button>
              <Button
                variant="ghost"
                size="sm"
                disabled={busy}
                onClick={() => void cleanSelected({ skipInstallers: true })}
              >
                {t("Skip installers, clean rest", "跳过升级包，只删其余")}
              </Button>
            </div>
          </div>
        ) : null}
      </ConfirmDialog>
    </div>
  );
}
