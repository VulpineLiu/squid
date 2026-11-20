import {
  AlertTriangle,
  AppWindow,
  Boxes,
  Check,
  ChevronDown,
  Code2,
  DownloadCloud,
  FileWarning,
  Globe2,
  Languages,
  LoaderCircle,
  PackageOpen,
  Puzzle,
  Trash2,
  X,
} from "lucide-react";
import {
  createContext,
  useCallback,
  useContext,
  useEffect,
  useMemo,
  useRef,
  useState,
  type CSSProperties,
  type ReactNode,
} from "react";
import { useI18n } from "../i18n";
import { formatBytes, formatCount, percent, shortPath } from "../lib/format";
import type { Category, JobProgress, LockReason } from "../types";

export interface CategoryMeta {
  en: string;
  zh: string;
  blurbEn: string;
  blurbZh: string;
  icon: typeof Boxes;
  tone: string;
}

export const CATEGORY_META: Record<Category, CategoryMeta> = {
  "system-junk": {
    en: "System junk",
    zh: "系统垃圾",
    blurbEn: "Temporary files and system scratch data",
    blurbZh: "临时文件与系统缓存",
    icon: Trash2,
    tone: "blue",
  },
  "app-caches": {
    en: "App caches",
    zh: "应用缓存",
    blurbEn: "Caches apps can rebuild",
    blurbZh: "应用可自动重建的缓存",
    icon: AppWindow,
    tone: "cyan",
  },
  "browser-caches": {
    en: "Browser caches",
    zh: "浏览器缓存",
    blurbEn: "Web content and offline data",
    blurbZh: "网页内容与离线数据",
    icon: Globe2,
    tone: "violet",
  },
  logs: {
    en: "Logs & reports",
    zh: "日志与报告",
    blurbEn: "Rotated logs and crash reports",
    blurbZh: "旧日志与崩溃报告",
    icon: FileWarning,
    tone: "orange",
  },
  developer: {
    en: "Developer files",
    zh: "开发文件",
    blurbEn: "Simulators, archives and tool caches",
    blurbZh: "模拟器、归档与工具缓存",
    icon: Code2,
    tone: "indigo",
  },
  "project-artifacts": {
    en: "Build artefacts",
    zh: "构建产物",
    blurbEn: "Regenerable project build folders",
    blurbZh: "可重新生成的构建目录",
    icon: PackageOpen,
    tone: "teal",
  },
  trash: {
    en: "Recycle Bin",
    zh: "回收站",
    blurbEn: "Files waiting to be emptied",
    blurbZh: "等待清空的文件",
    icon: Trash2,
    tone: "slate",
  },
  "language-packs": {
    en: "Language packs",
    zh: "语言包",
    blurbEn: "Unused voice and localisation data",
    blurbZh: "未使用的语音与本地化数据",
    icon: Languages,
    tone: "pink",
  },
  leftovers: {
    en: "Leftovers",
    zh: "卸载残留",
    blurbEn: "Data left by removed applications",
    blurbZh: "已删除应用留下的数据",
    icon: Puzzle,
    tone: "amber",
  },
  "update-installers": {
    en: "Update installers",
    zh: "升级安装包",
    blurbEn: "Installer packages apps keep after updating",
    blurbZh: "应用升级后留下的安装包，需要时会自动重新下载",
    icon: DownloadCloud,
    tone: "green",
  },
};

export const CATEGORY_ORDER = Object.keys(CATEGORY_META) as Category[];

export function categoryLabel(
  category: Category,
  language: "zh" | "en",
  binName?: string,
): string {
  // The bin category borrows the OS word for it: 回收站 on Windows, 废纸篓 on macOS.
  if (category === "trash" && binName) return binName;
  return language === "zh" ? CATEGORY_META[category].zh : CATEGORY_META[category].en;
}

export function lockLabel(
  lock: LockReason | null,
  t: (en: string, zh: string) => string,
): string {
  if (!lock) return "";
  switch (lock) {
    case "app-running":
      return t("App is running", "应用正在运行");
    case "shared-with-other-install":
      return t("Shared with another install", "与其他安装共用");
    case "whitelisted":
      return t("Protected by you", "已加入保护名单");
    case "not-owned":
      return t("Needs administrator rights", "需要管理员权限");
    case "protected":
      return t("Protected by the safety policy", "受安全策略保护");
  }
}

export function PageHeader({
  eyebrow,
  title,
  description,
  actions,
}: {
  eyebrow?: string;
  title: string;
  description: string;
  actions?: ReactNode;
}) {
  return (
    <header className="page-header">
      <div>
        {eyebrow ? <div className="page-eyebrow">{eyebrow}</div> : null}
        <h1>{title}</h1>
        <p>{description}</p>
      </div>
      {actions ? <div className="page-header-actions">{actions}</div> : null}
    </header>
  );
}

export function Button({
  children,
  variant = "secondary",
  size = "md",
  icon,
  className = "",
  ...props
}: React.ButtonHTMLAttributes<HTMLButtonElement> & {
  variant?: "primary" | "secondary" | "ghost" | "danger";
  size?: "sm" | "md" | "lg";
  icon?: ReactNode;
}) {
  return (
    <button className={`button is-${variant} is-${size} ${className}`} {...props}>
      {icon ? <span className="button-icon">{icon}</span> : null}
      <span>{children}</span>
    </button>
  );
}

export function IconButton({
  label,
  children,
  className = "",
  ...props
}: React.ButtonHTMLAttributes<HTMLButtonElement> & { label: string }) {
  return (
    <button className={`icon-button ${className}`} aria-label={label} title={label} {...props}>
      {children}
    </button>
  );
}

export function Toggle({
  checked,
  onChange,
  disabled,
  label,
}: {
  checked: boolean;
  onChange: (next: boolean) => void;
  disabled?: boolean;
  label: string;
}) {
  return (
    <button
      type="button"
      role="switch"
      aria-checked={checked}
      aria-label={label}
      className={`toggle ${checked ? "is-on" : ""}`}
      disabled={disabled}
      onClick={() => onChange(!checked)}
    >
      <span />
    </button>
  );
}

export function Checkbox({
  checked,
  indeterminate = false,
  disabled = false,
  onChange,
  label,
}: {
  checked: boolean;
  indeterminate?: boolean;
  disabled?: boolean;
  onChange: () => void;
  label: string;
}) {
  return (
    <button
      type="button"
      role="checkbox"
      aria-checked={indeterminate ? "mixed" : checked}
      aria-label={label}
      disabled={disabled}
      className={`checkbox ${checked ? "is-checked" : ""} ${
        indeterminate ? "is-mixed" : ""
      }`}
      onClick={(event) => {
        event.stopPropagation();
        onChange();
      }}
    >
      {checked ? <Check size={13} strokeWidth={3} /> : null}
      {!checked && indeterminate ? <span /> : null}
    </button>
  );
}

export function ProgressCard({
  progress,
  title,
  onCancel,
  compact = false,
  liveLine,
  capAt99 = false,
}: {
  progress: JobProgress | null;
  title: string;
  onCancel?: () => void;
  compact?: boolean;
  /** "已统计 128,431 文件 · 38.2 GB" streamed side-band (A①). */
  liveLine?: string | null;
  /** Scans hold the bar at 99% until the final number lands — a full bar
   * that is still working reads as a lie (Mole's cap discipline). */
  capAt99?: boolean;
}) {
  const { t, lang } = useI18n();
  const phase = useMemo(() => {
    if (!progress?.phase) return title;
    const labels: Record<string, [string, string]> = {
      match: ["Finding candidates", "正在查找可清理项目"],
      measure: ["Measuring sizes", "正在计算大小"],
      purge: ["Searching for build artefacts", "正在查找构建产物"],
      verify: ["Checking Git and file ages", "正在核对 Git 与文件年龄"],
      delete: ["Removing approved items", "正在删除已确认项目"],
      apps: ["Reading installed applications", "正在读取已安装应用"],
      plan: ["Finding application leftovers", "正在查找应用残留"],
      storage: ["Measuring folders", "正在测量文件夹"],
      files: ["Finding the largest files", "正在查找大文件"],
      orphans: ["Finding unused leftovers", "正在查找孤立残留"],
    };
    const pair = labels[progress.phase];
    return pair ? (lang === "zh" ? pair[1] : pair[0]) : title;
  }, [lang, progress?.phase, t, title]);
  const denominator = progress?.total ?? 0;
  const done = progress?.done ?? 0;
  const ratio =
    denominator > 0
      ? Math.min(capAt99 ? 99 : 100, Math.max(0, (done / denominator) * 100))
      : null;
  const detail = progress?.detail || progress?.subject || "";

  return (
    <section className={`progress-card ${compact ? "is-compact" : ""}`}>
      <div className="progress-orbit" aria-hidden="true">
        <span />
        <span />
        <span />
        <LoaderCircle size={24} className="spin" />
      </div>
      <div className="progress-copy">
        <div className="progress-title-row">
          <strong>{phase}</strong>
          {denominator > 0 ? (
            <span className="progress-count">
              {formatCount(done)} / {formatCount(denominator)}
            </span>
          ) : null}
        </div>
        <div
          className={`progress-track ${ratio === null ? "is-indeterminate" : ""}`}
          role="progressbar"
          aria-valuemin={0}
          aria-valuemax={100}
          aria-valuenow={ratio ?? undefined}
        >
          <span style={{ width: ratio === null ? undefined : `${ratio}%` }} />
        </div>
        {liveLine ? (
          <div className="progress-live" role="status">
            {liveLine}
          </div>
        ) : null}
        <div className="progress-detail" title={detail}>
          {detail || t("Working safely in the background…", "正在后台安全处理…")}
        </div>
      </div>
      {onCancel ? (
        <Button variant="ghost" size="sm" onClick={onCancel}>
          {t("Cancel", "取消")}
        </Button>
      ) : null}
    </section>
  );
}

export function InlineError({
  message,
  onDismiss,
}: {
  message: string;
  onDismiss?: () => void;
}) {
  const { t } = useI18n();
  return (
    <div className="inline-error" role="alert">
      <AlertTriangle size={17} />
      <span>{message}</span>
      {onDismiss ? (
        <IconButton label={t("Dismiss", "关闭")} onClick={onDismiss}>
          <X size={14} />
        </IconButton>
      ) : null}
    </div>
  );
}

export function Modal({
  open,
  onClose,
  title,
  subtitle,
  children,
  footer,
  size = "md",
  tone = "default",
  dismissible = true,
}: {
  open: boolean;
  onClose: () => void;
  title: ReactNode;
  subtitle?: ReactNode;
  children: ReactNode;
  footer?: ReactNode;
  size?: "sm" | "md" | "lg" | "drawer";
  tone?: "default" | "danger" | "success";
  dismissible?: boolean;
}) {
  const { t } = useI18n();
  const panel = useRef<HTMLDivElement>(null);

  useEffect(() => {
    if (!open) return;
    const onKey = (event: KeyboardEvent) => {
      if (event.key === "Escape" && dismissible) onClose();
    };
    document.addEventListener("keydown", onKey);
    const timer = window.setTimeout(() => {
      panel.current?.querySelector<HTMLElement>(
        "button:not([disabled]), input:not([disabled]), select:not([disabled])",
      )?.focus();
    }, 30);
    return () => {
      document.removeEventListener("keydown", onKey);
      window.clearTimeout(timer);
    };
  }, [dismissible, onClose, open]);

  if (!open) return null;
  return (
    <div
      className="modal-backdrop"
      role="presentation"
      onMouseDown={(event) => {
        if (event.target === event.currentTarget && dismissible) onClose();
      }}
    >
      <div
        ref={panel}
        className={`modal-panel is-${size} is-${tone}`}
        role="dialog"
        aria-modal="true"
        aria-label={typeof title === "string" ? title : undefined}
      >
        <header className="modal-head">
          <div>
            <h2>{title}</h2>
            {subtitle ? <p>{subtitle}</p> : null}
          </div>
          {dismissible ? (
            <IconButton label={t("Close", "关闭")} onClick={onClose}>
              <X size={17} />
            </IconButton>
          ) : null}
        </header>
        <div className="modal-body">{children}</div>
        {footer ? <footer className="modal-footer">{footer}</footer> : null}
      </div>
    </div>
  );
}

export function ConfirmDialog({
  open,
  title,
  message,
  note,
  confirmLabel,
  cancelLabel,
  tone = "danger",
  icon,
  onConfirm,
  onClose,
  busy = false,
  children,
}: {
  open: boolean;
  title: string;
  message: ReactNode;
  note?: ReactNode;
  confirmLabel: string;
  cancelLabel: string;
  tone?: "danger" | "default";
  icon?: ReactNode;
  onConfirm: () => void;
  onClose: () => void;
  busy?: boolean;
  children?: ReactNode;
}) {
  return (
    <Modal
      open={open}
      onClose={onClose}
      title={title}
      size="sm"
      dismissible={!busy}
      footer={
        <>
          <Button variant="ghost" onClick={onClose} disabled={busy}>
            {cancelLabel}
          </Button>
          <Button
            variant={tone === "danger" ? "danger" : "primary"}
            onClick={onConfirm}
            disabled={busy}
            icon={busy ? <LoaderCircle size={15} className="spin" /> : icon}
          >
            {confirmLabel}
          </Button>
        </>
      }
    >
      <div className={`confirm-content is-${tone}`}>
        <div className="confirm-icon">{icon ?? <AlertTriangle size={24} />}</div>
        <div>
          <div className="confirm-message">{message}</div>
          {note ? <div className="confirm-note">{note}</div> : null}
        </div>
      </div>
      {children ? <div className="confirm-extra">{children}</div> : null}
    </Modal>
  );
}

export function Gauge({
  value,
  size = 160,
  stroke = 12,
  tone = "blue",
  children,
  label,
}: {
  value: number;
  size?: number;
  stroke?: number;
  tone?: "blue" | "green" | "orange" | "violet" | "red";
  children?: ReactNode;
  label?: string;
}) {
  const normalized = Math.max(0, Math.min(100, value));
  const radius = (size - stroke) / 2;
  const circumference = Math.PI * 2 * radius;
  return (
    <div
      className={`gauge tone-${tone}`}
      style={{ width: size, height: size } as CSSProperties}
      role="img"
      aria-label={label ?? `${Math.round(normalized)}%`}
    >
      <svg width={size} height={size} viewBox={`0 0 ${size} ${size}`}>
        <circle
          className="gauge-track"
          cx={size / 2}
          cy={size / 2}
          r={radius}
          fill="none"
          strokeWidth={stroke}
        />
        <circle
          className="gauge-value"
          cx={size / 2}
          cy={size / 2}
          r={radius}
          fill="none"
          strokeWidth={stroke}
          strokeLinecap="round"
          strokeDasharray={circumference}
          strokeDashoffset={circumference * (1 - normalized / 100)}
          transform={`rotate(-90 ${size / 2} ${size / 2})`}
        />
      </svg>
      <div className="gauge-center">{children}</div>
    </div>
  );
}

export function MetricBar({
  value,
  tone = "blue",
}: {
  value: number;
  tone?: "blue" | "green" | "orange" | "violet" | "red";
}) {
  return (
    <div className={`metric-bar tone-${tone}`}>
      <span style={{ width: `${Math.max(0, Math.min(100, value))}%` }} />
    </div>
  );
}

export function EmptyState({
  icon,
  title,
  description,
  action,
}: {
  icon: ReactNode;
  title: string;
  description: string;
  action?: ReactNode;
}) {
  return (
    <div className="empty-state">
      <div className="empty-icon">{icon}</div>
      <h2>{title}</h2>
      <p>{description}</p>
      {action}
    </div>
  );
}

export function SectionTitle({
  title,
  subtitle,
  action,
}: {
  title: string;
  subtitle?: string;
  action?: ReactNode;
}) {
  return (
    <div className="section-title">
      <div>
        <h2>{title}</h2>
        {subtitle ? <p>{subtitle}</p> : null}
      </div>
      {action}
    </div>
  );
}

export function CategoryCard({
  category,
  total,
  count,
  selected,
  disabled,
  onToggle,
  onInspect,
  focused = false,
}: {
  category: Category;
  total: number;
  count: number;
  selected: boolean;
  disabled?: boolean;
  onToggle: () => void;
  onInspect: () => void;
  focused?: boolean;
}) {
  const { lang, t, bin } = useI18n();
  const meta = CATEGORY_META[category];
  const Icon = meta.icon;
  return (
    <article
      className={`category-card tone-${meta.tone} ${selected ? "is-selected" : ""} ${
        disabled ? "is-disabled" : ""
      } ${focused ? "is-focused" : ""}`}
      onClick={disabled ? undefined : onToggle}
    >
      <div className="category-card-top">
        <span className="category-icon">
          <Icon size={21} />
        </span>
        <Checkbox
          checked={selected}
          disabled={disabled}
          onChange={onToggle}
          label={categoryLabel(category, lang, bin)}
        />
      </div>
      <div className="category-copy">
        <strong>{categoryLabel(category, lang, bin)}</strong>
        <span>{lang === "zh" ? meta.blurbZh : meta.blurbEn}</span>
      </div>
      <div className="category-card-foot">
        <span className="category-size">{formatBytes(total)}</span>
        <button
          className="text-button"
          onClick={(event) => {
            event.stopPropagation();
            onInspect();
          }}
        >
          {t(`${formatCount(count)} items`, `${formatCount(count)} 项`)}
          <ChevronDown size={13} />
        </button>
      </div>
    </article>
  );
}

type ToastTone = "default" | "success" | "danger";
interface ToastItem {
  id: number;
  message: string;
  tone: ToastTone;
}

const ToastContext = createContext<{
  push: (message: string, tone?: ToastTone) => void;
} | null>(null);

export function ToastProvider({ children }: { children: ReactNode }) {
  const [toasts, setToasts] = useState<ToastItem[]>([]);
  const sequence = useRef(0);

  const remove = useCallback((id: number) => {
    setToasts((current) => current.filter((toast) => toast.id !== id));
  }, []);

  const push = useCallback(
    (message: string, tone: ToastTone = "default") => {
      const id = ++sequence.current;
      setToasts((current) => [...current.slice(-3), { id, message, tone }]);
      window.setTimeout(() => remove(id), 4200);
    },
    [remove],
  );

  return (
    <ToastContext.Provider value={{ push }}>
      {children}
      <div className="toast-stack" aria-live="polite">
        {toasts.map((toast) => (
          <div key={toast.id} className={`toast is-${toast.tone}`}>
            {toast.tone === "success" ? <Check size={16} /> : null}
            {toast.tone === "danger" ? <AlertTriangle size={16} /> : null}
            <span>{toast.message}</span>
            <button onClick={() => remove(toast.id)} aria-label="Dismiss">
              <X size={13} />
            </button>
          </div>
        ))}
      </div>
    </ToastContext.Provider>
  );
}

export function useToast() {
  const value = useContext(ToastContext);
  if (!value) throw new Error("useToast must be used inside ToastProvider");
  return value;
}

export function ResultSummary({
  removed,
  freed,
  failed,
  mode,
  onAgain,
  onHistory,
}: {
  removed: number;
  freed: number;
  failed: number;
  mode: string | null;
  onAgain: () => void;
  onHistory: () => void;
}) {
  const { t, lang, bin } = useI18n();
  return (
    <div className="result-summary">
      <div className="result-check" aria-hidden="true">
        <Check size={28} strokeWidth={2.6} />
      </div>
      <div className="result-copy">
        <span className="result-eyebrow">{t("Cleanup complete", "清理完成")}</span>
        <h2>
          {t(
            `${formatBytes(freed)} reclaimed`,
            `已释放 ${formatBytes(freed)}`,
          )}
        </h2>
        <p>
          {t(
            `${formatCount(removed)} items removed${
              failed > 0 ? ` · ${failed} could not be removed` : ""
            }.`,
            `已处理 ${formatCount(removed)} 项${
              failed > 0 ? ` · ${failed} 项未能删除` : ""
            }。`,
          )}
          {mode === "trash"
            ? t(
                ` Items are in the ${bin} and can still be restored from History.`,
                ` 文件已移入${bin}，可在历史记录中还原。`,
              )
            : ""}
        </p>
      </div>
      <div className="result-actions">
        <Button variant="ghost" onClick={onHistory}>
          {t("View history", "查看历史")}
        </Button>
        <Button variant="primary" onClick={onAgain}>
          {t("Scan again", "再次扫描")}
        </Button>
      </div>
    </div>
  );
}

export function PathRow({
  path,
  note,
  size,
  selected,
  disabled,
  onToggle,
  onReveal,
}: {
  path: string;
  note?: string | null;
  size: number;
  selected: boolean;
  disabled: boolean;
  onToggle: () => void;
  onReveal: () => void;
}) {
  const { t, lang } = useI18n();
  return (
    <div className={`path-row ${disabled ? "is-disabled" : ""}`}>
      <Checkbox
        checked={selected}
        disabled={disabled}
        onChange={onToggle}
        label={lang === "zh" ? "选择项目" : "Select item"}
      />
      <div className="path-row-copy">
        <span className="path-row-path" title={path}>
          {shortPath(path, 78)}
        </span>
        {note ? (
          <span className="path-row-note">{note}</span>
        ) : null}
      </div>
      <span className="path-row-size">{formatBytes(size)}</span>
      <IconButton label={t("Show in folder", "在文件夹中显示")} onClick={onReveal}>
        <ChevronDown size={15} className="reveal-arrow" />
      </IconButton>
    </div>
  );
}

export function UsageBar({
  used,
  total,
  tone = "blue",
}: {
  used: number;
  total: number;
  tone?: "blue" | "green" | "orange" | "violet" | "red";
}) {
  return (
    <div className={`usage-bar tone-${tone}`}>
      <span style={{ width: `${percent(used, total)}%` }} />
    </div>
  );
}
