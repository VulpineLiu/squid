import {
  Activity,
  BatteryCharging,
  Cpu,
  HardDrive,
  MonitorSmartphone,
  Network,
  Power,
  RefreshCw,
  ShieldAlert,
  Timer,
} from "lucide-react";
import { useEffect, useState } from "react";
import { useI18n } from "../i18n";
import { api } from "../lib/api";
import {
  formatBytes,
  formatCount,
  formatMemoryBytes,
  formatRate,
  formatUptime,
  messageFrom,
  percent,
} from "../lib/format";
import type { ProcessSample, SystemSnapshot, WidgetPrefs } from "../types";
import type { SecurityWorkspace } from "../hooks/useSecurityWorkspace";
import { ProcessTable } from "../components/ProcessTable";
import {
  Button,
  ConfirmDialog,
  Gauge,
  InlineError,
  MetricBar,
  PageHeader,
  SectionTitle,
  Toggle,
  UsageBar,
  useToast,
} from "../components/ui";

export function SystemPage({
  snapshot,
  error,
  loading,
  refresh,
  initialProcessFilter,
  security,
  onOpenStartup,
}: {
  snapshot: SystemSnapshot | null;
  error: string | null;
  loading: boolean;
  refresh: () => Promise<void>;
  initialProcessFilter: string | null;
  security: SecurityWorkspace;
  onOpenStartup: (query: string) => void;
}) {
  const { t, lang } = useI18n();
  const { push } = useToast();
  const [victim, setVictim] = useState<ProcessSample | null>(null);
  const [stopping, setStopping] = useState(false);
  // Two tabs rather than a third page: performance and processes are the same
  // question asked at different resolutions, and the roadmap did not want a
  // separate "Processes" entry in a sidebar that is already at ten.
  const [tab, setTab] = useState<"performance" | "processes">(
    initialProcessFilter ? "processes" : "performance",
  );
  const [widget, setWidget] = useState<WidgetPrefs | null>(null);
  const [widgetBusy, setWidgetBusy] = useState(false);

  useEffect(() => {
    let cancelled = false;
    let stop: (() => void) | null = null;
    void api
      .widgetGet()
      .then((view) => !cancelled && setWidget(view.prefs))
      .catch(() => undefined);
    void api
      .onWidgetSettings((next) => !cancelled && setWidget(next))
      .then((unlisten) => {
        if (cancelled) unlisten();
        else stop = unlisten;
      });
    return () => {
      cancelled = true;
      stop?.();
    };
  }, []);

  const toggleWidget = async (enabled: boolean) => {
    setWidgetBusy(true);
    try {
      const next = await api.widgetSet({ enabled });
      setWidget(next);
      push(
        enabled
          ? t("Desktop widget is now on.", "桌面实时监控已显示。")
          : t("Desktop widget is now hidden.", "桌面实时监控已隐藏。"),
        "success",
      );
    } catch (reason) {
      push(messageFrom(reason), "danger");
    } finally {
      setWidgetBusy(false);
    }
  };

  const stop = async () => {
    if (!victim) return;
    setStopping(true);
    try {
      await api.stopProcess(victim);
      push(
        t(`${victim.name} was closed`, `已结束 ${victim.name}`),
        "success",
      );
      setVictim(null);
      await refresh();
    } catch (reason) {
      push(messageFrom(reason), "danger");
    } finally {
      setStopping(false);
    }
  };

  const memoryPercent = snapshot
    ? percent(snapshot.memory.used, snapshot.memory.total)
    : 0;

  return (
    <div className="page system-page">
      <PageHeader
        title={t("The pulse of your machine.", "掌握设备的每一次呼吸。")}
        description={t(
          "Live processor, memory, disk and network activity, sampled locally every couple of seconds.",
          "实时查看处理器、内存、磁盘与网络活动，数据每两秒在本地刷新。",
        )}
        actions={
          <button
            className={`refresh-pill ${loading ? "is-busy" : ""}`}
            onClick={() => void refresh()}
            disabled={loading}
          >
            <RefreshCw size={14} className={loading ? "spin" : ""} />
            {t("Refresh", "刷新")}
          </button>
        }
      />

      {error ? <InlineError message={error} /> : null}

      <div className="chip-row page-tabs" role="tablist">
        {(
          [
            ["performance", t("Performance", "性能")],
            ["processes", t("Processes", "进程")],
          ] as ["performance" | "processes", string][]
        ).map(([key, label]) => (
          <button
            key={key}
            role="tab"
            aria-selected={tab === key}
            className={`filter-chip ${tab === key ? "is-active" : ""}`}
            onClick={() => setTab(key)}
          >
            {label}
          </button>
        ))}
      </div>

      {tab === "processes" ? (
        <ProcessTable
          active
          initialFilter={initialProcessFilter}
          security={security}
          onOpenStartup={onOpenStartup}
        />
      ) : null}

      <div className="system-performance" hidden={tab !== "performance"}>
        <section className={`system-widget-bar ${widget?.enabled ? "is-on" : ""}`}>
          <span className="system-widget-icon">
            <MonitorSmartphone size={18} />
          </span>
          <div className="system-widget-copy">
            <strong>{t("Desktop live monitor", "桌面实时监控")}</strong>
            <span>
              {t(
                "Keep these live stats on your desktop. Open the settings from the top of the widget.",
                "把实时数据固定在桌面；设置可在小工具顶部下拉打开。",
              )}
            </span>
          </div>
          {widget ? (
            <div className="system-widget-switch">
              <span>{widget.enabled ? t("On desktop", "已显示") : t("Hidden", "已隐藏")}</span>
              <Toggle
                checked={widget.enabled}
                disabled={widgetBusy}
                onChange={(next) => void toggleWidget(next)}
                label={t("Desktop live monitor", "桌面实时监控")}
              />
            </div>
          ) : (
            <span className="system-widget-loading">{t("Reading…", "读取中…")}</span>
          )}
        </section>

        <div className="system-gauges">
        <section className="system-gauge-card">
          <div className="system-gauge-head">
            <span className="metric-icon tone-primary">
              <Cpu size={18} />
            </span>
            <div>
              <strong>{t("Processor", "处理器")}</strong>
              <span>{snapshot?.cpu.brand || t("Reading processor…", "正在读取处理器…")}</span>
            </div>
          </div>
          <Gauge
            value={snapshot?.cpu.total ?? 0}
            size={168}
            stroke={13}
            tone="primary"
            label={t("Processor usage", "处理器占用")}
          >
            <strong className="gauge-number is-small">
              {Math.round(snapshot?.cpu.total ?? 0)}%
            </strong>
            <span className="gauge-sub">{t("in use", "占用")}</span>
          </Gauge>
          <div className="gauge-facts">
            <span>
              {snapshot?.cpu.physical_cores ?? 0} {t("physical", "物理")} /{" "}
              {snapshot?.cpu.logical_cores ?? 0} {t("logical", "逻辑")}
            </span>
            <span>
              {t("Load", "负载")} {(snapshot?.cpu.load[0] ?? 0).toFixed(2)}
            </span>
          </div>
          <div className="core-bars">
            {(snapshot?.cpu.per_core ?? []).slice(0, 16).map((value, index) => (
              <span
                key={index}
                className={`core-bar ${
                  value > 75 ? "is-hot" : value > 40 ? "is-busy" : ""
                }`}
                title={`${t("Core", "核心")} ${index + 1}: ${Math.round(value)}%`}
              >
                <i style={{ height: `${Math.max(6, Math.min(100, value))}%` }} />
              </span>
            ))}
          </div>
        </section>

        <section className="system-gauge-card">
          <div className="system-gauge-head">
            <span className="metric-icon tone-neutral">
              <Activity size={18} />
            </span>
            <div>
              <strong>{t("Memory", "内存")}</strong>
              <span>
                {snapshot
                  ? `${formatMemoryBytes(snapshot.memory.used)} / ${formatMemoryBytes(
                      Math.max(snapshot.memory.installed, snapshot.memory.total),
                    )}`
                  : t("Reading memory…", "正在读取内存…")}
              </span>
            </div>
          </div>
          <Gauge
            value={memoryPercent}
            size={168}
            stroke={13}
            tone="primary"
            label={t("Memory usage", "内存占用")}
          >
            <strong className="gauge-number is-small">{Math.round(memoryPercent)}%</strong>
            <span className="gauge-sub">
              {snapshot ? formatMemoryBytes(snapshot.memory.available) : "—"}{" "}
              {t("available", "可用")}
            </span>
          </Gauge>
          <div className="memory-split">
            <div>
              <span>{t("Used", "已用")}</span>
              <strong>{formatMemoryBytes(snapshot?.memory.used ?? 0)}</strong>
            </div>
            {(snapshot?.memory.installed ?? 0) > (snapshot?.memory.total ?? 0) ? (
              <div>
                <span>{t("Reserved (GPU)", "硬件保留（含核显）")}</span>
                <strong>
                  {formatMemoryBytes(
                    Math.max(
                      0,
                      (snapshot?.memory.installed ?? 0) - (snapshot?.memory.total ?? 0),
                    ),
                  )}
                </strong>
              </div>
            ) : null}
            <div>
              <span>{t("Swap", "交换")}</span>
              <strong>
                {formatMemoryBytes(snapshot?.memory.swap_used ?? 0)}
                <small> / {formatMemoryBytes(snapshot?.memory.swap_total ?? 0)}</small>
              </strong>
            </div>
          </div>
        </section>

        <section className="system-gauge-card system-activity-card">
          <div className="system-gauge-head">
            <span className="metric-icon tone-primary">
              <Network size={18} />
            </span>
            <div>
              <strong>{t("Network", "网络")}</strong>
              <span>
                {snapshot?.network.primary || t("All interfaces", "全部接口")}
              </span>
            </div>
          </div>
          <div className="activity-pair">
            <div>
              <span className="activity-arrow is-down">↓</span>
              <strong>{formatRate(snapshot?.network.rx_per_sec ?? 0)}</strong>
              <small>{t("Download", "下载")}</small>
            </div>
            <div>
              <span className="activity-arrow is-up">↑</span>
              <strong>{formatRate(snapshot?.network.tx_per_sec ?? 0)}</strong>
              <small>{t("Upload", "上传")}</small>
            </div>
          </div>
          <div className="activity-divider" />
          <div className="activity-pair is-compact">
            <div>
              <span className="activity-arrow is-down">R</span>
              <strong>{formatRate(snapshot?.disk.read_per_sec ?? 0)}</strong>
              <small>{t("Disk read", "磁盘读取")}</small>
            </div>
            <div>
              <span className="activity-arrow is-up">W</span>
              <strong>{formatRate(snapshot?.disk.write_per_sec ?? 0)}</strong>
              <small>{t("Disk write", "磁盘写入")}</small>
            </div>
          </div>
          <div className="activity-uptime">
            <Timer size={14} />
            {snapshot
              ? t(
                  `Up for ${formatUptime(snapshot.uptime_secs, lang)}`,
                  `已运行 ${formatUptime(snapshot.uptime_secs, lang)}`,
                )
              : t("Sampling…", "正在采样…")}
          </div>
        </section>
      </div>

      {snapshot?.battery?.present ? (
        <section className="battery-panel">
          <div className="battery-icon">
            <BatteryCharging size={24} />
          </div>
          <div className="battery-main">
            <span className="field-label">{t("Battery", "电池")}</span>
            <strong>{snapshot.battery.percent}%</strong>
            <span>
              {snapshot.battery.charging
                ? t("Charging", "正在充电")
                : snapshot.battery.on_ac
                  ? t("Connected to power", "已接通电源")
                  : t("On battery", "使用电池")}
            </span>
          </div>
          <div className="battery-facts">
            {snapshot.battery.health_percent !== null ? (
              <div>
                <span>{t("Health", "健康度")}</span>
                <strong>{snapshot.battery.health_percent}%</strong>
              </div>
            ) : null}
            {snapshot.battery.cycle_count !== null ? (
              <div>
                <span>{t("Cycles", "循环次数")}</span>
                <strong>{snapshot.battery.cycle_count}</strong>
              </div>
            ) : null}
            {snapshot.battery.temperature_c !== null ? (
              <div>
                <span>{t("Temperature", "温度")}</span>
                <strong>{snapshot.battery.temperature_c.toFixed(1)}°C</strong>
              </div>
            ) : null}
          </div>
          <div className="battery-track">
            <span style={{ width: `${snapshot.battery.percent}%` }} />
          </div>
        </section>
      ) : null}

      <div className="system-lower">
        <section className="process-panel">
          <SectionTitle
            title={t("Top processes", "高占用进程")}
            action={
              <Button variant="ghost" size="sm" onClick={() => setTab("processes")}>
                {t("View all processes", "查看全部进程")}
              </Button>
            }
          />
          <div className="process-head">
            <span />
            <span>{t("Process", "进程")}</span>
            <span>CPU</span>
            <span>{t("Memory", "内存")}</span>
            <span />
          </div>
          <div className="process-list">
            {(snapshot?.top_processes ?? []).map((process) => (
              <div className="process-row" key={process.pid}>
                <span className="process-dot" />
                <span className="process-name">
                  <strong>{process.name}</strong>
                  <small>PID {process.pid}</small>
                </span>
                <span className="process-cpu">{process.cpu.toFixed(1)}%</span>
                <span className="process-memory">{formatMemoryBytes(process.memory)}</span>
                <button
                  className="process-stop"
                  title={t("Stop process", "结束进程")}
                  onClick={() => setVictim(process)}
                >
                  <Power size={13} />
                </button>
              </div>
            ))}
          </div>
        </section>

        <section className="health-side">
          <SectionTitle
            title={t("Volumes", "磁盘卷")}
            subtitle={t("Mounted storage and free space.", "已挂载存储与可用空间。")}
          />
          <div className="volume-list">
            {(snapshot?.volumes ?? []).map((volume) => {
              const use = percent(volume.total - volume.free, volume.total);
              return (
                <div className="volume-row" key={volume.mount_point}>
                  <span className="metric-icon tone-primary">
                    <HardDrive size={17} />
                  </span>
                  <div>
                    <strong>{volume.name || volume.mount_point}</strong>
                    <span>
                      {formatBytes(volume.free)} {t("free of", "可用，共")}{" "}
                      {formatBytes(volume.total)}
                    </span>
                    <UsageBar
                      used={volume.total - volume.free}
                      total={volume.total}
                      tone={use > 88 ? "danger" : "primary"}
                    />
                  </div>
                  <b>{Math.round(use)}%</b>
                </div>
              );
            })}
          </div>
          <div className="host-card">
            <ShieldAlert size={17} />
            <div>
              <strong>{snapshot?.host.hostname || t("This computer", "这台电脑")}</strong>
              <span>
                {[
                  snapshot?.host.os_version || snapshot?.host.os_name,
                  snapshot?.host.arch,
                ]
                  .filter(Boolean)
                  .join(" · ")}
              </span>
            </div>
          </div>
        </section>
      </div>

      <div className="system-process-count">
        <Activity size={13} />
        {t(
          `${formatCount(snapshot?.processes ?? 0)} processes · ${formatCount(
            snapshot?.network.interfaces ?? 0,
          )} network interfaces`,
          `${formatCount(snapshot?.processes ?? 0)} 个进程 · ${formatCount(
            snapshot?.network.interfaces ?? 0,
          )} 个网络接口`,
        )}
        <MetricBar value={snapshot?.disk.activity ?? 0} tone="success" />
      </div>
      </div>

      <ConfirmDialog
        open={Boolean(victim)}
        title={t("Stop this process?", "结束此进程？")}
        message={
          victim
            ? `${victim.name} · PID ${victim.pid}`
            : ""
        }
        note={t(
          "Unsaved work in this process may be lost. System processes are refused.",
          "此进程中未保存的内容可能会丢失。系统关键进程会被拒绝结束。",
        )}
        confirmLabel={t("Stop process", "结束进程")}
        cancelLabel={t("Cancel", "取消")}
        onConfirm={() => void stop()}
        onClose={() => setVictim(null)}
        busy={stopping}
        icon={<Power size={22} />}
      />
    </div>
  );
}
