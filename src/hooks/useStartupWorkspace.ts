import { useCallback, useEffect, useRef, useState } from "react";
import { api } from "../lib/api";
import { messageFrom } from "../lib/format";
import type { StartupItem, StartupView } from "../types";

export interface StartupWorkspace {
  view: StartupView | null;
  loading: boolean;
  error: string | null;
  /** One change at a time: a startup edit is a filesystem or registry write. */
  busy: boolean;
  refresh: () => Promise<void>;
  setEnabled: (item: StartupItem, enabled: boolean) => Promise<string>;
}

/**
 * The startup inventory.
 *
 * Nothing is cached between visits on purpose. A login item can be re-created by
 * its own app at any moment, and a list that remembers would show an entry as
 * enabled after the app re-armed it. Every visit re-reads the real thing, and a
 * change returns the whole refreshed inventory rather than the one edited row.
 */
export function useStartupWorkspace(active: boolean): StartupWorkspace {
  const [view, setView] = useState<StartupView | null>(null);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const inFlight = useRef(false);

  const refresh = useCallback(async () => {
    if (inFlight.current) return;
    inFlight.current = true;
    setLoading(true);
    try {
      setView(await api.startupItems());
      setError(null);
    } catch (reason) {
      setError(messageFrom(reason));
    } finally {
      inFlight.current = false;
      setLoading(false);
    }
  }, []);

  useEffect(() => {
    if (active && !view) void refresh();
  }, [active, refresh, view]);

  const setEnabled = useCallback(
    async (item: StartupItem, enabled: boolean) => {
      setBusy(true);
      try {
        const result = await api.startupSetEnabled(item.id, enabled);
        setView(result.view);
        return result.message;
      } finally {
        setBusy(false);
      }
    },
    [],
  );

  return { view, loading, error, busy, refresh, setEnabled };
}
