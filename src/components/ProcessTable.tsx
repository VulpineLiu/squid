import {
  Ban,
  Copy,
  ExternalLink,
  FileSearch,
  Power,
  RefreshCw,
  Search,
  ShieldAlert,
  ShieldCheck,
} from "lucide-react";
import { useCallback, useEffect, useMemo, useState } from "react";
import { useI18n } from "../i18n";
import { api } from "../lib/api";
import { formatBytes, formatCount, messageFrom, shortPath } from "../lib/format";
import type { Finding, ProcessDetail, ProcessRow } from "../types";
import type { SecurityWorkspace } from "../hooks/useSecurityWorkspace";
import {
  Button,
  ConfirmDialog,
  EmptyState,
  InlineError,
  Modal,
  useToast,
} from "./ui";

type SortKey = "name" | "pid" | "cpu" | "memory" | "startedMs";

const PAGE = 120;

/**
 * The whole process table, as opposed to the twelve busiest rows the meters show.
 *
 * It is loaded when the tab is opened and refreshed by hand, never on the 1 Hz
 * sample: a signature lookup per distinct executable is the expensive part, and
 * doing it twice a second for four hundred processes is how a monitor panel
 * becomes the thing it is monitoring.
 */
export function ProcessTable({
  active,
  initialFilter,
  security,
  onOpenStartup,
}: {
  active: boolean;
  initialFilter: string | null;
  security: SecurityWorkspace;
  onOpenStartup: (query: string) => void;
}) {
  const { t, lang } = useI18n();
  const { push } = useToast();
  const [rows, setRows] = useState<ProcessRow[] | null>(null);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [query, setQuery] = useState(initialFilter ?? "");
  const [sort, setSort] = useState<SortKey>("cpu");
  const [mineOnly, setMineOnly] = useState(false);
  const [unsignedOnly, setUnsignedOnly] = useState(false);
  const [startupOnly, setStartupOnly] = useState(false);
  const [grouped, setGrouped] = useState(false);
  const [limit, setLimit] = useState(PAGE);
  const [detail, setDetail] = useState<ProcessDetail | null>(null);
  const [opening, setOpening] = useState(false);
  const [victim, setVictim] = useState<ProcessRow | null>(null);
  const [stopping, setStopping] = useState(false);
  const [trust, setTrust] = useState(true);

  const load = useCallback(
    async (withTrust: boolean) => {
      setLoading(true);
      try {
        setRows(await api.processInventory(withTrust));
        setError(null);
      } catch (reason) {
        setError(messageFrom(reason));
      } finally {
        setLoading(false);
      }
    },
    [],
  );

  useEffect(() => {
    if (active && rows === null) void load(true);
  }, [active, load, rows]);

  useEffect(() => {
    if (initialFilter) setQuery(initialFilter);
  }, [initialFilter]);

  /**
   * The confirmed finding whose file this process is running, if any.
   *
   * This is what gates "stop and quarantine": the roadmap says quarantine may not
   * be offered for an arbitrary process, so the offer appears only when the
   * backend has actually confirmed something about this exact path. Suspicious is
   * not enough, and neither is a guess made from the process name.
   */
  const findingFor = useCallback(
    (row: ProcessRow): Finding | null => {
      const scan = security.status?.scan ?? null;
      if (!row.program || !scan) return null;
      const needle = row.program.toLowerCase();
      return (
        scan.findings.find(
          (finding) =>
            finding.confidence === "confirmed" &&
            !finding.quarantined &&
            finding.path?.toLowerCase() === needle,
        ) ?? null
      );
    },
    [security.status],
  );

  const visible = useMemo(() => {
    const needle = query.trim().toLowerCase();
    let list = (rows ?? []).filter((row) => {
      if (mineOnly && !row.owned) return false;
      if (unsignedOnly && row.signed) return false;
      if (startupOnly && !row.startupItem) return false;
      if (!needle) return true;
      return (
        row.name.toLowerCase().includes(needle) ||
        row.command.toLowerCase().includes(needle) ||
        (row.program ?? "").toLowerCase().includes(needle) ||
        String(row.pid) === needle
      );
    });
    list = [...list].sort((a, b) => {
      if (sort === "name") return a.name.localeCompare(b.name);
      if (sort === "pid") return a.pid - b.pid;
      if (sort === "startedMs") return (b.startedMs ?? 0) - (a.startedMs ?? 0);
      if (sort === "memory") return b.memory - a.memory;
      return b.cpu - a.cpu;
    });
    return list;
  }, [mineOnly, query, rows, sort, startupOnly, unsignedOnly]);

  const grouped_rows = useMemo(() => {
    if (!grouped) return null;
    const merged = new Map<
      string,
      { name: string; cpu: number; memory: number; instances: number; rows: ProcessRow[] }
    >();
    for (const row of visible) {
      const entry = merged.get(row.name) ?? {
        name: row.name,
        cpu: 0,
        memory: 0,
        instances: 0,
        rows: [],
      };
      entry.cpu += row.cpu;
      entry.memory += row.memory;
      entry.instances += 1;
      entry.rows.push(row);
      merged.set(row.name, entry);
    }
    return [...merged.values()].sort((a, b) => b.cpu - a.cpu);
  }, [grouped, visible]);

  const openDetail = async (row: ProcessRow) => {
    setOpening(true);
    try {
      setDetail(await api.processDetail(row.pid));
    } catch (reason) {
      push(messageFrom(reason), "danger");
    } finally {
      setOpening(false);
    }
  };

  const stop = async (row: ProcessRow) => {
    setStopping(true);
    try {
      const result = await api.processStop(row.pid, row.name);
      push(result.message, "success");
      setVictim(null);
      setDetail(null);
      await load(trust);
    } catch (reason) {
      push(messageFrom(reason), "danger");
    } finally {
      setStopping(false);
    }
  };

  /**
   * Stop the process, then move the file it was running out of the way.
   *
   * The order is not interchangeable: a running executable cannot be quarantined
   * reliably, and quarantining something that is still in memory would leave the
   * threat running with its file gone. Only reachable when the backend has a
   * confirmed finding for this exact path, which is what `findingFor` guarantees.
   */
  const stopAndQuarantine = async (row: ProcessRow) => {
    const finding = findingFor(row);
    if (!finding) return;
    setStopping(true);
    try {
      await api.processStop(row.pid, row.name);
      const message = await security.quarantine(finding.id);
      push(message, "success");
      setVictim(null);
      setDetail(null);
      await load(trust);
    } catch (reason) {
      push(messageFrom(reason), "danger");
    } finally {
      setStopping(false);
    }
  };

  const copy = async (value: string) => {
    try {
      await navigator.clipboard.writeText(value);
      push(t("Copied", "已复制"), "success");
    } catch {
      push(t("Could not copy", "无法复制"), "danger");
    }
  };

  if (error && !rows) return <InlineError message={error} />;

  return (
    <section className="panel">
      <div className="process-controls">
        <label className="search-field">
          <Search size={14} />
          <input
            value={query}
            onChange={(event) => setQuery(event.target.value)}
            placeholder={t("Search name, path or PID", "搜索名称、路径或 PID")}
          />
        </label>
        <div className="chip-row">
          <button
            className={`filter-chip ${mineOnly ? "is-active" : ""}`}
            onClick={() => setMineOnly((current) => !current)}
          >
            {t("My processes", "我的进程")}
          </button>
          <button
            className={`filter-chip ${unsignedOnly ? "is-active" : ""}`}
            onClick={() => setUnsignedOnly((current) => !current)}
          >
            {t("Unsigned only", "仅未签名")}
          </button>
          <button
            className={`filter-chip ${startupOnly ? "is-active" : ""}`}
            onClick={() => setStartupOnly((current) => !current)}
          >
            {t("Started at login", "由登录项启动")}
          </button>
          <button
            className={`filter-chip ${grouped ? "is-active" : ""}`}
            onClick={() => setGrouped((current) => !current)}
          >
            {t("Group by app", "按应用聚合")}
          </button>
          <button
            className={`filter-chip ${trust ? "is-active" : ""}`}
            onClick={() => {
              const next = !trust;
              setTrust(next);
              void load(next);
            }}
            title={t(
              "Signature lookups make each refresh slower",
              "读取签名会让每次刷新变慢",
            )}
          >
            {t("Check signatures", "检查签名")}
          </button>
          <Button
            variant="ghost"
            size="sm"
            icon={<RefreshCw size={14} className={loading ? "spin" : ""} />}
            onClick={() => void load(trust)}
            disabled={loading}
          >
            {t("Refresh", "刷新")}
          </Button>
        </div>
      </div>

      {visible.length === 0 ? (
        <EmptyState
          icon={<Search size={24} />}
          title={t("No process matches", "没有匹配的进程")}
          description={t(
            "Clear the filters, or search by part of a path.",
            "请清除筛选条件，或按路径片段搜索。",
          )}
        />
      ) : grouped && grouped_rows ? (
        <ul className="process-grouped">
          {grouped_rows.slice(0, limit).map((group) => (
            <li key={group.name}>
              <button className="process-group-head" onClick={() => void openDetail(group.rows[0])}>
                <strong>{group.name}</strong>
                <small>
                  {t(`${group.instances} instances`, `${group.instances} 个实例`)}
                </small>
                <span className="pg-cpu">{group.cpu.toFixed(1)}%</span>
                <span className="pg-memory">{formatBytes(group.memory)}</span>
              </button>
            </li>
          ))}
        </ul>
      ) : (
        <>
          <div className="ptable pt-head">
            <button onClick={() => setSort("name")}>{t("Process", "进程")}</button>
            <button onClick={() => setSort("pid")}>PID</button>
            <span>{t("User", "用户")}</span>
            <button onClick={() => setSort("cpu")}>CPU</button>
            <button onClick={() => setSort("memory")}>{t("Memory", "内存")}</button>
            <button onClick={() => setSort("startedMs")}>{t("Started", "启动时间")}</button>
            <span>{t("Signature", "签名")}</span>
            <span />
          </div>
          <ul className="ptable pt-rows">
            {visible.slice(0, limit).map((row) => {
              const finding = findingFor(row);
              return (
                <li key={row.pid} className={finding ? "is-flagged" : ""}>
                  <button className="pt-name" onClick={() => void openDetail(row)}>
                    <strong title={row.program ?? row.name}>{row.name}</strong>
                    <small title={row.program ?? undefined}>
                      {row.program ? shortPath(row.program, 62) : t("path unavailable", "路径不可用")}
                    </small>
                  </button>
                  <span className="pt-pid">{row.pid}</span>
                  <span className="pt-user" title={row.user}>
                    {row.user}
                  </span>
                  <span className="pt-cpu">{row.cpu.toFixed(1)}%</span>
                  <span className="pt-memory">{formatBytes(row.memory)}</span>
                  <span className="pt-start">
                    {row.startedMs ? new Date(row.startedMs).toLocaleTimeString() : "—"}
                  </span>
                  <span className={`pt-sign ${row.signed ? "is-signed" : ""}`}>
                    {row.signed ? (
                      <>
                        <ShieldCheck size={12} />
                        {row.publisher ?? t("signed", "已签名")}
                      </>
                    ) : (
                      t("unsigned", "未签名")
                    )}
                  </span>
                  <span className="pt-actions">
                    {row.startupItem ? (
                      <button
                        className="icon-button"
                        title={t("Started by a startup item", "由启动项启动")}
                        onClick={() => onOpenStartup(row.program ?? row.name)}
                      >
                        <ShieldAlert size={14} />
                      </button>
                    ) : null}
                    {finding ? <span className="pt-flag">{t("confirmed finding", "已确认发现")}</span> : null}
                    <button
                      className="icon-button"
                      title={
                        row.owned
                          ? t("Stop process", "结束进程")
                          : t("Not yours to stop", "无权结束")
                      }
                      disabled={!row.owned}
                      onClick={() => setVictim(row)}
                    >
                      <Power size={14} />
                    </button>
                  </span>
                </li>
              );
            })}
          </ul>
          {visible.length > limit ? (
            <button className="text-button is-center" onClick={() => setLimit((c) => c + PAGE * 3)}>
              {t(
                `Show ${Math.min(PAGE * 3, visible.length - limit)} more of ${formatCount(
                  visible.length,
                )}`,
                `再显示 ${Math.min(PAGE * 3, visible.length - limit)} 项（共 ${formatCount(
                  visible.length,
                )}）`,
              )}
            </button>
          ) : null}
        </>
      )}

      <Modal
        open={Boolean(detail)}
        onClose={() => setDetail(null)}
        size="drawer"
        title={detail ? `${detail.row.name} · PID ${detail.row.pid}` : ""}
        subtitle={detail?.row.program ?? undefined}
        footer={
          detail ? (
            <>
              {detail.row.program ? (
                <>
                  <Button
                    variant="ghost"
                    icon={<ExternalLink size={15} />}
                    onClick={() =>
                      void api
                        .revealPath(detail.row.program as string)
                        .catch((reason) => push(messageFrom(reason), "danger"))
                    }
                  >
                    {t("Open file location", "打开文件位置")}
                  </Button>
                  <Button
                    variant="ghost"
                    icon={<FileSearch size={15} />}
                    onClick={() => onOpenStartup(detail.row.program as string)}
                  >
                    {t("Startup entry", "查看启动项")}
                  </Button>
                </>
              ) : null}
              {detail.row.owned ? (
                <Button variant="danger" icon={<Power size={15} />} onClick={() => setVictim(detail.row)}>
                  {t("Stop process", "结束进程")}
                </Button>
              ) : null}
              {detail.row.owned && findingFor(detail.row) ? (
                <Button
                  variant="danger"
                  icon={<Ban size={15} />}
                  disabled={stopping}
                  onClick={() => void stopAndQuarantine(detail.row)}
                >
                  {t("Stop and quarantine", "结束并隔离")}
                </Button>
              ) : null}
            </>
          ) : null
        }
      >
        {detail ? (
          <ProcessDetailBody
            detail={detail}
            lang={lang}
            onCopy={copy}
            stopNote={
              detail.stopNote ??
              (detail.row.owned
                ? null
                : t("This process belongs to another user.", "该进程属于其他用户。"))
            }
            onOpenStartup={onOpenStartup}
          />
        ) : opening ? (
          <p>{t("Reading that process…", "正在读取该进程…")}</p>
        ) : null}
      </Modal>

      <ConfirmDialog
        open={Boolean(victim)}
        title={t("Stop this process?", "结束此进程？")}
        message={victim ? `${victim.name} · PID ${victim.pid}` : ""}
        note={t(
          "Unsaved work here may be lost. Only your own processes can be stopped.",
          "进程中未保存的内容可能丢失。只会结束当前用户的进程。",
        )}
        confirmLabel={t("Stop process", "结束进程")}
        cancelLabel={t("Cancel", "取消")}
        busy={stopping}
        icon={<Power size={22} />}
        onConfirm={() => victim && void stop(victim)}
        onClose={() => setVictim(null)}
      />
    </section>
  );
}

function ProcessDetailBody({
  detail,
  lang,
  onCopy,
  stopNote,
  onOpenStartup,
}: {
  detail: ProcessDetail;
  lang: "zh" | "en";
  onCopy: (value: string) => Promise<void>;
  stopNote: string | null;
  onOpenStartup: (query: string) => void;
}) {
  const { t } = useI18n();
  const { row, trust } = detail;
  const state = trust.signed
    ? trust.adHoc
      ? t("Ad-hoc signature", "临时签名")
      : t("Trusted signature", "受信任签名")
    : trust.untrusted
      ? t("Signature present but not trusted", "有签名但不受信任")
      : t("Unsigned", "未签名");
  const rows: [string, string][] = [
    [t("Parent", "父进程"), row.parent ? `${row.parent}` : t("none", "无")],
    [t("User", "用户"), row.user],
    [t("Working directory", "工作目录"), row.workingDirectory ?? t("unknown", "未知")],
    [t("Signature", "签名"), trust.signer ? `${state} · ${trust.signer}` : state],
  ];
  if (detail.sha256) rows.push(["SHA-256", detail.sha256]);
  if (row.startedMs)
    rows.push([t("Started", "启动时间"), new Date(row.startedMs).toLocaleString(lang)]);
  return (
    <div className="item-detail">
      <div className="detail-command">
        <span>{t("Command line", "命令行")}</span>
        <code>{row.command || t("(nothing recorded)", "（未记录）")}</code>
        {row.command ? (
          <button className="icon-button" title={t("Copy", "复制")} onClick={() => void onCopy(row.command)}>
            <Copy size={13} />
          </button>
        ) : null}
      </div>
      {detail.startup ? (
        <div className="detail-startup">
          <ShieldAlert size={14} />
          <div>
            <strong>{t("Started by a startup item", "由启动项启动")}</strong>
            <small>{detail.startup.name}</small>
          </div>
          <Button size="sm" variant="ghost" onClick={() => onOpenStartup(detail.startup?.name ?? "")}>
            {t("Open startup items", "打开启动项页面")}
          </Button>
        </div>
      ) : null}
      <dl className="detail-grid">
        {rows.map(([label, value]) => (
          <div key={`${label}-${value}`}>
            <dt>{label}</dt>
            <dd title={value}>{value}</dd>
          </div>
        ))}
      </dl>
      {stopNote ? (
        <p className="detail-note">
          <Power size={13} />
          {stopNote}
        </p>
      ) : null}
      <p className="detail-note">
        {t(
          "Stopping a process never touches its file. Quarantine needs a confirmed finding.",
          "结束进程不会处理其文件；只有已确认发现才会隔离。",
        )}
      </p>
    </div>
  );
}
