/* 桌面小工具 panel widgets: shared between the desktop widget window and the
 * main-window preview. Pure SVG rendering (rings + sparklines), no deps. */
import { useMemo, useRef } from "react";
import { formatBytes, formatRate, percent } from "../lib/format";
import type { SystemSnapshot, WidgetPrefs } from "../types";

const RING_R = 30;
const RING_C = 2 * Math.PI * RING_R;

export function Ring({
  label,
  sub,
  value,
  color,
}: {
  label: string;
  sub: string;
  value: number;
  color: string;
}) {
  const arc = (Math.max(0, Math.min(100, value)) / 100) * RING_C;
  return (
    <div className="wd-ring">
      <svg width="76" height="76" viewBox="0 0 76 76" aria-hidden="true">
        <circle cx="38" cy="38" r={RING_R} fill="none" stroke="#eef1f6" strokeWidth="7" />
        <circle
          cx="38"
          cy="38"
          r={RING_R}
          fill="none"
          stroke={arc > 0 ? color : "#dfe4ec"}
          strokeWidth="7"
          strokeLinecap="round"
          strokeDasharray={`${arc} ${RING_C}`}
          transform="rotate(-90 38 38)"
        />
      </svg>
      <div className="wd-ring-text">
        <b>{label}</b>
        <span>{sub}</span>
      </div>
    </div>
  );
}

const HISTORY = 64;

export interface MetricHistory {
  netUp: number[];
  netDown: number[];
  diskRead: number[];
  diskWrite: number[];
}

/** Append-only ring buffers over the snapshot stream (sparkline source). */
export function useHistory(): [MetricHistory, (snapshot: SystemSnapshot) => void] {
  const ref = useRef<MetricHistory>({
    netUp: [],
    netDown: [],
    diskRead: [],
    diskWrite: [],
  });
  const bump = useMemo(
    () => (snapshot: SystemSnapshot) => {
      const push = (list: number[], value: number) => {
        list.push(value);
        if (list.length > HISTORY) list.shift();
      };
      push(ref.current.netUp, snapshot.network.tx_per_sec);
      push(ref.current.netDown, snapshot.network.rx_per_sec);
      push(ref.current.diskRead, snapshot.disk.read_per_sec);
      push(ref.current.diskWrite, snapshot.disk.write_per_sec);
    },
    [],
  );
  return [ref.current, bump];
}

function Spark({ values, color }: { values: number[]; color: string }) {
  // The history array is mutated in place between renders, so the polyline
  // must be rebuilt every render — a useMemo on identity would freeze the
  // first (empty) result forever.
  if (values.length < 2) return <svg className="wd-spark" viewBox="0 0 300 56" />;
  const top = Math.max(...values, 1);
  const width = 300;
  const step = width / (HISTORY - 1);
  const offset = HISTORY - values.length;
  const points = values
    .map((value, index) => {
      const x = (index + offset) * step;
      const y = 52 - (value / top) * 46;
      return `${x.toFixed(1)},${y.toFixed(1)}`;
    })
    .join(" ");
  return (
    <svg className="wd-spark" viewBox="0 0 300 56" preserveAspectRatio="none">
      <polyline points={points} fill="none" stroke={color} strokeWidth="1.6" />
    </svg>
  );
}

export function uptimeFull(seconds: number, zh: boolean): string {
  const d = Math.floor(seconds / 86_400);
  const h = Math.floor((seconds % 86_400) / 3_600);
  const m = Math.floor((seconds % 3_600) / 60);
  const s = Math.floor(seconds % 60);
  return zh
    ? `${d} 天 ${h} 时 ${m} 分 ${s} 秒`
    : `${d}d ${h}h ${m}m ${s}s`;
}

export function DiskGlyph() {
  return (
    <svg width="22" height="22" viewBox="0 0 24 24" fill="none" aria-hidden="true">
      <rect x="3" y="4" width="18" height="16" rx="3" fill="#5a6b80" />
      <rect x="3" y="4" width="18" height="7" rx="3" fill="#7f90a6" />
      <circle cx="17" cy="16" r="4" fill="#16b364" />
      <path
        d="m15.3 16 1.2 1.2 2.2-2.4"
        stroke="#fff"
        strokeWidth="1.3"
        fill="none"
        strokeLinecap="round"
      />
    </svg>
  );
}

export function WidgetPanels({
  snapshot,
  prefs,
  history,
  zh,
}: {
  snapshot: SystemSnapshot | null;
  prefs: WidgetPrefs;
  history: MetricHistory;
  zh: boolean;
}) {
  const memory = snapshot?.memory;
  const memoryPct = memory ? percent(memory.used, memory.total) : 0;
  return (
    <>
      {prefs.panelRun ? (
        <div className="wd-card">
          <div className="wd-run">
            <div className="wd-run-left">
              <h4>{zh ? "运行状态" : "Status"}</h4>
              <strong>{snapshot?.host.hostname || "Squid"}</strong>
              <div className="wd-uptime">
                <span>{zh ? "本次运行" : "Uptime"}</span>
                <b>{uptimeFull(snapshot?.uptime_secs ?? 0, zh)}</b>
              </div>
            </div>
            <div className="wd-rings">
              <Ring
                label="CPU"
                sub={`${Math.round(snapshot?.cpu.total ?? 0)}%`}
                value={snapshot?.cpu.total ?? 0}
                color="#f2571c"
              />
              <Ring
                label={zh ? "内存" : "Mem"}
                sub={memory ? formatBytes(memory.used) : "—"}
                value={memoryPct}
                color="#2f6bff"
              />
              <span />
              <Ring
                label="GPU"
                sub={`${Math.round(snapshot?.gpu_usage ?? 0)}%`}
                value={snapshot?.gpu_usage ?? 0}
                color="#2f6bff"
              />
            </div>
          </div>
        </div>
      ) : null}

      {prefs.panelNetwork ? (
        <div className="wd-card">
          <div className="wd-line-head">
            <h4>{zh ? "网络" : "Network"}</h4>
            <div className="wd-rate">
              <b className="up">↑ {formatRate(snapshot?.network.tx_per_sec ?? 0)}</b>
              <b className="down">↓ {formatRate(snapshot?.network.rx_per_sec ?? 0)}</b>
            </div>
          </div>
          <Spark values={history.netUp} color="#16b364" />
          <Spark values={history.netDown} color="#2f6bff" />
        </div>
      ) : null}

      {prefs.panelDisk ? (
        <div className="wd-card">
          <div className="wd-line-head">
            <h4>{zh ? "存储读写" : "Disk I/O"}</h4>
            <div className="wd-rate">
              <b className="up">R {formatRate(snapshot?.disk.read_per_sec ?? 0)}</b>
              <b className="down">W {formatRate(snapshot?.disk.write_per_sec ?? 0)}</b>
            </div>
          </div>
          <Spark values={history.diskRead} color="#16b364" />
        </div>
      ) : null}

      {prefs.panelStorage ? (
        <div className="wd-card">
          <h4 className="wd-pad">{zh ? "存储空间" : "Storage"}</h4>
          {(snapshot?.volumes ?? []).slice(0, 4).map((volume) => {
            const used = volume.total - volume.free;
            return (
              <div className="wd-vol" key={volume.mount_point}>
                <span className="wd-vol-ico">
                  <DiskGlyph />
                </span>
                <div className="wd-vol-body">
                  <div className="wd-vol-top">
                    <span>
                      {volume.name && !/^[A-Za-z]:$/.test(volume.name)
                        ? `${volume.name} (${volume.mount_point})`
                        : volume.mount_point}
                    </span>
                    <b>
                      {formatBytes(used)} / {formatBytes(volume.total)}
                    </b>
                  </div>
                  <div className="wd-vol-bar">
                    <i style={{ width: `${percent(used, volume.total)}%` }} />
                  </div>
                </div>
              </div>
            );
          })}
        </div>
      ) : null}
    </>
  );
}
