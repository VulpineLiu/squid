/* The desktop live-monitor window content (index.html?view=widget): header bar
 * with settings + collapse, live panels, and the settings dropdown. */
import { Activity, ChevronUp, Settings2 } from "lucide-react";
import { useCallback, useEffect, useRef, useState } from "react";
import { getCurrentWindow, LogicalSize } from "@tauri-apps/api/window";
import { api } from "../lib/api";
import { messageFrom } from "../lib/format";
import type { WidgetCorner, WidgetPrefs } from "../types";
import { WidgetPanels, useHistory } from "../components/WidgetPanels";
import { Select } from "../components/ui";
import type { SystemSnapshot } from "../types";

const PANELS: { key: keyof WidgetPrefs; en: string; zh: string }[] = [
  { key: "panelRun", en: "Status", zh: "运行状态" },
  { key: "panelCpu", en: "CPU", zh: "CPU" },
  { key: "panelNetwork", en: "Network", zh: "网络" },
  { key: "panelDisk", en: "Disk I/O", zh: "存储读写" },
  { key: "panelStorage", en: "Storage", zh: "存储空间" },
];

const CORNERS: { id: WidgetCorner; en: string; zh: string }[] = [
  { id: "top-right", en: "Top right", zh: "右上角" },
  { id: "top-left", en: "Top left", zh: "左上角" },
  { id: "bottom-right", en: "Bottom right", zh: "右下角" },
  { id: "bottom-left", en: "Bottom left", zh: "左下角" },
];

const INTERVALS = [2_000, 5_000, 10_000];

export function WidgetWindow() {
  const [prefs, setPrefs] = useState<WidgetPrefs | null>(null);
  const [language, setLanguage] = useState<"zh" | "en">("en");
  const [snapshot, setSnapshot] = useState<SystemSnapshot | null>(null);
  const [open, setOpen] = useState(false);
  const [history, bump] = useHistory();
  const windowRef = useRef(getCurrentWindow());
  const settingsRef = useRef<HTMLDivElement | null>(null);
  const settingsButtonRef = useRef<HTMLButtonElement | null>(null);

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
      .then((view) => {
        if (cancelled) return;
        setPrefs(view.prefs);
        setLanguage(view.language);
      })
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
  const hasPanels = Boolean(
    prefs &&
      (prefs.panelRun ||
        prefs.panelCpu ||
        prefs.panelNetwork ||
        prefs.panelDisk ||
        prefs.panelStorage),
  );
  const intervalMs = prefs?.intervalMs ?? 2000;
  useEffect(() => {
    if (!prefs?.enabled || collapsed || !hasPanels) return;
    let live = true;
    const tick = async () => {
      if (!live) return;
      // Desktop-layer windows report `document.hidden` whenever another app
      // occludes them; ask Tauri whether the window is actually hidden.
      try {
        if ((await windowRef.current?.isVisible()) === false) return;
      } catch {
        /* older runtimes: keep sampling */
      }
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
  }, [bump, collapsed, hasPanels, intervalMs, prefs?.enabled]);

  // Last persisted drag position; null while the widget is corner-docked.
  const lastPos = useRef<{ x: number; y: number } | null>(null);

  const patch = useCallback((values: Parameters<typeof api.widgetSet>[0]) => {
    if (values.corner) lastPos.current = null;
    void api
      .widgetSet(values)
      .then(setPrefs)
      .catch((reason) => console.error(messageFrom(reason)));
  }, []);

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
        const height = Math.max(64, root.scrollHeight + 12);
        if (height === lastFit.current) return;
        lastFit.current = height;
        void windowRef.current
          ?.setSize(new LogicalSize(350, height))
          .then(() => {
            // Keep the docked corner anchored after the size change.
            if (!lastPos.current) void api.widgetSet({ redock: true }).catch(() => undefined);
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

  const toggleSettings = () => {
    if (!open && collapsed) patch({ collapsed: false });
    setOpen((value) => !value);
  };

  useEffect(() => {
    if (!open) return;
    const close = (event: PointerEvent) => {
      const target = event.target as Node;
      if (
        settingsRef.current?.contains(target) ||
        settingsButtonRef.current?.contains(target)
      ) {
        return;
      }
      setOpen(false);
    };
    document.addEventListener("pointerdown", close);
    return () => document.removeEventListener("pointerdown", close);
  }, [open]);

  // Persist a dragged position after the pointer settles (the header is a
  // native drag region, so poll once shortly after mouseup).
  useEffect(() => {
    const pressed = { current: null as { x: number; y: number } | null };
    const onDown = (event: MouseEvent) => {
      const target = event.target as HTMLElement;
      if (
        !target.closest("[data-tauri-drag-region]") ||
        target.closest("button, input, select, .wd-settings")
      ) {
        return;
      }
      pressed.current = { x: event.screenX, y: event.screenY };
    };
    const onUp = (event: MouseEvent) => {
      const start = pressed.current;
      pressed.current = null;
      if (!start || Math.hypot(event.screenX - start.x, event.screenY - start.y) < 4) {
        return;
      }
      window.setTimeout(() => {
        void windowRef.current
          ?.outerPosition()
          .then((position) => {
            const previous = lastPos.current;
            if (previous && previous.x === position.x && previous.y === position.y) return;
            lastPos.current = position;
            patch({ position: [position.x, position.y], keepPosition: true });
          })
          .catch(() => undefined);
      }, 350);
    };
    window.addEventListener("mousedown", onDown);
    window.addEventListener("mouseup", onUp);
    return () => {
      window.removeEventListener("mousedown", onDown);
      window.removeEventListener("mouseup", onUp);
    };
  }, [patch]);

  if (!prefs) return <div className="widget widget-loading" />;
  const zh = language === "zh";
  return (
    <div
      ref={rootRef}
      className="widget widget-window"
      style={{ opacity: (prefs.opacity ?? 100) / 100 }}
    >
      <div className="wd-card wd-head" data-tauri-drag-region>
        <span className="wd-brand">
          <Activity size={15} />
          <b>{zh ? "实时监控" : "Live monitor"}</b>
        </span>
        <span className="wd-head-spacer" />
        <button
          ref={settingsButtonRef}
          type="button"
          className={`wd-head-btn ${open ? "is-active" : ""}`}
          aria-label={zh ? "打开设置" : "Open settings"}
          title={zh ? "设置" : "Settings"}
          onClick={toggleSettings}
        >
          <Settings2 size={17} />
        </button>
        <button
          type="button"
          className={`wd-head-btn wd-chevron ${collapsed ? "is-collapsed" : ""}`}
          aria-label={collapsed ? (zh ? "展开" : "Expand") : zh ? "折叠" : "Collapse"}
          title={collapsed ? (zh ? "展开" : "Expand") : zh ? "折叠" : "Collapse"}
          onClick={toggleCollapsed}
        >
          <ChevronUp size={18} />
        </button>
      </div>

      {open ? (
        <div ref={settingsRef} className="wd-card wd-settings">
          <div className="wd-settings-head">
            <div>
              <h4>{zh ? "实时监控设置" : "Live monitor settings"}</h4>
              <span>{zh ? "更改会立即生效" : "Changes apply immediately"}</span>
            </div>
            <button type="button" className="wd-settings-done" onClick={() => setOpen(false)}>
              {zh ? "完成" : "Done"}
            </button>
          </div>

          <div className="wd-settings-section">
            <span className="wd-settings-label">{zh ? "面板" : "Panels"}</span>
            {PANELS.map((panel) => (
              <div className="wd-set-row" key={panel.key}>
                <span>{zh ? panel.zh : panel.en}</span>
                <button
                  type="button"
                  className={`wd-set-toggle ${prefs[panel.key] ? "is-on" : ""}`}
                  role="switch"
                  aria-checked={Boolean(prefs[panel.key])}
                  aria-label={zh ? panel.zh : panel.en}
                  onClick={() => patch({ [panel.key]: !prefs[panel.key] })}
                />
              </div>
            ))}
          </div>

          <div className="wd-settings-section">
            <span className="wd-settings-label">{zh ? "外观与行为" : "Appearance & behaviour"}</span>
            <div className="wd-set-row">
              <span>{zh ? "不透明度" : "Opacity"}</span>
              <div className="wd-set-control">
                <input
                  className="wd-range"
                  type="range"
                  min={90}
                  max={100}
                  value={prefs.opacity}
                  aria-label={zh ? "不透明度" : "Opacity"}
                  onChange={(event) => patch({ opacity: Number(event.target.value) })}
                />
                <b>{prefs.opacity}%</b>
              </div>
            </div>
            <div className="wd-set-row">
              <span>{zh ? "更新频率" : "Refresh rate"}</span>
              <Select
                value={prefs.intervalMs}
                label={zh ? "更新频率" : "Refresh rate"}
                options={INTERVALS.map((ms) => ({
                  value: ms,
                  label: zh ? `${ms / 1000} 秒` : `${ms / 1000} seconds`,
                }))}
                onChange={(intervalMs) => patch({ intervalMs })}
              />
            </div>
            <div className="wd-set-row">
              <span>{zh ? "显示位置" : "Dock corner"}</span>
              <Select
                value={prefs.corner}
                label={zh ? "显示位置" : "Dock corner"}
                placement="up"
                options={CORNERS.map((corner) => ({
                  value: corner.id,
                  label: zh ? corner.zh : corner.en,
                }))}
                onChange={(corner) => patch({ corner })}
              />
            </div>
            <div className="wd-set-row">
              <span>{zh ? "记住折叠状态" : "Remember folded state"}</span>
              <button
                type="button"
                className={`wd-set-toggle ${prefs.rememberCollapsed ? "is-on" : ""}`}
                role="switch"
                aria-checked={prefs.rememberCollapsed}
                aria-label={zh ? "记住折叠状态" : "Remember folded state"}
                onClick={() =>
                  patch({ rememberCollapsed: !prefs.rememberCollapsed })
                }
              />
            </div>
          </div>

          <button
            type="button"
            className="wd-close-line"
            onClick={() => patch({ enabled: false })}
          >
            {zh ? "关闭桌面实时监控" : "Turn off desktop monitor"}
          </button>
        </div>
      ) : null}

      {!collapsed ? (
        <WidgetPanels snapshot={snapshot} prefs={prefs} history={history} zh={zh} />
      ) : null}
      {!collapsed && !hasPanels ? (
        <div className="wd-card wd-empty">
          {zh
            ? "所有面板都已关闭，点击右上角设置重新开启。"
            : "All panels are off. Open Settings at the top right to bring them back."}
        </div>
      ) : null}
    </div>
  );
}
