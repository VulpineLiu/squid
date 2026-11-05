export type Language = "zh" | "en";
export type PageId =
  | "overview"
  | "cleanup"
  | "applications"
  | "purge"
  | "storage"
  | "system"
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
  | "leftovers";
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
  event: "phase" | "tick" | "note" | "warning" | "deleted" | "bucket";
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
  effectiveLanguage: Language;
  historyKeep: number;
  setupReviewed: boolean;
  errors: string[];
}

export interface PermissionStatus {
  fullDiskAccess: "granted" | "denied" | "unknown" | "not-required";
}

export interface Bootstrap {
  version: string;
  platform: "macos" | "windows" | "linux" | "unknown";
  osName: string;
  osVersion: string;
  hardware: string;
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
  memory: {
    total: number;
    used: number;
    available: number;
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
  is_dir: boolean;
  readable: boolean;
  children: StorageNode[];
  folded: number;
  folded_size: number;
  complete: boolean;
}

export interface LargestFile {
  path: string;
  logical: number;
  allocated: number;
  modified_ms: number | null;
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
