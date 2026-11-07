import {
  AlertTriangle,
  ChevronRight,
  ExternalLink,
  FileSearch,
  KeyRound,
  Lock,
  RefreshCw,
  Search,
  Settings2,
  ShieldAlert,
  ToggleLeft,
  ToggleRight,
} from "lucide-react";
import { useEffect, useMemo, useState } from "react";
import { useI18n } from "../i18n";
import { api } from "../lib/api";
import { formatDate, messageFrom, shortPath } from "../lib/format";
import type { StartupItem, StartupRisk, StartupSource } from "../types";
import type { SecurityWorkspace } from "../hooks/useSecurityWorkspace";
import type { StartupWorkspace } from "../hooks/useStartupWorkspace";
import {
  Button,
  Checkbox,
  ConfirmDialog,
  EmptyState,
  InlineError,
  Modal,
  PageHeader,
  SectionTitle,
  Toggle,
  useToast,
} from "../components/ui";

const SOURCE_LABEL: Record<StartupSource, [string, string]> = {
  "launch-agent": ["LaunchAgent", "LaunchAgent"],
  "launch-daemon": ["LaunchDaemon", "LaunchDaemon"],
  "login-item": ["Login item", "登录项"],
  "registry-run": ["Registry Run", "注册表 Run"],
  "startup-folder": ["Startup folder", "启动文件夹"],
  "scheduled-task": ["Scheduled task", "计划任务"],
  service: ["Service", "服务"],
};

const RISK_LABEL: Record<StartupRisk, [string, string]> = {
  "missing-target": ["Target is gone", "目标已不存在"],
  "in-temp-dir": ["Runs from a temp folder", "位于临时目录"],
  unsigned: ["No usable signature", "无可用签名"],
  "script-interpreter": ["Started through a script", "经由脚本启动"],
  "recently-created": ["Created recently", "最近创建"],
  "hidden-entry": ["Hidden entry", "隐藏条目"],
  "hidden-home-directory": ["Runs from a dot-folder", "位于隐藏目录"],
};

type ScopeFilter = "all" | "user" | "system";

/// Where to reveal an entry: the program it runs if that exists, otherwise the
/// plist, key or task file that declares it. A login item whose target is gone can
/// still be shown by its declaration.
function revealTarget(item: StartupItem): string | null {
  return item.program ?? (item.id.includes("/") || item.id.includes("\\") ? item.id : null);
}

/**
 * Startup items: what the machine has been told to run by itself.
 *
 * A risk label here means "look at this", never "this is malware" — the page says
 * so because the alternative is a wall of orange next to entries the operating
 * system owns. System items are hidden by default and read-only wherever squid
 * has no business writing without an authorisation it does not have.
 */
export function StartupPage({
  startup,
  security,
  initialQuery,
  onInspectProcess,
  onShowSecurity,
}: {
  startup: StartupWorkspace;
  security: SecurityWorkspace;
  initialQuery: string | null;
  onInspectProcess: (filter: string) => void;
  onShowSecurity: () => void;
}) {
  const { t, lang } = useI18n();
  const { push } = useToast();
  const [scope, setScope] = useState<ScopeFilter>("user");
  const [source, setSource] = useState<StartupSource | "all">("all");
  const [disabledOnly, setDisabledOnly] = useState(false);
  const [riskyOnly, setRiskyOnly] = useState(false);
  const [showVendor, setShowVendor] = useState(false);
  const [query, setQuery] = useState(initialQuery ?? "");
  const [detail, setDetail] = useState<StartupItem | null>(null);
  const [pending, setPending] = useState<{ item: StartupItem; enabled: boolean } | null>(null);
  const [selectMode, setSelectMode] = useState(false);
  const [selected, setSelected] = useState<Set<string>>(new Set());
  const [batch, setBatch] = useState(false);

  /**
   * One file, scanned now, and the user is taken to the page that shows the
   * result. Scanning in place would hide the answer behind whichever page they
   * happen to be standing on.
   */
  const scanFile = async (path: string) => {
    try {
      await security.start("custom", [path]);
      onShowSecurity();
    } catch (reason) {
      push(messageFrom(reason), "danger");
    }
  };

  const items = useMemo(() => {
    const all = startup.view?.items ?? [];
    const needle = query.trim().toLowerCase();
    return all.filter((item) => {
      if (!showVendor && item.systemVendor) return false;
      if (scope !== "all" && item.scope !== scope) return false;
      if (source !== "all" && item.source !== source) return false;
      if (disabledOnly && !item.disabled) return false;
      if (riskyOnly && item.risks.length === 0) return false;
      if (!needle) return true;
      return (
        item.name.toLowerCase().includes(needle) ||
        item.command.toLowerCase().includes(needle) ||
        (item.program ?? "").toLowerCase().includes(needle)
      );
    });
  }, [disabledOnly, query, scope, showVendor, source, startup.view]);

  const change = async (item: StartupItem, enabled: boolean) => {
    try {
      push(await startup.setEnabled(item, enabled), "success");
    } catch (reason) {
      push(messageFrom(reason), "danger");
    } finally {
      setPending(null);
    }
  };

  /**
   * One entry at a time is the rule, and it exists for a reason: a plist that
   * fails to reload, a task the vendor re-arms on next launch, and a login item
   * that belongs to an update helper all need to be visible as themselves. "All
   * user items" is offered rather than "everything", so the worst case a batch
   * can do is bounded by what the current user owns.
   */
  const toggleSelection = (item: StartupItem) => {
    setSelected((current) => {
      const next = new Set(current);
      if (next.has(item.id)) next.delete(item.id);
      else next.add(item.id);
      return next;
    });
  };

  const actionable = useMemo(
    () => new Set(items.filter((item) => item.modifiable && !item.systemVendor).map((item) => item.id)),
    [items],
  );
  const allSelected = actionable.size > 0 && [...actionable].every((id) => selected.has(id));

  const runBatch = async (enabled: boolean) => {
    const ids = [...selected].filter((id) => actionable.has(id));
    setBatch(true);
    let done = 0;
    const failures: string[] = [];
    for (const id of ids) {
      const item = startup.view?.items.find((candidate) => candidate.id === id);
      if (!item || item.disabled === !enabled) continue;
      try {
        await startup.setEnabled(item, enabled);
        done += 1;
      } catch (reason) {
        failures.push(`${item.name}: ${messageFrom(reason)}`);
      }
    }
    setBatch(false);
    setSelected(new Set());
    setSelectMode(false);
    push(
      failures.length
        ? t(
            `${done} changed · ${failures.length} refused: ${failures[0]}`,
            `已修改 ${done} 项 · ${failures.length} 项被拒绝：${failures[0]}`,
          )
        : t(
            `${done} startup items are now ${enabled ? "on" : "off"}.`,
            `${done} 个启动项已${enabled ? "启用" : "停用"}。`,
          ),
      failures.length ? "danger" : "success",
    );
  };

  // Coming from a process row means coming with a question: "is this one of
  // mine, and where is it declared?" So the handed-over path becomes the search,
  // and the vendor items the process might belong to stop being hidden.
  useEffect(() => {
    if (!initialQuery) return;
    setQuery(initialQuery);
    setShowVendor(true);
    setScope("all");
    setSource("all");
  }, [initialQuery]);

  const view = startup.view;
  return (
    <div className="page is-stacked">
      <PageHeader
        eyebrow={t("Startup items", "启动项")}
        title={t("What your machine starts by itself.", "你的电脑自己会启动什么。")}
        description={t(
          "A risk label means worth a look, never malicious.",
          "风险标记只代表值得检查，不代表恶意。",
        )}
        actions={
          <>
            <Button
              variant="ghost"
              size="sm"
              icon={<Settings2 size={15} />}
              onClick={() => setSelectMode((current) => !current)}
            >
              {selectMode
                ? t("Done selecting", "完成选择")
                : t("Select user items", "选择用户项目")}
            </Button>
            <Button
              variant="ghost"
              size="sm"
              icon={<RefreshCw size={15} />}
              onClick={() => void startup.refresh()}
              disabled={startup.loading}
            >
              {t("Read again", "重新读取")}
            </Button>
          </>
        }
      />

      {startup.error ? <InlineError message={startup.error} /> : null}

      {view ? (
        <div className="stat-grid">
          <div className="stat">
            <strong>{view.userCount}</strong>
            <span>{t("for your user", "属于当前用户")}</span>
          </div>
          <div className="stat">
            <strong>{view.systemCount}</strong>
            <span>{t("system-wide", "系统级")}</span>
          </div>
          <div className="stat">
            <strong>{view.disabledCount}</strong>
            <span>{t("turned off", "已停用")}</span>
          </div>
          <div className="stat">
            <strong>{view.actionableCount}</strong>
            <span>{t("squid may change", "Squid 可修改")}</span>
          </div>
          <div className="stat is-warning">
            <strong>{view.riskyCount}</strong>
            <span>{t("worth a look", "值得检查")}</span>
          </div>
        </div>
      ) : null}

      {view?.notes.length ? (
        <div className="notice">
          {view.notes.map((note) => (
            <span key={note}>
              <AlertTriangle size={13} />
              {note}
            </span>
          ))}
        </div>
      ) : null}

      <section className="panel">
        <label className="search-field">
          <Search size={14} />
          <input
            value={query}
            onChange={(event) => setQuery(event.target.value)}
            placeholder={t("Search name, command or path", "搜索名称、命令或路径")}
          />
        </label>
        <div className="chip-row">
          {(
            [
              ["user", t("My user", "当前用户")],
              ["system", t("System", "系统")],
              ["all", t("All scopes", "全部范围")],
            ] as [ScopeFilter, string][]
          ).map(([key, label]) => (
            <button
              key={key}
              className={`filter-chip ${scope === key ? "is-active" : ""}`}
              onClick={() => setScope(key)}
            >
              {label}
            </button>
          ))}
          <select
            className="settings-select"
            value={source}
            onChange={(event) => setSource(event.target.value as StartupSource | "all")}
          >
            <option value="all">{t("Every kind", "所有类型")}</option>
            {(Object.keys(SOURCE_LABEL) as StartupSource[]).map((key) => (
              <option key={key} value={key}>
                {lang === "zh" ? SOURCE_LABEL[key][1] : SOURCE_LABEL[key][0]}
              </option>
            ))}
          </select>
          <button
            className={`filter-chip ${disabledOnly ? "is-active" : ""}`}
            onClick={() => setDisabledOnly((current) => !current)}
          >
            {t("Turned off", "已停用")}
          </button>
          <button
            className={`filter-chip ${riskyOnly ? "is-active" : ""}`}
            onClick={() => setRiskyOnly((current) => !current)}
          >
            {t("Risky only", "仅看需注意")}
          </button>
          <button
            className={`filter-chip ${showVendor ? "is-active" : ""}`}
            onClick={() => setShowVendor((current) => !current)}
          >
            {t("Show Apple / Microsoft items", "显示系统条目")}
          </button>
        </div>
      </section>

      {selectMode ? (
        <div className="startup-batch">
          <label className="startup-batch-all">
            <Checkbox
              checked={allSelected}
              indeterminate={selected.size > 0 && !allSelected}
              onChange={() =>
                setSelected(allSelected ? new Set() : new Set(actionable))
              }
              label={t("Select all shown user items", "全选当前显示的用户项目")}
            />
            <span>
              {t(
                `${selected.size} of ${actionable.size} changeable items shown`,
                `已选 ${selected.size} / 可修改 ${actionable.size} 项`,
              )}
            </span>
          </label>
          <div className="startup-batch-actions">
            <Button
              size="sm"
              variant="ghost"
              disabled={batch || selected.size === 0}
              onClick={() => void runBatch(true)}
            >
              {t("Enable selected", "启用所选")}
            </Button>
            <Button
              size="sm"
              variant="danger"
              disabled={batch || selected.size === 0}
              onClick={() => void runBatch(false)}
            >
              {t("Disable selected", "停用所选")}
            </Button>
          </div>
        </div>
      ) : null}

      <SectionTitle
        title={t("Startup entries", "启动条目")}
        subtitle={t(
          `${items.length} shown${
            view ? ` of ${view.items.length} read from this machine` : ""
          }. Disabling keeps the original file or value so it can be put back.`,
          `当前显示 ${items.length} 项${
            view ? `，本机共读取 ${view.items.length} 项` : ""
          }。停用会保留原始文件或注册表值，可随时恢复。`,
        )}
      />

      {items.length === 0 ? (
        <EmptyState
          icon={<ToggleLeft size={26} />}
          title={t("Nothing matches these filters", "没有符合筛选条件的条目")}
          description={t(
            "System-wide entries stay hidden until you show them above.",
            "系统级条目需要在上方手动显示。",
          )}
        />
      ) : (
        <ul className="item-list">
          {items.map((item) => (
            <StartupRow
              key={item.id}
              item={item}
              lang={lang}
              busy={startup.busy || batch}
              selectMode={selectMode}
              selected={selected.has(item.id)}
              selectable={actionable.has(item.id)}
              onToggleSelected={() => toggleSelection(item)}
              onOpen={() => setDetail(item)}
              onAsk={(enabled) => setPending({ item, enabled })}
            />
          ))}
        </ul>
      )}

      <Modal
        open={Boolean(detail)}
        onClose={() => setDetail(null)}
        size="drawer"
        title={detail?.name ?? ""}
        subtitle={
          detail
            ? `${lang === "zh" ? SOURCE_LABEL[detail.source][1] : SOURCE_LABEL[detail.source][0]} · ${
                detail.scope === "user"
                  ? t("your user", "当前用户")
                  : t("system", "系统")
              }`
            : undefined
        }
        footer={
          detail ? (
            <>
              {revealTarget(detail) ? (
                <Button
                  variant="ghost"
                  icon={<ExternalLink size={15} />}
                  onClick={() =>
                    void api
                      .revealPath(revealTarget(detail) as string)
                      .catch((reason) => push(messageFrom(reason), "danger"))
                  }
                >
                  {t("Show in folder", "在文件夹中显示")}
                </Button>
              ) : null}
              <Button
                variant={detail.disabled ? "primary" : "danger"}
                disabled={!detail.modifiable || startup.busy}
                icon={detail.disabled ? <ToggleRight size={15} /> : <ToggleLeft size={15} />}
                onClick={() => {
                  const next = !detail.disabled;
                  setDetail(null);
                  setPending({ item: detail, enabled: next });
                }}
              >
                {detail.disabled ? t("Enable", "启用") : t("Disable", "停用")}
              </Button>
            </>
          ) : null
        }
      >
        {detail ? (
          <StartupDetail
            item={detail}
            lang={lang}
            onInspectProcess={onInspectProcess}
            onScan={async (path) => {
              setDetail(null);
              await scanFile(path);
            }}
          />
        ) : null}
      </Modal>

      <ConfirmDialog
        open={Boolean(pending)}
        title={
          pending?.enabled
            ? t("Enable this startup item?", "要启用这个启动项吗？")
            : t("Disable this startup item?", "要停用这个启动项吗？")
        }
        tone={pending?.enabled ? "default" : "danger"}
        busy={startup.busy}
        icon={<KeyRound size={22} />}
        confirmLabel={pending?.enabled ? t("Enable", "启用") : t("Disable", "停用")}
        cancelLabel={t("Cancel", "取消")}
        message={pending?.item.name ?? ""}
        note={t(
          "The original is kept, so this can be undone here.",
          "Squid 会保留原始值，可在此页面恢复。",
        )}
        onConfirm={() => pending && void change(pending.item, pending.enabled)}
        onClose={() => setPending(null)}
      >
        <div className="confirm-path">
          <code>{pending?.item.command}</code>
        </div>
        {pending?.item.scope === "system" ? (
          <div className="confirm-warn">
            {t(
              "System-wide: changing it needs one authorisation.",
              "系统级条目，修改需要一次系统授权。",
            )}
          </div>
        ) : null}
      </ConfirmDialog>
    </div>
  );
}

function StartupRow({
  item,
  lang,
  busy,
  selectMode,
  selected,
  selectable,
  onToggleSelected,
  onOpen,
  onAsk,
}: {
  item: StartupItem;
  lang: "zh" | "en";
  busy: boolean;
  selectMode: boolean;
  selected: boolean;
  selectable: boolean;
  onToggleSelected: () => void;
  onOpen: () => void;
  onAsk: (enabled: boolean) => void;
}) {
  const { t } = useI18n();
  const target = item.program ?? item.command;
  return (
    <li className={`item-row ${item.disabled ? "is-off" : ""} ${item.risks.length ? "is-risky" : ""}`}>
      {selectMode ? (
        <Checkbox
          checked={selected}
          disabled={!selectable}
          onChange={onToggleSelected}
          label={t("Select startup item", "选择启动项")}
        />
      ) : null}
      <button className="item-main" onClick={onOpen}>
        <span className="item-copy">
          <strong>{item.name}</strong>
          <small title={target}>{shortPath(target, 76)}</small>
        </span>
        <span className="startup-meta">
          <span className="startup-source">{lang === "zh" ? SOURCE_LABEL[item.source][1] : SOURCE_LABEL[item.source][0]}</span>
          <span className={`status-chip is-${item.scope === "user" ? "info" : "neutral"}`}>
            {item.scope === "user" ? t("user", "用户") : t("system", "系统")}
          </span>
          {item.publisher ? (
            <span className="status-chip is-success">
              {t("signed", "已签名")} · {item.publisher}
            </span>
          ) : item.signed ? null : (
            <span className="status-chip is-neutral">{t("unsigned", "未签名")}</span>
          )}
          {item.risks.map((risk) => (
            <span key={risk} className="risk-chip" title={t(RISK_LABEL[risk][0], RISK_LABEL[risk][1])}>
              <ShieldAlert size={11} />
              {t(RISK_LABEL[risk][0], RISK_LABEL[risk][1])}
            </span>
          ))}
        </span>
      </button>
      {item.modifiable ? (
        <Toggle
          checked={!item.disabled}
          disabled={busy}
          label={t("Toggle startup item", "切换启动项")}
          onChange={(next) => onAsk(next)}
        />
      ) : (
        <span className="muted-note" title={item.readonlyReason ?? undefined}>
          <Lock size={14} />
          {item.readonlyReason ?? t("read-only", "只读")}
        </span>
      )}
      <button className="icon-button" title={t("Details", "详情")} onClick={onOpen}>
        <ChevronRight size={16} />
      </button>
    </li>
  );
}

function StartupDetail({
  item,
  lang,
  onInspectProcess,
  onScan,
}: {
  item: StartupItem;
  lang: "zh" | "en";
  onInspectProcess: (filter: string) => void;
  onScan: (path: string) => Promise<void>;
}) {
  const { t } = useI18n();
  const rows: [string, string][] = [];
  if (item.workingDirectory) rows.push([t("Working directory", "工作目录"), item.workingDirectory]);
  if (item.program) rows.push([t("Program", "程序"), item.program]);
  rows.push([t("Entry", "声明位置"), item.id]);
  if (item.modifiedMs)
    rows.push([t("Modified", "修改时间"), formatDate(new Date(item.modifiedMs).toISOString(), lang)]);
  rows.push([
    t("Signature", "签名"),
    item.publisher
      ? item.publisher
      : item.signed
        ? t("Signed, vendor unknown", "已签名，发布者未知")
        : t("No usable signature", "无可用签名"),
  ]);
  return (
    <div className="item-detail">
      <div className="detail-command">
        <span>{t("Command", "命令")}</span>
        <code>{item.command || t("(nothing recorded)", "（未记录）")}</code>
      </div>
      {item.risks.length ? (
        <ul className="detail-risks">
          {item.risks.map((risk) => (
            <li key={risk}>
              <ShieldAlert size={13} />
              {t(RISK_LABEL[risk][0], RISK_LABEL[risk][1])}
            </li>
          ))}
        </ul>
      ) : null}
      <dl className="detail-grid">
        {rows.map(([label, value]) => (
          <div key={`${label}-${value}`}>
            <dt>{label}</dt>
            <dd title={value}>{value}</dd>
          </div>
        ))}
      </dl>
      {!item.modifiable ? (
        <p className="detail-note">
          <Lock size={13} />
          {item.readonlyReason ??
            t(
              "Squid installs no privileged helper, so system-wide entries stay read-only.",
              "Squid 不安装特权 helper，系统级条目保持只读。",
            )}
        </p>
      ) : null}
      <div className="detail-actions">
        {item.program ? (
          <>
            <Button
              variant="ghost"
              size="sm"
              icon={<FileSearch size={14} />}
              onClick={() => void onScan(item.program as string)}
            >
              {t("Scan this file", "扫描该文件")}
            </Button>
            <Button
              variant="ghost"
              size="sm"
              icon={<ToggleRight size={14} />}
              onClick={() => onInspectProcess(item.program as string)}
            >
              {t("Find its process", "查看关联进程")}
            </Button>
          </>
        ) : null}
      </div>
    </div>
  );
}
