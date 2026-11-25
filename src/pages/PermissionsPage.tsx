import {
  AlertTriangle,
  BellRing,
  ChevronRight,
  FolderOpen,
  LayoutGrid,
  Power,
  RefreshCw,
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
  Badge,
  Button,
  Chip,
  ConfirmDialog,
  EmptyState,
  IconButton,
  InlineError,
  Notice,
  PageHeader,
  Row,
  SearchField,
  Section,
  Subhead,
  Tabs,
  Toggle,
  Toolbar,
} from "../components/ui";

function scopeLabel(scope: string, t: (en: string, zh: string) => string): string {
  return scope === "machine" ? t("All users", "所有用户") : t("Current user", "当前用户");
}

// A row whose program binary no longer exists on disk — the config survived
// an uninstall. MangoDisk calls these 残留项 and offers to delete the config.
function isLeftover(item: StartupItem): boolean {
  return item.targetExists === false;
}

// 残留 / 已启用 / 已禁用 row tag; leftover wins the colour fight. The
// source / trigger / signature chips used to sit on the group header — the
// user found them noise; the detailed path already rides on every row.
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

// 其他软件 carries no real product identity, so it never rides a member's
// icon (ChatGPT's blue tile labelled 其他软件 read as a bug). A neutral
// apps-grid glyph says "a pile of things we could not attribute".
function GroupIcon({
  software,
  items,
  neutral,
}: {
  software: string;
  items: { icon: string | null }[];
  neutral: boolean;
}) {
  if (neutral) {
    return (
      <span className="perm-group-icon perm-group-icon-neutral" aria-hidden="true">
        <LayoutGrid size={16} />
      </span>
    );
  }
  const icon = items.find((item) => item.icon)?.icon;
  return icon ? (
    <img className="perm-group-icon" src={icon} alt="" />
  ) : (
    <span className="perm-group-icon perm-group-icon-fallback" aria-hidden="true">
      {software.trim().slice(0, 1).toUpperCase()}
    </span>
  );
}

export function PermissionsPage({
  workspace,
  platform,
}: {
  workspace: PermissionsWorkspace;
  platform?: string;
}) {
  const { t } = useI18n();
  const [query, setQuery] = useState("");
  const [openGroups, setOpenGroups] = useState<Set<string>>(new Set());
  const [openExtGroups, setOpenExtGroups] = useState<Set<string>>(new Set());
  const [extCat, setExtCat] = useState<string | null>(null);
  const [startupFilter, setStartupFilter] = useState<StartupFilter>("all");
  const [showSystem, setShowSystem] = useState(false);
  const [removingIds, setRemovingIds] = useState<string[] | null>(null);
  const view = workspace.view;

  const tabs = useMemo(() => {
    const all = [
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
      ];
    // Context menus, advanced autostart hooks and the popup blocker are
    // Windows shell surfaces; macOS only has login/background startup items.
    return platform === "windows" ? all : all.filter((item) => item.id === "startup");
  }, [platform, t, view, workspace.advanced]);

  if (!view && workspace.scanning) {
    return (
      <div className="page">
        <PageHeader
          eyebrow={t("Permissions", "权限管理")}
          title={t("Scanning startup and shell…", "正在扫描开机启动项与右键菜单…")}
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
          title={t("Startup, context menus and popups", "开机启动、右键菜单与弹窗拦截")}
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
  // Same software-grouping rule as the startup list: rows whose module
  // carries real product strings group under that product (微信's shell
  // extensions across several surfaces become one card); the rest sink to
  // 其他软件. Groups open folded, searching force-expands.
  const extGrouped = new Map<string, typeof view.extensions>();
  for (const entry of activeCategory ? groupedExtensions[activeCategory] : []) {
    const named = entry.named && entry.friendlyName;
    const key = named ? entry.friendlyName! : otherName;
    const list = extGrouped.get(key) ?? [];
    list.push(entry);
    extGrouped.set(key, list);
  }
  const extGroups = [...extGrouped.entries()].sort(
    ([a], [b]) => Number(a === otherName) - Number(b === otherName),
  );
  const isExtGroupOpen = (name: string) => searching || openExtGroups.has(name);
  const toggleExtGroupOpen = (name: string) =>
    setOpenExtGroups((current) => {
      const next = new Set(current);
      if (next.has(name)) next.delete(name);
      else next.add(name);
      return next;
    });

  return (
    <div className="page permissions-page">
      <PageHeader
        eyebrow={t("Permissions", "权限管理")}
        title={t("Startup, context menus and popups", "开机启动、右键菜单与弹窗拦截")}
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

      <Tabs
        label={t("Permission sections", "权限管理分区")}
        value={workspace.tab}
        onChange={workspace.setTab}
        items={tabs.map((tab) => ({
          id: tab.id,
          label: tab.label,
          badge: tab.count,
        }))}
      />

      {workspace.error ? (
        <InlineError message={workspace.error} onDismiss={workspace.dismissError} />
      ) : null}

      {workspace.tab === "startup" ? (
        <Section
          icon={<Power size={16} />}
          title={t("Startup items", "开机启动项")}
          hint={t(
            `${view.startupEnabledCount} of ${view.startupCount} entries start with Windows.`,
            `共 ${view.startupCount} 项，其中 ${view.startupEnabledCount} 项已开启。`,
          )}
        >
          <Toolbar>
            <div className="chip-row" role="group" aria-label={t("Status filter", "状态筛选")}>
              {(
                [
                  ["all", "All", "全部"],
                  ["enabled", "Enabled", "已启用"],
                  ["disabled", "Disabled", "已禁用"],
                  ["leftover", "Leftover", "残留"],
                ] as const
              ).map(([id, en, zh]) => (
                <Chip
                  key={id}
                  active={startupFilter === id}
                  count={counts[id]}
                  tone={id === "leftover" && counts.leftover > 0 ? "danger" : undefined}
                  onClick={() => setStartupFilter(id)}
                >
                  {t(en, zh)}
                </Chip>
              ))}
            </div>
            <div className="toolbar-group">
              <label className="toggle-label">
                <Toggle
                  checked={showSystem}
                  onChange={setShowSystem}
                  label={t("Show system items", "显示系统项")}
                />
                <span>{t("Show system items", "显示系统项")}</span>
              </label>
              <SearchField
                value={query}
                onChange={setQuery}
                label={t("Filter startup items", "筛选开机启动项")}
                placeholder={t("Filter by name or path…", "按名称或路径筛选…")}
              />
            </div>
          </Toolbar>
          {orderedGroups.map(([software, items]) => {
            const onCount = items.filter((item) => item.enabled && !isLeftover(item)).length;
            const allOn = onCount === items.length;
            const open = isGroupOpen(software);
            const groupIds = items.map((item) => item.id);
            const leftovers = items.filter(isLeftover);
            return (
              <div
                key={software}
                className={`perm-group-card ${open ? "is-open" : ""}`}
              >
                <Row
                  className="perm-group-head"
                  leading={
                    <GroupIcon
                      software={software}
                      items={items}
                      neutral={software === otherName}
                    />
                  }
                  title={
                    <>
                      {software}
                      {leftovers.length > 0 ? (
                        <Badge tone="danger">
                          {t(`${leftovers.length} leftover`, `${leftovers.length} 个残留`)}
                        </Badge>
                      ) : null}
                    </>
                  }
                  subtitle={
                    allOn
                      ? t("All on", "全部开启")
                      : onCount === 0
                        ? t("All off", "已全部关闭")
                        : t(
                            `${onCount}/${items.length} on`,
                            `部分开启 ${onCount}/${items.length}`,
                          )
                  }
                  trailing={
                    <span className="row-actions" onClick={(event) => event.stopPropagation()}>
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
                  }
                  onClick={() => toggleGroupOpen(software)}
                />
                {open ? (
                  <div className="row-list perm-rows">
                    {items.map((item) => {
                      const [statusText, statusTone] = statusChip(item, t);
                      const reveal = rowRevealPath(item);
                      return (
                        <Row
                          key={item.id}
                          leading={<RowIcon icon={item.icon} label={item.name} />}
                          title={item.name}
                          tags={
                            <Badge
                              tone={
                                statusTone === "is-leftover"
                                  ? "danger"
                                  : item.enabled
                                    ? "success"
                                    : "neutral"
                              }
                            >
                              {statusText}
                            </Badge>
                          }
                          subtitle={`${item.sourceLabel} · ${scopeLabel(item.scope, t)}`}
                          detail={
                            item.command || t("(no command line)", "（无命令行记录）")
                          }
                          muted={isLeftover(item)}
                          busy={workspace.busyId === item.id}
                          trailing={
                            <>
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
                            </>
                          }
                        />
                      );
                    })}
                    {leftovers.length > 0 && leftovers.length < items.length ? (
                      <Notice
                        tone="danger"
                        icon={<AlertTriangle size={13} />}
                        actions={
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
                        }
                      >
                        {t(
                          `${leftovers.length} leftover entries in this group — their programs are gone.`,
                          `该组有 ${leftovers.length} 个残留项：程序已不存在，只剩启动配置。`,
                        )}
                      </Notice>
                    ) : null}
                  </div>
                ) : null}
              </div>
            );
          })}
          {startup.length === 0 ? (
            <p className="row-empty">
              {counts.all === 0
                ? t("No matches.", "没有匹配的项目。")
                : t("Nothing here under this filter.", "当前筛选下没有项目。")}
            </p>
          ) : null}
        </Section>
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
        <Section
          icon={<LayoutGrid size={16} />}
          title={t("Context menu handlers", "右键菜单扩展")}
          hint={t(
            "Unregistered handlers disappear immediately; Squid keeps a restore recipe for each.",
            "取消注册后右键菜单立即消失；Squid 为每一项保留恢复配方。",
          )}
        >
          <Toolbar>
            <div className="chip-row" role="group" aria-label={t("Menu surfaces", "菜单位置")}>
              {extCategories.map((point) => {
                const entries = groupedExtensions[point];
                const on = entries.filter((e) => e.registered).length;
                return (
                  <Chip
                    key={point}
                    active={activeCategory === point}
                    count={`${on}/${entries.length}`}
                    onClick={() => setExtCat(point)}
                  >
                    {point}
                  </Chip>
                );
              })}
            </div>
            <SearchField
              value={query}
              onChange={setQuery}
              label={t("Filter context menu handlers", "筛选右键菜单扩展")}
              placeholder={t("Filter by name, creator or module…", "按名称、创建者或模块筛选…")}
            />
          </Toolbar>
          {extGroups.map(([software, items]) => {
            const onCount = items.filter((entry) => entry.registered).length;
            const allOn = onCount === items.length;
            const open = isExtGroupOpen(software);
            return (
              <div key={software} className={`perm-group-card ${open ? "is-open" : ""}`}>
                <Row
                  className="perm-group-head"
                  leading={
                    <GroupIcon
                      software={software}
                      items={items}
                      neutral={software === otherName}
                    />
                  }
                  title={software}
                  subtitle={
                    allOn
                      ? t("All on", "全部开启")
                      : onCount === 0
                        ? t("All off", "已全部关闭")
                        : t(
                            `${onCount}/${items.length} on`,
                            `部分开启 ${onCount}/${items.length}`,
                          )
                  }
                  trailing={
                    <span className="row-actions" onClick={(event) => event.stopPropagation()}>
                    <span
                      className="perm-group-count"
                      onClick={(event) => {
                        event.stopPropagation();
                        toggleExtGroupOpen(software);
                      }}
                    >
                      {items.length > 1
                        ? t(
                            `${items.length} places`,
                            `共${items.length}个菜单位置`,
                          )
                        : t("1 place", "1个菜单位置")}
                      <ChevronRight
                        size={13}
                        className={`perm-group-chv ${open ? "is-open" : ""}`}
                      />
                    </span>
                    <Toggle
                      checked={allOn}
                      disabled={workspace.busyId !== null}
                      onChange={async (next) => {
                        for (const entry of items) {
                          if (entry.registered !== next) {
                            await workspace.toggleExtension(entry.id, next);
                          }
                        }
                      }}
                      label={`${software} master ${allOn ? "disable" : "enable"}`}
                    />
                    </span>
                  }
                  onClick={() => toggleExtGroupOpen(software)}
                />
                {open ? (
                  <div className="row-list perm-rows">
                    {items.map((entry) => (
                      <Row
                        key={entry.id}
                        title={entry.friendlyName ?? entry.name}
                        tags={
                          !entry.registered ? (
                            <Badge tone="neutral">{t("removed", "已移除")}</Badge>
                          ) : undefined
                        }
                        subtitle={
                          <>
                            {entry.publisher
                              ? `${t("Creator", "创建者")} ${entry.publisher} · `
                              : ""}
                            {scopeLabel(entry.scope, t)}
                          </>
                        }
                        detail={entry.module ?? entry.handler}
                        muted={!entry.registered}
                        busy={workspace.busyId === entry.id}
                        trailing={
                          <Toggle
                            checked={entry.registered}
                            disabled={workspace.busyId !== null}
                            onChange={(next) => void workspace.toggleExtension(entry.id, next)}
                            label={`${entry.name} ${entry.registered ? "remove" : "restore"}`}
                          />
                        }
                      />
                    ))}
                  </div>
                ) : null}
              </div>
            );
          })}
          {extCategories.length === 0 ? (
            <p className="row-empty">{t("No matches.", "没有匹配的项目。")}</p>
          ) : null}
        </Section>
      ) : null}

      {workspace.tab === "advanced" ? (
        <Section
          className="perm-advanced"
          icon={<ShieldCheck size={16} />}
          title={t("Advanced startup", "进阶启动项")}
        >
          <Notice tone="warning" icon={<AlertTriangle size={13} />}>
            {t(
              "Advanced rows can break boot or logon if changed carelessly — Squid only displays them here, every change records its original value, and nothing is auto-disabled.",
              "进阶区的项目改错了可能影响系统行为——默认只展示不勾选，每一项带数字签名与位置说明；改动会原样记录，可逐项恢复。",
            )}
          </Notice>
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
            <p className="row-empty">
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
                  <div key={kind}>
                    <Subhead
                      title={t(titles[kind][0], titles[kind][1])}
                      hint={t(`${rows.length} item(s)`, `发现 ${rows.length} 项`)}
                    />
                    <div className="row-list">
                      {rows.map((entry) => (
                        <Row
                          key={entry.id}
                          title={entry.name}
                          tags={
                            <Badge
                              tone={
                                entry.signature === "microsoft"
                                  ? "success"
                                  : entry.signature === "vendor"
                                    ? "info"
                                    : entry.signature === "invalid"
                                      ? "danger"
                                      : "warning"
                              }
                              title={entry.signer ?? undefined}
                            >
                              {entry.signature === "microsoft"
                                ? t("Microsoft signed", "微软签名")
                                : entry.signature === "vendor"
                                  ? t("Signed by", "已知厂商")
                                  : entry.signature === "unsigned"
                                    ? t("Not signed", "无数字签名")
                                    : entry.signature === "invalid"
                                      ? t("Signature invalid", "签名无效")
                                      : t("Signature n/a", "无签名信息")}
                            </Badge>
                          }
                          subtitle={shortPath(entry.command, 90)}
                          detail={
                            entry.warning
                              ? `${entry.location} · ${entry.warning}`
                              : entry.location
                          }
                          busy={workspace.busyId === entry.id}
                          trailing={
                            <Toggle
                              checked={entry.enabled}
                              disabled={workspace.busyId !== null}
                              onChange={(next) => void workspace.toggleAdvanced(entry.id, next)}
                              label={entry.name}
                            />
                          }
                        />
                      ))}
                    </div>
                  </div>
                );
              })
          : null}
          {workspace.advanced && workspace.advanced.extras.length > 0 ? (
            <div>
              <Subhead
                title={t("Automatic services", "服务（自动启动）")}
                hint={t(
                  "Services set to start automatically — toggling uses the same rights as Task Manager.",
                  "设置为自动启动的服务；开关与任务管理器同一套权限。",
                )}
              />
              <div className="row-list">
                {workspace.advanced.extras.map((entry) => (
                  <Row
                    key={entry.id}
                    title={entry.displayName || entry.name}
                    tags={<Badge tone="neutral">{scopeLabel(entry.scope, t)}</Badge>}
                    subtitle={shortPath(entry.command, 90)}
                    detail={entry.sourceLabel}
                    busy={workspace.busyId === entry.id}
                    trailing={
                      <Toggle
                        checked={entry.enabled}
                        disabled={workspace.busyId !== null}
                        onChange={(next) => void workspace.toggleStartup(entry.id, next)}
                        label={entry.name}
                      />
                    }
                  />
                ))}
              </div>
            </div>
          ) : null}
        </Section>
      ) : null}

      {workspace.tab === "popup" ? (
        <>
          <Section
            icon={<BellRing size={16} />}
            title={t("Popup blocker", "弹窗拦截")}
            hint={t(
              `Blocked ${workspace.closedTotal} popup windows since install.`,
              `累计拦截 ${workspace.closedTotal} 次弹窗。`,
            )}
            actions={
                <Toggle
                  checked={view.watcherEnabled}
                  onChange={(next) => void workspace.setWatcher(next)}
                  label={t("Popup blocker switch", "弹窗拦截总开关")}
                />
            }
          >
            {workspace.recentClosed.length > 0 ? (
              <Notice icon={<BellRing size={13} />}>
                {workspace.recentClosed[0].process} · {workspace.recentClosed[0].title}
              </Notice>
            ) : null}
          </Section>

          <Section
            icon={<ShieldCheck size={16} />}
            title={t("Rules", "拦截规则")}
            hint={t(
              "Each rule closes windows from one program whose title contains the text.",
              "每条规则会关闭指定程序中标题包含该文字的窗口。",
            )}
          >
            <div className="row-list">
              {view.rules.map((rule, index) => (
                <Row
                  key={`${rule.exe}-${index}`}
                  leading={
                    <RowIcon icon={rule.icon} label={rule.exe.split(/[\\/]/).pop() ?? rule.exe} />
                  }
                  title={rule.title}
                  subtitle={`${shortPath(rule.exe, 64)} · ${t(
                    `closed ${rule.closed}×`,
                    `已关闭 ${rule.closed} 次`,
                  )}`}
                  trailing={
                    <>
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
                    </>
                  }
                />
              ))}
              {view.rules.length === 0 ? (
                <p className="row-empty">
                  {t(
                    "No rules yet — pick a window below.",
                    "还没有规则，先从下面的当前窗口里“拦截”一个。",
                  )}
                </p>
              ) : null}
            </div>
          </Section>

          <Section
            icon={<LayoutGrid size={16} />}
            title={t("Current windows", "当前窗口")}
            hint={t(
              "Refresh, then block a noisy window or close it once.",
              "刷新后可直接拦截某个烦人的窗口，或只关闭这一次。",
            )}
            actions={
                <Button
                  variant="ghost"
                  size="sm"
                  icon={<RefreshCw size={14} />}
                  onClick={() => void workspace.refreshWindows()}
                >
                  {t("Refresh", "刷新")}
                </Button>
            }
          >
            <div className="row-list">
              {workspace.windows.map((window) => (
                <Row
                  key={window.hwnd}
                  leading={<RowIcon icon={window.icon} label={window.process} />}
                  title={window.title}
                  subtitle={window.process}
                  detail={window.exe ?? undefined}
                  trailing={
                    <>
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
                    </>
                  }
                />
              ))}
            </div>
          </Section>
        </>
      ) : null}
    </div>
  );
}
