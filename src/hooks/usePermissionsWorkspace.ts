import { useCallback, useEffect, useState } from "react";
import { api } from "../lib/api";
import { isCancelled, messageFrom } from "../lib/format";
import { useToast } from "../components/ui";
import type { AdvancedScanResult, ManageView, PopupWindowItem } from "../types";

export type PermissionsTab = "startup" | "extensions" | "advanced" | "popup";

export interface PermissionsWorkspace {
  tab: PermissionsTab;
  setTab: (tab: PermissionsTab) => void;
  view: ManageView | null;
  windows: PopupWindowItem[];
  /** Advanced startup rows; scanned lazily the first time the tab opens —
   * the signature batch call is not something the plain page should pay. */
  advanced: AdvancedScanResult | null;
  advancedScanning: boolean;
  scanning: boolean;
  busyId: string | null;
  error: string | null;
  dismissError: () => void;
  closedTotal: number;
  recentClosed: { process: string; title: string }[];
  scan: () => Promise<void>;
  scanAdvanced: () => Promise<void>;
  refreshWindows: () => Promise<void>;
  toggleStartup: (id: string, enabled: boolean) => Promise<void>;
  /** 电脑管家's master switch: flip every entry of one software group. */
  toggleStartupGroup: (ids: string[], enabled: boolean) => Promise<void>;
  toggleAdvanced: (id: string, enabled: boolean) => Promise<void>;
  toggleExtension: (id: string, registered: boolean) => Promise<void>;
  addRule: (hwnd: number, matcher: "exact" | "contains") => Promise<void>;
  toggleRule: (index: number, enabled: boolean) => Promise<void>;
  removeRule: (index: number) => Promise<void>;
  closeWindow: (hwnd: number) => Promise<void>;
  setWatcher: (enabled: boolean) => Promise<void>;
}

export function usePermissionsWorkspace(active: boolean): PermissionsWorkspace {
  const [tab, setTab] = useState<PermissionsTab>("startup");
  const [view, setView] = useState<ManageView | null>(null);
  const [advanced, setAdvanced] = useState<AdvancedScanResult | null>(null);
  const [advancedScanning, setAdvancedScanning] = useState(false);
  const [windows, setWindows] = useState<PopupWindowItem[]>([]);
  const [scanning, setScanning] = useState(false);
  const [busyId, setBusyId] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [closedTotal, setClosedTotal] = useState(0);
  const [recentClosed, setRecentClosed] = useState<{ process: string; title: string }[]>([]);
  const { push } = useToast();

  const scan = useCallback(async () => {
    setScanning(true);
    setError(null);
    try {
      const next = await api.manageScan();
      setView(next);
      setWindows(next.windows);
      setClosedTotal(next.rules.reduce((sum, rule) => sum + rule.closed, 0));
    } catch (reason) {
      if (!isCancelled(reason)) setError(messageFrom(reason));
    } finally {
      setScanning(false);
    }
  }, []);

  const refreshWindows = useCallback(async () => {
    try {
      setWindows(await api.popupList());
    } catch (reason) {
      setError(messageFrom(reason));
    }
  }, []);

  const scanAdvanced = useCallback(async () => {
    setAdvancedScanning(true);
    setError(null);
    try {
      setAdvanced(await api.advancedScan());
    } catch (reason) {
      if (!isCancelled(reason)) setError(messageFrom(reason));
    } finally {
      setAdvancedScanning(false);
    }
  }, []);

  useEffect(() => {
    if (active && !view && !scanning) void scan();
  }, [active, scan, view, scanning]);

  // Lazy first load for the advanced tab: the PowerShell signature batch
  // runs once, when the user actually opens that tab.
  useEffect(() => {
    if (active && tab === "advanced" && !advanced && !advancedScanning) void scanAdvanced();
  }, [active, advanced, advancedScanning, scanAdvanced, tab]);

  useEffect(() => {
    if (!active) return;
    let stop: (() => void) | null = null;
    let cancelled = false;
    void api
      .onPopupClosed((event) => {
        setClosedTotal((total) => total + 1);
        setRecentClosed((recent) => [{ ...event }, ...recent].slice(0, 3));
        setView((current) =>
          current
            ? {
                ...current,
                rules: current.rules.map((rule, index) =>
                  index === event.ruleIndex ? { ...rule, closed: rule.closed + 1 } : rule,
                ),
              }
            : current,
        );
      })
      .then((unlisten) => {
        if (cancelled) unlisten();
        else stop = unlisten;
      });
    return () => {
      cancelled = true;
      stop?.();
    };
  }, [active]);

  const toggleStartup = useCallback(
    async (id: string, enabled: boolean) => {
      setBusyId(id);
      setError(null);
      try {
        await api.startupSetEnabled(id, enabled);
        await scan();
      } catch (reason) {
        const why = messageFrom(reason);
        setError(why);
        // The row snaps back without this; a toast right where the user is
        // looking explains *why* (cancelled UAC prompt, protected key, …).
        push(why, "danger");
      } finally {
        setBusyId(null);
      }
    },
    [push, scan],
  );

  const toggleStartupGroup = useCallback(
    async (ids: string[], enabled: boolean) => {
      if (ids.length === 0) return;
      setBusyId(ids[0]);
      setError(null);
      const failures: string[] = [];
      try {
        for (const id of ids) {
          try {
            await api.startupSetEnabled(id, enabled);
          } catch (reason) {
            failures.push(messageFrom(reason));
          }
        }
        await scan();
      } finally {
        setBusyId(null);
      }
      if (failures.length > 0) {
        setError(failures[0]);
        push(failures[0], "danger");
      }
    },
    [push, scan],
  );

  const toggleAdvanced = useCallback(
    async (id: string, enabled: boolean) => {
      setBusyId(id);
      setError(null);
      try {
        await api.advancedSetEnabled(id, enabled);
        setAdvanced(await api.advancedScan());
      } catch (reason) {
        const why = messageFrom(reason);
        setError(why);
        push(why, "danger");
      } finally {
        setBusyId(null);
      }
    },
    [push],
  );

  const toggleExtension = useCallback(
    async (id: string, registered: boolean) => {
      setBusyId(id);
      setError(null);
      try {
        await api.extensionSetRegistered(id, registered);
        setView((current) =>
          current
            ? {
                ...current,
                extensions: current.extensions.map((entry) =>
                  entry.id === id ? { ...entry, registered } : entry,
                ),
              }
            : current,
        );
      } catch (reason) {
        setError(messageFrom(reason));
      } finally {
        setBusyId(null);
      }
    },
    [],
  );

  const addRule = useCallback(
    async (hwnd: number, matcher: "exact" | "contains") => {
      setError(null);
      try {
        await api.popupAddRule(hwnd, matcher);
        await scan();
      } catch (reason) {
        setError(messageFrom(reason));
      }
    },
    [scan],
  );

  const toggleRule = useCallback(async (index: number, enabled: boolean) => {
    setError(null);
    try {
      await api.popupRuleUpdate(index, enabled);
      setView((current) =>
        current
          ? {
              ...current,
              rules: current.rules.map((rule, i) => (i === index ? { ...rule, enabled } : rule)),
            }
          : current,
      );
    } catch (reason) {
      setError(messageFrom(reason));
    }
  }, []);

  const removeRule = useCallback(
    async (index: number) => {
      setError(null);
      try {
        await api.popupRuleRemove(index);
        await scan();
      } catch (reason) {
        setError(messageFrom(reason));
      }
    },
    [scan],
  );

  const closeWindow = useCallback(async (hwnd: number) => {
    setError(null);
    try {
      await api.popupClose(hwnd);
      setWindows((current) => current.filter((window) => window.hwnd !== hwnd));
    } catch (reason) {
      setError(messageFrom(reason));
    }
  }, []);

  const setWatcher = useCallback(async (enabled: boolean) => {
    setError(null);
    try {
      await api.popupWatcherSet(enabled);
      setView((current) => (current ? { ...current, watcherEnabled: enabled } : current));
    } catch (reason) {
      setError(messageFrom(reason));
    }
  }, []);

  return {
    tab,
    setTab,
    view,
    windows,
    advanced,
    advancedScanning,
    scanning,
    busyId,
    error,
    dismissError: () => setError(null),
    closedTotal,
    recentClosed,
    scan,
    scanAdvanced,
    refreshWindows,
    toggleStartup,
    toggleStartupGroup,
    toggleAdvanced,
    toggleExtension,
    addRule,
    toggleRule,
    removeRule,
    closeWindow,
    setWatcher,
  };
}
