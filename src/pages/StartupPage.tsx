import {
  AlertTriangle,
  ChevronDown,
  ChevronRight,
  ExternalLink,
  FileSearch,
  KeyRound,
  Lock,
  RefreshCw,
  Search,
  ShieldAlert,
  ToggleLeft,
  ToggleRight,
} from "lucide-react";
import { useEffect, useMemo, useRef, useState } from "react";
import { useI18n } from "../i18n";
import { api } from "../lib/api";
import { formatDate, formatCount, messageFrom, shortPath } from "../lib/format";
import type {
  StartupCategory,
  StartupItem,
  StartupLock,
  StartupRisk,
  StartupSource,
} from "../types";
import type { SecurityWorkspace } from "../hooks/useSecurityWorkspace";
import type { StartupWorkspace } from "../hooks/useStartupWorkspace";
import {
  Button,
  ConfirmDialog,
  EmptyState,
  InlineError,
  Modal,
  PageHeader,
  SectionTitle,
  Select,
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
  "signature-unreadable": ["Signature could not be read", "无法读取签名"],
  "script-interpreter": ["Started through a script", "经由脚本启动"],
  "recently-created": ["Added recently", "最近添加"],
  "hidden-entry": ["Hidden entry", "隐藏条目"],
  "hidden-home-directory": ["Runs from a dot-folder", "位于隐藏目录"],
};

const LOCK_LABEL: Record<StartupLock, [string, string]> = {
  "operating-system": ["Part of the operating system", "系统自带条目"],
  "needs-authorisation": ["Needs an administrator", "需要管理员权限"],
  "not-writable": ["Not writable by this user", "当前用户不可写"],
  "managed-by-system-settings": ["Managed in System Settings", "由系统设置管理"],
  "not-managed": ["Read-only in this release", "本版本只读"],
};

type Tab = StartupCategory;
type Scope = "all" | "user" | "system";

/// Where to reveal an entry: the program it runs if that exists, otherwise the
/// plist, key or task file that declares it. A login item whose target is gone can
/// still be shown by its declaration.
///
/// An id is only a place when it looks like one. macOS login items are named rather
/// than filed, and their `login-item://` id is a key, not a path to open.
function revealTarget(item: StartupItem): string | null {
  if (item.program) return item.program;
  return /^[\\/]|[A-Za-z]:[\\/]/.test(item.id) ? item.id : null;
}

/**
 * Startup items: what the machine has been told to run by itself.
 *
 * Read the way the operating system reads it. macOS shows two lists — what opens
 * when you log in, and what may keep running behind it — and files each entry under
 * the application that installed it. A flat list of 870 launchd labels asks the user
 * to do the grouping squid can do from paths alone.
 *
 * A risk label means "look at this", never "this is malware"; the alternative is a
 * wall of orange beside entries the operating system owns, which is why those are
 * hidden until asked for.
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
  const [tab, setTab] = useState<Tab>("open-at-login");
  const [scope, setScope] = useState<Scope>("all");
  const [showVendor, setShowVendor] = useState(false);
  const [riskyOnly, setRiskyOnly] = useState(false);
  const [query, setQuery] = useState(initialQuery ?? "");
  const [open, setOpen] = useState<Set<string>>(new Set());
  const [detail, setDetail] = useState<StartupItem | null>(null);
  const [pending, setPending] = useState<{ item: StartupItem; enabled: boolean } | null>(null);

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

  /** Everything but the search, so the counts beside the tabs are what clicking shows. */
  const listed = useMemo(() => {
    const all = startup.view?.items ?? [];
    return all.filter((item) => {
      if (!showVendor && item.systemVendor) return false;
      if (scope !== "all" && item.scope !== scope) return false;
      if (riskyOnly && item.risks.length === 0) return false;
      return true;
    });
  }, [riskyOnly, scope, showVendor, startup.view]);

  const groups = useMemo(() => {
    const needle = query.trim().toLowerCase();
    const matching = needle
      ? listed.filter(
          (item) =>
            item.name.toLowerCase().includes(needle) ||
            item.command.toLowerCase().includes(needle) ||
            (item.program ?? "").toLowerCase().includes(needle) ||
            (item.owner?.name ?? "").toLowerCase().includes(needle),
        )
      : listed;
    const byOwner = new Map<string, StartupItem[]>();
    for (const item of matching.filter((entry) => entry.category === tab)) {
      const key = item.owner?.name ?? "";
      byOwner.set(key, [...(byOwner.get(key) ?? []), item]);
    }
    return [...byOwner.entries()]
      .map(([name, items]) => ({
        key: name || "—",
        name,
        items: items.sort(
          (a, b) => Number(a.systemVendor) - Number(b.systemVendor) || a.name.localeCompare(b.name),
        ),
      }))
      .sort(
        (a, b) =>
          Number(a.key === "—") - Number(b.key === "—") ||
          a.name.localeCompare(b.name, "zh", { sensitivity: "base" }),
      );
  }, [listed, query, tab]);

  const counts = {
    "open-at-login": listed.filter((item) => item.category === "open-at-login").length,
    background: listed.filter((item) => item.category === "background").length,
  };
  const shown = groups.reduce((total, group) => total + group.items.length, 0);
  const searching = query.trim().length > 0;

  // Coming from a process row means coming with a question: "is this one of mine,
  // and where is it declared?" So the handed-over path becomes the search, vendor
  // items stop being hidden, and whichever list holds it is opened.
  //
  // Applied once per handover. The inventory is re-read whenever the page opens and
  // after every change, and an effect that keyed on it alone would restore this
  // search and clear the filters the person has set since.
  const handed = useRef<string | null>(null);
  useEffect(() => {
    if (!initialQuery || handed.current === initialQuery) return;
    setQuery(initialQuery);
    setShowVendor(true);
    setScope("all");
    const match = startup.view?.items.find(
      (item) => item.program === initialQuery || item.command === initialQuery,
    );
    if (match) setTab(match.category);
    // Until the inventory has arrived the tab is still a guess, so the handover
    // stays open and runs again when the list it needs is finally there.
    if (startup.view) handed.current = initialQuery;
  }, [initialQuery, startup.view]);

  /**
   * Login items belong to macOS, which refuses to let another process write them, so
   * the honest action beside a read-only row is the door to the pane that can.
   */
  const openSettings = async () => {
    try {
      await api.openStartupSettings();
    } catch (reason) {
      push(messageFrom(reason), "danger");
    }
  };

  const change = async (item: StartupItem, enabled: boolean) => {
    try {
      push(await startup.setEnabled(item, enabled), "success");
    } catch (reason) {
      push(messageFrom(reason), "danger");
    } finally {
      setPending(null);
    }
  };

  const view = startup.view;
  return (
    <div className="page is-stacked">
      <PageHeader
        title={t("What your machine starts by itself", "你的电脑自己会启动什么")}
        description={t(
          "Grouped by the app that installed each entry. A risk label means worth a look.",
          "按安装该条目的应用分组。风险标记只代表值得检查。",
        )}
        actions={
          <Button
            variant="ghost"
            size="sm"
            icon={<RefreshCw size={15} />}
            onClick={() => void startup.refresh()}
            disabled={startup.loading}
          >
            {t("Read again", "重新读取")}
          </Button>
        }
      />

      {startup.error ? <InlineError message={startup.error} /> : null}

      {view?.notes.length ? (
        <div className="notice">
          {view.notes.map((note) => (
            <span key={`${note.kind}-${note.subject}`}>
              <AlertTriangle size={13} />
              {note.kind === "login-items-unavailable"
                ? t(
                    "macOS has not let Squid read the login items.",
                    "macOS 尚未允许 Squid 读取登录项。",
                  )
                : t("This folder could not be listed:", "以下目录无法读取：")}{" "}
              {note.kind === "unreadable-root" ? note.subject : null}
              {note.detail ? ` — ${note.detail}` : ""}
            </span>
          ))}
        </div>
      ) : null}

      <div className="chip-row page-tabs" role="tablist">
        {(
          [
            ["open-at-login", t("Login items", "登录项")],
            ["background", t("Background", "后台程序")],
          ] as [Tab, string][]
        ).map(([key, label]) => (
          <button
            key={key}
            role="tab"
            aria-selected={tab === key}
            className={`filter-chip ${tab === key ? "is-active" : ""}`}
            onClick={() => setTab(key)}
          >
            {label}
            <b>{counts[key]}</b>
          </button>
        ))}
      </div>

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
          <Select
            value={scope}
            align="start"
            label={t("Scope", "范围")}
            onChange={setScope}
            options={[
              { value: "all", label: t("All scopes", "全部范围") },
              { value: "user", label: t("My user", "当前用户") },
              { value: "system", label: t("System", "系统") },
            ]}
          />
          <button
            className={`filter-chip ${riskyOnly ? "is-active" : ""}`}
            onClick={() => setRiskyOnly((current) => !current)}
          >
            {t("Worth a look", "值得检查")}
          </button>
          <button
            className={`filter-chip ${showVendor ? "is-active" : ""}`}
            onClick={() => setShowVendor((current) => !current)}
          >
            {t("Operating system items", "系统自带条目")}
            <b>{formatCount((view?.items ?? []).filter((item) => item.systemVendor).length)}</b>
          </button>
        </div>
      </section>

      {shown === 0 ? (
        <EmptyState
          icon={<ToggleLeft size={26} />}
          title={
            searching || riskyOnly
              ? t("Nothing matches", "没有符合条件的条目")
              : tab === "open-at-login"
                ? t("Nothing opens when you log in", "登录时不会打开任何东西")
                : t("No background entries are shown", "没有显示后台条目")
          }
          description={t(
            "Operating system entries stay hidden until you show them above.",
            "系统自带条目需要在上方手动显示。",
          )}
        />
      ) : (
        <section className="panel">
          <SectionTitle
            title={
              tab === "open-at-login"
                ? t("Opens at login", "登录时打开")
                : t("Runs in the background", "在后台运行")
            }
            subtitle={t(
              "Disabling keeps the original file or value so it can be put back.",
              "停用会保留原始文件或注册表值，可随时恢复。",
            )}
            action={
              tab === "open-at-login" ? (
                <button className="text-button" onClick={() => void openSettings()}>
                  <ExternalLink size={13} />
                  {t("Open System Settings", "打开系统设置")}
                </button>
              ) : null
            }
          />
          {tab === "open-at-login" && !searching ? (
            <ul className="item-list">
              {groups.flatMap((group) => group.items).map((item) => (
                <StartupRow
                  key={item.id}
                  item={item}
                  lang={lang}
                  busy={startup.busy}
                  showSource={false}
                  onOpen={setDetail}
                  onAsk={(next) => setPending({ item, enabled: next })}
                />
              ))}
            </ul>
          ) : (
            <ul className="group-list">
              {groups.map((group) => (
                <StartupGroup
                  key={group.key}
                  group={group}
                  expanded={searching || open.has(group.key)}
                  busy={startup.busy}
                  lang={lang}
                  onToggle={() =>
                    setOpen((current) => {
                      const next = new Set(current);
                      if (next.has(group.key)) next.delete(group.key);
                      else next.add(group.key);
                      return next;
                    })
                  }
                  onOpen={setDetail}
                  onAsk={(item, enabled) => setPending({ item, enabled })}
                />
              ))}
            </ul>
          )}
        </section>
      )}

      <Modal
        open={Boolean(detail)}
        onClose={() => setDetail(null)}
        size="drawer"
        title={detail?.name ?? ""}
        subtitle={
          detail
            ? `${lang === "zh" ? SOURCE_LABEL[detail.source][1] : SOURCE_LABEL[detail.source][0]} · ${
                detail.scope === "user" ? t("your user", "当前用户") : t("system", "系统")
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

/**
 * One application and the entries it installed. Login items are shown flat, the way
 * System Settings shows them, because there are usually only a few; background
 * entries are folded under their application because a machine has twenty or more.
 */
function StartupGroup({
  group,
  expanded,
  busy,
  lang,
  onToggle,
  onOpen,
  onAsk,
}: {
  group: { key: string; name: string; items: StartupItem[] };
  expanded: boolean;
  busy: boolean;
  lang: "zh" | "en";
  onToggle: () => void;
  onOpen: (item: StartupItem) => void;
  onAsk: (item: StartupItem, enabled: boolean) => void;
}) {
  const { t } = useI18n();
  const risky = group.items.filter((item) => item.risks.length > 0).length;
  const off = group.items.filter((item) => item.disabled).length;
  const bundle = group.items.find((item) => item.owner?.bundlePath)?.owner?.bundlePath;
  const signed = group.items.some((item) => item.signed);
  return (
    <li>
      <button className="group-head" onClick={onToggle} aria-expanded={expanded}>
        <span className="item-copy">
          <strong>
            {group.name || t("Not attributed to an app", "未归属到具体应用")}
          </strong>
          <small title={bundle ?? undefined}>
            {bundle
              ? shortPath(bundle, 64)
              : lang === "zh"
                ? SOURCE_LABEL[group.items[0].source][1]
                : SOURCE_LABEL[group.items[0].source][0]}
          </small>
        </span>
        <span className="group-meta">
          {off > 0 ? (
            <span className="status-chip is-neutral">{t(`${off} off`, `${off} 项已停用`)}</span>
          ) : null}
          {risky > 0 ? (
            <span className="status-chip is-warning">
              <ShieldAlert size={11} />
              {t(`${risky} worth a look`, `${risky} 项值得检查`)}
            </span>
          ) : null}
          {signed ? (
            <span className="status-chip is-success">{t("signed", "已签名")}</span>
          ) : null}
          <span className="group-count">{formatCount(group.items.length)}</span>
        </span>
        <ChevronDown size={15} className={`group-arrow ${expanded ? "is-open" : ""}`} />
      </button>
      {expanded ? (
        <ul className="item-list is-nested">
          {group.items.map((item) => (
            <StartupRow
              key={item.id}
              item={item}
              lang={lang}
              busy={busy}
              showSource
              onOpen={onOpen}
              onAsk={(enabled) => onAsk(item, enabled)}
            />
          ))}
        </ul>
      ) : null}
    </li>
  );
}

function StartupRow({
  item,
  lang,
  busy,
  showSource,
  onOpen,
  onAsk,
}: {
  item: StartupItem;
  lang: "zh" | "en";
  busy: boolean;
  /** Where every row in sight comes from the same place, naming it again says nothing. */
  showSource: boolean;
  onOpen: (item: StartupItem) => void;
  onAsk: (enabled: boolean) => void;
}) {
  const { t } = useI18n();
  const target = item.program ?? item.command;
  return (
    <li className={`item-row ${item.disabled ? "is-off" : ""} ${item.risks.length ? "is-risky" : ""}`}>
      <button className="item-main" onClick={() => onOpen(item)}>
        <span className="item-copy">
          <strong>{item.name}</strong>
          {target ? <small title={target}>{shortPath(target, 76)}</small> : null}
        </span>
        {item.risks.map((risk) => (
          <span key={risk} className="status-chip is-warning">
            <ShieldAlert size={11} />
            {t(RISK_LABEL[risk][0], RISK_LABEL[risk][1])}
          </span>
        ))}
        {showSource ? (
          <span className="startup-source">
            {lang === "zh" ? SOURCE_LABEL[item.source][1] : SOURCE_LABEL[item.source][0]}
          </span>
        ) : null}
        {item.scope === "user" ? null : (
          <span className="status-chip is-neutral">{t("system", "系统")}</span>
        )}
      </button>
      {item.modifiable ? (
        <Toggle
          checked={!item.disabled}
          disabled={busy}
          label={t("Toggle startup item", "切换启动项")}
          onChange={onAsk}
        />
      ) : (
        <span className="muted-note">
          <Lock size={14} />
          {item.lock ? t(LOCK_LABEL[item.lock][0], LOCK_LABEL[item.lock][1]) : t("read-only", "只读")}
        </span>
      )}
      <button className="icon-button" title={t("Details", "详情")} onClick={() => onOpen(item)}>
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
  if (item.owner) rows.push([t("Application", "所属应用"), item.owner.name]);
  if (item.workingDirectory) rows.push([t("Working directory", "工作目录"), item.workingDirectory]);
  if (item.program) rows.push([t("Program", "程序"), item.program]);
  rows.push([t("Entry", "声明位置"), item.id]);
  if (item.modifiedMs)
    rows.push([
      t("Modified", "修改时间"),
      formatDate(new Date(item.modifiedMs).toISOString(), lang),
    ]);
  rows.push([
    t("Signature", "签名"),
    item.publisher
      ? item.publisher
      : item.signed
        ? t("Signed, vendor unknown", "已签名，发布者未知")
        : item.risks.includes("signature-unreadable")
          ? t("Could not be read by this user", "当前用户无法读取")
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
          {item.lock
            ? t(LOCK_LABEL[item.lock][0], LOCK_LABEL[item.lock][1])
            : t(
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
