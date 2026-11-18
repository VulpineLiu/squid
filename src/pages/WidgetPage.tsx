/* 桌面小工具 management page: master switch, per-panel toggles, appearance &
 * behaviour, and a live desktop preview. All edits go through widget_set, so
 * the desktop window reacts immediately (and back-syncs via the event). */
import { useEffect, useState } from "react";
import { useI18n } from "../i18n";
import { api } from "../lib/api";
import { messageFrom } from "../lib/format";
import type { SystemSnapshot, WidgetCorner, WidgetPrefs } from "../types";
import { InlineError, PageHeader, SectionTitle, Toggle } from "../components/ui";
import { WidgetPanels, useHistory } from "../components/WidgetPanels";

const CORNERS: { id: WidgetCorner; en: string; zh: string }[] = [
  { id: "top-right", en: "Top right", zh: "右上角" },
  { id: "top-left", en: "Top left", zh: "左上角" },
  { id: "bottom-right", en: "Bottom right", zh: "右下角" },
  { id: "bottom-left", en: "Bottom left", zh: "左下角" },
];

const INTERVALS = [2_000, 5_000, 10_000];

export function WidgetPage() {
  const { t, lang } = useI18n();
  const zh = lang === "zh";
  const [prefs, setPrefs] = useState<WidgetPrefs | null>(null);
  const [snapshot, setSnapshot] = useState<SystemSnapshot | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [history, bump] = useHistory();

  useEffect(() => {
    let cancelled = false;
    let stop: (() => void) | null = null;
    void api
      .widgetGet()
      .then((view) => !cancelled && setPrefs(view.prefs))
      .catch((reason) => !cancelled && setError(messageFrom(reason)));
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

  // The preview runs on the same live data as the desktop widget.
  useEffect(() => {
    let live = true;
    const tick = async () => {
      try {
        const next = await api.systemSnapshot();
        if (live) {
          setSnapshot(next);
          bump(next);
        }
      } catch {
        /* ignore — the widget page is advisory */
      }
    };
    void tick();
    const timer = window.setInterval(() => void tick(), 2_000);
    return () => {
      live = false;
      window.clearInterval(timer);
    };
  }, [bump]);

  const patch = (values: Parameters<typeof api.widgetSet>[0]) => {
    setError(null);
    void api
      .widgetSet(values)
      .then(setPrefs)
      .catch((reason) => setError(messageFrom(reason)));
  };

  if (!prefs) return null;
  const panels = [
    {
      key: "panelRun" as const,
      icon: "◔",
      name: t("Status rings", "运行状态"),
      desc: t(
        "CPU / memory / GPU rings, device name and session uptime.",
        "CPU / 内存 / GPU 占用圆环、设备名、本次运行时长",
      ),
    },
    {
      key: "panelNetwork" as const,
      icon: "⇅",
      name: t("Network", "网络"),
      desc: t(
        "Live up/down throughput with history curves.",
        "实时上下行速率与历史曲线",
      ),
    },
    {
      key: "panelDisk" as const,
      icon: "⛁",
      name: t("Disk I/O", "存储读写"),
      desc: t(
        "Live disk read/write throughput with history curve.",
        "磁盘实时读写速率与历史曲线",
      ),
    },
    {
      key: "panelStorage" as const,
      icon: "▾",
      name: t("Storage volumes", "存储空间"),
      desc: t(
        "Used / total bars for every drive.",
        "每个盘符的已用 / 总容量进度条",
      ),
    },
  ];
  return (
    <div className="page widget-page">
      <PageHeader
        eyebrow={t("Desktop", "桌面")}
        title={t("A live monitor right on your desktop.", "桌面上放一块实时看板。")}
        description={t(
          "Pin system status to the desktop — check it without opening Squid. Every panel toggles on its own.",
          "把系统状态钉在桌面上，不打开 Squid 也能随时查看。每个面板可独立开关。",
        )}
        actions={
          <Toggle
            checked={prefs.enabled}
            onChange={(next) => patch({ enabled: next })}
            label={t("Desktop widget switch", "桌面小工具总开关")}
          />
        }
      />

      {error ? <InlineError message={error} onDismiss={() => setError(null)} /> : null}

      <div className="widget-note">
        {t(
          "The widget is a borderless desktop-layer window (below your app windows) that never steals focus; it hides itself when every panel or the master switch is off.",
          "小工具以无边框的桌面层窗口显示：贴在壁纸上、位于其他软件窗口之下，不抢焦点；关闭全部面板或总开关后窗口自动隐藏。",
        )}
      </div>

      <div className="widget-grid">
        <section className="settings-section">
          <SectionTitle
            title={t("Panels", "面板")}
            subtitle={t(
              "Pick the cards that show on the desktop; the order matches the widget.",
              "选择要出现在桌面小工具里的卡片，顺序与桌面上的排列一致。",
            )}
          />
          <div className="perm-rows">
            {panels.map((panel) => (
              <div className="settings-row" key={panel.key}>
                <div className="perm-row">
                  <span className="widget-row-ico">{panel.icon}</span>
                  <div>
                    <span>{panel.name}</span>
                    <small>{panel.desc}</small>
                  </div>
                </div>
                <Toggle
                  checked={prefs[panel.key]}
                  disabled={!prefs.enabled}
                  onChange={(next) => patch({ [panel.key]: next })}
                  label={panel.name}
                />
              </div>
            ))}
          </div>
        </section>

        <section className="settings-section">
          <SectionTitle
            title={t("Appearance & behaviour", "外观与行为")}
            subtitle={t(
              "The widget window stays borderless, desktop-layer and focus-free.",
              "小工具窗口始终无边框、贴在桌面层、不抢焦点。",
            )}
          />
          <div className="perm-rows">
            <div className="settings-row">
              <div className="perm-row">
                <div>
                  <span>{t("Opacity", "不透明度")}</span>
                  <small>
                    {t(
                      "100% is fully solid; at most 10% of the desktop shows through.",
                      "100% 为完全不透明，最多透出 10% 桌面背景",
                    )}
                  </small>
                </div>
              </div>
              <input
                className="widget-range"
                type="range"
                min={90}
                max={100}
                value={prefs.opacity}
                disabled={!prefs.enabled}
                onChange={(event) => patch({ opacity: Number(event.target.value) })}
              />
            </div>
            <div className="settings-row">
              <div className="perm-row">
                <div>
                  <span>{t("Refresh rate", "更新频率")}</span>
                  <small>
                    {t(
                      "Sampling interval; higher rates cost a little more power.",
                      "采样间隔，频率越高越耗电",
                    )}
                  </small>
                </div>
              </div>
              <select
                className="settings-select"
                value={prefs.intervalMs}
                disabled={!prefs.enabled}
                onChange={(event) => patch({ intervalMs: Number(event.target.value) })}
              >
                {INTERVALS.map((ms) => (
                  <option key={ms} value={ms}>
                    {t(`${ms / 1000} seconds`, `${ms / 1000} 秒`)}
                  </option>
                ))}
              </select>
            </div>
            <div className="settings-row">
              <div className="perm-row">
                <div>
                  <span>{t("Dock corner", "显示位置")}</span>
                  <small>
                    {t(
                      "Where it lands when opened; drag it anywhere afterwards.",
                      "初次打开停靠的屏幕角，之后可用鼠标拖到任意位置",
                    )}
                  </small>
                </div>
              </div>
              <select
                className="settings-select"
                value={prefs.corner}
                disabled={!prefs.enabled}
                onChange={(event) =>
                  patch({ corner: event.target.value as WidgetCorner })
                }
              >
                {CORNERS.map((corner) => (
                  <option key={corner.id} value={corner.id}>
                    {zh ? corner.zh : corner.en}
                  </option>
                ))}
              </select>
            </div>
            <div className="settings-row">
              <div className="perm-row">
                <div>
                  <span>{t("Remember folded state", "折叠记忆")}</span>
                  <small>
                    {t(
                      "Keep the collapsed/expanded choice across restarts.",
                      "记住上次是否折叠，重启后保持",
                    )}
                  </small>
                </div>
              </div>
              <Toggle
                checked={prefs.rememberCollapsed}
                disabled={!prefs.enabled}
                onChange={(next) => patch({ rememberCollapsed: next })}
                label={t("Remember folded state", "折叠记忆")}
              />
            </div>
          </div>
        </section>
      </div>

      <section className="settings-section widget-preview-section">
        <SectionTitle
          title={t("Preview", "预览")}
          subtitle={t(
            "Live look of the desktop widget (top of the screen).",
            "桌面小工具的实时效果（屏幕上方）。",
          )}
        />
        <div className="widget-preview-stage">
          <div className="widget widget-preview" style={{ opacity: prefs.opacity / 100 }}>
            <div className="wd-card wd-head">
              <span className="wd-head-dot" />
              <span className="wd-head-spacer" />
              <span className="wd-head-dot" />
            </div>
            {prefs.enabled ? (
              <WidgetPanels snapshot={snapshot} prefs={prefs} history={history} zh={zh} />
            ) : (
              <div className="wd-card widget-off-hint">
                {t(
                  "The widget is switched off — flip the master switch to see it on your desktop.",
                  "小工具已关闭 —— 打开总开关后即可在桌面看到它。",
                )}
              </div>
            )}
          </div>
        </div>
      </section>
    </div>
  );
}
