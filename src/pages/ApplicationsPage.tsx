import {
  AppWindow,
  ChevronRight,
  FolderOpen,
  PackageX,
  Play,
  Search,
  ShieldCheck,
  Trash2,
} from "lucide-react";
import { useMemo, useState } from "react";
import { useI18n } from "../i18n";
import { formatBytes, formatCount, shortPath } from "../lib/format";
import type { AppInfo, DeleteMode } from "../types";
import type { AppsWorkspace } from "../hooks/useAppsWorkspace";
import {
  Button,
  Checkbox,
  ConfirmDialog,
  EmptyState,
  InlineError,
  PageHeader,
  ProgressCard,
  ResultSummary,
  categoryLabel,
  lockLabel,
  useToast,
} from "../components/ui";

function AppRow({
  app,
  selected,
  onSelect,
}: {
  app: AppInfo;
  selected: boolean;
  onSelect: () => void;
}) {
  const { t } = useI18n();
  const letter = app.name.trim().slice(0, 1).toUpperCase() || "?";
  return (
    <button className={`app-row ${selected ? "is-selected" : ""}`} onClick={onSelect}>
      <span className="app-glyph" aria-hidden="true">
        {app.icon ? <img src={app.icon} alt="" /> : letter}
      </span>
      <span className="app-row-copy">
        <span className="app-row-title">
          <strong>{app.name}</strong>
          {app.running ? (
            <span className="status-chip is-live">
              <Play size={10} fill="currentColor" />
              {t("Running", "运行中")}
            </span>
          ) : null}
        </span>
        <span>{app.bundle_id || shortPath(app.path, 45)}</span>
      </span>
      <span className="app-row-meta">
        <strong>{formatBytes(app.size)}</strong>
        <small>{app.version ? `v${app.version}` : app.source}</small>
      </span>
      <ChevronRight size={16} />
    </button>
  );
}
export function ApplicationsPage({
  workspace,
  onHistory,
}: {
  workspace: AppsWorkspace;
  onHistory: () => void;
}) {
  const { t, lang, bin } = useI18n();
  const { push } = useToast();
  const [query, setQuery] = useState("");
  const [confirming, setConfirming] = useState(false);
  const [mode, setMode] = useState<DeleteMode>("trash");

  const apps = useMemo(() => {
    const needle = query.trim().toLowerCase();
    if (!needle) return workspace.apps;
    return workspace.apps.filter(
      (app) =>
        app.name.toLowerCase().includes(needle) ||
        app.bundle_id?.toLowerCase().includes(needle) ||
        app.publisher?.toLowerCase().includes(needle),
    );
  }, [query, workspace.apps]);

  // The scan carries icons; the plan's AppInfo does not, so look it up.
  const planIcon = useMemo(() => {
    const planApp = workspace.plan?.app;
    if (!planApp) return null;
    const key = planApp.bundle_id || planApp.path;
    return workspace.apps.find((a) => (a.bundle_id || a.path) === key)?.icon ?? null;
  }, [workspace.plan, workspace.apps]);

  const uninstall = async () => {
    const result = await workspace.uninstall(mode);
    setConfirming(false);
    if (result) {
      push(
        t(
          `${result.removed} items removed`,
          `已移除 ${result.removed} 个项目`,
        ),
        result.failed > 0 ? "danger" : "success",
      );
    }
  };

  return (
    <div className="page apps-page">
      <PageHeader
        eyebrow={t("Applications", "应用程序")}
        title={t("Uninstall completely.", "卸载，干净彻底。")}
        description={t(
          "Remove an app together with the caches, preferences and support files it leaves behind.",
          "卸载应用，并一并处理它留下的缓存、偏好设置与支持文件。",
        )}
        actions={
          <Button
            variant={workspace.scanned ? "secondary" : "primary"}
            icon={<Search size={16} />}
            onClick={() => void workspace.scan()}
            disabled={workspace.scanning || workspace.deleting}
          >
            {workspace.scanned ? t("Scan again", "重新扫描") : t("Find applications", "查找应用")}
          </Button>
        }
      />

      {workspace.error ? (
        <InlineError message={workspace.error} onDismiss={workspace.clearError} />
      ) : null}

      {workspace.scanning ? (
        <ProgressCard
          progress={workspace.progress}
          title={t("Reading installed applications…", "正在读取已安装应用…")}
          onCancel={() => void workspace.cancel()}
        />
      ) : null}

      {workspace.summary && !workspace.scanning ? (
        <ResultSummary
          removed={workspace.summary.removed}
          freed={workspace.summary.freedAllocated || workspace.summary.freed}
          failed={workspace.summary.failed}
          mode={workspace.summary.mode}
          onAgain={() => void workspace.scan()}
          onHistory={onHistory}
        />
      ) : null}

      {workspace.scanned && !workspace.scanning && !workspace.summary ? (
        <div className="apps-layout">
          <section className="apps-list-panel">
            <div className="apps-list-head">
              <div>
                <h2>{t("Installed applications", "已安装应用")}</h2>
                <p>
                  {t(
                    `${formatCount(workspace.apps.length)} apps · ${formatBytes(
                      workspace.total,
                    )}`,
                    `${formatCount(workspace.apps.length)} 个应用 · ${formatBytes(
                      workspace.total,
                    )}`,
                  )}
                </p>
              </div>
              <label className="search-field">
                <Search size={15} />
                <input
                  value={query}
                  onChange={(event) => setQuery(event.target.value)}
                  placeholder={t("Search apps", "搜索应用")}
                />
              </label>
            </div>
            <div className="app-list">
              {apps.map((app) => (
                <AppRow
                  key={app.bundle_id || app.path}
                  app={app}
                  selected={
                    (workspace.selectedApp?.bundle_id || workspace.selectedApp?.path) ===
                    (app.bundle_id || app.path)
                  }
                  onSelect={() => void workspace.buildPlan(app)}
                />
              ))}
              {apps.length === 0 ? (
                <div className="empty-inline">
                  <Search size={20} />
                  <span>{t("No application matches that search.", "没有匹配的应用。")}</span>
                </div>
              ) : null}
            </div>
          </section>

          <aside className="app-plan-panel">
            {workspace.planning ? (
              <ProgressCard
                progress={workspace.progress}
                title={t("Tracing app leftovers…", "正在查找应用残留…")}
                onCancel={() => void workspace.cancel()}
                compact
              />
            ) : workspace.plan ? (
              <>
                <div className="plan-head">
                  <span className="app-glyph is-large">
                    {planIcon ? <img src={planIcon} alt="" /> : workspace.plan.app.name.slice(0, 1).toUpperCase()}
                  </span>
                  <div>
                    <span className="plan-kicker">{t("Removal plan", "卸载方案")}</span>
                    <h2>{workspace.plan.app.name}</h2>
                    <p>
                      {workspace.plan.app.version
                        ? `v${workspace.plan.app.version} · `
                        : ""}
                      {formatBytes(workspace.plan.total)}
                    </p>
                  </div>
                  <button
                    className="plan-close"
                    aria-label={t("Close plan", "关闭方案")}
                    onClick={() => workspace.chooseApp(null)}
                  >
                    <PackageX size={17} />
                  </button>
                </div>

                {workspace.plan.warnings.length > 0 ? (
                  <div className="plan-warnings">
                    {workspace.plan.warnings.map((warning) => (
                      <p key={warning}>{warning}</p>
                    ))}
                  </div>
                ) : null}

                <div className="plan-summary">
                  <div>
                    <span>{t("Selected", "已选择")}</span>
                    <strong>{formatBytes(workspace.planSelectedTotal)}</strong>
                  </div>
                  <div>
                    <span>{t("Items", "项目")}</span>
                    <strong>{workspace.planSelected.size}</strong>
                  </div>
                  <Button variant="ghost" size="sm" onClick={workspace.togglePlanAll}>
                    {workspace.planSelected.size ===
                    workspace.plan.items.filter((item) => !item.lock).length
                      ? t("Clear", "清除")
                      : t("Select all", "全选")}
                  </Button>
                </div>

                <div className="plan-items">
                  {workspace.plan.items.map((item, index) => (
                    <div
                      key={`${item.path}-${index}`}
                      className={`plan-item ${item.lock ? "is-locked" : ""}`}
                      onClick={() => {
                        if (!item.lock) workspace.togglePlanItem(index);
                      }}
                    >
                      <Checkbox
                        checked={workspace.planSelected.has(index)}
                        disabled={Boolean(item.lock)}
                        onChange={() => workspace.togglePlanItem(index)}
                        label={item.label}
                      />
                      <span className="plan-item-copy">
                        <strong>{item.label}</strong>
                        <span title={item.displayPath}>{shortPath(item.displayPath, 44)}</span>
                        <small>
                          {categoryLabel(item.category, lang, bin)}
                          {item.note ? ` · ${item.note}` : ""}
                          {item.lock ? ` · ${lockLabel(item.lock, t)}` : ""}
                        </small>
                      </span>
                      <b>{formatBytes(item.onDisk)}</b>
                    </div>
                  ))}
                </div>

                <div className="plan-actions">
                  <Button
                    variant="ghost"
                    icon={<FolderOpen size={15} />}
                    onClick={() => void workspace.buildPlan(workspace.plan!.app)}
                  >
                    {t("Refresh", "刷新")}
                  </Button>
                  <Button
                    variant="danger"
                    icon={<Trash2 size={16} />}
                    disabled={workspace.planSelected.size === 0}
                    onClick={() => setConfirming(true)}
                  >
                    {t("Uninstall", "卸载")}
                  </Button>
                </div>
              </>
            ) : (
              <div className="plan-placeholder">
                <span className="plan-placeholder-icon">
                  <ShieldCheck size={26} />
                </span>
                <h2>{t("Choose an application", "选择一个应用")}</h2>
                <p>
                  {t(
                    "Squid will map its caches, settings and support files before offering to remove anything.",
                    "Squid 会先整理应用的缓存、设置与支持文件，再让您决定是否移除。",
                  )}
                </p>
              </div>
            )}
          </aside>
        </div>
      ) : null}

      {!workspace.scanned && !workspace.scanning ? (
        <EmptyState
          icon={<AppWindow size={29} />}
          title={t("See what is installed", "查看已安装的应用")}
          description={t(
            "Scan your applications to remove an app completely, including files that would otherwise stay behind.",
            "扫描应用后可连同原本会遗留的文件一起移除。",
          )}
          action={
            <Button
              variant="primary"
              icon={<Search size={16} />}
              onClick={() => void workspace.scan()}
            >
              {t("Scan applications", "扫描应用程序")}
            </Button>
          }
        />
      ) : null}

      <ConfirmDialog
        open={confirming}
        title={t(`Uninstall ${workspace.plan?.app.name ?? ""}?`, `卸载 ${workspace.plan?.app.name ?? ""}？`)}
        message={t(
          `${workspace.planSelected.size} items · ${formatBytes(workspace.planSelectedTotal)}`,
          `${workspace.planSelected.size} 项 · ${formatBytes(workspace.planSelectedTotal)}`,
        )}
        note={
          mode === "trash"
            ? t(
                `The app and selected leftovers will move to the ${bin}.`,
                `应用与所选残留将移入${bin}。`,
              )
            : t(
                "This cannot be undone. Make sure the application is closed.",
                "此操作无法撤销，请确认应用已关闭。",
              )
        }
        confirmLabel={t("Uninstall app", "卸载应用")}
        cancelLabel={t("Keep app", "保留应用")}
        onClose={() => setConfirming(false)}
        onConfirm={() => void uninstall()}
        busy={workspace.deleting}
        icon={<Trash2 size={23} />}
      >
        <div className="mode-picker">
          <button
            className={mode === "trash" ? "is-active" : ""}
            onClick={() => setMode("trash")}
          >
            <FolderOpen size={17} />
            <span>{bin}</span>
          </button>
          <button
            className={mode === "permanent" ? "is-active" : ""}
            onClick={() => setMode("permanent")}
          >
            <Trash2 size={17} />
            <span>{t("Permanent", "永久")}</span>
          </button>
        </div>
      </ConfirmDialog>

      {workspace.deleting ? (
        <div className="operation-shade">
          <ProgressCard
            progress={workspace.progress}
            title={t("Removing the application…", "正在移除应用…")}
            compact
          />
        </div>
      ) : null}
    </div>
  );
}
