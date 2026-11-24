import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import type {
  AppsResult,
  AdvancedScanResult,
  Bootstrap,
  DeleteMode,
  DeleteResult,
  DupsResult,
  ExploreResult,
  HistorySession,
  JobProgress,
  LargeFile,
  LargestFile,
  MaintenanceResult,
  MaintenanceTask,
  ManageView,
  PageId,
  PermissionStatus,
  PlanResult,
  PopupClosedEvent,
  PopupWindowItem,
  PreflightResult,
  ScanResult,
  SettingsView,
  SpaceInsight,
  SystemSnapshot,
  TweakState,
  UndoResult,
  UpdateCheckResult,
  UpdateProgress,
  UpdateStatus,
  WidgetPatch,
  WidgetPrefs,
} from "../types";

export interface SettingsPatch {
  deleteMode?: DeleteMode;
  language?: string;
  showAllocated?: boolean;
  purgeMinAgeDays?: number;
  purgeRoots?: string[];
  historyKeep?: number;
  setupReviewed?: boolean;
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
  cleanupPreflight: (scanId: string, selected: number[]) =>
    invoke<PreflightResult>("cleanup_preflight", { scanId, selected }),
  purgeScan: (jobId: string, roots: string[] = [], minAgeDays: number | null = null, force = false) =>
    invoke<ScanResult>("purge_scan", { jobId, roots, minAgeDays, force }),
  purgeDelete: (jobId: string, scanId: string, selected: number[], mode: DeleteMode) =>
    invoke<DeleteResult>("purge_delete", { jobId, scanId, selected, mode }),
  duplicatesScan: (jobId: string, roots: string[] = [], minSize: number | null = null) =>
    invoke<DupsResult>("duplicates_scan", { jobId, roots, minSize }),
  duplicatesDelete: (
    jobId: string,
    scanId: string,
    // (group index, copy indexes to delete) — the keep decision is the UI's.
    selections: [number, number[]][],
    mode: DeleteMode,
  ) => invoke<DeleteResult>("duplicates_delete", { jobId, scanId, selections, mode }),
  largeFilesScan: (
    jobId: string,
    roots: string[] = [],
    minBytes: number | null = null,
    fileClass: string | null = null,
  ) => invoke<LargeFile[]>("large_files_scan", { jobId, roots, minBytes, class: fileClass }),
  largeFilesDelete: (jobId: string, paths: string[], mode: DeleteMode) =>
    invoke<DeleteResult>("large_files_delete", { jobId, paths, mode }),
  maintenanceTasks: () => invoke<MaintenanceTask[]>("maintenance_tasks"),
  maintenanceRun: (jobId: string, task: string) =>
    invoke<MaintenanceResult>("maintenance_run", { jobId, task }),
  privacyScan: (jobId: string) => invoke<ScanResult>("privacy_scan", { jobId }),
  privacyDelete: (jobId: string, scanId: string, selected: number[]) =>
    invoke<DeleteResult>("privacy_delete", { jobId, scanId, selected }),
  tweaksStates: () => invoke<TweakState[]>("tweaks_states"),
  tweaksSet: (id: string, enabled: boolean) => invoke<TweakState>("tweaks_set", { id, enabled }),
  tweaksRestoreAll: () => invoke<string[]>("tweaks_restore_all"),
  appsScan: (jobId: string) => invoke<AppsResult>("apps_scan", { jobId }),
  appPlan: (jobId: string, appKey: string) => invoke<PlanResult>("app_plan", { jobId, appKey }),
  appUninstall: (jobId: string, planId: string, selected: number[], mode: DeleteMode) =>
    invoke<DeleteResult>("app_uninstall", { jobId, planId, selected, mode }),
  appRemoveRecord: (regKey: string) => invoke<void>("app_remove_record", { regKey }),
  storageExplore: (jobId: string, path?: string | null, depth = 1, top = 16, force = false) =>
    invoke<ExploreResult>("storage_explore", { jobId, path, depth, top, force }),
  storageInsights: () => invoke<SpaceInsight[]>("storage_insights"),
  storageLargestFiles: (jobId: string, path: string, limit = 40) =>
    invoke<LargestFile[]>("storage_largest_files", { jobId, path, limit }),
  systemSnapshot: (full = false) => invoke<SystemSnapshot>("system_snapshot", { full }),
  stopProcess: (process: SystemSnapshot["top_processes"][number]) =>
    invoke<void>("stop_process", { process }),
  historyList: (limit = 60) => invoke<HistorySession[]>("history_list", { limit }),
  historyUndo: (sessionId: string) => invoke<UndoResult>("history_undo", { sessionId }),
  settingsSave: (patch: SettingsPatch) => invoke<SettingsView>("settings_save", { patch }),
  whitelistAdd: (path: string) => invoke<SettingsView>("whitelist_add", { path }),
  whitelistRemove: (pattern: string) => invoke<SettingsView>("whitelist_remove", { pattern }),
  whitelistAudit: () => invoke<{ pattern: string; live: boolean }[]>("whitelist_audit"),
  writeTextFile: (path: string, contents: string) =>
    invoke<void>("write_text_file", { path, contents }),
  revealPath: (path: string) => invoke<void>("reveal_path", { path }),
  openTrash: () => invoke<void>("open_trash"),
  permissionStatus: () => invoke<PermissionStatus>("permission_status"),
  openFullDiskAccessSettings: () => invoke<void>("open_full_disk_access_settings"),
  manageScan: () => invoke<ManageView>("manage_scan"),
  startupSetEnabled: (id: string, enabled: boolean) =>
    invoke<number>("startup_set_enabled", { id, enabled }),
  advancedScan: () => invoke<AdvancedScanResult>("advanced_scan"),
  advancedSetEnabled: (id: string, enabled: boolean) =>
    invoke<void>("advanced_set_enabled", { id, enabled }),
  extensionSetRegistered: (id: string, registered: boolean) =>
    invoke<void>("extension_set_registered", { id, registered }),
  popupList: () => invoke<PopupWindowItem[]>("popup_list"),
  popupClose: (hwnd: number) => invoke<void>("popup_close", { hwnd }),
  popupAddRule: (hwnd: number, matcher: string) =>
    invoke<void>("popup_add_rule", { hwnd, matcher }),
  popupRuleUpdate: (index: number, enabled?: boolean, resetCount?: boolean) =>
    invoke<void>("popup_rule_update", { index, enabled, resetCount }),
  popupRuleRemove: (index: number) => invoke<void>("popup_rule_remove", { index }),
  popupWatcherSet: (enabled: boolean) => invoke<void>("popup_watcher_set", { enabled }),
  widgetGet: () => invoke<{ prefs: WidgetPrefs }>("widget_get"),
  widgetSet: (patch: WidgetPatch) => invoke<WidgetPrefs>("widget_set", { patch }),
  widgetHide: () => invoke<void>("widget_hide"),
  updateCheck: () => invoke<UpdateCheckResult>("update_check"),
  updateStatus: () => invoke<UpdateStatus | null>("update_status"),
  updateDownload: () => invoke<UpdateStatus>("update_download"),
  updateInstall: () => invoke<string>("update_install"),
  updateClear: () => invoke<void>("update_clear"),
  setUpdateCheckEnabled: (enabled: boolean) =>
    invoke<boolean>("set_update_check_enabled", { enabled }),
  onProgress: (handler: (progress: JobProgress) => void) =>
    listen<JobProgress>("squid://progress", (event) => handler(event.payload)),
  onUpdateProgress: (handler: (progress: UpdateProgress) => void) =>
    listen<UpdateProgress>("update://progress", (event) => handler(event.payload)),
  onUpdateStatus: (handler: (status: UpdateStatus | null) => void) =>
    listen<UpdateStatus | null>("update://status", (event) => handler(event.payload)),
  onNavigate: (handler: (page: PageId) => void) =>
    listen<PageId>("squid://navigate", (event) => handler(event.payload)),
  onWindowVisible: (handler: (visible: boolean) => void) =>
    listen<boolean>("squid://visible", (event) => handler(event.payload)),
  onPopupClosed: (handler: (event: PopupClosedEvent) => void) =>
    listen<PopupClosedEvent>("popup://closed", (event) => handler(event.payload)),
  onWidgetSettings: (handler: (prefs: WidgetPrefs) => void) =>
    listen<WidgetPrefs>("widget://settings", (event) => handler(event.payload)),
};

export async function subscribeJobProgress(
  jobId: string,
  handler: (progress: JobProgress) => void,
): Promise<UnlistenFn> {
  return api.onProgress((progress) => {
    if (progress.jobId === jobId) handler(progress);
  });
}
