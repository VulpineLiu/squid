import { useCallback, useRef, useState } from "react";
import { api, subscribeJobProgress } from "../lib/api";
import { isCancelled, messageFrom } from "../lib/format";
import type { JobProgress, LargestFile, StorageNode } from "../types";

function jobId(): string {
  return crypto.randomUUID?.() ?? `${Date.now()}-${Math.random().toString(36).slice(2)}`;
}

export interface StorageWorkspace {
  node: StorageNode | null;
  files: LargestFile[];
  progress: JobProgress | null;
  exploring: boolean;
  scanningFiles: boolean;
  error: string | null;
  explore: (path?: string | null) => Promise<void>;
  largestFiles: (path: string) => Promise<void>;
  cancel: () => Promise<void>;
  clearError: () => void;
  clearFiles: () => void;
}

export function useStorageWorkspace(): StorageWorkspace {
  const [node, setNode] = useState<StorageNode | null>(null);
  const [files, setFiles] = useState<LargestFile[]>([]);
  const [progress, setProgress] = useState<JobProgress | null>(null);
  const [exploring, setExploring] = useState(false);
  const [scanningFiles, setScanningFiles] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const activeJob = useRef<string | null>(null);

  const explore = useCallback(async (path?: string | null) => {
    if (exploring || scanningFiles) return;
    const id = jobId();
    activeJob.current = id;
    setExploring(true);
    setError(null);
    setProgress(null);
    setFiles([]);
    const stop = await subscribeJobProgress(id, setProgress);
    try {
      const next = await api.storageExplore(id, path ?? null, 1, 16);
      if (activeJob.current !== id) return;
      setNode(next);
    } catch (reason) {
      if (!isCancelled(reason)) setError(messageFrom(reason));
    } finally {
      stop();
      if (activeJob.current === id) {
        setExploring(false);
        setProgress(null);
        activeJob.current = null;
      }
    }
  }, [exploring, scanningFiles]);

  const largestFiles = useCallback(
    async (path: string) => {
      if (exploring || scanningFiles) return;
      const id = jobId();
      activeJob.current = id;
      setScanningFiles(true);
      setError(null);
      setProgress(null);
      setFiles([]);
      const stop = await subscribeJobProgress(id, setProgress);
      try {
        const next = await api.storageLargestFiles(id, path, 40);
        if (activeJob.current !== id) return;
        setFiles(next);
      } catch (reason) {
        if (!isCancelled(reason)) setError(messageFrom(reason));
      } finally {
        stop();
        if (activeJob.current === id) {
          setScanningFiles(false);
          setProgress(null);
          activeJob.current = null;
        }
      }
    },
    [exploring, scanningFiles],
  );

  const cancel = useCallback(async () => {
    if (activeJob.current) await api.cancelJob(activeJob.current).catch(() => false);
  }, []);

  return {
    node,
    files,
    progress,
    exploring,
    scanningFiles,
    error,
    explore,
    largestFiles,
    cancel,
    clearError: () => setError(null),
    clearFiles: () => setFiles([]),
  };
}
