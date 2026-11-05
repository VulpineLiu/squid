import { useCallback, useMemo, useState } from "react";
import { api, subscribeJobProgress } from "../lib/api";
import { isCancelled, messageFrom } from "../lib/format";
import type {
  AppInfo,
  DeleteMode,
  DeleteResult,
  JobProgress,
  PlanResult,
} from "../types";

function jobId(): string {
  return crypto.randomUUID?.() ?? `${Date.now()}-${Math.random().toString(36).slice(2)}`;
}

export interface AppsWorkspace {
  apps: AppInfo[];
  total: number;
  scanned: boolean;
  scanning: boolean;
  planning: boolean;
  deleting: boolean;
  progress: JobProgress | null;
  error: string | null;
  selectedApp: AppInfo | null;
  plan: PlanResult | null;
  planSelected: Set<number>;
  summary: DeleteResult | null;
  planSelectedTotal: number;
  scan: () => Promise<void>;
  chooseApp: (app: AppInfo | null) => void;
  buildPlan: (app: AppInfo) => Promise<void>;
  togglePlanItem: (index: number) => void;
  togglePlanAll: () => void;
  uninstall: (mode: DeleteMode) => Promise<DeleteResult | null>;
  cancel: () => Promise<void>;
  resetPlan: () => void;
  clearError: () => void;
  clearSummary: () => void;
}

export function useAppsWorkspace(): AppsWorkspace {
  const [apps, setApps] = useState<AppInfo[]>([]);
  const [total, setTotal] = useState(0);
  const [scanned, setScanned] = useState(false);
  const [scanning, setScanning] = useState(false);
  const [planning, setPlanning] = useState(false);
  const [deleting, setDeleting] = useState(false);
  const [progress, setProgress] = useState<JobProgress | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [selectedApp, setSelectedApp] = useState<AppInfo | null>(null);
  const [plan, setPlan] = useState<PlanResult | null>(null);
  const [planSelected, setPlanSelected] = useState<Set<number>>(new Set());
  const [summary, setSummary] = useState<DeleteResult | null>(null);
  const [activeJob, setActiveJob] = useState<string | null>(null);

  const scan = useCallback(async () => {
    if (scanning || deleting) return;
    const id = jobId();
    setActiveJob(id);
    setScanning(true);
    setError(null);
    setSummary(null);
    const stop = await subscribeJobProgress(id, setProgress);
    try {
      const result = await api.appsScan(id);
      setApps(result.apps);
      setTotal(result.total);
      setScanned(true);
    } catch (reason) {
      if (!isCancelled(reason)) setError(messageFrom(reason));
    } finally {
      stop();
      setScanning(false);
      setProgress(null);
      setActiveJob(null);
    }
  }, [deleting, scanning]);

  const chooseApp = useCallback((app: AppInfo | null) => {
    setSelectedApp(app);
    setPlan(null);
    setPlanSelected(new Set());
  }, []);

  const buildPlan = useCallback(
    async (app: AppInfo) => {
      if (planning) return;
      const id = jobId();
      setActiveJob(id);
      setPlanning(true);
      setSelectedApp(app);
      setPlan(null);
      setError(null);
      const stop = await subscribeJobProgress(id, setProgress);
      try {
        const next = await api.appPlan(id, app.bundle_id || app.path);
        setPlan(next);
        setPlanSelected(
          new Set(
            next.items
              .map((item, index) => ({ item, index }))
              .filter(({ item }) => item.selected && !item.lock)
              .map(({ index }) => index),
          ),
        );
      } catch (reason) {
        if (!isCancelled(reason)) setError(messageFrom(reason));
      } finally {
        stop();
        setPlanning(false);
        setProgress(null);
        setActiveJob(null);
      }
    },
    [planning],
  );

  const togglePlanItem = useCallback((index: number) => {
    setPlanSelected((current) => {
      const next = new Set(current);
      if (next.has(index)) next.delete(index);
      else next.add(index);
      return next;
    });
  }, []);

  const togglePlanAll = useCallback(() => {
    if (!plan) return;
    setPlanSelected((current) => {
      const all = plan.items
        .map((item, index) => ({ item, index }))
        .filter(({ item }) => !item.lock)
        .map(({ index }) => index);
      if (all.length > 0 && all.every((index) => current.has(index))) return new Set();
      return new Set(all);
    });
  }, [plan]);

  const planSelectedTotal = useMemo(() => {
    if (!plan) return 0;
    let total = 0;
    for (const index of planSelected) total += plan.items[index]?.onDisk ?? 0;
    return total;
  }, [plan, planSelected]);

  const uninstall = useCallback(
    async (mode: DeleteMode) => {
      if (!plan || planSelected.size === 0 || deleting) return null;
      const id = jobId();
      setActiveJob(id);
      setDeleting(true);
      setError(null);
      const stop = await subscribeJobProgress(id, setProgress);
      try {
        const result = await api.appUninstall(id, plan.planId, [...planSelected], mode);
        setSummary(result);
        setApps((current) =>
          current.filter(
            (app) =>
              (app.bundle_id || app.path) !==
              (plan.app.bundle_id || plan.app.path),
          ),
        );
        setPlan(null);
        setPlanSelected(new Set());
        setSelectedApp(null);
        return result;
      } catch (reason) {
        if (!isCancelled(reason)) setError(messageFrom(reason));
        return null;
      } finally {
        stop();
        setDeleting(false);
        setProgress(null);
        setActiveJob(null);
      }
    },
    [deleting, plan, planSelected],
  );

  const cancel = useCallback(async () => {
    if (activeJob) await api.cancelJob(activeJob).catch(() => false);
  }, [activeJob]);

  const resetPlan = useCallback(() => {
    setPlan(null);
    setPlanSelected(new Set());
  }, []);

  return {
    apps,
    total,
    scanned,
    scanning,
    planning,
    deleting,
    progress,
    error,
    selectedApp,
    plan,
    planSelected,
    summary,
    planSelectedTotal,
    scan,
    chooseApp,
    buildPlan,
    togglePlanItem,
    togglePlanAll,
    uninstall,
    cancel,
    resetPlan,
    clearError: () => setError(null),
    clearSummary: () => setSummary(null),
  };
}
