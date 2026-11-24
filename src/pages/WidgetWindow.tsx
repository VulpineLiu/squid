/* The 桌面小工具 window content (index.html?view=widget): header bar with
 * settings + collapse, live panels, and a compact settings popover that
 * writes straight through to widget_set (broadcast keeps the main page in
 * sync). */
import { useCallback, useEffect, useRef, useState } from "react";
import { getCurrentWindow, LogicalSize } from "@tauri-apps/api/window";
import { api } from "../lib/api";
import { messageFrom } from "../lib/format";
import type { WidgetPrefs } from "../types";
import { WidgetPanels, useHistory } from "../components/WidgetPanels";
import type { SystemSnapshot } from "../types";

const PANELS: { key: keyof WidgetPrefs; label: string }[] = [
  { key: "panelRun", label: "运行状态" },
  { key: "panelCpu", label: "CPU" },
  { key: "panelNetwork", label: "网络" },
  { key: "panelDisk", label: "存储读写" },
  { key: "panelStorage", label: "存储空间" },
];

export function WidgetWindow() {
  const [prefs, setPrefs] = useState<WidgetPrefs | null>(null);
  const [snapshot, setSnapshot] = useState<SystemSnapshot | null>(null);
  const [open, setOpen] = useState(false);
  const [history, bump] = useHistory();
  const windowRef = useRef(getCurrentWindow());

  useEffect(() => {
    // The window is transparent; punch through the app's opaque body paint.
    document.documentElement.classList.add("widget-mode");
    return () => document.documentElement.classList.remove("widget-mode");
  }, []);

  useEffect(() => {
    let cancelled = false;
    let stop: (() => void) | null = null;
    void api
      .widgetGet()
      .then((view) => !cancelled && setPrefs(view.prefs))
      .catch((reason) => console.error(messageFrom(reason)));
    void api
      .onWidgetSettings((next) => !cancelled && setPrefs(next))
      .then((unlisten) => {
        if (cancelled) unlisten();
        else stop = unlisten;
      });
    return () => {
      cancelled = true;
      stop?.();
    };
  }, []);

  // Poll at the configured cadence. The widget never shows the per-process
  // table, so it rides the light refresh, and an all-collapsed widget goes
  // silent until the header is opened again.
  const collapsed = Boolean(prefs?.collapsed);
  const intervalMs = prefs?.intervalMs ?? 2000;
  useEffect(() => {
    if (!prefs?.enabled || collapsed) return;
    let live = true;
    const tick = async () => {
      if (!live || document.hidden) return;
      try {
        const next = await api.systemSnapshot(false);
        if (live) {
          setSnapshot(next);
          bump(next);
        }
      } catch {
        /* sampler busy — next beat will catch up */
      }
    };
    void tick();
    const timer = window.setInterval(() => void tick(), intervalMs);
    return () => {
      live = false;
      window.clearInterval(timer);
    };
  }, [bump, collapsed, intervalMs, prefs?.enabled]);

  const patch = useCallback((values: Parameters<typeof api.widgetSet>[0]) => {
    void api
      .widgetSet(values)
      .then(setPrefs)
      .catch((reason) => console.error(messageFrom(reason)));
  }, []);

  // Last persisted drag position; null while the widget is corner-docked.
  const lastPos = useRef<{ x: number; y: number } | null>(null);

  // Grow/shrink the window to the stacked cards so transparent space never
  // eats desktop clicks (scrollHeight is CSS px == LogicalSize units).
  // Only an actual height change may call setSize/redock: setSize answers
  // with a resize event that re-arms the observer, and redock round-trips
  // through widget_set → widget://settings → setPrefs. Without this gate the
  // two effects re-fire each other forever (IPC storm + a config write per
  // loop turn — the old "widget costs 3% CPU" bug).
  const rootRef = useRef<HTMLDivElement | null>(null);
  const lastFit = useRef(0);
  const ready = Boolean(prefs);
  useEffect(() => {
    const root = rootRef.current;
    if (!root) return;
    let frame = 0;
    const fit = () => {
      window.cancelAnimationFrame(frame);
      frame = window.requestAnimationFrame(() => {
        const height = Math.max(64, root.scrollHeight + 4);
        if (height === lastFit.current) return;
        lastFit.current = height;
        void windowRef.current
          ?.setSize(new LogicalSize(350, height))
          .then(() => {
            // Keep the widget anchored after the size change: redock runs
            // the backend maths, which re-corners a corner-docked widget and
            // keeps (clamped) a remembered drag spot — correct in both modes.
            void api.widgetSet({ redock: true }).catch(() => undefined);
          })
          .catch(() => {
            lastFit.current = 0;
          });
      });
    };
    const observer = new ResizeObserver(fit);
    observer.observe(root);
    fit();
    return () => {
      observer.disconnect();
      window.cancelAnimationFrame(frame);
    };
  }, [ready, collapsed, open]);

  const toggleCollapsed = () => {
    setOpen(false);
    patch({ collapsed: !collapsed });
  };

  // Persist a *real* drag after the pointer settles (the header is a native
  // drag region, so poll once shortly after mouseup). Mere clicks must not
  // pin the position: seed lastPos with the true current spot on mount, and
  // only persist once the window has actually moved (≥5 px). An earlier bug
  // let a click right after an RDP re-layout save the drifted mid-screen
  // spot as "the user's chosen position" forever.
  useEffect(() => {
    void windowRef.current
      ?.outerPosition()
      .then((position) => {
        lastPos.current = position;
      })
      .catch(() => undefined);
    const onUp = () => {
      window.setTimeout(() => {
        void windowRef.current
          ?.outerPosition()
          .then((position) => {
            const previous = lastPos.current;
            if (
              previous &&
              Math.abs(previous.x - position.x) < 5 &&
              Math.abs(previous.y - position.y) < 5
            ) {
              return;
            }
            lastPos.current = position;
            patch({ position: [position.x, position.y], keepPosition: true });
          })
          .catch(() => undefined);
      }, 350);
    };
    window.addEventListener("mouseup", onUp);
    return () => window.removeEventListener("mouseup", onUp);
  }, [patch]);

  if (!prefs) return <div className="widget widget-loading" />;
  return (
    <div
      ref={rootRef}
      className="widget widget-window"
      style={{ opacity: (prefs.opacity ?? 100) / 100 }}
    >
      <div className="wd-card wd-head" data-tauri-drag-region>
        <button
          type="button"
          className="wd-head-btn"
          aria-label="小部件设置"
          title="小部件设置"
          onClick={() => setOpen((value) => !value)}
        >
          <svg width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round">
            <path d="M4 7h10M18 7h2M4 12h2M10 12h10M4 17h13" />
            <circle cx="16" cy="7" r="2" />
            <circle cx="8" cy="12" r="2" />
            <circle cx="19" cy="17" r="2" />
          </svg>
        </button>
        <span className="wd-head-spacer" />
        <button
          type="button"
          className={`wd-head-btn wd-chevron ${collapsed ? "is-collapsed" : ""}`}
          aria-label={collapsed ? "展开" : "折叠"}
          title={collapsed ? "展开" : "折叠"}
          onClick={toggleCollapsed}
        >
          <svg width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2.2" strokeLinecap="round" strokeLinejoin="round">
            <path d="m6 15 6-6 6 6" />
          </svg>
        </button>
      </div>

      {open && !collapsed ? (
        <div className="wd-card wd-settings">
          <h4>小部件设置</h4>
          {PANELS.map((panel) => (
            <div className="wd-set-row" key={panel.key}>
              <span>{panel.label}</span>
              <button
                type="button"
                className={`wd-set-toggle ${prefs[panel.key] ? "is-on" : ""}`}
                role="switch"
                aria-checked={Boolean(prefs[panel.key])}
                aria-label={panel.label}
                onClick={() => patch({ [panel.key]: !prefs[panel.key] })}
              />
            </div>
          ))}
          <div className="wd-set-row">
            <span>不透明度</span>
            <input
              className="wd-range"
              type="range"
              min={90}
              max={100}
              value={prefs.opacity}
              onChange={(event) => patch({ opacity: Number(event.target.value) })}
            />
          </div>
          <button
            type="button"
            className="wd-close-line"
            onClick={() => patch({ enabled: false })}
          >
            关闭小工具
          </button>
        </div>
      ) : null}

      {!collapsed ? (
        <WidgetPanels snapshot={snapshot} prefs={prefs} history={history} zh />
      ) : null}
    </div>
  );
}
