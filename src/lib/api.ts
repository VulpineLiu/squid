import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import type {
  AppsResult,
  Bootstrap,
  DeleteMode,
  DeleteResult,
  HistorySession,
  JobProgress,
  LargestFile,
  IntelStatus,
  PageId,
  PermissionStatus,
  PlanResult,
  ScanResult,
  SettingsView,
  SecurityAction,
  SecurityScan,
  SecuritySettings,
  SecurityStatus,
  StartupChange,
  StartupView,
  StorageNode,
  SystemSnapshot,
  ProcessDetail,
  ProcessRow,
  UndoResult,
  UpdateCheckResult,
  UpdateProgress,
  UpdateStatus,
} from "../types";

export interface SettingsPatch {
  deleteMode?: DeleteMode;
  language?: string;
  showAllocated?: boolean;
  purgeMinAgeDays?: number;
  historyKeep?: number;
  setupReviewed?: boolean;
}

export interface SecurityPatch {
  scanSchedule?: string;
  notifyOnFindings?: boolean;
  autoQuarantine?: boolean;
  updateRules?: boolean;
}

export const api = {
  bootstrap: () => invoke<Bootstrap>("bootstrap"),
  cancelJob: (jobId: string) => invoke<boolean>("cancel_job", { jobId }),
  cleanupScan: (
    jobId: string,
    categories: string[] = [],
    minAgeDays: number | null = null,
    orphans = false,
  ) =>
    invoke<ScanResult>("cleanup_scan", {
      jobId,
      categories,
      minAgeDays,
      orphans,
    }),
  cleanupDelete: (jobId: string, scanId: string, selected: number[], mode: DeleteMode) =>
    invoke<DeleteResult>("cleanup_delete", { jobId, scanId, selected, mode }),
  purgeScan: (jobId: string, roots: string[] = [], minAgeDays: number | null = null, force = false) =>
    invoke<ScanResult>("purge_scan", { jobId, roots, minAgeDays, force }),
  purgeDelete: (jobId: string, scanId: string, selected: number[], mode: DeleteMode) =>
    invoke<DeleteResult>("purge_delete", { jobId, scanId, selected, mode }),
  appsScan: (jobId: string) => invoke<AppsResult>("apps_scan", { jobId }),
  appIcon: (appKey: string) => invoke<string | null>("app_icon", { appKey }),
  appPlan: (jobId: string, appKey: string) => invoke<PlanResult>("app_plan", { jobId, appKey }),
  appUninstall: (jobId: string, planId: string, selected: number[], mode: DeleteMode) =>
    invoke<DeleteResult>("app_uninstall", { jobId, planId, selected, mode }),
  storageExplore: (jobId: string, path?: string | null, depth = 1, top = 16) =>
    invoke<StorageNode>("storage_explore", { jobId, path, depth, top }),
  storageLargestFiles: (jobId: string, path: string, limit = 40) =>
    invoke<LargestFile[]>("storage_largest_files", { jobId, path, limit }),
  systemSnapshot: () => invoke<SystemSnapshot>("system_snapshot"),
  stopProcess: (process: SystemSnapshot["top_processes"][number]) =>
    invoke<void>("stop_process", { process }),
  historyList: (limit = 60) => invoke<HistorySession[]>("history_list", { limit }),
  historyUndo: (sessionId: string) => invoke<UndoResult>("history_undo", { sessionId }),
  settingsSave: (patch: SettingsPatch) => invoke<SettingsView>("settings_save", { patch }),
  whitelistAdd: (path: string) => invoke<SettingsView>("whitelist_add", { path }),
  whitelistRemove: (pattern: string) => invoke<SettingsView>("whitelist_remove", { pattern }),
  autostartStatus: () => invoke<boolean>("autostart_status"),
  setAutostart: (enabled: boolean) => invoke<boolean>("set_autostart", { enabled }),
  revealPath: (path: string) => invoke<void>("reveal_path", { path }),
  openTrash: () => invoke<void>("open_trash"),
  permissionStatus: () => invoke<PermissionStatus>("permission_status"),
  openFullDiskAccessSettings: () => invoke<void>("open_full_disk_access_settings"),
  updateCheck: () => invoke<UpdateCheckResult>("update_check"),
  updateStatus: () => invoke<UpdateStatus | null>("update_status"),
  updateDownload: () => invoke<UpdateStatus>("update_download"),
  updateInstall: () => invoke<string>("update_install"),
  updateClear: () => invoke<void>("update_clear"),
  setUpdateCheckEnabled: (enabled: boolean) =>
    invoke<boolean>("set_update_check_enabled", { enabled }),
  securityStatus: () => invoke<SecurityStatus>("security_status"),
  securityScan: (jobId: string, mode: string, paths: string[] = []) =>
    invoke<SecurityScan>("security_scan", { jobId, request: { mode, paths } }),
  securityCancel: (jobId: string) => invoke<boolean>("security_cancel", { jobId }),
  securityOpen: () => invoke<void>("security_open"),
  securityQuarantine: (findingId: string) =>
    invoke<SecurityAction>("security_quarantine", { findingId }),
  securityRestore: (id: string) => invoke<SecurityAction>("security_restore", { id }),
  securityDeleteQuarantine: (id: string) =>
    invoke<SecurityAction>("security_delete_quarantine", { id }),
  securityIgnore: (findingId: string, ignored: boolean) =>
    invoke<SecurityAction>("security_ignore", { findingId, ignored }),
  securityExclusions: (action: "add" | "remove", path: string) =>
    invoke<SecuritySettings>("security_exclusions", { action, path }),
  securitySettingsSave: (patch: SecurityPatch) =>
    invoke<SecuritySettings>("security_settings_save", { patch }),
  securityUpdateRules: () => invoke<IntelStatus>("security_update_rules"),
  securityRollbackRules: () => invoke<IntelStatus>("security_rollback_rules"),
  openQuarantineFolder: () => invoke<void>("open_quarantine_folder"),
  startupItems: () => invoke<StartupView>("startup_items"),
  startupSetEnabled: (id: string, enabled: boolean) =>
    invoke<StartupChange>("startup_set_enabled", { id, enabled }),
  openStartupSettings: () => invoke<void>("open_startup_settings"),
  processInventory: (withTrust = true) =>
    invoke<ProcessRow[]>("process_inventory", { withTrust }),
  processDetail: (pid: number) => invoke<ProcessDetail>("process_detail", { pid }),
  processStop: (pid: number, name: string) =>
    invoke<{ message: string }>("process_stop", { pid, name }),
  onProgress: (handler: (progress: JobProgress) => void) =>
    listen<JobProgress>("squid://progress", (event) => handler(event.payload)),
  onUpdateProgress: (handler: (progress: UpdateProgress) => void) =>
    listen<UpdateProgress>("update://progress", (event) => handler(event.payload)),
  onUpdateStatus: (handler: (status: UpdateStatus | null) => void) =>
    listen<UpdateStatus | null>("update://status", (event) => handler(event.payload)),
  onNavigate: (handler: (page: PageId) => void) =>
    listen<PageId>("squid://navigate", (event) => handler(event.payload)),
  onSecurityScan: (handler: (scan: SecurityScan) => void) =>
    listen<SecurityScan>("squid://security", (event) => handler(event.payload)),
};

/// Progress events for one job, whatever kind of job it is. Security scans share
/// the channel with cleanup so a page subscribes the same way.
export async function subscribeJobProgress(
  jobId: string,
  handler: (progress: JobProgress) => void,
): Promise<UnlistenFn> {
  return api.onProgress((progress) => {
    if (progress.jobId === jobId) handler(progress);
  });
}
