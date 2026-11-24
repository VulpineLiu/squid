import {
  AlertTriangle,
  BellRing,
  ChevronRight,
  FolderOpen,
  RefreshCw,
  Search,
  ShieldCheck,
  Trash2,
  X,
} from "lucide-react";
import { useMemo, useState } from "react";
import { useI18n } from "../i18n";
import { api } from "../lib/api";
import { shortPath } from "../lib/format";
import type { PermissionsWorkspace } from "../hooks/usePermissionsWorkspace";
import type { StartupItem } from "../types";
import {
  Button,
  ConfirmDialog,
  EmptyState,
  IconButton,
  InlineError,
  PageHeader,
  SectionTitle,
  Toggle,
} from "../components/ui";

function scopeLabel(scope: string, t: (en: string, zh: string) => string): string {
  return scope === "machine" ? t("All users", "所有用户") : t("Current user", "当前用户");
}

// The icon shared by a software group: the first entry that carries one.
function softwareIcon(items: StartupItem[]): string {
  return items.find((item) => item.icon)?.icon ?? "";
}

// A row whose program binary no longer exists on disk — the config survived
// an uninstall. MangoDisk calls these 残留项 and offers to delete the config.
function isLeftover(item: StartupItem): boolean {
  return item.targetExists === false;
}

// MangoDisk's source-kind chips: compact kind label, the detailed path lives
// in the expanded row.
const SOURCE_KIND: Record<string, [string, string]> = {
  registry: ["Registry", "注册表"],
  startupFolder: ["Startup folder", "启动文件夹"],
  scheduledTask: ["Task", "计划任务"],
  service: ["Service", "服务"],
  launchAgent: ["LaunchAgent", "LaunchAgent"],
  autostart: ["Autostart", "自启动"],
};

// When the row fires, phrased the way the user experiences it.
const TRIGGER_LABEL: Record<string, [string, string]> = {
  boot: ["On boot", "开机后"],
  logon: ["On logon", "登录后"],
  background: ["Background", "后台常驻"],
};

type SigTone = "microsoft" | "vendor" | "unsigned" | "invalid" | "unknown";

function signatureChip(
  verdict: string,
  t: (en: string, zh: string) => string,
): [string, string] | null {
  switch (verdict as SigTone) {
    case "microsoft":
      return [t("Microsoft", "微软"), "is-microsoft"];
    case "vendor":
      return [t("Signed", "签名有效"), "is-vendor"];
    case "unsigned":
      return [t("Not signed", "未签名"), "is-unsigned"];
    case "invalid":
      return [t("Invalid", "签名无效"), "is-invalid"];
    default:
      return null;
  }
}

// The one signature story a group tells: the worst verdict inside it, so a
// group never looks greener than its sketchiest row.
function groupSignature(items: StartupItem[]): string {
  const seen = new Set(items.map((item) => item.signature ?? "unknown"));
  if (seen.has("invalid")) return "invalid";
  if (seen.has("unsigned")) return "unsigned";
  if (seen.has("vendor")) return "vendor";
  if (seen.has("microsoft")) return "microsoft";
  return "unknown";
}

// 残留 / 已启用 / 已禁用 row tag; leftover wins the colour fight.
function statusChip(
  item: StartupItem,
  t: (en: string, zh: string) => string,
): [string, string] {
  if (isLeftover(item)) return [t("Leftover", "残留"), "is-leftover"];
  return item.enabled
    ? [t("Enabled", "已启用"), "is-on"]
    : [t("Disabled", "已禁用"), "is-off"];
}

// 启动文件夹 rows are removable by deleting the shortcut; the folder is a
// nice human landing spot for anything else.
function rowRevealPath(item: StartupItem): string {
  const anchor = item.target || item.command;
  if (!anchor) return "";
  return anchor.replace(/"$/, "").replace(/^(.*\.(exe|lnk|bat|cmd))\s+/i, "$1");
}

// MangoDisk's status filter: a segment with live counts, 残留 included.
type StartupFilter = "all" | "enabled" | "disabled" | "leftover";


// 电脑管家 shows the owning software's icon on every row it can attribute to
// an installed program; a grey initial covers the leftovers.
function RowIcon({ icon, label }: { icon: string | null | undefined; label: string }) {
  return icon ? (
    <img className="perm-row-icon" src={icon} alt="" />
  ) : (
    <span className="perm-row-icon perm-row-icon-fallback" aria-hidden="true">
      {label.trim().slice(0, 1).toUpperCase()}
    </span>
  );
}

export function PermissionsPage({ workspace }: { workspace: PermissionsWorkspace }) {
  const { t } = useI18n();
  const [query, setQuery] = useState("");
  const [openGroups, setOpenGroups] = useState<Set<string>>(new Set());
  const [extCat, setExtCat] = useState<string | null>(null);
  const [startupFilter, setStartupFilter] = useState<StartupFilter>("all");
  const [showSystem, setShowSystem] = useState(false);
  const [removingIds, setRemovingIds] = useState<string[] | null>(null);
  const view = workspace.view;

  const tabs = useMemo(
    () =>
      [
        {
          id: "startup" as const,
          label: t("Startup items", "开机启动项"),
          count: view?.startupCount ?? 0,
          fresh: false,
        },
        {
          id: "extensions" as const,
          label: t("Context menus", "右键菜单"),
          count: view?.extensions.length ?? 0,
          fresh: false,
        },
        {
          id: "advanced" as const,
          label: t("Advanced startup", "进阶启动项"),
          count: workspace.advanced?.entries.length ?? 0,
          fresh: true,
        },
        {
          id: "popup" as const,
          label: t("Popup blocker", "弹窗拦截"),
          count: view?.rules.length ?? 0,
          fresh: false,
        },
      ],
    [t, view, workspace.advanced],
  );

  if (!view && workspace.scanning) {
    return (
      <div className="page">
        <PageHeader
          eyebrow={t("Permissions", "权限管理")}
          title={t("Scanning startup and shell…", "正在扫描开机启动项与右键菜单…")}
          description={t(
            "Reading registry Run keys, startup folders, scheduled tasks, context menus and open windows.",
            "正在读取注册表 Run 键、启动文件夹、计划任务、右键菜单与当前窗口。",
          )}
        />
        <div className="perm-loading">
          <RefreshCw size={18} className="perm-spin" />
          <span>{t("One moment…", "请稍候…")}</span>
        </div>
      </div>
    );
  }

  if (!view) {
    return (
      <div className="page">
        <PageHeader
          eyebrow={t("Permissions", "权限管理")}
          title={t("Startup, context menus and popups.", "开机启动、右键菜单与弹窗拦截。")}
          description={t(
            "Everything the shell runs before you ask it to.",
            "管理系统在你开口之前就自己运行的东西。",
          )}
        />
        <EmptyState
          icon={<ShieldCheck size={30} />}
          title={t("Nothing scanned yet", "还没有扫描结果")}
          description={t(
            "Squid will list startup entries, context-menu handlers and live windows.",
            "Squid 会列出启动项、右键菜单扩展和当前窗口。",
          )}
          action={
            <Button variant="primary" onClick={() => void workspace.scan()}>
              {t("Scan now", "立即扫描")}
            </Button>
          }
        />
      </div>
    );
  }

  const needle = query.trim().toLowerCase();
  const startupSearch = view.startup.filter(
    (item) =>
      !needle ||
      item.name.toLowerCase().includes(needle) ||
      item.displayName.toLowerCase().includes(needle) ||
      item.command.toLowerCase().includes(needle),
  );
  // MangoDisk's 显示系统项: Microsoft-signed / %SystemRoot% rows fold into a
  // hideable 系统组件 group instead of mixing into the software list.
  const systemName = t("System components", "系统组件");
  const startupVisible = showSystem
    ? startupSearch
    : startupSearch.filter((item) => !item.systemItem);
  const leftoverCount = startupVisible.filter(isLeftover).length;
  const enabledCount = startupVisible.filter((item) => item.enabled && !isLeftover(item)).length;
  const disabledCount = startupVisible.length - enabledCount - leftoverCount;
  const counts: Record<StartupFilter, number> = {
    all: startupVisible.length,
    enabled: enabledCount,
    disabled: disabledCount,
    leftover: leftoverCount,
  };
  const startup = startupVisible.filter((item) =>
    startupFilter === "all"
      ? true
      : startupFilter === "leftover"
        ? isLeftover(item)
        : startupFilter === "enabled"
          ? item.enabled && !isLeftover(item)
          : !item.enabled && !isLeftover(item),
  );
  // 电脑管家 style: one group per software (Chinese name in the header);
  // every entry the backend could not attribute to a real product lands in
  // a catch-all 其他软件 group; system rows sink to their own group last.
  const otherName = t("Other software", "其他软件");
  const startupGroups = new Map<string, typeof startup>();
  for (const item of startup) {
    const key = item.systemItem
      ? systemName
      : item.named && item.displayName
        ? item.displayName
        : otherName;
    const list = startupGroups.get(key) ?? [];
    list.push(item);
    startupGroups.set(key, list);
  }
  const orderedGroups = [...startupGroups.entries()].sort(
    ([a], [b]) =>
      Number(a === otherName) - Number(b === otherName) ||
      Number(a === systemName) - Number(b === systemName),
  );
  // Groups start folded (管家 habit): the header carries a master switch
  // and the status line; expanding shows the per-entry detail. Searching
  // force-expands everything so matches are never hidden.
  const searching = query.trim().length > 0;
  const isGroupOpen = (name: string) => searching || openGroups.has(name);
  const toggleGroupOpen = (name: string) =>
    setOpenGroups((current) => {
      const next = new Set(current);
      if (next.has(name)) next.delete(name);
      else next.add(name);
      return next;
    });
  const groupedExtensions = view.extensions
    .filter(
      (entry) =>
        !needle ||
        entry.name.toLowerCase().includes(needle) ||
        (entry.friendlyName ?? "").toLowerCase().includes(needle) ||
        (entry.publisher ?? "").toLowerCase().includes(needle) ||
        (entry.module ?? "").toLowerCase().includes(needle),
    )
    .reduce<Record<string, typeof view.extensions>>((groups, entry) => {
      (groups[entry.extensionPoint] ??= []).push(entry);
      return groups;
    }, {});
  // Categories stay in the backend's 文件 / 文件夹 / 桌面 / 驱动器 / 所有对象
  // order; the horizontal nav shows one at a time so long lists never make
  // the page scroll for miles.
  const extCategories = Object.keys(groupedExtensions);
  const activeCategory =
    extCat && extCategories.includes(extCat) ? extCat : extCategories[0] ?? null;

  return (
    <div className="page permissions-page">
      <PageHeader
        eyebrow={t("Permissions", "权限管理")}
        title={t("Startup, context menus and popups.", "开机启动、右键菜单与弹窗拦截。")}
        description={t(
          "Everything the shell runs before you ask it to.",
          "管理系统在你开口之前就自己运行的东西。",
        )}
        actions={
          <Button
            variant="secondary"
            icon={<RefreshCw size={15} />}
            onClick={() => void workspace.scan()}
            disabled={workspace.scanning}
          >
            {t("Rescan", "重新扫描")}
          </Button>
        }
      />

      <div className="perm-tabs" role="tablist">
        {tabs.map((tab) => (
          <button
            key={tab.id}
            role="tab"
            aria-selected={workspace.tab === tab.id}
            className={`perm-tab ${workspace.tab === tab.id ? "is-active" : ""}`}
            onClick={() => workspace.setTab(tab.id)}
          >
            {tab.label}
            <b>{tab.count}</b>
          </button>
        ))}
      </div>

      {workspace.error ? (
        <InlineError message={workspace.error} onDismiss={workspace.dismissError} />
      ) : null}

      {workspace.tab === "startup" ? (
        <section className="settings-section">
          <SectionTitle
            title={t("Startup items", "开机启动项")}
            subtitle={t(
              `${view.startupEnabledCount} of ${view.startupCount} entries start with Windows.`,
              `共 ${view.startupCount} 项，其中 ${view.startupEnabledCount} 项已开启。`,
            )}
          />
          <div className="perm-startup-bar">
            <div className="perm-filter" role="tablist" aria-label={t("Status filter", "状态筛选")}>
              {(
                [
                  ["all", "All", "全部"],
                  ["enabled", "Enabled", "已启用"],
                  ["disabled", "Disabled", "已禁用"],
                  ["leftover", "Leftover", "残留"],
                ] as const
              ).map(([id, en, zh]) => (
                <button
                  key={id}
                  role="tab"
                  aria-selected={startupFilter === id}
                  className={`perm-filter-tab ${startupFilter === id ? "is-active" : ""} ${
                    id === "leftover" && counts.leftover > 0 ? "has-leftover" : ""
                  }`}
                  onClick={() => setStartupFilter(id)}
                >
                  {t(en, zh)}
                  <b>{counts[id]}</b>
                </button>
              ))}
            </div>
            <div className="perm-startup-tools">
              <label className="perm-system-toggle">
                <Toggle
                  checked={showSystem}
                  onChange={setShowSystem}
                  label={t("Show system items", "显示系统项")}
                />
                <span>{t("Show system items", "显示系统项")}</span>
              </label>
              <label className="search-field">
                <Search size={14} />
                <input
                  value={query}
                  onChange={(event) => setQuery(event.target.value)}
                  placeholder={t("Filter by name or path…", "按名称或路径筛选…")}
                />
              </label>
            </div>
          </div>
          {orderedGroups.map(([software, items]) => {
            const onCount = items.filter((item) => item.enabled && !isLeftover(item)).length;
            const allOn = onCount === items.length;
            const open = isGroupOpen(software);
            const groupIds = items.map((item) => item.id);
            const leftovers = items.filter(isLeftover);
            const sources = [...new Set(items.map((item) => item.source))];
            const triggers = [...new Set(items.map((item) => item.trigger ?? "unknown"))];
            const groupSigChip = signatureChip(groupSignature(items), t);
            return (
              <div
                key={software}
                className={`perm-group-card ${open ? "is-open" : ""} ${
                  leftovers.length > 0 && leftovers.length === items.length ? "is-leftover" : ""
                }`}
              >
                <h4 className="perm-group" onClick={() => toggleGroupOpen(software)}>
                  {items.find((item) => item.icon)?.icon ? (
                    <img className="perm-group-icon" src={softwareIcon(items)} alt="" />
                  ) : (
                    <span className="perm-group-icon perm-group-icon-fallback" aria-hidden="true">
                      {software.trim().slice(0, 1).toUpperCase()}
                    </span>
                  )}
                  <span className="perm-group-name">{software}</span>
                  <span className="perm-group-chips">
                    {leftovers.length > 0 ? (
                      <span className="perm-sig is-leftover">
                        {t(`${leftovers.length} leftover`, `${leftovers.length} 个残留`)}
                      </span>
                    ) : null}
                    {sources.map((source) => {
                      const n = items.filter((item) => item.source === source).length;
                      return (
                        <span className="perm-src-chip" key={source}>
                          {t(SOURCE_KIND[source]?.[0] ?? source, SOURCE_KIND[source]?.[1] ?? source)}
                          {n > 1 ? <i>×{n}</i> : null}
                        </span>
                      );
                    })}
                    {triggers.map((trigger) => (
                      <span className="perm-src-chip" key={trigger}>
                        {t(TRIGGER_LABEL[trigger]?.[0] ?? "—", TRIGGER_LABEL[trigger]?.[1] ?? "—")}
                      </span>
                    ))}
                    {groupSigChip ? (
                      <span className={`perm-sig ${groupSigChip[1]}`}>{groupSigChip[0]}</span>
                    ) : null}
                  </span>
                  <b className={allOn ? "is-all" : onCount === 0 ? "is-none" : "is-part"}>
                    {allOn
                      ? t("All on", "全部开启")
                      : onCount === 0
                        ? t("All off", "已全部关闭")
                        : t(
                            `${onCount}/${items.length} on`,
                            `部分开启 ${onCount}/${items.length}`,
                          )}
                  </b>
                  <span
                    className="perm-group-tail"
                    onClick={(event) => event.stopPropagation()}
                  >
                    <span
                      className="perm-group-count"
                      onClick={(event) => {
                        event.stopPropagation();
                        toggleGroupOpen(software);
                      }}
                    >
                      {items.length > 1
                        ? t(`${items.length} items`, `共${items.length}项`)
                        : t("1 item", "共1项")}
                      <ChevronRight
                        size={13}
                        className={`perm-group-chv ${open ? "is-open" : ""}`}
                      />
                    </span>
                    <Toggle
                      checked={allOn}
                      disabled={workspace.busyId !== null}
                      onChange={(next) => void workspace.toggleStartupGroup(groupIds, next)}
                      label={`${software} master ${allOn ? "disable" : "enable"}`}
                    />
                  </span>
                </h4>
                {open ? (
                  <div className="perm-rows">
                    {items.map((item) => {
                      const [statusText, statusTone] = statusChip(item, t);
                      const sigChip = signatureChip(item.signature ?? "unknown", t);
                      const reveal = rowRevealPath(item);
                      return (
                        <div
                          className={`settings-row ${
                            workspace.busyId === item.id ? "is-busy" : ""
                          } ${isLeftover(item) ? "is-leftover-row" : ""}`}
                          key={item.id}
                        >
                          <div className="perm-row perm-row-noicon">
                            <div>
                              <span>
                                {item.name}{" "}
                                <em className={`perm-row-tag ${statusTone}`}>{statusText}</em>
                              </span>
                              <small title={item.command}>
                                {item.sourceLabel} · {scopeLabel(item.scope, t)}
                                {item.command ? ` · ${shortPath(item.command, 72)}` : ""}
                              </small>
                              <small className="perm-row-full" title={item.command}>
                                {item.command || t("(no command line)", "（无命令行记录）")}
                              </small>
                            </div>
                          </div>
                          <div className="perm-row-actions">
                            {sigChip ? (
                              <span
                                className={`perm-sig ${sigChip[1]}`}
                                title={item.signer ?? undefined}
                              >
                                {sigChip[0]}
                              </span>
                            ) : null}
                            {isLeftover(item) ? (
                              <Button
                                size="sm"
                                variant="danger"
                                icon={<Trash2 size={13} />}
                                disabled={workspace.busyId !== null}
                                onClick={() => setRemovingIds([item.id])}
                              >
                                {t("Delete entry", "删除启动项")}
                              </Button>
                            ) : null}
                            {reveal ? (
                              <IconButton
                                label={t("Open its folder", "打开所在文件夹")}
                                onClick={() => void api.revealPath(reveal).catch(() => {})}
                              >
                                <FolderOpen size={15} />
                              </IconButton>
                            ) : null}
                            <Toggle
                              checked={item.enabled}
                              disabled={workspace.busyId !== null}
                              onChange={(next) => void workspace.toggleStartup(item.id, next)}
                              label={`${item.name} ${item.enabled ? "disable" : "enable"}`}
                            />
                          </div>
                        </div>
                      );
                    })}
                    {leftovers.length > 0 && leftovers.length < items.length ? (
                      <div className="perm-group-leftover-bar">
                        <AlertTriangle size={13} />
                        <span>
                          {t(
                            `${leftovers.length} leftover entries in this group — their programs are gone.`,
                            `该组有 ${leftovers.length} 个残留项：程序已不存在，只剩启动配置。`,
                          )}
                        </span>
                        <Button
                          size="sm"
                          variant="danger"
                          disabled={workspace.busyId !== null}
                          onClick={() => setRemovingIds(leftovers.map((item) => item.id))}
                        >
                          {t(
                            `Delete ${leftovers.length} leftover(s)`,
                            `删除 ${leftovers.length} 个残留项`,
                          )}
                        </Button>
                      </div>
                    ) : null}
                  </div>
                ) : null}
              </div>
            );
          })}
          {startup.length === 0 ? (
            <p className="perm-empty">
              {counts.all === 0
                ? t("No matches.", "没有匹配的项目。")
                : t("Nothing here under this filter.", "当前筛选下没有项目。")}
            </p>
          ) : null}
        </section>
      ) : null}

      <ConfirmDialog
        open={removingIds !== null}
        tone="danger"
        icon={<Trash2 size={24} />}
        title={t("Delete startup entries?", "删除启动项？")}
        message={t(
          `Squid removes only the autostart registration (${removingIds?.length ?? 0} item(s)); no program files are touched.`,
          `Squid 只删除启动配置（共 ${removingIds?.length ?? 0} 项），不会动任何程序文件。`,
        )}
        note={t(
          "Registry values are deleted; Startup-folder shortcuts are removed; tasks are unscheduled; leftover services are set to disabled.",
          "注册表值将被删除；启动文件夹的快捷方式会被移除；计划任务会被注销；残留服务会改为禁用。",
        )}
        confirmLabel={t("Delete", "删除")}
        cancelLabel={t("Cancel", "取消")}
        busy={workspace.busyId !== null}
        onClose={() => setRemovingIds(null)}
        onConfirm={async () => {
          if (!removingIds) return;
          await workspace.removeStartup(removingIds);
          setRemovingIds(null);
        }}
      />

      {workspace.tab === "extensions" ? (
        <section className="settings-section">
          <SectionTitle
            title={t("Context menu handlers", "右键菜单扩展")}
            subtitle={t(
              "Unregistered handlers disappear immediately; Squid keeps a restore recipe for each.",
              "取消注册后右键菜单立即消失；Squid 为每一项保留恢复配方。",
            )}
          />
          <div className="perm-ext-head">
            <div className="perm-cats" role="tablist" aria-label={t("Menu surfaces", "菜单位置")}>
              {extCategories.map((point) => {
                const entries = groupedExtensions[point];
                const on = entries.filter((e) => e.registered).length;
                return (
                  <button
                    key={point}
                    role="tab"
                    aria-selected={activeCategory === point}
                    className={`perm-cat ${activeCategory === point ? "is-active" : ""}`}
                    onClick={() => setExtCat(point)}
                  >
                    {point}
                    <b>
                      {on}/{entries.length}
                    </b>
                  </button>
                );
              })}
            </div>
            <label className="search-field">
              <Search size={14} />
              <input
                value={query}
                onChange={(event) => setQuery(event.target.value)}
                placeholder={t("Filter by name, creator or module…", "按名称、创建者或模块筛选…")}
              />
            </label>
          </div>
          <div className="perm-rows">
            {(activeCategory ? groupedExtensions[activeCategory] : []).map((entry) => (
              <div
                className={`settings-row ${!entry.registered ? "is-muted" : ""} ${
                  workspace.busyId === entry.id ? "is-busy" : ""
                }`}
                key={entry.id}
              >
                <div className="perm-row">
                  <RowIcon icon={entry.icon} label={entry.friendlyName ?? entry.name} />
                  <div>
                    <span>{entry.friendlyName ?? entry.name}</span>
                    <small title={entry.module ?? entry.handler}>
                      {entry.publisher
                        ? `${t("Creator", "创建者")} ${entry.publisher} · `
                        : ""}
                      {scopeLabel(entry.scope, t)}
                      {entry.module ? ` · ${shortPath(entry.module, 56)}` : ""}
                      {!entry.registered ? ` · ${t("removed", "已移除")}` : ""}
                    </small>
                  </div>
                </div>
                <Toggle
                  checked={entry.registered}
                  disabled={workspace.busyId !== null}
                  onChange={(next) => void workspace.toggleExtension(entry.id, next)}
                  label={`${entry.name} ${entry.registered ? "remove" : "restore"}`}
                />
              </div>
            ))}
          </div>
          {extCategories.length === 0 ? (
            <p className="perm-empty">{t("No matches.", "没有匹配的项目。")}</p>
          ) : null}
        </section>
      ) : null}

      {workspace.tab === "advanced" ? (
        <section className="settings-section perm-advanced">
          <div className="perm-adv-note">
            {t(
              "Advanced rows can break boot or logon if changed carelessly — Squid only displays them here, every change records its original value, and nothing is auto-disabled.",
              "进阶区的项目改错了可能影响系统行为——默认只展示不勾选，每一项带数字签名与位置说明；改动会原样记录，可逐项恢复。",
            )}
          </div>
          {workspace.advancedScanning && !workspace.advanced ? (
            <div className="perm-loading">
              <RefreshCw size={18} className="perm-spin" />
              <span>
                {t(
                  "Reading Winlogon / IFEO / BHO and verifying signatures…",
                  "正在读取 Winlogon / IFEO / BHO 并核验数字签名…",
                )}
              </span>
            </div>
          ) : null}
          {workspace.advanced && workspace.advanced.entries.length === 0 ? (
            <p className="perm-empty">
              {t(
                "No third-party Winlogon hooks, AppInit DLLs, IFEO debuggers or BHOs found — the machine is clean.",
                "没有发现第三方 Winlogon 挂钩、AppInit DLL、IFEO 调试器或 BHO——系统很干净。",
              )}
            </p>
          ) : null}
          {workspace.advanced
            ? (["winlogon", "appinit", "ifeo", "bho"] as const).map((kind) => {
                const rows = workspace.advanced!.entries.filter((entry) => entry.kind === kind);
                if (rows.length === 0) return null;
                const titles: Record<string, [string, string]> = {
                  winlogon: ["Winlogon hooks", "Winlogon 与登录链"],
                  appinit: ["AppInit DLL injection", "AppInit_DLLs 全局注入"],
                  ifeo: ["IFEO debuggers (can hijack any program)", "IFEO 调试器（可劫持任意程序启动）"],
                  bho: ["Browser helper objects", "浏览器加载项 BHO"],
                };
                return (
                  <div className="perm-adv-group" key={kind}>
                    <SectionTitle
                      title={t(titles[kind][0], titles[kind][1])}
                      subtitle={t(`${rows.length} item(s)`, `发现 ${rows.length} 项`)}
                    />
                    {rows.map((entry) => (
                      <div className="perm-adv-row" key={entry.id}>
                        <span className={`perm-sig is-${entry.signature}`} title={entry.signer ?? undefined}>
                          {entry.signature === "microsoft"
                            ? t("Microsoft signed", "微软签名")
                            : entry.signature === "vendor"
                              ? t("Signed by", "已知厂商")
                              : entry.signature === "unsigned"
                                ? t("Not signed", "无数字签名")
                                : entry.signature === "invalid"
                                  ? t("Signature invalid", "签名无效")
                                  : t("Signature n/a", "无签名信息")}
                        </span>
                        <div className="perm-adv-main">
                          <b>{entry.name}</b>
                          <small title={entry.command}>{shortPath(entry.command, 90)}</small>
                          <small className="perm-adv-loc">{entry.location}</small>
                          {entry.warning ? (
                            <small className="perm-adv-warn">{entry.warning}</small>
                          ) : null}
                        </div>
                        <Toggle
                          checked={entry.enabled}
                          disabled={workspace.busyId !== null}
                          onChange={(next) => void workspace.toggleAdvanced(entry.id, next)}
                          label={entry.name}
                        />
                      </div>
                    ))}
                  </div>
                );
              })
          : null}
          {workspace.advanced && workspace.advanced.extras.length > 0 ? (
            <div className="perm-adv-group">
              <SectionTitle
                title={t("Automatic services", "服务（自动启动）")}
                subtitle={t(
                  "Services set to start automatically — toggling uses the same rights as Task Manager.",
                  "设置为自动启动的服务；开关与任务管理器同一套权限。",
                )}
              />
              {workspace.advanced.extras.map((entry) => (
                <div className="perm-adv-row" key={entry.id}>
                  <span className="perm-scope">{scopeLabel(entry.scope, t)}</span>
                  <div className="perm-adv-main">
                    <b>{entry.displayName || entry.name}</b>
                    <small title={entry.command}>{shortPath(entry.command, 90)}</small>
                    <small className="perm-adv-loc">{entry.sourceLabel}</small>
                  </div>
                  <Toggle
                    checked={entry.enabled}
                    disabled={workspace.busyId !== null}
                    onChange={(next) => void workspace.toggleStartup(entry.id, next)}
                    label={entry.name}
                  />
                </div>
              ))}
            </div>
          ) : null}
        </section>
      ) : null}

      {workspace.tab === "popup" ? (
        <>
          <section className="settings-section">
            <SectionTitle
              title={t("Popup blocker", "弹窗拦截")}
              subtitle={t(
                `Blocked ${workspace.closedTotal} popup windows since install.`,
                `累计拦截 ${workspace.closedTotal} 次弹窗。`,
              )}
              action={
                <Toggle
                  checked={view.watcherEnabled}
                  onChange={(next) => void workspace.setWatcher(next)}
                  label={t("Popup blocker switch", "弹窗拦截总开关")}
                />
              }
            />
            {workspace.recentClosed.length > 0 ? (
              <div className="perm-recent">
                <BellRing size={14} />
                <span>
                  {workspace.recentClosed[0].process} · {workspace.recentClosed[0].title}
                </span>
              </div>
            ) : null}
          </section>

          <section className="settings-section">
            <SectionTitle
              title={t("Rules", "拦截规则")}
              subtitle={t(
                "Each rule closes windows from one program whose title contains the text.",
                "每条规则会关闭指定程序中标题包含该文字的窗口。",
              )}
            />
            <div className="perm-rows">
              {view.rules.map((rule, index) => (
                <div className="settings-row" key={`${rule.exe}-${index}`}>
                  <div className="perm-row">
                    <RowIcon icon={rule.icon} label={rule.exe.split(/[\\/]/).pop() ?? rule.exe} />
                    <div>
                      <span>{rule.title}</span>
                      <small title={rule.exe}>
                        {shortPath(rule.exe, 64)} ·{" "}
                        {t(`closed ${rule.closed}×`, `已关闭 ${rule.closed} 次`)}
                      </small>
                    </div>
                  </div>
                  <div className="perm-row-actions">
                    <Toggle
                      checked={rule.enabled}
                      onChange={(next) => void workspace.toggleRule(index, next)}
                      label={`${rule.title} ${rule.enabled ? "disable" : "enable"}`}
                    />
                    <IconButton
                      label={t("Remove rule", "删除规则")}
                      onClick={() => void workspace.removeRule(index)}
                    >
                      <Trash2 size={15} />
                    </IconButton>
                  </div>
                </div>
              ))}
              {view.rules.length === 0 ? (
                <p className="perm-empty">
                  {t(
                    "No rules yet — pick a window below.",
                    "还没有规则，先从下面的当前窗口里“拦截”一个。",
                  )}
                </p>
              ) : null}
            </div>
          </section>

          <section className="settings-section">
            <SectionTitle
              title={t("Current windows", "当前窗口")}
              subtitle={t(
                "Refresh, then block a noisy window or close it once.",
                "刷新后可直接拦截某个烦人的窗口，或只关闭这一次。",
              )}
              action={
                <Button
                  variant="ghost"
                  size="sm"
                  icon={<RefreshCw size={14} />}
                  onClick={() => void workspace.refreshWindows()}
                >
                  {t("Refresh", "刷新")}
                </Button>
              }
            />
            <div className="perm-rows">
              {workspace.windows.map((window) => (
                <div className="settings-row" key={window.hwnd}>
                  <div className="perm-row">
                    <RowIcon icon={window.icon} label={window.process} />
                    <div>
                      <span>{window.title}</span>
                      <small title={window.exe ?? undefined}>{window.process}</small>
                    </div>
                  </div>
                  <div className="perm-row-actions">
                    <Button
                      size="sm"
                      variant="secondary"
                      disabled={!window.exe}
                      onClick={() => void workspace.addRule(window.hwnd, "contains")}
                    >
                      {t("Block", "拦截")}
                    </Button>
                    <IconButton
                      label={t("Close window", "关闭这个窗口")}
                      onClick={() => void workspace.closeWindow(window.hwnd)}
                    >
                      <X size={15} />
                    </IconButton>
                  </div>
                </div>
              ))}
            </div>
          </section>
        </>
      ) : null}
    </div>
  );
}
