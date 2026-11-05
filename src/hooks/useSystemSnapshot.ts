import { useCallback, useEffect, useRef, useState } from "react";
import { api } from "../lib/api";
import { messageFrom } from "../lib/format";
import type { SystemSnapshot } from "../types";

export function useSystemSnapshot(active: boolean, intervalMs = 1800) {
  const [snapshot, setSnapshot] = useState<SystemSnapshot | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [loading, setLoading] = useState(false);
  const inFlight = useRef(false);

  const refresh = useCallback(async () => {
    if (inFlight.current) return;
    inFlight.current = true;
    setLoading(true);
    try {
      setSnapshot(await api.systemSnapshot());
      setError(null);
    } catch (reason) {
      setError(messageFrom(reason));
    } finally {
      inFlight.current = false;
      setLoading(false);
    }
  }, []);

  useEffect(() => {
    if (!active) return;
    void refresh();
    const timer = window.setInterval(() => void refresh(), intervalMs);
    return () => window.clearInterval(timer);
  }, [active, intervalMs, refresh]);

  return { snapshot, error, loading, refresh };
}
