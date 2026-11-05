import { useCallback, useEffect, useRef, useState } from "react";
import { api } from "../lib/api";
import { messageFrom } from "../lib/format";
import type { UpdateProgress, UpdateStatus } from "../types";

export interface UpdateFlow {
  status: UpdateStatus | null;
  checking: boolean;
  downloading: boolean;
  installing: boolean;
  installError: string | null;
  progress: UpdateProgress | null;
  percent: number | null;
  check: () => Promise<void>;
  download: () => Promise<void>;
  install: () => Promise<void>;
  clear: () => Promise<void>;
}

export function useUpdateFlow(feedUrl: string, autoCheck = true): UpdateFlow {
  const [status, setStatus] = useState<UpdateStatus | null>(null);
  const [checking, setChecking] = useState(false);
  const [downloading, setDownloading] = useState(false);
  const [installing, setInstalling] = useState(false);
  const [installError, setInstallError] = useState<string | null>(null);
  const [progress, setProgress] = useState<UpdateProgress | null>(null);
  const [percent, setPercent] = useState<number | null>(null);
  const mounted = useRef(true);
  const statusRef = useRef<UpdateStatus | null>(null);

  const publish = useCallback((next: UpdateStatus | null) => {
    statusRef.current = next;
    if (mounted.current) setStatus(next);
  }, []);

  const check = useCallback(async () => {
    if (checking) return;
    setChecking(true);
    try {
      const outcome = await api.updateCheck();
      publish(outcome.status);
    } catch {
      publish(await api.updateStatus().catch(() => null));
    } finally {
      if (mounted.current) setChecking(false);
    }
  }, [checking, publish]);

  const download = useCallback(async () => {
    if (downloading) return;
    setDownloading(true);
    setInstallError(null);
    setProgress(null);
    setPercent(null);
    try {
      const ready = await api.updateDownload();
      publish(ready);
    } catch (error) {
      if (mounted.current) setInstallError(messageFrom(error));
      publish(await api.updateStatus().catch(() => null));
    } finally {
      if (mounted.current) {
        setDownloading(false);
        setProgress(null);
        setPercent(null);
      }
    }
  }, [downloading, publish]);

  const install = useCallback(async () => {
    if (installing || !statusRef.current?.path) return;
    setInstalling(true);
    try {
      await api.updateInstall();
    } catch (error) {
      if (mounted.current) {
        setInstallError(messageFrom(error));
        setInstalling(false);
      }
    }
  }, [installing]);

  const clear = useCallback(async () => {
    setInstallError(null);
    await api.updateClear().catch(() => undefined);
    publish(null);
  }, [publish]);

  useEffect(() => {
    mounted.current = true;
    void api
      .updateStatus()
      .then((last) => {
        if (last) publish(last);
      })
      .catch(() => undefined);
    const unlisteners: (() => void)[] = [];
    void api
      .onUpdateProgress((next) => {
        if (!mounted.current) return;
        setProgress(next);
        setPercent(
          next.total > 0
            ? Math.min(100, Math.round((next.downloaded / next.total) * 100))
            : null,
        );
      })
      .then((stop) => unlisteners.push(stop));
    void api
      .onUpdateStatus((next) => {
        if (!mounted.current) return;
        statusRef.current = next;
        setStatus(next);
      })
      .then((stop) => unlisteners.push(stop));
    return () => {
      mounted.current = false;
      unlisteners.forEach((stop) => stop());
    };
  }, [publish]);

  useEffect(() => {
    if (!feedUrl || !autoCheck) return;
    // A startup check runs in Rust. This is a quiet fallback for builds whose
    // first check is due but whose event was emitted before the webview mounted.
    const timer = window.setTimeout(() => {
      if (!statusRef.current) void check();
    }, 1200);
    return () => window.clearTimeout(timer);
  }, [autoCheck, check, feedUrl]);

  return {
    status,
    checking,
    downloading,
    installing,
    installError,
    progress,
    percent,
    check,
    download,
    install,
    clear,
  };
}
