import { useCallback, useEffect, useRef, useState } from "react";
import { api, type SecurityPatch } from "../lib/api";
import { isCancelled, messageFrom } from "../lib/format";
import type { AllowKind, JobProgress, ScanMode, SecurityStatus } from "../types";

function jobId(): string {
  return crypto.randomUUID?.() ?? `${Date.now()}-${Math.random().toString(36).slice(2)}`;
}

export interface SecurityWorkspace {
  status: SecurityStatus | null;
  loading: boolean;
  error: string | null;
  /** A scan started from this window is running. */
  scanning: boolean;
  progress: JobProgress | null;
  refresh: () => Promise<void>;
  start: (mode: ScanMode, paths?: string[]) => Promise<void>;
  cancel: () => Promise<void>;
  quarantine: (findingId: string) => Promise<string>;
  restore: (id: string) => Promise<string>;
  purge: (id: string) => Promise<string>;
  dismiss: (findingId: string, ignored: boolean) => Promise<string>;
  trust: (
    findingId: string,
    scope: "publisher" | "folder" | "evidence",
  ) => Promise<string>;
  untrust: (kind: AllowKind, value: string) => Promise<string>;
  exclude: (action: "add" | "remove", path: string) => Promise<string>;
  updateRules: () => Promise<string>;
  rollbackRules: () => Promise<string>;
  saveSettings: (patch: SecurityPatch) => Promise<void>;
}

/**
 * The security state for the whole window, owned here rather than per page.
 *
 * Three surfaces show it — the overview strip, the sidebar badge, and the
 * security page — and they must not disagree, because a badge that says 2 next to
 * a page that says 1 is worse than no badge. One fetch, one refresh, passed down.
 *
 * It is fetched lazily in practice: `App` loads it once at startup for the badge
 * and the overview, and refreshes when the security page opens.
 */
export function useSecurityWorkspace(active: boolean): SecurityWorkspace {
  const [status, setStatus] = useState<SecurityStatus | null>(null);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [scanning, setScanning] = useState(false);
  const [progress, setProgress] = useState<JobProgress | null>(null);
  const activeJob = useRef<string | null>(null);
  const inFlight = useRef(false);

  const refresh = useCallback(async () => {
    if (inFlight.current) return;
    inFlight.current = true;
    setLoading(true);
    try {
      setStatus(await api.securityStatus());
      setError(null);
    } catch (reason) {
      setError(messageFrom(reason));
    } finally {
      inFlight.current = false;
      setLoading(false);
    }
  }, []);

  useEffect(() => {
    if (active && !status) void refresh();
  }, [active, refresh, status]);

  /**
   * Progress comes from the shared job channel, filtered by kind, rather than from
   * a subscription to the id this window happened to start with.
   *
   * That distinction is the whole point: a scan can also be started by the tray
   * menu or by the schedule, and a security page that showed nothing while a scan
   * it did not start was running would look broken. The same channel reports every
   * security job, so the page draws whichever one is live.
   */
  useEffect(() => {
    if (!active) return;
    let stop: (() => void) | null = null;
    let cancelled = false;
    void api
      .onProgress((event) => {
        if (event.kind !== "security") return;
        activeJob.current = event.jobId;
        setScanning(true);
        setProgress(event);
      })
      .then((unlisten) => {
        if (cancelled) unlisten();
        else stop = unlisten;
      });
    // The backend broadcasts every finished scan. That is both the refresh and the
    // only reliable "it is over" for a scan this window did not start — from the
    // tray, or from the schedule — where there is no local promise to settle on.
    void api
      .onSecurityScan(() => {
        setScanning(false);
        setProgress(null);
        activeJob.current = null;
        void refresh();
      })
      .catch(() => undefined);
    return () => {
      cancelled = true;
      stop?.();
    };
  }, [active, refresh]);

  const start = useCallback(
    async (mode: ScanMode, paths: string[] = []) => {
      if (scanning) return;
      const id = jobId();
      activeJob.current = id;
      setScanning(true);
      setProgress(null);
      setError(null);
      try {
        await api.securityScan(id, mode, paths);
      } catch (reason) {
        if (!isCancelled(reason)) setError(messageFrom(reason));
      } finally {
        // The finished scan arrives on the broadcast channel; this is the fallback
        // for the window between the reply and the event, and for a scan whose
        // progress never reached this window at all.
        setScanning(false);
        setProgress(null);
        activeJob.current = null;
        await refresh();
      }
    },
    [refresh, scanning],
  );

  const cancel = useCallback(async () => {
    if (!activeJob.current) return;
    await api.securityCancel(activeJob.current).catch(() => false);
  }, []);

  const apply = useCallback(
    async (run: () => Promise<{ message: string; status: SecurityStatus }>) => {
      const result = await run();
      setStatus(result.status);
      return result.message;
    },
    [],
  );

  const quarantine = useCallback(
    (findingId: string) => apply(() => api.securityQuarantine(findingId)),
    [apply],
  );
  const restore = useCallback((id: string) => apply(() => api.securityRestore(id)), [apply]);
  const purge = useCallback(
    (id: string) => apply(() => api.securityDeleteQuarantine(id)),
    [apply],
  );
  const dismiss = useCallback(
    (findingId: string, ignored: boolean) => apply(() => api.securityIgnore(findingId, ignored)),
    [apply],
  );
  const trust = useCallback(
    (findingId: string, scope: "publisher" | "folder" | "evidence") =>
      apply(() => api.securityTrust(findingId, scope)),
    [apply],
  );
  const untrust = useCallback(
    (kind: AllowKind, value: string) => apply(() => api.securityUntrust(kind, value)),
    [apply],
  );

  const exclude = useCallback(
    async (action: "add" | "remove", path: string) => {
      const settings = await api.securityExclusions(action, path);
      setStatus((current) => (current ? { ...current, settings } : current));
      return action === "add" ? `Excluded ${path}` : `No longer excluding ${path}`;
    },
    [],
  );

  const updateRules = useCallback(async () => {
    const intel = await api.securityUpdateRules();
    await refresh();
    return intel.source === "pack"
      ? `Rule pack r${intel.revision} installed — ${intel.installedRules} rules.`
      : `Already on the newest rules this build ships with (${intel.builtinRules}).`;
  }, [refresh]);

  const rollbackRules = useCallback(async () => {
    const intel = await api.securityRollbackRules();
    await refresh();
    return `Rolled back to rule pack r${intel.revision}.`;
  }, [refresh]);

  const saveSettings = useCallback(
    async (patch: SecurityPatch) => {
      const settings = await api.securitySettingsSave(patch);
      setStatus((current) => (current ? { ...current, settings } : current));
    },
    [],
  );

  return {
    status,
    loading,
    error,
    scanning,
    progress,
    refresh,
    start,
    cancel,
    quarantine,
    restore,
    purge,
    dismiss,
    trust,
    untrust,
    exclude,
    updateRules,
    rollbackRules,
    saveSettings,
  };
}
