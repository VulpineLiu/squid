import { AlertTriangle, Check, LoaderCircle } from "lucide-react";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { useI18n } from "../i18n";
import { formatBytes, formatCount } from "../lib/format";
import { skipGroupOf, skipTally, formatDuration } from "../lib/skipGroups";
import { Button } from "./ui";
import type { DeleteResult, JobProgress } from "../types";

/**
 * The one clean-progress language every page speaks (垃圾清理 / 重复文件 /
 * 大文件): the confirm strip morphs in place into an honest per-file
 * progress bar, then settles into a green (or amber when items stayed
 * behind) result banner that fades itself after 5 seconds. After 10 seconds
 * of work a「转入后台」link shrinks the bar into a corner capsule so the
 * page stays usable — there is deliberately no mid-delete cancel.
 */

export interface CleanPhase {
  phase: "busy" | "done";
  /** Final outcome; required (and rendered) once phase === "done". */
  outcome?: DeleteResult | null;
  /** Wall-clock delete duration measured by the page. */
  elapsedMs?: number;
}

const DISMISS_SECONDS = 5;
const BACKDROP_SECONDS = 10;

export function CleanBar({
  spec,
  progress,
  noun,
  mode,
  onHistory,
  onDismiss,
  onRescan,
  auto = true,
  showSkips = true,
}: {
  spec: CleanPhase;
  /** Live job events (tick carries done/total, deleted carries freed). */
  progress: JobProgress | null;
  /** Noun the counted items are called, e.g. 副本 / 项 / 大文件. */
  noun: [string, string];
  mode: "trash" | "permanent";
  onHistory: () => void;
  onDismiss: () => void;
  /** Offered on the amber banner's reason groups when a rescan helps. */
  onRescan?: () => void;
  /** false = a sticky result screen: no countdown, 知道了 dismisses. */
  auto?: boolean;
  /** false = the page renders the skipped-reason groups itself below. */
  showSkips?: boolean;
}) {
  const { t, lang } = useI18n();
  const zh = lang === "zh";
  const unit = zh ? noun[0] : noun[1];
  const binWord = mode === "trash" ? t("to the Recycle Bin", "移入回收站") : t("deleted for good", "彻底删除");

  // Honest cumulative state across the event stream: keep the last known
  // done/total (ticks), sum freed over unique deleted paths (two engine
  // halves can stream in sequence; paths dedupe the pair).
  const freedPaths = useRef<Map<string, number>>(new Map());
  const lastTick = useRef<{ done: number; total: number } | null>(null);
  const [mirror, setMirror] = useState<{ done: number; total: number; freed: number }>({
    done: 0,
    total: 0,
    freed: 0,
  });
  const [elapsed, setElapsed] = useState(0);

  const resetRun = useCallback(() => {
    freedPaths.current = new Map();
    lastTick.current = null;
    setMirror({ done: 0, total: 0, freed: 0 });
    setElapsed(0);
  }, []);

  useEffect(() => {
    if (spec.phase !== "busy") return;
    if (!progress) return;
    if (progress.event === "deleted") {
      const path = progress.detail ?? "";
      if (path && !freedPaths.current.has(path)) {
        freedPaths.current.set(path, progress.freed ?? 0);
      }
      let freed = 0;
      freedPaths.current.forEach((value) => (freed += value));
      setMirror((current) => ({ ...current, freed }));
      return;
    }
    if (progress.event === "tick" && progress.done !== null && progress.total !== null) {
      lastTick.current = { done: progress.done, total: progress.total };
      setMirror((current) => ({ ...current, ...lastTick.current }));
    }
  }, [progress, spec.phase]);

  const [minimized, setMinimized] = useState(false);
  const [skipsOpen, setSkipsOpen] = useState(false);
  const [secondsLeft, setSecondsLeft] = useState(DISMISS_SECONDS);
  const dismiss = useRef(onDismiss);
  dismiss.current = onDismiss;

  // Busy clock: powers the 转入后台 threshold and a live elapsed feel.
  useEffect(() => {
    if (spec.phase !== "busy") return;
    resetRun();
    const started = Date.now();
    const timer = window.setInterval(() => setElapsed(Date.now() - started), 1000);
    return () => window.clearInterval(timer);
  }, [spec.phase, resetRun]);

  // Done banner: countdown to self-dismiss (sticky result screens opt out).
  useEffect(() => {
    if (spec.phase !== "done") return;
    if (!auto) return;
    setSecondsLeft(DISMISS_SECONDS);
    const timer = window.setInterval(() => {
      setSecondsLeft((current) => {
        if (current <= 1) {
          window.clearInterval(timer);
          dismiss.current();
          return 0;
        }
        return current - 1;
      });
    }, 1000);
    return () => window.clearInterval(timer);
  }, [spec.phase, auto]);

  const pct =
    spec.phase === "busy" && mirror.total > 0
      ? Math.min(100, Math.round((mirror.done / mirror.total) * 100))
      : null;

  const skipped = spec.outcome?.skipped ?? [];
  const groups = useMemo(() => {
    const map = new Map<
      string,
      {
        title: [string, string];
        hint: [string, string];
        action?: "rescan";
        rows: { path: string; reason: string }[];
      }
    >();
    for (const row of skipped) {
      const meta = skipGroupOf(row.reason);
      const bucket =
        map.get(meta.key) ?? { title: meta.title, hint: meta.hint, action: meta.action, rows: [] };
      bucket.rows.push(row);
      map.set(meta.key, bucket);
    }
    return [...map.values()];
  }, [skipped]);

  if (spec.phase === "busy") {
    if (minimized) {
      return (
        <button
          type="button"
          className="cleanbar-capsule"
          onClick={() => setMinimized(false)}
          title={t("Back to the clean progress", "回到清理进度")}
        >
          <LoaderCircle size={13} className="spin" />
          {t(`Cleaning ${formatCount(mirror.done)}/${formatCount(mirror.total)}`, `清理中 ${formatCount(mirror.done)}/${formatCount(mirror.total)}`)}
        </button>
      );
    }
    return (
      <section className="cleanbar is-busy" role="status">
        <div className="cleanbar-row">
          <LoaderCircle size={17} className="spin is-primary" />
          <div className="cleanbar-main">
            <b>
              {t(`Cleaning ${formatCount(mirror.done)} / ${formatCount(mirror.total)} ${unit}`, `正在清理 ${formatCount(mirror.done)} / ${formatCount(mirror.total)} ${unit}`)}
              {" · "}
              {t(`${formatBytes(mirror.freed)} freed so far`, `已释放 ${formatBytes(mirror.freed)}`)}
            </b>
            <small className="cleanbar-current">
              {progress?.subject
                ? `${t("Current", "当前")}：${progress.subject}（${binWord}）`
                : t("Preparing…", "准备中…")}
            </small>
          </div>
          {pct !== null ? <span className="cleanbar-pct">{pct}%</span> : null}
          {elapsed >= BACKDROP_SECONDS * 1000 ? (
            <button type="button" className="cleanbar-back" onClick={() => setMinimized(true)}>
              {t("Move to background", "转入后台")}
            </button>
          ) : null}
        </div>
        <div className="cleanbar-track" role="progressbar" aria-valuenow={pct ?? undefined} aria-valuemin={0} aria-valuemax={100}>
          <span className={pct === null ? "is-indeterminate" : ""} style={pct === null ? undefined : { width: `${pct}%` }} />
        </div>
      </section>
    );
  }

  const outcome = spec.outcome;
  if (!outcome) return null;
  const partial = skipped.length > 0;
  const freed = outcome.freedAllocated || outcome.freed;
  return (
    <section className={`cleanbar ${partial ? "is-partial" : "is-done"}`} role="status">
      <div className="cleanbar-row">
        <span className={`cleanbar-mark ${partial ? "is-warn" : "is-ok"}`}>
          {partial ? <AlertTriangle size={14} /> : <Check size={15} />}
        </span>
        <div className="cleanbar-main">
          <b>
            {partial
              ? t(
                  `Cleaned ${formatCount(outcome.removed)} ${unit} · ${formatBytes(freed)} freed · ${formatCount(skipped.length)} left behind`,
                  `已清理 ${formatCount(outcome.removed)} ${unit} · 释放 ${formatBytes(freed)} · ${formatCount(skipped.length)} 个未处理`,
                )
              : t(
                  `Clean finished · ${formatCount(outcome.removed)} ${unit} ${binWord} · ${formatBytes(freed)} freed`,
                  `清理完成 · ${formatCount(outcome.removed)} ${unit}${binWord} · 释放 ${formatBytes(freed)}`,
                )}
          </b>
          <small>
            {partial ? skipTally(skipped) : ""}
            {partial && spec.elapsedMs ? " · " : ""}
            {spec.elapsedMs ? t(`took ${formatDuration(spec.elapsedMs)} · undo from History`, `用时 ${formatDuration(spec.elapsedMs)} · 可从「历史」撤销`) : t("undo from History", "可从「历史」撤销")}
          </small>
        </div>
        {partial && showSkips ? (
          <Button variant="ghost" size="sm" onClick={() => setSkipsOpen((v) => !v)}>
            {skipsOpen ? t("Hide reasons", "收起原因") : t("Why & what next", "查看原因并处理")}
          </Button>
        ) : null}
        <Button variant="ghost" size="sm" onClick={onHistory}>
          {t("Open history", "查看历史")}
        </Button>
        <Button size="sm" onClick={onDismiss}>
          {auto
            ? t(`Got it (${secondsLeft}s)`, `知道了（${secondsLeft}s）`)
            : t("Got it", "知道了")}
        </Button>
      </div>
      {showSkips && partial && skipsOpen ? (
        <div className="cleanbar-skips">
          {groups.map((group) => (
            <div className="result-skip-group" key={group.title[0]}>
              <div className="result-skip-title">
                <strong>{zh ? group.title[1] : group.title[0]}</strong>
                <span>{group.rows.length}</span>
              </div>
              <ul>
                {group.rows.slice(0, 5).map((row) => (
                  <li key={row.path} title={row.reason}>
                    <code>{row.path}</code>
                    <small>{row.reason}</small>
                  </li>
                ))}
                {group.rows.length > 5 ? (
                  <li className="more">
                    {t(`… ${group.rows.length - 5} more`, `… 另有 ${group.rows.length - 5} 项`)}
                  </li>
                ) : null}
              </ul>
              <small className="result-skip-hint">{zh ? group.hint[1] : group.hint[0]}</small>
              {group.action === "rescan" && onRescan ? (
                <Button size="sm" variant="secondary" onClick={onRescan}>
                  {t("Rescan", "重新扫描")}
                </Button>
              ) : null}
            </div>
          ))}
        </div>
      ) : null}
    </section>
  );
}
