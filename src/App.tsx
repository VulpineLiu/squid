import {
  AppWindow,
  Boxes,
  ChevronDown,
  CircleGauge,
  Eraser,
  History,
  LayoutDashboard,
  RefreshCw,
  Settings,
  ShieldCheck,
  Sparkles,
  WandSparkles,
} from "lucide-react";
import { useCallback, useEffect, useState, type ReactNode } from "react";
import { useAppsWorkspace } from "./hooks/useAppsWorkspace";
import { useScanWorkspace } from "./hooks/useScanWorkspace";
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
  SettingsView,
} from "./types";
import { useToast } from "./components/ui";
import { SetupPanel } from "./components/SetupPanel";
import appIconUrl from "../src-tauri/icons/128x128.png";
import { ApplicationsPage } from "./pages/ApplicationsPage";
import { CleanupPage } from "./pages/CleanupPage";
import { HistoryPage } from "./pages/HistoryPage";
import { OverviewPage } from "./pages/OverviewPage";
import { PurgePage } from "./pages/PurgePage";
import { SettingsPage } from "./pages/SettingsPage";
import { StoragePage } from "./pages/StoragePage";
import { SystemPage } from "./pages/SystemPage";

const NAV_GROUPS: {
  labelEn: string;
  labelZh: string;
  items: {
    id: PageId;
    labelEn: string;
    labelZh: string;
    icon: typeof LayoutDashboard;
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
        id: "cleanup",
        labelEn: "Daily cleanup",
        labelZh: "日常清理",
        icon: Eraser,
      },
      {
        id: "applications",
        labelEn: "Uninstall apps",
        labelZh: "应用卸载",
        icon: AppWindow,
      },
      {
        id: "purge",
        labelEn: "Deep Clean",
        labelZh: "深度清理",
        icon: WandSparkles,
      },
    ],
  },
  {
    labelEn: "Insights",
    labelZh: "洞察",
    items: [
      {
        id: "storage",
        labelEn: "Storage",
        labelZh: "存储空间",
        icon: Boxes,
      },
      {
        id: "system",
        labelEn: "Real-time monitor",
        labelZh: "实时监控",
        icon: CircleGauge,
      },
    ],
  },
];

const PAGE_TITLES: Record<PageId, [string, string]> = {
  overview: ["Computer overview", "电脑概览"],
  cleanup: ["Daily cleanup", "日常清理"],
  applications: ["Uninstall apps", "应用卸载"],
  purge: ["Deep Clean", "深度清理"],
  storage: ["Storage", "存储空间"],
  system: ["Real-time monitor", "实时监控"],
  history: ["History", "历史记录"],
  settings: ["Settings", "设置"],
};

function BrandMark({ small = false }: { small?: boolean }) {
  return (
    <img
      className={`brand-mark ${small ? "is-small" : ""}`}
      src={appIconUrl}
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

export default function App() {
  const { push } = useToast();
  const [bootstrap, setBootstrap] = useState<Bootstrap | null>(null);
  const [bootError, setBootError] = useState<string | null>(null);
  const [page, setPage] = useState<PageId>("overview");
  const [lang, setLang] = useState<Language>(detectLanguage);
  const [settings, setSettings] = useState<SettingsView | null>(null);
  const [autoCheck, setAutoCheck] = useState(true);
  const [setupOpen, setSetupOpen] = useState(false);
  const cleanup = useScanWorkspace("cleanup");
  const purge = useScanWorkspace("purge");
  const apps = useAppsWorkspace();
  const systemActive = page === "overview" || page === "system";
  const system = useSystemSnapshot(systemActive);
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
      .onNavigate((next) => {
        if (!cancelled) setPage(next);
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

  useEffect(() => {
    const order: PageId[] = [
      "overview",
      "cleanup",
      "applications",
      "purge",
      "storage",
      "system",
      "history",
      "settings",
    ];
    const onKey = (event: KeyboardEvent) => {
      if (!(event.metaKey || event.ctrlKey) || event.altKey || event.shiftKey) return;
      const index = Number(event.key) - 1;
      if (!Number.isInteger(index) || index < 0 || index >= order.length) return;
      event.preventDefault();
      setPage(order[index]);
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, []);

  const changeLanguage = useCallback(
    async (nextLanguage: Language) => {
      try {
        const next = await api.settingsSave({ language: nextLanguage });
        setSettings(next);
        setLang(next.effectiveLanguage);
        document.documentElement.lang = next.effectiveLanguage === "zh" ? "zh-CN" : "en";
      } catch (reason) {
        push(messageFrom(reason), "danger");
      }
    },
    [push],
  );

  const changeAutoCheck = useCallback(
    (enabled: boolean) => {
      setAutoCheck(enabled);
      void api
        .setUpdateCheckEnabled(enabled)
        .catch((reason) => {
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

  if (bootError) return <BootstrapError message={bootError} />;
  if (!bootstrap || !settings) return <LoadingScreen />;

  const deleteMode: DeleteMode = settings.deleteMode;
  const activeUpdate =
    update.status?.state === "available" || update.status?.state === "ready"
      ? update.status
      : null;

  return (
    <I18nProvider lang={lang} setLang={setLang}>
      <AppChrome
        bootstrap={bootstrap}
        page={page}
        setPage={setPage}
        updateVersion={activeUpdate?.availableVersion ?? null}
      >
        {page === "overview" ? (
          <OverviewPage
            bootstrap={bootstrap}
            snapshot={system.snapshot}
            cleanup={cleanup}
            onNavigate={setPage}
          />
        ) : null}
        {page === "cleanup" ? (
          <CleanupPage
            workspace={cleanup}
            deleteMode={deleteMode}
            onHistory={() => setPage("history")}
          />
        ) : null}
        {page === "applications" ? (
          <ApplicationsPage workspace={apps} onHistory={() => setPage("history")} />
        ) : null}
        {page === "purge" ? (
          <PurgePage
            workspace={purge}
            bootstrap={bootstrap}
            deleteMode={deleteMode}
            onHistory={() => setPage("history")}
          />
        ) : null}
        {page === "storage" ? <StoragePage bootstrap={bootstrap} /> : null}
        {page === "system" ? (
          <SystemPage
            snapshot={system.snapshot}
            error={system.error}
            loading={system.loading}
            refresh={system.refresh}
          />
        ) : null}
        {page === "history" ? <HistoryPage /> : null}
        {page === "settings" ? (
          <SettingsPage
            bootstrap={bootstrap}
            settings={settings}
            flow={update}
            autoCheck={autoCheck}
            onSettingsChange={setSettings}
            onLanguageChange={changeLanguage}
            onAutoCheckChange={changeAutoCheck}
          />
        ) : null}
      </AppChrome>
      {setupOpen ? <SetupPanel onDone={() => void finishSetup()} /> : null}
    </I18nProvider>
  );
}

function AppChrome({
  bootstrap,
  page,
  setPage,
  updateVersion,
  children,
}: {
  bootstrap: Bootstrap;
  page: PageId;
  setPage: (page: PageId) => void;
  updateVersion: string | null;
  children: ReactNode;
}) {
  const { t, lang } = useI18n();
  const title = PAGE_TITLES[page][lang === "zh" ? 1 : 0];

  return (
    <div
      className="app"
      data-platform={bootstrap.platform}
      data-page={page}
    >
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
              {group.items.map((item) => {
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
                    {item.id === "cleanup" && page !== "cleanup" ? (
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
              onClick={() => {
                setPage("settings");
              }}
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
            onClick={() => setPage("settings")}
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
              <button className="topbar-update" onClick={() => setPage("settings")}>
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
