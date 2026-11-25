import {
  Activity,
  AlertTriangle,
  Check,
  Search,
  ShieldCheck,
  Wrench,
  X,
} from "lucide-react";
import { useCallback, useEffect, useRef, useState } from "react";
import { api, subscribeJobProgress } from "../lib/api";
import { useI18n } from "../i18n";
import { messageFrom } from "../lib/format";
import {
  Button,
  InlineError,
  PageHeader,
  ProgressCard,
  useToast,
} from "../components/ui";
import type { JobProgress, MaintenanceResult, MaintenanceTask } from "../types";

const RISK_CHIP: Record<MaintenanceTask["risk"], [string, string, string]> = {
  // [class, zh, en]
  safe: ["is-safe", "安全", "Safe"],
  admin: ["is-admin", "需要管理员", "Admin rights"],
  slow: ["is-slow", "较慢", "Slow"],
};

export function MaintenancePage({
  embedded = false,
  onBadge,
}: {
  /** Rendered inside 系统维护 hub: page title hides, task count flows up. */
  embedded?: boolean;
  onBadge?: (value: string | null) => void;
} = {}) {
  const { t, lang } = useI18n();
  const { push } = useToast();
  const [tasks, setTasks] = useState<MaintenanceTask[]>([]);
  const [running, setRunning] = useState<string | null>(null);
  const [progress, setProgress] = useState<JobProgress | null>(null);
  const [logs, setLogs] = useState<string[]>([]);
  const [result, setResult] = useState<MaintenanceResult | null>(null);
  const [error, setError] = useState<string | null>(null);
  const job = useRef<string | null>(null);

  useEffect(() => {
    if (embedded && onBadge) onBadge(tasks.length > 0 ? `${tasks.length} 项` : null);
  }, [embedded, onBadge, tasks.length]);

  useEffect(() => {
    let alive = true;
    void api
      .maintenanceTasks()
      .then((list) => alive && setTasks(list))
      .catch((reason) => alive && setError(messageFrom(reason)));
    return () => {
      alive = false;
    };
  }, []);

  const runTask = useCallback(
    async (task: MaintenanceTask) => {
      if (running) return;
      const id = crypto.randomUUID?.() ?? String(Date.now());
      job.current = id;
      setRunning(task.id);
      setResult(null);
      setError(null);
      setLogs([]);
      setProgress(null);
      const stop = await subscribeJobProgress(id, (event) => {
        if (event.event === "note" && event.detail) {
          setLogs((current) => [...current.slice(-200), event.detail as string]);
        }
        setProgress(event);
      });
      try {
        const outcome = await api.maintenanceRun(id, task.id);
        setResult(outcome);
        if (outcome.ok) {
          push(
            lang === "zh"
              ? `${task.titleZh}：完成（用时 ${Math.round(outcome.durationMs / 1000)} 秒）`
              : `${task.titleEn}: done (${Math.round(outcome.durationMs / 1000)}s)`,
            "success",
          );
        } else {
          push(
            lang === "zh"
              ? `${task.titleZh}：在「${outcome.failedStep ?? "某一步"}」失败`
              : `${task.titleEn}: failed at ${outcome.failedStep ?? "a step"}`,
            "danger",
          );
        }
      } catch (reason) {
        setError(messageFrom(reason));
      } finally {
        stop();
        setRunning(null);
        setProgress(null);
        job.current = null;
      }
    },
    [lang, push, running],
  );

  return (
    <div className={`page maintenance-page ${embedded ? "is-embedded" : ""}`}>
      {embedded ? null : (
        <PageHeader
          title={t("System maintenance", "系统维护")}
        />
      )}
      {error ? <InlineError message={error} onDismiss={() => setError(null)} /> : null}

      <div className="maint-summary">
        <ShieldCheck size={15} />
        <span>
          {t(
            "All repairs below touch only system caches / services / indexes — never your documents.",
            "以下修复只影响系统缓存 / 服务 / 索引，不碰你的文档。",
          )}
        </span>
      </div>

      <ul className="maint-list">
        {tasks.map((task) => {
          const [chipClass, chipZh, chipEn] = RISK_CHIP[task.risk];
          const isRunning = running === task.id;
          return (
            <li key={task.id} className={`maint-row ${isRunning ? "is-running" : ""}`}>
              <span className="maint-icon">
                <Wrench size={16} />
              </span>
              <div className="maint-copy">
                <b>{lang === "zh" ? task.titleZh : task.titleEn}</b>
                <small>{lang === "zh" ? task.detailZh : task.detailEn}</small>
                {isRunning && progress ? (
                  <ProgressCard
                    progress={progress}
                    title={lang === "zh" ? task.titleZh : task.titleEn}
                    compact
                    onCancel={() => {
                      if (job.current) void api.cancelJob(job.current);
                    }}
                  />
                ) : null}
                {isRunning && logs.length > 0 ? (
                  <pre className="maint-log" role="status">
                    {logs.slice(-8).join("\n")}
                  </pre>
                ) : null}
              </div>
              <span className="maint-estimate">{task.estimate}</span>
              <span className={`maint-risk ${chipClass}`}>{lang === "zh" ? chipZh : chipEn}</span>
              {isRunning ? (
                <Button variant="ghost" size="sm" onClick={() => job.current && void api.cancelJob(job.current)}>
                  {t("Cancel", "取消")}
                </Button>
              ) : (
                <Button
                  variant="primary"
                  size="sm"
                  disabled={running !== null}
                  onClick={() => void runTask(task)}
                >
                  {t("Repair", "修复")}
                </Button>
              )}
            </li>
          );
        })}
        {tasks.length === 0 && !error ? (
          <li className="maint-row is-muted">
            <Search size={16} />
            {t("Loading the repair list…", "正在加载修复清单…")}
          </li>
        ) : null}
      </ul>

      {result ? (
        <div className={`maint-result ${result.ok ? "is-ok" : "is-fail"}`}>
          {result.ok ? <Check size={15} /> : <AlertTriangle size={15} />}
          <div>
            <b>
              {result.ok
                ? t("Repair completed", "修复完成")
                : t("Repair stopped early", "修复提前停止")}
            </b>
            <ul>
              {result.steps.map((step, index) => (
                <li key={index}>
                  <Activity size={11} /> {step}
                </li>
              ))}
            </ul>
            {result.outputTail ? (
              <details>
                <summary>{t("Command output", "命令输出")}</summary>
                <pre>{result.outputTail}</pre>
              </details>
            ) : null}
          </div>
          <Button variant="ghost" size="sm" onClick={() => setResult(null)}>
            <X size={13} />
          </Button>
        </div>
      ) : null}
    </div>
  );
}
