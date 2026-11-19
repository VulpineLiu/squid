import { useCallback, useMemo, useRef, useState } from "react";
import { api, subscribeJobProgress } from "../lib/api";
import { isCancelled, messageFrom } from "../lib/format";
import type {
  Category,
  DeleteMode,
  DeleteResult,
  JobProgress,
  ScanResult,
} from "../types";

function jobId(): string {
  return crypto.randomUUID?.() ?? `${Date.now()}-${Math.random().toString(36).slice(2)}`;
}

export type ScanKind = "cleanup" | "purge";

export interface ScanWorkspace {
  kind: ScanKind;
  result: ScanResult | null;
  selected: Set<number>;
  progress: JobProgress | null;
  scanning: boolean;
  deleting: boolean;
  error: string | null;
  summary: DeleteResult | null;
  currentJob: string | null;
  selectedTotal: number;
  selectedCount: number;
  runScan: (options?: {
    force?: boolean;
    minAgeDays?: number | null;
    roots?: string[];
  }) => Promise<void>;
  cancel: () => Promise<void>;
  toggleItem: (index: number) => void;
  toggleCategory: (category: Category) => void;
  deselect: (indexes: number[]) => void;
  selectAll: () => void;
  clearSelection: () => void;
  clean: (mode: DeleteMode, indexes?: number[]) => Promise<DeleteResult | null>;
  clearError: () => void;
  reset: () => void;
}

export function useScanWorkspace(kind: ScanKind): ScanWorkspace {
  const [result, setResult] = useState<ScanResult | null>(null);
  const [selected, setSelected] = useState<Set<number>>(new Set());
  const [progress, setProgress] = useState<JobProgress | null>(null);
  const [scanning, setScanning] = useState(false);
  const [deleting, setDeleting] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [summary, setSummary] = useState<DeleteResult | null>(null);
  const [currentJob, setCurrentJob] = useState<string | null>(null);
  const activeJob = useRef<string | null>(null);

  const runScan = useCallback(
    async (options?: {
      force?: boolean;
      minAgeDays?: number | null;
      roots?: string[];
    }) => {
      if (scanning || deleting) return;
      const id = jobId();
      activeJob.current = id;
      setCurrentJob(id);
      setScanning(true);
      setError(null);
      setSummary(null);
      setProgress(null);
      const stop = await subscribeJobProgress(id, setProgress);
      try {
        const next =
          kind === "cleanup"
            ? await api.cleanupScan(id, [], options?.minAgeDays ?? null, false)
            : await api.purgeScan(
                id,
                options?.roots ?? [],
                options?.minAgeDays ?? null,
                options?.force ?? false,
              );
        if (activeJob.current !== id) return;
        setResult(next);
        setSelected(
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
        if (activeJob.current === id) {
          setScanning(false);
          setProgress(null);
          setCurrentJob(null);
        }
      }
    },
    [deleting, kind, scanning],
  );

  const cancel = useCallback(async () => {
    if (activeJob.current) await api.cancelJob(activeJob.current).catch(() => false);
  }, []);

  const toggleItem = useCallback((index: number) => {
    setSelected((current) => {
      const next = new Set(current);
      if (next.has(index)) next.delete(index);
      else next.add(index);
      return next;
    });
  }, []);

  const toggleCategory = useCallback(
    (category: Category) => {
      if (!result) return;
      const indexes = result.items
        .map((item, index) => ({ item, index }))
        .filter(({ item }) => item.category === category && !item.lock)
        .map(({ index }) => index);
      setSelected((current) => {
        const next = new Set(current);
        const allSelected = indexes.length > 0 && indexes.every((index) => next.has(index));
        for (const index of indexes) {
          if (allSelected) next.delete(index);
          else next.add(index);
        }
        return next;
      });
    },
    [result],
  );

  const selectAll = useCallback(() => {
    if (!result) return;
    setSelected((current) => {
      const all = result.items
        .map((item, index) => ({ item, index }))
        .filter(({ item }) => !item.lock)
        .map(({ index }) => index);
      if (all.length > 0 && all.every((index) => current.has(index))) return new Set();
      return new Set(all);
    });
  }, [result]);

  const clearSelection = useCallback(() => setSelected(new Set()), []);

  const deselect = useCallback((indexes: number[]) => {
    if (indexes.length === 0) return;
    const drop = new Set(indexes);
    setSelected((current) => {
      const next = new Set(current);
      for (const index of drop) next.delete(index);
      return next;
    });
  }, []);

  const selectedTotal = useMemo(() => {
    if (!result) return 0;
    let total = 0;
    for (const index of selected) total += result.items[index]?.onDisk ?? 0;
    return total;
  }, [result, selected]);

  const selectedCount = selected.size;

  const clean = useCallback(
    async (mode: DeleteMode, indexes?: number[]) => {
      if (!result || deleting) return null;
      const targets = indexes ?? [...selected];
      if (targets.length === 0) return null;
      const id = jobId();
      activeJob.current = id;
      setCurrentJob(id);
      setDeleting(true);
      setError(null);
      setProgress(null);
      const stop = await subscribeJobProgress(id, setProgress);
      try {
        const next =
          kind === "cleanup"
            ? await api.cleanupDelete(id, result.scanId, targets, mode)
            : await api.purgeDelete(id, result.scanId, targets, mode);
        if (activeJob.current !== id) return null;
        setResult(null);
        setSelected(new Set());
        setSummary(next);
        return next;
      } catch (reason) {
        if (!isCancelled(reason)) setError(messageFrom(reason));
        return null;
      } finally {
        stop();
        if (activeJob.current === id) {
          setDeleting(false);
          setProgress(null);
          setCurrentJob(null);
        }
      }
    },
    [deleting, kind, result, selected],
  );

  const reset = useCallback(() => {
    setResult(null);
    setSelected(new Set());
    setSummary(null);
    setError(null);
    setProgress(null);
  }, []);

  return {
    kind,
    result,
    selected,
    progress,
    scanning,
    deleting,
    error,
    summary,
    currentJob,
    selectedTotal,
    selectedCount,
    runScan,
    cancel,
    toggleItem,
    toggleCategory,
    deselect,
    selectAll,
    clearSelection,
    clean,
    clearError: () => setError(null),
    reset,
  };
}
