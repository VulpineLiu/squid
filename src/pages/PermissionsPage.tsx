import { BellRing, RefreshCw, Search, ShieldCheck, Trash2, X } from "lucide-react";
import { useMemo, useState } from "react";
import { useI18n } from "../i18n";
import { shortPath } from "../lib/format";
import type { PermissionsWorkspace } from "../hooks/usePermissionsWorkspace";
import type { StartupItem } from "../types";
import {
  Button,
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
  const [extCat, setExtCat] = useState<string | null>(null);
  const view = workspace.view;

  const tabs = useMemo(
    () =>
      [
        {
          id: "startup" as const,
          label: t("Startup items", "开机启动项"),
          count: view?.startupCount ?? 0,
        },
        {
          id: "extensions" as const,
          label: t("Context menus", "右键菜单"),
          count: view?.extensions.length ?? 0,
        },
        {
          id: "popup" as const,
          label: t("Popup blocker", "弹窗拦截"),
          count: view?.rules.length ?? 0,
        },
      ],
    [t, view],
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
  const startup = view.startup.filter(
    (item) =>
      !needle ||
      item.name.toLowerCase().includes(needle) ||
      item.displayName.toLowerCase().includes(needle) ||
      item.command.toLowerCase().includes(needle),
  );
  // 电脑管家 style: one group per software, Chinese name in the header.
  const startupGroups = new Map<string, typeof startup>();
  for (const item of startup) {
    const list = startupGroups.get(item.displayName) ?? [];
    list.push(item);
    startupGroups.set(item.displayName, list);
  }
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
          <label className="search-field">
            <Search size={14} />
            <input
              value={query}
              onChange={(event) => setQuery(event.target.value)}
              placeholder={t("Filter by name or path…", "按名称或路径筛选…")}
            />
          </label>
          {Array.from(startupGroups.entries()).map(([software, items]) => (
            <div key={software}>
              <h4 className="perm-group">
                {items.find((item) => item.icon)?.icon ? (
                  <img className="perm-group-icon" src={softwareIcon(items)} alt="" />
                ) : (
                  <span className="perm-group-icon perm-group-icon-fallback" aria-hidden="true">
                    {software.trim().slice(0, 1).toUpperCase()}
                  </span>
                )}
                <span>{software}</span>
                <b>
                  {t(
                    `${items.filter((item) => item.enabled).length}/${items.length} on`,
                    `${items.filter((item) => item.enabled).length}/${items.length} 开启`,
                  )}
                </b>
              </h4>
              <div className="perm-rows">
                {items.map((item) => (
                  <div
                    className={`settings-row ${workspace.busyId === item.id ? "is-busy" : ""}`}
                    key={item.id}
                  >
                    <div className="perm-row perm-row-noicon">
                      <div>
                        <span>{item.name}</span>
                        <small title={item.command}>
                          {item.sourceLabel} · {scopeLabel(item.scope, t)}
                          {item.command ? ` · ${shortPath(item.command, 72)}` : ""}
                        </small>
                      </div>
                    </div>
                    <Toggle
                      checked={item.enabled}
                      disabled={workspace.busyId !== null}
                      onChange={(next) => void workspace.toggleStartup(item.id, next)}
                      label={`${item.name} ${item.enabled ? "disable" : "enable"}`}
                    />
                  </div>
                ))}
              </div>
            </div>
          ))}
          {startup.length === 0 ? (
            <p className="perm-empty">{t("No matches.", "没有匹配的项目。")}</p>
          ) : null}
        </section>
      ) : null}

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
