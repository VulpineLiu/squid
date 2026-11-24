export type Language = "zh" | "en";
export type PageId =
  | "overview"
  | "space"
  | "applications"
  | "permissions"
  | "widget"
  | "storage"
  | "duplicates"
  | "large-files"
  | "system"
  | "maintenance"
  | "privacy"
  | "optimization"
  | "history"
  | "settings";

export type DeleteMode = "trash" | "permanent";
export type Category =
  | "system-junk"
  | "app-caches"
  | "browser-caches"
  | "logs"
  | "developer"
  | "project-artifacts"
  | "trash"
  | "language-packs"
  | "leftovers"
  | "update-installers"
  | "duplicates"
  | "ai-tools"
  | "privacy";
export type ItemKind = "path" | "contents" | "file";
export type LockReason =
  | "app-running"
  | "shared-with-other-install"
  | "whitelisted"
  | "not-owned"
  | "protected";

export interface CleanupItem {
  path: string;
  displayPath: string;
  kind: ItemKind;
  size: number;
  allocated: number;
  onDisk: number;
  category: Category;
  label: string;
  ruleId: string;
  selected: boolean;
  lock: LockReason | null;
  note: string | null;
  /** Seconds since the newest last-write inside the item (A③). */
  lastWriteSecs?: number | null;
  fileCount?: number;
  /** Machine tokens for the B3 evidence panel ("lock:package-lock.json"). */
  evidence?: string[];
}

export interface ScanResult {
  scanId: string;
  items: CleanupItem[];
  total: number;
  totalSelected: number;
  selectedCount: number;
  unreadable: number;
  elapsedMs: number;
}

export interface DeleteResult {
  freed: number;
  freedAllocated: number;
  removed: number;
  failed: number;
  errors: string[];
  mode: DeleteMode | null;
  sessionId: string | null;
  skipped: { path: string; reason: string }[];
}

export interface JobProgress {
  jobId: string;
  kind: string;
  event:
    | "phase"
    | "tick"
    | "note"
    | "warning"
    | "deleted"
    | "bucket"
    | "metered"
    | "group";
  phase: string | null;
  detail: string | null;
  done: number | null;
  total: number | null;
  subject: string | null;
  freed: number | null;
}

export interface SettingsView {
  deleteMode: DeleteMode;
  whitelist: string[];
  purgeRoots: string[];
  purgeMinAgeDays: number;
  showAllocated: boolean;
  language: string | null;
  historyKeep: number;
  setupReviewed: boolean;
  errors: string[];
}

export interface PermissionStatus {
  fullDiskAccess: "granted" | "denied" | "unknown" | "not-required";
}

export interface MemoryStick {
  manufacturer: string;
  size_bytes: number;
  speed_mhz: number;
}

/** Static component facts, snake_case straight from squid-platform. */
export interface HardwareInfo {
  board: string;
  cpu_model: string;
  memory_sticks: MemoryStick[];
  disk_vendor: string;
  disk_model: string;
  disk_bus: string;
  disk_size_bytes: number;
  gpu_vendor: string;
  gpu_model: string;
  gpu_vram_bytes: number;
}

export interface Bootstrap {
  version: string;
  platform: "macos" | "windows" | "linux" | "unknown";
  osName: string;
  osVersion: string;
  hardware: string;
  hardwareInfo: HardwareInfo;
  arch: string;
  device: "mac" | "pc" | "machine";
  language: Language;
  binNoun: "trash" | "recycle-bin";
  configPath: string;
  historyPath: string;
  updateFeedUrl: string;
  updateCheckEnabled: boolean;
  settings: SettingsView;
}

export interface AppInfo {
  name: string;
  bundle_id: string | null;
  path: string;
  version: string | null;
  size: number;
  source: string;
  running: boolean;
  has_sibling_install: boolean;
  publisher: string | null;
  installed_ms: number | null;
  display_icon: string | null;
  /** data-URL PNG icon (apps_scan only; null when extraction failed). */
  icon?: string | null;
}

export interface AppsResult {
  apps: AppInfo[];
  total: number;
}

export interface PlanResult {
  planId: string;
  app: AppInfo;
  items: CleanupItem[];
  warnings: string[];
  total: number;
  selectedCount: number;
  selectedTotal: number;
}

export interface Volume {
  name: string;
  mount_point: string;
  total: number;
  free: number;
  is_root: boolean;
  is_removable: boolean;
  filesystem: string;
}

export interface ProcessSample {
  pid: number;
  name: string;
  cpu: number;
  memory: number;
}

export interface Battery {
  percent: number;
  charging: boolean;
  on_ac: boolean;
  present: boolean;
  minutes_to_full: number | null;
  minutes_to_empty: number | null;
  health_percent: number | null;
  cycle_count: number | null;
  temperature_c: number | null;
}

export interface SystemSnapshot {
  taken_at_ms: number;
  cpu: {
    total: number;
    per_core: number[];
    load: [number, number, number];
    physical_cores: number;
    logical_cores: number;
    brand: string;
    base_frequency_ghz: number;
  };
  gpu_usage: number | null;
  memory: {
    total: number;
    used: number;
    available: number;
    installed: number;
    swap_total: number;
    swap_used: number;
    compressed: number | null;
    wired: number | null;
  };
  network: {
    rx_per_sec: number;
    tx_per_sec: number;
    rx_total: number;
    tx_total: number;
    interfaces: number;
    primary: string | null;
  };
  disk: {
    read_per_sec: number;
    write_per_sec: number;
    activity: number;
    disks: number;
  };
  uptime_secs: number;
  processes: number;
  top_processes: ProcessSample[];
  volumes: Volume[];
  battery: Battery | null;
  host: {
    os_name: string;
    os_version: string;
    hardware: string;
    hostname: string;
    user: string;
    arch: string;
  };
}

export interface StorageNode {
  path: string;
  logical: number;
  allocated: number;
  files: number;
  dirs: number;
  isDir: boolean;
  readable: boolean;
  children: StorageNode[];
  folded: number;
  foldedSize: number;
  complete: boolean;
}

/** storage_explore now answers with freshness metadata (A⑥ size cache). */
export interface ExploreResult {
  node: StorageNode;
  cached: boolean;
  measuredAtMs: number | null;
}

/** Hidden-space insight row (A⑤, Mole overview idiom). */
export interface SpaceInsight {
  id: string;
  title: string;
  path: string;
  size: number;
  action: "cleanable" | "command" | "external" | "info";
  command: string | null;
  hint: string;
}

/** cleanup_preflight verdict (B1 last-second re-check). */
export interface PreflightResult {
  checked: number;
  blocked: { path: string; reason: string }[];
}

export interface LargestFile {
  path: string;
  logical: number;
  allocated: number;
  modified_ms: number | null;
}

// ---- duplicate files (重复文件) ------------------------------------------

export interface DupCopy {
  /** Display spelling; `~` folded for the home folder. */
  path: string;
  mtimeMs: number;
  /** Why this copy may not be deleted (protected user asset), when it may not. */
  locked: string | null;
}

export interface DupGroup {
  size: number;
  copies: DupCopy[];
  /** Index the UI marks 「保留」 by default. */
  keep: number;
  /** At least one copy sits in a protected personal folder. */
  personal: boolean;
}

export interface DupReport {
  groups: DupGroup[];
  walked: number;
  sameSize: number;
  sampleHits: number;
  confirmedGroups: number;
  confirmedFiles: number;
  wasted: number;
}

export interface DupsResult {
  scanId: string;
  report: DupReport;
  elapsedMs: number;
}

// ---- large files (大文件) -------------------------------------------------

export interface LargeFile {
  path: string;
  displayPath: string;
  size: number;
  modifiedMs: number;
  class: "video" | "archive" | "installer" | "image" | "other";
  /** A cloud-sync folder sits above this file — there may be a copy online. */
  cloudHint: boolean;
  /** Disk image: something (Hyper-V/WSL) may hold it open. */
  maybeMounted: boolean;
}

export interface LedgerEntry {
  path: string;
  kind: ItemKind;
  logical: number;
  allocated: number;
  trash_path: string | null;
  category: string | null;
}

export interface HistorySession {
  id: string;
  started: string;
  command: string;
  mode: DeleteMode;
  logical_freed: number;
  allocated_freed: number;
  removed: number;
  failed: number;
  entries: LedgerEntry[];
  duration_ms: number;
  restored: boolean;
}

export interface UpdateStatus {
  state: "current" | "available" | "ready" | "unavailable" | "error";
  currentVersion: string;
  availableVersion: string;
  fileName: string;
  size: number;
  sha256: string;
  notes: string;
  date: string;
  path: string | null;
  message: string;
  feedUrl: string;
  signed: boolean;
  artifactKey: string;
}

export interface UpdateProgress {
  downloaded: number;
  total: number;
  state: string;
}

export interface UpdateCheckResult {
  status: UpdateStatus;
  downloadReady: boolean;
}

export interface UndoResult {
  restored: number;
  skipped: number;
}


// ---- permissions (权限管理) ---------------------------------------------

export interface StartupItem {
  id: string;
  name: string;
  displayName: string;
  /** data-URL PNG of the binary's icon; null on platforms without extraction. */
  icon: string | null;
  command: string;
  source: string;
  sourceLabel: string;
  scope: string;
  enabled: boolean;
  /** Attributed to a real product; false lands in the 其他软件 group. */
  named: boolean;
}

export interface ShellExtension {
  id: string;
  name: string;
  handler: string;
  extensionPoint: string;
  scope: string;
  friendlyName: string | null;
  module: string | null;
  /** publisher (CompanyName) of the owning dll — 电脑管家's 创建者 column. */
  publisher: string | null;
  /** data-URL PNG of the owning module's icon; null without extraction. */
  icon: string | null;
  registered: boolean;
}

export interface PopupWindowItem {
  hwnd: number;
  title: string;
  process: string;
  exe: string | null;
  /** data-URL PNG of the owning process's icon; null without extraction. */
  icon: string | null;
}

export interface PopupRuleItem {
  exe: string;
  title: string;
  matcher: string;
  enabled: boolean;
  closed: number;
  /** data-URL PNG of the rule's exe, filled by manage_scan. */
  icon?: string | null;
}

export interface ManageView {
  watcherEnabled: boolean;
  intervalMs: number;
  rules: PopupRuleItem[];
  windows: PopupWindowItem[];
  startup: StartupItem[];
  extensions: ShellExtension[];
  startupCount: number;
  startupEnabledCount: number;
}

export interface PopupClosedEvent {
  process: string;
  title: string;
  ruleIndex: number;
}

// ---- desktop widget (桌面小工具) ------------------------------------------

export type WidgetCorner = "top-right" | "top-left" | "bottom-right" | "bottom-left";

export interface WidgetPrefs {
  enabled: boolean;
  panelRun: boolean;
  panelCpu: boolean;
  panelNetwork: boolean;
  panelDisk: boolean;
  panelStorage: boolean;
  /** 90–100 percent window opacity. */
  opacity: number;
  intervalMs: number;
  corner: WidgetCorner;
  rememberCollapsed: boolean;
  collapsed: boolean;
  position: [number, number] | null;
}

export interface WidgetPatch {
  enabled?: boolean;
  panelRun?: boolean;
  panelCpu?: boolean;
  panelNetwork?: boolean;
  panelDisk?: boolean;
  panelStorage?: boolean;
  opacity?: number;
  intervalMs?: number;
  corner?: WidgetCorner;
  rememberCollapsed?: boolean;
  collapsed?: boolean;
  position?: [number, number] | null;
  keepPosition?: boolean;
  redock?: boolean;
}
