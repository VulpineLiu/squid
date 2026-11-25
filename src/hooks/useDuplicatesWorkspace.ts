import { useCallback, useRef, useState } from "react";
import { api, subscribeJobProgress } from "../lib/api";
import { messageFrom } from "../lib/format";
import type { DeleteResult, DupsResult, DupGroup, JobProgress } from "../types";

function jobId(): string {
  return crypto.randomUUID?.() ?? `${Date.now()}-${Math.random().toString(36).slice(2)}`;
}

/** Funnel counters streamed as `funnel:<walked>:<same>:<hits>:<confirmed>`
 * notes — the scan tells the UI how much each stage eliminated, live. */
export interface Funnel {
  walked: number;
  sameSize: number;
  sampleHits: number;
  confirmed: number;
}

export type KeepStrategy = "shortest" | "oldest";

/** Which copy of a group survives under a strategy. Locked (personal)
 * copies are preferred keeps — a WeChat FileStorage copy should never be
 * the one an automatic rule picks for deletion. */
export function keepIndexOf(group: DupGroup, strategy: KeepStrategy): number {
  let best = 0;
  for (let i = 1; i < group.copies.length; i += 1) {
    const candidate = group.copies[i];
    const current = group.copies[best];
    if (candidate.locked && !current.locked) {
      best = i;
      continue;
    }
    if (!candidate.locked && current.locked) continue;
    if (strategy === "oldest") {
      if (candidate.mtimeMs < current.mtimeMs) best = i;
    } else if (
      candidate.path.length < current.path.length ||
      (candidate.path.length === current.path.length && candidate.mtimeMs < current.mtimeMs)
    ) {
      best = i;
    }
  }
  return best;
}

/** Deletion picks per group: group index → copy indexes marked for removal. */
export type Picks = Map<number, Set<number>>;

export function smartPicks(groups: DupGroup[], strategy: KeepStrategy): Picks {
  const picks: Picks = new Map();
  groups.forEach((group, groupIndex) => {
    const keep = keepIndexOf(group, strategy);
    const copies = new Set<number>();
    group.copies.forEach((copy, copyIndex) => {
      if (copyIndex !== keep && !copy.locked) copies.add(copyIndex);
    });
    if (copies.size > 0) picks.set(groupIndex, copies);
  });
  return picks;
}

/** Stable signature for a group: ignore decisions survive the final report
 * re-sorting the confirmation-order stream. */
export function groupKey(group: DupGroup): string {
  return `${group.size}|${group.copies[0]?.path ?? ""}`;
}

export interface DuplicatesWorkspace {
  result: DupsResult | null;
  /** Visible groups with their backend scan indices (picks/ignore keep both
   * layers addressed by the same stable index). */
  visible: { group: DupGroup; index: number }[];
  funnel: Funnel;
  progress: JobProgress | null;
  scanning: boolean;
  deleting: boolean;
  error: string | null;
  summary: DeleteResult | null;
  picks: Picks;
  keepStrategy: KeepStrategy;
  /** Group signatures the user dismissed with 「忽略本组」 — filtered from
   * the visible list, sums adjust live, backend indices stay stable. */
  ignored: Set<string>;
  selectedCount: number;
  selectedBytes: number;
  runScan: (roots: string[], minSize: number | null) => Promise<void>;
  cancel: () => Promise<void>;
  smartSelect: (strategy?: KeepStrategy) => void;
  invertSelection: () => void;
  clearPicks: () => void;
  toggleCopy: (groupIndex: number, copyIndex: number) => void;
  setKeepStrategy: (strategy: KeepStrategy) => void;
  removeGroup: (groupIndex: number) => void;
  clean: (mode: "trash" | "permanent") => Promise<DeleteResult | null>;
  clearError: () => void;
}

export function useDuplicatesWorkspace(): DuplicatesWorkspace {
  const [result, setResult] = useState<DupsResult | null>(null);
  const [liveGroups, setLiveGroups] = useState<DupGroup[]>([]);
  const [funnel, setFunnel] = useState<Funnel>({
    walked: 0,
    sameSize: 0,
    sampleHits: 0,
    confirmed: 0,
  });
  const [progress, setProgress] = useState<JobProgress | null>(null);
  const [scanning, setScanning] = useState(false);
  const [deleting, setDeleting] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [summary, setSummary] = useState<DeleteResult | null>(null);
  const [picks, setPicks] = useState<Picks>(new Map());
  const [ignored, setIgnored] = useState<Set<string>>(new Set());
  const [keepStrategy, setStrategy] = useState<KeepStrategy>("shortest");
  const currentJob = useRef<string | null>(null);
  const strategyRef = useRef(keepStrategy);
  strategyRef.current = keepStrategy;

  const runScan = useCallback(
    async (roots: string[], minSize: number | null) => {
      if (scanning || deleting) return;
      const id = jobId();
      currentJob.current = id;
      setScanning(true);
      setError(null);
      setSummary(null);
      setResult(null);
      setProgress(null);
      setLiveGroups([]);
      setIgnored(new Set());
      setFunnel({ walked: 0, sameSize: 0, sampleHits: 0, confirmed: 0 });
      setPicks(new Map());
      const stop = await subscribeJobProgress(id, (event) => {
        if (event.event === "note" && event.detail?.startsWith("funnel:")) {
          const [, walked, same, hits, confirmed] = event.detail.split(":");
          setFunnel({
            walked: Number(walked) || 0,
            sameSize: Number(same) || 0,
            sampleHits: Number(hits) || 0,
            confirmed: Number(confirmed) || 0,
          });
          return;
        }
        if (event.event === "group" && event.detail?.startsWith("dup:")) {
          try {
            const group = JSON.parse(event.detail.slice(4)) as DupGroup;
            setLiveGroups((current) =>
              [...current, group].sort(
                (a, b) => b.size * (b.copies.length - 1) - a.size * (a.copies.length - 1),
              ),
            );
          } catch {
            /* malformed frame: the final report replaces the stream anyway */
          }
          return;
        }
        if (event.event === "metered" && event.done !== null) {
          // Stage-1 walk: files streamed; stage-2/3 ticks drive `progress`.
          setFunnel((current) => ({ ...current, walked: event.done as number }));
          return;
        }
        setProgress(event);
      });
      try {
        const view = await api.duplicatesScan(id, roots, minSize);
        setResult(view);
        setLiveGroups(view.report.groups);
        setFunnel((current) => ({
          walked: view.report.walked || current.walked,
          sameSize: view.report.sameSize || current.sameSize,
          sampleHits: view.report.sampleHits || current.sampleHits,
          confirmed: view.report.confirmedFiles || current.confirmed,
        }));
        setPicks(smartPicks(view.report.groups, strategyRef.current));
      } catch (reason) {
        if (!messageFrom(reason).includes("cancel")) setError(messageFrom(reason));
      } finally {
        stop();
        setScanning(false);
        currentJob.current = null;
      }
    },
    [deleting, scanning],
  );

  const cancel = useCallback(async () => {
    if (currentJob.current) {
      await api.cancelJob(currentJob.current).catch(() => undefined);
    }
  }, []);

  const groups = result?.report.groups ?? liveGroups;
  const ignoredRef = useRef(ignored);
  ignoredRef.current = ignored;

  const smartSelect = useCallback(
    (strategy?: KeepStrategy) => {
      const fresh = smartPicks(groups, strategy ?? strategyRef.current);
      // An ignored group stays ignored: no picks come back for it.
      fresh.forEach((_copies, index) => {
        const group = groups[index];
        if (group && ignoredRef.current.has(groupKey(group))) fresh.delete(index);
      });
      setPicks(fresh);
    },
    [groups],
  );

  const invertSelection = useCallback(() => {
    setPicks((current) => {
      const next: Picks = new Map();
      groups.forEach((group, groupIndex) => {
        if (ignoredRef.current.has(groupKey(group))) return;
        const keep = keepIndexOf(group, strategyRef.current);
        const chosen = current.get(groupIndex) ?? new Set<number>();
        const copies = new Set<number>();
        group.copies.forEach((copy, copyIndex) => {
          if (copyIndex === keep || copy.locked) return;
          if (!chosen.has(copyIndex)) copies.add(copyIndex);
        });
        if (copies.size > 0) next.set(groupIndex, copies);
      });
      return next;
    });
  }, [groups]);

  const clearPicks = useCallback(() => setPicks(new Map()), []);

  const toggleCopy = useCallback((groupIndex: number, copyIndex: number) => {
    setPicks((current) => {
      const next = new Map(current);
      const chosen = new Set(next.get(groupIndex) ?? []);
      if (chosen.has(copyIndex)) chosen.delete(copyIndex);
      else chosen.add(copyIndex);
      if (chosen.size > 0) next.set(groupIndex, chosen);
      else next.delete(groupIndex);
      return next;
    });
  }, []);

  const setKeepStrategy = useCallback(
    (strategy: KeepStrategy) => {
      setStrategy(strategy);
      smartSelect(strategy);
    },
    [smartSelect],
  );

  /** Ignore keeps the group out of the list and out of every sum, but the
   * backend scan indices stay untouched — deletion talks in those. */
  const removeGroup = useCallback((groupIndex: number) => {
    const group = groups[groupIndex];
    if (!group) return;
    const key = groupKey(group);
    setIgnored((current) => new Set(current).add(key));
    setPicks((current) => {
      const next = new Map(current);
      next.delete(groupIndex);
      return next;
    });
  }, [groups]);

  /** Visible groups paired with their *backend scan index* so picks and
   * deletes never drift after a group is ignored. */
  const entries = groups
    .map((group, index) => ({ group, index }))
    .filter(({ group }) => !ignored.has(groupKey(group)));

  const clean = useCallback(
    async (mode: "trash" | "permanent") => {
      if (!result || deleting) return null;
      const selections: [number, number[]][] = [];
      picks.forEach((copies, groupIndex) => {
        if (groupIndex < result.report.groups.length && copies.size > 0) {
          selections.push([groupIndex, [...copies]]);
        }
      });
      if (selections.length === 0) return null;
      const id = jobId();
      currentJob.current = id;
      setDeleting(true);
      setError(null);
      setProgress(null);
      // The delete loop streams the same tick/deleted frames the scan used
      // to, so the page's in-place CleanBar can count files honestly.
      const stop = await subscribeJobProgress(id, (event) => {
        if (event.event === "tick" || event.event === "deleted") setProgress(event);
      });
      try {
        const outcome = await api.duplicatesDelete(id, result.scanId, selections, mode);
        setSummary(outcome);
        setResult(null);
        setLiveGroups([]);
        setPicks(new Map());
        setIgnored(new Set());
        setFunnel({ walked: 0, sameSize: 0, sampleHits: 0, confirmed: 0 });
        return outcome;
      } catch (reason) {
        setError(messageFrom(reason));
        return null;
      } finally {
        stop();
        setDeleting(false);
        currentJob.current = null;
      }
    },
    [deleting, picks, result],
  );

  const clearError = useCallback(() => setError(null), []);

  let count = 0;
  let bytes = 0;
  picks.forEach((copies, groupIndex) => {
    const group = groups[groupIndex];
    if (!group) return;
    count += copies.size;
    bytes += copies.size * group.size;
  });

  return {
    result,
    visible: entries,
    funnel,
    progress,
    scanning,
    deleting,
    error,
    summary,
    picks,
    keepStrategy,
    ignored,
    selectedCount: count,
    selectedBytes: bytes,
    runScan,
    cancel,
    smartSelect,
    invertSelection,
    clearPicks,
    toggleCopy,
    setKeepStrategy,
    removeGroup,
    clean,
    clearError,
  };
}
