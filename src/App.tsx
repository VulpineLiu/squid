import {
  AppWindow,
  ChevronDown,
  CircleGauge,
  Eraser,
  History,
  LayoutDashboard,
  MonitorSmartphone,
  RefreshCw,
  Settings,
  ShieldCheck,
  Sparkles,
  Wrench,
} from "lucide-react";
import { useCallback, useEffect, useState, type ReactNode } from "react";
import { useAppsWorkspace } from "./hooks/useAppsWorkspace";
import { usePermissionsWorkspace } from "./hooks/usePermissionsWorkspace";
import { useScanWorkspace } from "./hooks/useScanWorkspace";
import { useSecurityWorkspace } from "./hooks/useSecurityWorkspace";
import { useSystemSnapshot } from "./hooks/useSystemSnapshot";
import { useUpdateFlow } from "./hooks/useUpdateFlow";
import { detectLanguage, I18nProvider, useI18n } from "./i18n";
import { api } from "./lib/api";
import { messageFrom } from "./lib/format";
import type {
  Bootstrap,
  DeleteMode,
  Language,
  PageId,
  SecurityStatus,
  SettingsView,
} from "./types";
import { useToast } from "./components/ui";
import { SetupPanel } from "./components/SetupPanel";
import brandMarkUrl from "../src-tauri/icons/brand-mark.svg";
import { ApplicationsPage } from "./pages/ApplicationsPage";
import { HistoryPage } from "./pages/HistoryPage";
import { MaintenanceHubPage, type MaintTab } from "./pages/MaintenanceHubPage";
import { OverviewPage } from "./pages/OverviewPage";
import { PermissionsPage } from "./pages/PermissionsPage";
import { SecurityPage } from "./pages/SecurityPage";
import { SettingsPage, type SettingsTab } from "./pages/SettingsPage";
import { SpaceHubPage, type SpaceTab } from "./pages/SpaceHubPage";
import { SystemPage } from "./pages/SystemPage";
import { WidgetPage } from "./pages/WidgetPage";

const NAV_GROUPS: {
  labelEn: string;
  labelZh: string;
  items: {
    id: PageId;
    labelEn: string;
    labelZh: string;
    icon: typeof LayoutDashboard;
    windowsOnly?: boolean;
  }[];
}[] = [
  {
    labelEn: "Maintenance",
    labelZh: "维护",
    items: [
      {
        id: "overview",
        labelEn: "Computer overview",
        labelZh: "电脑概览",
        icon: LayoutDashboard,
      },
      {
        id: "space",
        labelEn: "Space cleaning",
        labelZh: "空间清理",
        icon: Eraser,
      },
      {
        id: "applications",
        labelEn: "Uninstall apps",
        labelZh: "应用卸载",
        icon: AppWindow,
      },
      {
        id: "maintenance",
        labelEn: "System maintenance",
        labelZh: "系统维护",
        icon: Wrench,
        windowsOnly: true,
      },
    ],
  },
  {
    labelEn: "Protection",
    labelZh: "安全",
    items: [
      {
        id: "permissions",
        labelEn: "Permissions",
        labelZh: "权限管理",
        icon: ShieldCheck,
      },
      {
        id: "security",
        labelEn: "Security scan",
        labelZh: "安全扫描",
        icon: ShieldCheck,
      },
    ],
  },
  {
    labelEn: "Insights",
    labelZh: "洞察",
    items: [
      {
        id: "system",
        labelEn: "Real-time monitor",
        labelZh: "实时监控",
        icon: CircleGauge,
      },
    ],
  },
  {
    labelEn: "Desktop",
    labelZh: "桌面",
    items: [
      {
        id: "widget",
        labelEn: "Desktop widget",
        labelZh: "桌面小工具",
        icon: MonitorSmartphone,
      },
    ],
  },
];

const PAGE_TITLES: Record<PageId, [string, string]> = {
  overview: ["Computer overview", "电脑概览"],
  space: ["Space cleaning", "空间清理"],
  applications: ["Uninstall apps", "应用卸载"],
  permissions: ["Permissions", "权限管理"],
  security: ["Security scan", "安全扫描"],
  widget: ["Desktop widget", "桌面小工具"],
  maintenance: ["System maintenance", "系统维护"],
  system: ["Real-time monitor", "实时监控"],
  history: ["History", "历史记录"],
  settings: ["Settings", "设置"],
};

/** Pages that no longer exist still arrive from old tray/notification payloads. */
const LEGACY_PAGES: Record<string, PageId> = {
  cleanup: "space",
  purge: "space",
  storage: "space",
  startup: "permissions",
};

function BrandMark({ small = false }: { small?: boolean }) {
  return (
    <img
      className={`brand-mark ${small ? "is-small" : ""}`}
      src={brandMarkUrl}
      alt=""
      aria-hidden="true"
    />
  );
}

function LoadingScreen() {
  return (
    <div className="app-loading">
      <div className="loading-orb">
        <BrandMark />
      </div>
      <strong>Squid</strong>
      <span>Preparing your computer care workspace…</span>
      <div className="loading-track">
        <i />
      </div>
    </div>
  );
}

function BootstrapError({ message }: { message: string }) {
  return (
    <div className="app-loading is-error">
      <div className="loading-orb">
        <BrandMark />
      </div>
      <strong>Squid could not start</strong>
      <span>{message}</span>
      <button onClick={() => window.location.reload()}>
        <RefreshCw size={14} />
        Try again
      </button>
    </div>
  );
}

const SPACE_OPTIONAL_CATEGORIES = [
  "update-installers",
  "project-artifacts",
  "developer",
];

/** 空间清理 defaults: daily groups pre-checked, opt-in groups untouched.
 * AI is the exception that proves the rule — its caches come pre-checked
 * from the backend, its models never do, so the item's own `selected`
 * flag speaks there. */
function spaceDefaults(item: { category: string; selected: boolean }): boolean {
  if (item.category === "ai-tools") return item.selected;
  return !SPACE_OPTIONAL_CATEGORIES.includes(item.category);
}

export default function App() {
  const { push } = useToast();
  const [bootstrap, setBootstrap] = useState<Bootstrap | null>(null);
  const [bootError, setBootError] = useState<string | null>(null);
  const [page, setPage] = useState<PageId>("overview");
  const [lang, setLang] = useState<Language>(detectLanguage);
  const [settings, setSettings] = useState<SettingsView | null>(null);
  const [autoCheck, setAutoCheck] = useState(true);
  const [setupOpen, setSetupOpen] = useState(false);
  // The 托盘驻留 case: a hidden main window must stop sampling the machine.
  const [windowVisible, setWindowVisible] = useState(true);
  // Hub deep-links: the overview quick-cards and tray can request a specific
  // tab; the selection resets on every navigate so a later click re-arms it.
  const [spaceTab, setSpaceTab] = useState<SpaceTab>("junk");
  const [maintTab, setMaintTab] = useState<MaintTab>("repair");
  const [settingsTab, setSettingsTab] = useState<SettingsTab>("general");
  const [autostart, setAutostart] = useState(false);
  // A process row can ask "where did this come from"; the monitor opens the
  // process tab already filtered to that path.
  const [processFilter] = useState<string | null>(null);
  const cleanup = useScanWorkspace("cleanup", spaceDefaults);
  const purge = useScanWorkspace("purge", spaceDefaults);
  const apps = useAppsWorkspace();
  const permissions = usePermissionsWorkspace(page === "permissions");
  const security = useSecurityWorkspace(true);
  const systemActive = (page === "overview" || page === "system") && windowVisible;
  // Only the 实时监控 top-process table needs the expensive per-pid walk;
  // the overview tiles ride the light refresh.
  const system = useSystemSnapshot(systemActive, 1800, page === "system");
  const update = useUpdateFlow(bootstrap?.updateFeedUrl ?? "", autoCheck);

  useEffect(() => {
    let cancelled = false;
    void api
      .bootstrap()
      .then((next) => {
        if (cancelled) return;
        setBootstrap(next);
        setSettings(next.settings);
        setLang(next.language);
        setAutoCheck(next.updateCheckEnabled);
        setSetupOpen(!next.settings.setupReviewed);
        void api
          .autostartStatus()
          .then(setAutostart)
          .catch(() => setAutostart(false));
        document.documentElement.lang = next.language === "zh" ? "zh-CN" : "en";
      })
      .catch((reason) => {
        if (!cancelled) setBootError(messageFrom(reason));
      });
    return () => {
      cancelled = true;
    };
  }, []);

  useEffect(() => {
    let cancelled = false;
    let unlisten: (() => void) | null = null;
    void api
      .onWindowVisible((next) => {
        if (!cancelled) setWindowVisible(next);
      })
      .then((stop) => {
        if (cancelled) stop();
        else unlisten = stop;
      });
    return () => {
      cancelled = true;
      unlisten?.();
    };
  }, []);

  /**
   * A notification click goes to the findings, and brings the window back.
   *
   * Findings are the only thing Squid notifies about, so the tap needs no
   * destination in its payload. The backend is asked as well because closing
   * the window only hides it: the webview is gone, so nothing on this side can
   * undo the hide, and a click that appeared to do nothing would look like a
   * bug in the security feature rather than in window management.
   */
  useEffect(() => {
    let cancelled = false;
    let stop: { unregister: () => void } | null = null;
    void import("@tauri-apps/plugin-notification")
      .then(({ onAction }) =>
        onAction(() => {
          if (cancelled) return;
          void api.securityOpen().catch(() => undefined);
          void security.refresh();
        }),
      )
      .then((listener) => {
        if (cancelled) void listener.unregister();
        else stop = listener;
      })
      .catch(() => undefined);
    return () => {
      cancelled = true;
      void stop?.unregister();
    };
  }, [security]);

  useEffect(() => {
    let cancelled = false;
    let unlisten: (() => void) | null = null;
    void api
      .onNavigate((next) => {
        if (cancelled) return;
        const target = LEGACY_PAGES[next] ?? next;
        setPage(target);
        if (target === "space") setSpaceTab("junk");
        if (target === "maintenance") setMaintTab("repair");
        if (target === "security") void security.refresh();
      })
      .then((stop) => {
        if (cancelled) stop();
        else unlisten = stop;
      });
    return () => {
      cancelled = true;
      unlisten?.();
    };
  }, [security]);

  useEffect(() => {
    const order: PageId[] = ([
      "overview",
      "space",
      "applications",
      "permissions",
      "security",
      "maintenance",
      "system",
      "widget",
      "history",
    ] as PageId[]).filter(
      (id) => id !== "maintenance" || bootstrap?.platform === "windows",
    );
    // Ctrl/Cmd+1..9 follow the sidebar; 0 opens settings.
    const onKey = (event: KeyboardEvent) => {
      if (!(event.metaKey || event.ctrlKey) || event.altKey || event.shiftKey) return;
      if (event.key === "0") {
        event.preventDefault();
        setSettingsTab("general");
        setPage("settings");
        return;
      }
      const index = Number(event.key) - 1;
      if (!Number.isInteger(index) || index < 0 || index >= order.length) return;
      event.preventDefault();
      setPage(order[index]);
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [bootstrap?.platform]);

  const changeLanguage = useCallback(
    async (nextLanguage: Language) => {
      try {
        const next = await api.settingsSave({ language: nextLanguage });
        setSettings(next);
        setLang(next.effectiveLanguage);
        document.documentElement.lang =
          next.effectiveLanguage === "zh" ? "zh-CN" : "en";
      } catch (reason) {
        push(messageFrom(reason), "danger");
      }
    },
    [push],
  );

  const changeAutoCheck = useCallback(
    (enabled: boolean) => {
      setAutoCheck(enabled);
      void api.setUpdateCheckEnabled(enabled).catch((reason) => {
        setAutoCheck(!enabled);
        push(messageFrom(reason), "danger");
      });
    },
    [push],
  );

  const finishSetup = useCallback(async () => {
    setSetupOpen(false);
    try {
      const next = await api.settingsSave({ setupReviewed: true });
      setSettings(next);
    } catch (reason) {
      push(messageFrom(reason), "danger");
    }
  }, [push]);

  const openSettings = useCallback((tab: SettingsTab = "general") => {
    setSettingsTab(tab);
    setPage("settings");
  }, []);

  if (bootError) return <BootstrapError message={bootError} />;
  if (!bootstrap || !settings) return <LoadingScreen />;

  const deleteMode: DeleteMode = settings.deleteMode;
  const activeUpdate =
    update.status?.state === "available" || update.status?.state === "ready"
      ? update.status
      : null;

  return (
    <I18nProvider lang={lang} setLang={setLang} binNoun={bootstrap.binNoun}>
      <AppChrome
        bootstrap={bootstrap}
        page={page}
        setPage={setPage}
        onOpenSettings={openSettings}
        updateVersion={activeUpdate?.availableVersion ?? null}
        security={security.status}
      >
        {page === "overview" ? (
          <OverviewPage
            bootstrap={bootstrap}
            snapshot={system.snapshot}
            cleanup={cleanup}
            purge={purge}
            onNavigate={setPage}
            onOpenSpaceTab={(tab) => {
              setSpaceTab(tab);
              setPage("space");
            }}
          />
        ) : null}
        {page === "space" ? (
          <SpaceHubPage
            cleanup={cleanup}
            purge={purge}
            bootstrap={bootstrap}
            deleteMode={deleteMode}
            onHistory={() => setPage("history")}
            initialTab={spaceTab}
          />
        ) : null}
        {page === "applications" ? (
          <ApplicationsPage workspace={apps} onHistory={() => setPage("history")} />
        ) : null}
        {page === "permissions" ? (
          <PermissionsPage workspace={permissions} platform={bootstrap.platform} />
        ) : null}
        {page === "security" ? (
          <SecurityPage security={security} onOpenStartup={() => setPage("permissions")} />
        ) : null}
        {page === "widget" ? <WidgetPage /> : null}
        {page === "maintenance" ? (
          <MaintenanceHubPage
            initialTab={maintTab}
            platform={bootstrap.platform}
          />
        ) : null}
        {page === "system" ? (
          <SystemPage
            snapshot={system.snapshot}
            error={system.error}
            loading={system.loading}
            refresh={system.refresh}
            initialProcessFilter={processFilter}
            security={security}
            onOpenStartup={() => setPage("permissions")}
          />
        ) : null}
        {page === "history" ? <HistoryPage /> : null}
        {page === "settings" ? (
          <SettingsPage
            bootstrap={bootstrap}
            settings={settings}
            flow={update}
            autoCheck={autoCheck}
            security={security}
            initialTab={settingsTab}
            onSettingsChange={setSettings}
            onLanguageChange={changeLanguage}
            onAutoCheckChange={changeAutoCheck}
          />
        ) : null}
      </AppChrome>
      {setupOpen ? (
        <SetupPanel
          security={security}
          autostart={autostart}
          onDone={() => void finishSetup()}
        />
      ) : null}
    </I18nProvider>
  );
}

/// A number means "confirmed and unhandled", which is the only state that
/// justifies interrupting. A dot alone means there is something suspicious, worth
/// a look but not worth a badge that looks like an alarm.
function securityBadge(security: SecurityStatus, page: PageId) {
  if (page === "security") return null;
  if (security.confirmed > 0) {
    return <i className="nav-badge">{security.confirmed > 9 ? "9+" : security.confirmed}</i>;
  }
  if (security.suspicious > 0) return <i className="nav-dot" />;
  return null;
}

function AppChrome({
  bootstrap,
  page,
  setPage,
  onOpenSettings,
  updateVersion,
  security,
  children,
}: {
  bootstrap: Bootstrap;
  page: PageId;
  setPage: (page: PageId) => void;
  onOpenSettings: (tab?: SettingsTab) => void;
  updateVersion: string | null;
  security: SecurityStatus | null;
  children: ReactNode;
}) {
  const { t, lang } = useI18n();
  const title = PAGE_TITLES[page][lang === "zh" ? 1 : 0];

  return (
    <div className="app" data-platform={bootstrap.platform} data-page={page}>
      <aside className="sidebar">
        <div className="sidebar-drag" data-tauri-drag-region />
        <div className="sidebar-brand">
          <BrandMark />
          <div>
            <strong>Squid</strong>
            <span>{t("Give your computer a bath", "给你的电脑洗个澡")}</span>
          </div>
        </div>

        <nav className="sidebar-nav" aria-label={t("Main navigation", "主导航")}>
          {NAV_GROUPS.map((group) => (
            <div className="nav-group" key={group.labelEn}>
              <span className="nav-group-label">
                {lang === "zh" ? group.labelZh : group.labelEn}
              </span>
              {group.items
                .filter(
                  (item) => !item.windowsOnly || bootstrap.platform === "windows",
                )
                .map((item) => {
                const Icon = item.icon;
                return (
                  <button
                    key={item.id}
                    className={`nav-item ${page === item.id ? "is-active" : ""}`}
                    onClick={() => setPage(item.id)}
                    aria-current={page === item.id ? "page" : undefined}
                  >
                    <Icon size={18} />
                    <span>{lang === "zh" ? item.labelZh : item.labelEn}</span>
                    {item.id === "security" && security
                      ? securityBadge(security, page)
                      : null}
                    {item.id === "space" && page !== "space" ? (
                      <i className="nav-dot" />
                    ) : null}
                  </button>
                );
                })}
            </div>
          ))}
        </nav>

        <div className="sidebar-bottom">
          {updateVersion ? (
            <button
              className="sidebar-update"
              onClick={() => onOpenSettings("updates")}
            >
              <span>
                <Sparkles size={15} />
              </span>
              <div>
                <strong>{t("Update available", "有可用更新")}</strong>
                <small>Squid {updateVersion}</small>
              </div>
              <ChevronDown size={14} />
            </button>
          ) : null}
          <button
            className={`nav-item ${page === "history" ? "is-active" : ""}`}
            onClick={() => setPage("history")}
          >
            <History size={18} />
            <span>{t("History", "历史记录")}</span>
          </button>
          <button
            className={`nav-item ${page === "settings" ? "is-active" : ""}`}
            onClick={() => onOpenSettings("general")}
          >
            <Settings size={18} />
            <span>{t("Settings", "设置")}</span>
            {updateVersion ? <i className="nav-dot" /> : null}
          </button>
          <div className="sidebar-meta">
            <span>Squid {bootstrap.version}</span>
            <span>{bootstrap.platform === "windows" ? "Windows" : bootstrap.osName}</span>
          </div>
        </div>
      </aside>

      <main className="main">
        <header className="topbar" data-tauri-drag-region>
          <div className="topbar-title" data-tauri-drag-region>
            <span>{title}</span>
            <i />
            <small>{bootstrap.hardware}</small>
          </div>
          <div className="topbar-actions">
            <span className="privacy-pill">
              <ShieldCheck size={13} />
              {t("Local & private", "本地与私密")}
            </span>
            {updateVersion ? (
              <button
                className="topbar-update"
                onClick={() => onOpenSettings("updates")}
              >
                <Sparkles size={13} />
                {t("Update", "更新")} {updateVersion}
              </button>
            ) : null}
          </div>
        </header>
        <div className="page-scroll" key={page}>
          {children}
        </div>
      </main>
    </div>
  );
}
