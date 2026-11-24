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
  FileText,
  FolderOpen,
  ScanSearch,
  ShieldCheck,
  Trash2,
} from "lucide-react";
import { save } from "@tauri-apps/plugin-dialog";
import {
  useCallback,
  useEffect,
  useMemo,
  useRef,
  useState,
} from "react";
import { useI18n } from "../i18n";
import { api } from "../lib/api";
import {
  ageLabel,
  countLabel,
  formatBytes,
  formatCount,
  shortPath,
} from "../lib/format";
import type {
  Bootstrap,
  Category,
  CleanupItem,
  DeleteMode,
  DeleteResult,
  PreflightResult,
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
  "user-assets": ["Your own files (locked)", "您的个人文件（受保护）"],
  "ai-cache": ["AI app caches & update payloads", "AI 应用缓存与更新包"],
};

function ruleName(ruleId: string, lang: "zh" | "en"): string {
  const pair = RULE_NAMES[ruleId];
  if (pair) return lang === "zh" ? pair[1] : pair[0];
  if (!ruleId) return lang === "zh" ? "其他项目" : "Other items";
  return ruleId.replace(/[-_]/g, " ").replace(/\b\w/g, (c) => c.toUpperCase());
}

/* Flat rule id → a normalised cluster key. Update installers cluster per
 * app ("<app> · <file>" labels) so the list matches how people think;
 * HuggingFace repos share one rule id but deserve one 清理点 each. */
function clusterKey(item: CleanupItem): string {
  if (item.ruleId.startsWith("update-installers")) {
    return `update:${item.label.split(" · ")[0] ?? item.label}`;
  }
  if (item.ruleId === "ai-huggingface") {
    return `hf:${item.label}`;
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
  // AI model clusters carry a human label already ("Ollama · qwen3/8b").
  if (item.ruleId.startsWith("ai-ollama-") || item.ruleId === "ai-huggingface") {
    return item.label;
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

/* A⑦: skipped items on the result page are grouped by *why* they were
 * skipped, each group with the concrete next step — Mole's ledger reads as
 * "here is what to do about it", not as a wall of paths. */
type SkipGroup = {
  key: string;
  title: [string, string];
  hint: [string, string];
  action?: "rescan";
};

function skipGroupOf(reason: string): SkipGroup {
  const lower = reason.toLowerCase();
  if (lower.includes("running") || lower.includes("占用")) {
    return {
      key: "running",
      title: ["应用正在运行", "应用正在运行"],
      hint: ["退出这些应用后重新扫描，即可再次选中。", "退出这些应用后重新扫描，即可再次选中。"],
      action: "rescan",
    };
  }
  if (lower.includes("admin") || lower.includes("管理员")) {
    return {
      key: "admin",
      title: ["需要管理员权限", "需要管理员权限"],
      hint: [
        "以管理员身份重新启动 Squid 后再清理这批项目。",
        "以管理员身份重新启动 Squid 后再清理这批项目。",
      ],
      action: "rescan",
    };
  }
  if (lower.includes("已不存在") || lower.includes("no longer") || lower.includes("missing")) {
    return {
      key: "gone",
      title: ["扫描后已消失", "扫描后已消失"],
      hint: ["这些路径在清理开始前就不在了，无需处理。", "这些路径在清理开始前就不在了，无需处理。"],
    };
  }
  if (lower.includes("whitelist")) {
    return {
      key: "whitelist",
      title: ["命中白名单", "命中白名单"],
      hint: [
        "在 设置 → 白名单 中移除对应规则后才会被清理。",
        "在 设置 → 白名单 中移除对应规则后才会被清理。",
      ],
    };
  }
  return {
    key: "protected",
    title: ["受安全策略保护", "受安全策略保护"],
    hint: [
      "这些目录看似缓存，实际可能存有您自己的文件，Squid 不会删除。",
      "这些目录看似缓存，实际可能存有您自己的文件，Squid 不会删除。",
    ],
  };
}

/* A③: a cluster is "dormant" when its newest member stopped changing more
 * than 90 days ago — size plus dormancy is what makes a deletion call easy. */
const DORMANT_SECS = 90 * 24 * 3600;

function clusterDormancy(cluster: { entries: Entry[] }): number | null {
  let oldest: number | null = null;
  for (const entry of cluster.entries) {
    const secs = entry.item.lastWriteSecs;
    if (secs == null) continue;
    if (oldest === null || secs > oldest) oldest = secs;
  }
  return oldest;
}

/* B3 evidence tokens → human lines. Unknown tokens render verbatim, so a
 * future engine token still shows up instead of vanishing. */
function evidenceLines(
  tokens: string[],
  t: (en: string, zh: string) => string,
): { ok: boolean; text: string }[] {
  const lines: { ok: boolean; text: string }[] = [];
  for (const token of tokens) {
    if (token.startsWith("lock:")) {
      lines.push({
        ok: true,
        text: t(
          `${token.slice(5)} present — the tree rebuilds with one command`,
          `存在 ${token.slice(5)}，一条命令即可完整重建`,
        ),
      });
    } else if (token.startsWith("record:")) {
      lines.push({
        ok: true,
        text: t(
          `${token.slice(7)} present — install record intact`,
          `存在 ${token.slice(7)} 安装记录`,
        ),
      });
    } else if (token === "record-missing") {
      lines.push({
        ok: true,
        text: t("No install record found — skipped the hand-edit check", "缺少安装记录，未做手改检查"),
      });
    } else if (token.startsWith("modified:")) {
      const count = Number(token.slice(9)) || 0;
      lines.push(
        count === 0
          ? { ok: true, text: t("No file is newer than the install record (nobody hand-edited it)", "无文件晚于安装记录（没有被手工修改）") }
          : { ok: false, text: t(`${count} files are newer than the install record — possible local patches`, `${count} 个文件晚于安装记录，可能被手工修改过`) },
      );
    } else if (token === "git:ignored") {
      lines.push({ ok: true, text: t("Marked .gitignore by the project", "已被项目 .gitignore 忽略") });
    } else if (token === "git:untracked") {
      lines.push({ ok: true, text: t("Not tracked by git", "未被 Git 跟踪") });
    } else if (token === "git:tracked") {
      lines.push({ ok: false, text: t("Tracked by git — deleted only with --all", "被 Git 跟踪，需强制模式才删") });
    } else if (token === "git:none") {
      lines.push({ ok: true, text: t("No git repository around it", "附近没有 Git 仓库") });
    } else if (token.startsWith("age:")) {
      lines.push({
        ok: true,
        text: t(`Last change ${token.slice(4)} days ago`, `最后修改于 ${token.slice(4)} 天前`),
      });
    } else {
      lines.push({ ok: true, text: token });
    }
  }
  return lines;
}

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
  const dormancy = clusterDormancy(cluster);
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
        {dormancy !== null && dormancy >= DORMANT_SECS ? (
          <span className="space-rule-dormant">
            {t(
              `${ageLabel(dormancy, "en").replace(" untouched", "")} dormant`,
              ageLabel(dormancy, "zh").replace("未修改", "未使用"),
            )}
          </span>
        ) : null}
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
  const { t, lang } = useI18n();
  const { item, source, index } = entry;
  const checked = source.selected.has(index);
  const [showEvidence, setShowEvidence] = useState(false);
  const evidence = item.evidence ?? [];
  return (
    <>
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
        {item.lock ? (
          <span className="space-item-meta is-locked">{lockLabel(item.lock, t)}</span>
        ) : null}
        {item.lastWriteSecs != null ? (
          <span
            className={`space-item-meta ${item.lastWriteSecs >= DORMANT_SECS ? "is-dormant" : ""}`}
            title={t(
              "Newest last-write inside, from the scan walk",
              "扫描时记录的内部最新写入时间",
            )}
          >
            {ageLabel(item.lastWriteSecs, lang)}
          </span>
        ) : null}
        {evidence.length > 0 ? (
          <button
            type="button"
            className={`space-item-evidence ${
              evidence.some((token) => token.startsWith("modified:") && Number(token.slice(9)) > 0)
                ? "is-warn"
                : ""
            }`}
            onClick={(event) => {
              event.stopPropagation();
              setShowEvidence((current) => !current);
            }}
          >
            {showEvidence
              ? t("Hide evidence", "收起依据")
              : t("Why safe · evidence", "可删依据")}
          </button>
        ) : null}
        {!item.lock && item.note ? (
          <span className="space-item-meta">{item.note}</span>
        ) : null}
        <span className="space-item-size">{formatBytes(item.onDisk)}</span>
        <ArrowUpRight className="space-item-go" size={14} />
      </div>
      {showEvidence ? (
        <div className="space-evidence">
          <strong>{t("Why Squid judges this folder", "Squid 对该目录的判定依据")}</strong>
          <ul>
            {evidenceLines(evidence, t).map((line) => (
              <li key={line.text} className={line.ok ? "is-ok" : "is-warn"}>
                {line.text}
              </li>
            ))}
          </ul>
        </div>
      ) : null}
    </>
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
  const [minAge, setMinAge] = useState(bootstrap.settings.purgeMinAgeDays || 7);
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
    // A fresh scan supersedes any armed inline confirm: the old selection
    // (and its preflight verdict) no longer describes what is on screen.
    setInlineConfirm(false);
    setPreflight(null);
    try {
      await cleanup.runScan();
      await purge.runScan({
        // Empty = the engine's auto-discovery (drive-root containers + home).
        roots: [],
        minAgeDays: minAge > 0 ? minAge : null,
        force: false,
      });
    } finally {
      inFlight.current = false;
    }
  }, [cleanup, deleting, minAge, purge, scanning]);

  // Default checking is seeded at scan time by the workspaces' selection
  // policy (App.tsx spaceDefaults): daily groups on, opt-in groups off.

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
      setInlineConfirm(false);
      setPreflight(null);
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
  // A①: running "已统计 N 文件 · X GB" counters streamed by the engines.
  const liveStats = cleanup.stats ?? purge.stats;
  // A②: category skeletons filled live while the scan is still measuring.
  const liveBuckets = useMemo(() => {
    const merged: Partial<Record<Category, { total: number; count: number }>> = {};
    for (const source of [cleanup, purge]) {
      for (const [category, bucket] of Object.entries(source.buckets)) {
        const known = merged[category as Category];
        merged[category as Category] = known
          ? { total: known.total + bucket.total, count: known.count + bucket.count }
          : { ...bucket };
      }
    }
    return merged;
  }, [cleanup.buckets, purge.buckets]);
  const stage = cleanup.scanning
    ? t("Scanning caches and junk", "第一步 · 扫描缓存与垃圾")
    : purge.scanning
      ? t("Scanning project folders", "第二步 · 扫描项目目录")
      : t("Collecting results", "正在汇总结果");

  // A④ + B1: low-risk runs confirm inline on the sticky bar; anything with
  // update installers, many items, or preflight surprises opens the modal.
  const [inlineConfirm, setInlineConfirm] = useState(false);
  const [preflight, setPreflight] = useState<PreflightResult | null>(null);
  const [preflightBusy, setPreflightBusy] = useState(false);

  // Mole's inline strip: Esc backs out of the armed confirm.
  useEffect(() => {
    if (!inlineConfirm) return;
    const onKey = (event: KeyboardEvent) => {
      if (event.key === "Escape") setInlineConfirm(false);
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [inlineConfirm]);

  const selectedPlans = () => {
    const plans: Array<{ source: ScanWorkspace; indexes: number[] }> = [];
    for (const source of [cleanup, purge]) {
      const indexes = [...source.selected].filter((index) => {
        const item = source.result?.items[index];
        return item && !item.lock;
      });
      if (indexes.length > 0) plans.push({ source, indexes });
    }
    return plans;
  };

  const runPreflight = async () => {
    const plans = selectedPlans();
    if (plans.length === 0) return null;
    setPreflightBusy(true);
    const blocked: PreflightResult["blocked"] = [];
    let checked = 0;
    try {
      for (const plan of plans) {
        const scanId = plan.source.result?.scanId;
        if (!scanId) continue;
        try {
          const verdict = await api.cleanupPreflight(scanId, plan.indexes);
          checked += verdict.checked;
          blocked.push(...verdict.blocked);
        } catch {
          // A failed re-check must never block the run; delete re-guards anyway.
        }
      }
    } finally {
      setPreflightBusy(false);
    }
    const verdict = { checked, blocked };
    setPreflight(verdict);
    return verdict;
  };

  const startClean = async () => {
    const verdict = await runPreflight();
    const risky =
      selectedUpdaters.count > 0 ||
      selectedStats.count > 40 ||
      (verdict?.blocked.length ?? 0) > 0;
    if (risky) {
      setConfirming(true);
    } else {
      setInlineConfirm(true);
    }
  };

  // B5: export the current selection as a reviewable Markdown list.
  const exportList = async () => {
    const lines: string[] = [
      `# squid-clean-list · ${new Date().toLocaleString()} · ${t(
        `${formatCount(selectedStats.count)} items ${formatBytes(selectedStats.total)}`,
        `已选 ${formatCount(selectedStats.count)} 项 ${formatBytes(selectedStats.total)}`,
      )}`,
      "",
    ];
    for (const source of [cleanup, purge]) {
      for (const index of [...source.selected].sort((a, b) => a - b)) {
        const item = source.result?.items[index];
        if (!item || item.lock) continue;
        const evidence =
          item.evidence && item.evidence.length > 0
            ? ` · ${item.evidence.join(" · ")}`
            : "";
        lines.push(
          `- [${categoryLabel(item.category, lang, bin)}] ${item.displayPath} — ${formatBytes(
            item.onDisk,
          )}${item.note ? ` · ${item.note}` : ""}${evidence}`,
        );
      }
    }
    lines.push("");
    const locked = entries.filter((entry) => entry.item.lock === "protected");
    if (locked.length > 0) {
      lines.push(
        t("## Protected (not deleted)", "## 受保护项（未列入删除）"),
        ...locked
          .slice(0, 20)
          .map(
            (entry) =>
              `- ${entry.item.displayPath} — ${formatBytes(entry.item.onDisk)} · ${
                entry.item.note ?? lockLabel(entry.item.lock, t)
              }`,
          ),
      );
    }
    const chosen = await save({
      title: t("Export clean list", "导出清理清单"),
      defaultPath: "squid-clean-list.md",
      filters: [{ name: "Markdown", extensions: ["md", "txt"] }],
    });
    if (!chosen) return;
    try {
      await api.writeTextFile(chosen, lines.join("\n"));
      push(t("List exported", "清单已导出"), "success");
    } catch (reason) {
      push(
        t(`Export failed: ${String(reason)}`, `导出失败：${String(reason)}`),
        "danger",
      );
    }
  };

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
        {result.skipped && result.skipped.length > 0 ? (
          <div className="result-skipped">
            <div className="result-skipped-head">
              {t(
                `${result.skipped.length} item(s) were skipped`,
                `有 ${result.skipped.length} 项本次未处理`,
              )}
            </div>
            {Object.entries(
              result.skipped.reduce<Record<string, typeof result.skipped>>(
                (groups, row) => {
                  const key = skipGroupOf(row.reason).key;
                  (groups[key] ??= []).push(row);
                  return groups;
                },
                {},
              ),
            ).map(([key, rows]) => {
              const group = skipGroupOf(rows[0].reason);
              return (
                <div className="result-skip-group" key={key}>
                  <div className="result-skip-title">
                    <strong>{t(group.title[0], group.title[1])}</strong>
                    <span>{rows.length}</span>
                  </div>
                  <ul>
                    {rows.slice(0, 5).map((row) => (
                      <li key={row.path} title={row.reason}>
                        <code>{row.path}</code>
                        <small>{row.reason}</small>
                      </li>
                    ))}
                    {rows.length > 5 ? (
                      <li className="more">
                        {t(`… ${rows.length - 5} more`, `… 另有 ${rows.length - 5} 项`)}
                      </li>
                    ) : null}
                  </ul>
                  <div className="result-skip-foot">
                    <small>{t(group.hint[0], group.hint[1])}</small>
                    {group.action === "rescan" ? (
                      <Button variant="ghost" size="sm" onClick={() => void runScan()}>
                        {t("Scan again", "再扫一次")}
                      </Button>
                    ) : null}
                  </div>
                </div>
              );
            })}
          </div>
        ) : null}
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
        <span className="space-config-static">
          {t(
            "Auto-discovered: Desktop, Documents, project folders on every drive",
            "自动发现：桌面、文档、各磁盘根目录下的项目目录（无需设置）",
          )}
        </span>
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
        {/* Git 保护不再有开关：它永远是开着的（删前用 git 复核被跟踪的目录），
            需要绕过时走 CLI 的 --all。 */}
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
            capAt99
            liveLine={liveStats ? countLabel(liveStats.files, liveStats.bytes, lang) : null}
            onCancel={() => void (cleanup.scanning ? cleanup.cancel() : purge.cancel())}
          />
          {Object.keys(liveBuckets).length > 0 ? (
            <div className="space-live-cats">
              {CATEGORY_ORDER.filter((category) => liveBuckets[category]).map((category) => {
                const bucket = liveBuckets[category]!;
                const meta = CATEGORY_META[category];
                const Icon = meta.icon;
                return (
                  <div key={category} className="space-live-cat">
                    <span className={`space-cat-icon tone-${meta.tone}`}>
                      <Icon size={14} />
                    </span>
                    <span className="space-live-cat-name">{categoryLabel(category, lang, bin)}</span>
                    <span className="space-live-cat-cnt">
                      {t(`${formatCount(bucket.count)} items`, `${formatCount(bucket.count)} 项`)}
                    </span>
                    <b>{formatBytes(bucket.total)}</b>
                  </div>
                );
              })}
            </div>
          ) : null}
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
              "Not scanned yet. Start scan checks known cache spots and auto-discovers project folders across the machine.",
              "尚未扫描。点击「开始扫描」，Squid 会检查已知缓存位置，并自动发现本机各处的项目构建目录。",
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
              icon={<FileText size={14} />}
              onClick={() => void exportList()}
              disabled={selectedStats.count === 0}
            >
              {t("Export list", "导出清单")}
            </Button>
            <Button
              variant="ghost"
              onClick={() => {
                cleanup.clearSelection();
                purge.clearSelection();
                setInlineConfirm(false);
              }}
              disabled={selectedStats.count === 0 || inlineConfirm}
            >
              {t("Deselect all", "取消全选")}
            </Button>
            {inlineConfirm ? (
              <div className="space-inline-confirm">
                <ShieldCheck size={16} />
                <span>
                  <b>
                    {mode === "trash"
                      ? t(
                          `Move to ${bin}: ${formatCount(selectedStats.count)} items · ${formatBytes(selectedStats.total)}`,
                          `将移入${bin}：${formatCount(selectedStats.count)} 项 · ${formatBytes(selectedStats.total)}`,
                        )
                      : t(
                          `Delete ${formatCount(selectedStats.count)} items · ${formatBytes(selectedStats.total)}`,
                          `将永久删除 ${formatCount(selectedStats.count)} 项 · ${formatBytes(selectedStats.total)}`,
                        )}
                  </b>
                  {preflight && preflight.blocked.length > 0 ? (
                    <small>
                      {t(
                        `${preflight.blocked.length} items changed since the scan and were dropped from this run`,
                        `其中 ${preflight.blocked.length} 项在扫描后情况变化，本次已自动移出`,
                      )}
                    </small>
                  ) : (
                    <small>
                      {t(
                        "Esc cancels · every path was re-checked just now",
                        "按 Esc 取消 · 刚才已逐路径复核",
                      )}
                    </small>
                  )}
                </span>
                <Button
                  variant="ghost"
                  size="sm"
                  onClick={() => setInlineConfirm(false)}
                  disabled={busy}
                >
                  {t("Cancel", "取消")}
                </Button>
                <Button
                  variant="primary"
                  size="sm"
                  disabled={busy || preflightBusy}
                  icon={<Trash2 size={14} />}
                  onClick={() => void cleanSelected()}
                >
                  {t("Confirm clean", "确认清理")}
                </Button>
              </div>
            ) : (
              <Button
                variant="primary"
                icon={<Trash2 size={16} />}
                disabled={selectedStats.count === 0 || preflightBusy}
                onClick={() => void startClean()}
              >
                {t("Clean now", "立即清理")}
              </Button>
            )}
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
        {preflight && preflight.checked > 0 ? (
          <details className="confirm-preflight">
            <summary>
              <ShieldCheck size={14} />
              {preflight.blocked.length > 0
                ? t(
                    `Re-checked ${preflight.checked} paths just now — ${preflight.blocked.length} changed and were dropped`,
                    `刚刚逐路径复核过 ${preflight.checked} 项 — ${preflight.blocked.length} 项情况有变，已移出本次`,
                  )
                : t(
                    `Re-checked ${preflight.checked} paths just now — all still safe to remove`,
                    `刚刚逐路径复核过 ${preflight.checked} 项 — 全部仍可安全删除`,
                  )}
            </summary>
            {preflight.blocked.length > 0 ? (
              <ul>
                {preflight.blocked.slice(0, 8).map((row) => (
                  <li key={row.path}>
                    <code>{row.path}</code>
                    <small>{row.reason}</small>
                  </li>
                ))}
              </ul>
            ) : (
              <p>
                {t(
                  "Locks, whitelist, running apps and vanished paths were all re-verified at the click of this button.",
                  "锁、白名单、运行中的应用与已消失的路径，都在点击瞬间重新核验过。",
                )}
              </p>
            )}
          </details>
        ) : null}
      </ConfirmDialog>
    </div>
  );
}
