import { useCallback, useEffect, useRef, useState } from "react";
import { api } from "../lib/api";
import { messageFrom } from "../lib/format";
import type { SystemSnapshot } from "../types";

/**
 * Polls system_snapshot for a page.
 *
 * - `full` asks the backend for the expensive per-process walk (only the
 *   实时监控 top-process table needs it); light refreshes cost a fraction.
 * - Polling pauses while the window is hidden (tray) or the tab is hidden,
 *   so an icon in the notification area does not keep sampling forever.
 */
export function useSystemSnapshot(
  active: boolean,
  intervalMs = 1800,
  full = false,
) {
  const [snapshot, setSnapshot] = useState<SystemSnapshot | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [loading, setLoading] = useState(false);
  const inFlight = useRef(false);

  const refresh = useCallback(async () => {
    if (inFlight.current) return;
    inFlight.current = true;
    setLoading(true);
    try {
      setSnapshot(await api.systemSnapshot(full));
      setError(null);
    } catch (reason) {
      setError(messageFrom(reason));
    } finally {
      inFlight.current = false;
      setLoading(false);
    }
  }, [full]);

  useEffect(() => {
    if (!active) return;
    let live = true;
    let timer: number | null = null;
    const tick = () => {
      if (live && !document.hidden) void refresh();
    };
    const start = () => {
      if (timer === null) {
        void refresh();
        timer = window.setInterval(tick, intervalMs);
      }
    };
    const stop = () => {
      if (timer !== null) {
        window.clearInterval(timer);
        timer = null;
      }
    };
    const onVisibility = () => (document.hidden ? stop() : start());
    start();
    document.addEventListener("visibilitychange", onVisibility);
    return () => {
      live = false;
      stop();
      document.removeEventListener("visibilitychange", onVisibility);
    };
  }, [active, intervalMs, refresh]);

  return { snapshot, error, loading, refresh };
}
