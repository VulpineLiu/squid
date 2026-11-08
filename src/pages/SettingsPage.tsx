import {
  Check,
  FileCog,
  FolderClock,
  FolderOpen,
  HardDrive,
  Languages,
  LockKeyhole,
  Plus,
  Power,
  RefreshCw,
  ShieldCheck,
  Trash2,
  X,
} from "lucide-react";
import { useCallback, useEffect, useState } from "react";
import { open } from "@tauri-apps/plugin-dialog";
import { useI18n } from "../i18n";
import { api } from "../lib/api";
import { messageFrom, shortPath } from "../lib/format";
import type {
  Bootstrap,
  DeleteMode,
  Language,
  PermissionStatus,
  SettingsView,
} from "../types";
import type { UpdateFlow } from "../hooks/useUpdateFlow";
import type { SecurityWorkspace } from "../hooks/useSecurityWorkspace";
import { SecuritySettingsCard } from "../components/SecuritySettingsCard";
import { UpdatePanel } from "../components/UpdatePanel";
import {
  Button,
  InlineError,
  PageHeader,
  Select,
  Toggle,
  useToast,
} from "../components/ui";

export function SettingsPage({
  bootstrap,
  settings,
  flow,
  autoCheck,
  security,
  onSettingsChange,
  onLanguageChange,
  onAutoCheckChange,
}: {
  bootstrap: Bootstrap;
  settings: SettingsView;
  flow: UpdateFlow;
  autoCheck: boolean;
  security: SecurityWorkspace;
  onSettingsChange: (settings: SettingsView) => void;
  onLanguageChange: (language: Language) => Promise<void>;
  onAutoCheckChange: (enabled: boolean) => void;
}) {
  const { t, lang, bin } = useI18n();
  const { push } = useToast();
  const [whitelistInput, setWhitelistInput] = useState("");
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [permission, setPermission] = useState<PermissionStatus | null>(null);
  const [checkingPermission, setCheckingPermission] = useState(false);
  const [autostart, setAutostart] = useState<boolean | null>(null);
  const [savingAutostart, setSavingAutostart] = useState(false);

  const checkPermissions = useCallback(async () => {
    setCheckingPermission(true);
    try {
      setPermission(await api.permissionStatus());
    } catch (reason) {
      setError(messageFrom(reason));
    } finally {
      setCheckingPermission(false);
    }
  }, []);

  useEffect(() => {
    if (bootstrap.platform !== "macos") return;
    void checkPermissions();
    const onFocus = () => void checkPermissions();
    window.addEventListener("focus", onFocus);
    return () => window.removeEventListener("focus", onFocus);
  }, [bootstrap.platform, checkPermissions]);

  useEffect(() => {
    let cancelled = false;
    void api
      .autostartStatus()
      .then((enabled) => {
        if (!cancelled) setAutostart(enabled);
      })
      .catch((reason) => {
        if (!cancelled) setError(messageFrom(reason));
      });
    return () => {
      cancelled = true;
    };
  }, []);

  const save = async (patch: Parameters<typeof api.settingsSave>[0]) => {
    setSaving(true);
    setError(null);
    try {
      const next = await api.settingsSave(patch);
      onSettingsChange(next);
    } catch (reason) {
      setError(messageFrom(reason));
    } finally {
      setSaving(false);
    }
  };

  const addWhitelist = async (path = whitelistInput.trim()) => {
    if (!path) return;
    setSaving(true);
    setError(null);
    try {
      const next = await api.whitelistAdd(path);
      onSettingsChange(next);
      setWhitelistInput("");
      push(t("Protection added", "已添加保护"), "success");
    } catch (reason) {
      setError(messageFrom(reason));
    } finally {
      setSaving(false);
    }
  };

  const chooseWhitelist = async () => {
    const chosen = await open({
      directory: true,
      multiple: false,
      title: t("Choose a folder to protect", "选择要保护的文件夹"),
    });
    if (typeof chosen === "string") await addWhitelist(chosen);
  };

  const removeWhitelist = async (pattern: string) => {
    setSaving(true);
    try {
      const next = await api.whitelistRemove(pattern);
      onSettingsChange(next);
    } catch (reason) {
      setError(messageFrom(reason));
    } finally {
      setSaving(false);
    }
  };

  const setMode = async (mode: DeleteMode) => {
    await save({ deleteMode: mode });
  };

  const setLaunchAtLogin = async (enabled: boolean) => {
    setSavingAutostart(true);
    setError(null);
    try {
      setAutostart(await api.setAutostart(enabled));
      push(
        enabled
          ? t("Squid will launch when you sign in", "Squid 将在登录时自动启动")
          : t("Launch at login disabled", "已关闭开机自启"),
        "success",
      );
    } catch (reason) {
      setError(messageFrom(reason));
    } finally {
      setSavingAutostart(false);
    }
  };

  return (
    <div className="page settings-page">
      <PageHeader
        title={t("Make Squid yours.", "按您的方式使用 Squid。")}
        description={t(
          "Language, deletion behaviour, protected locations and updates all live here.",
          "语言、删除方式、保护路径与更新设置都在这里。",
        )}
      />

      {error ? <InlineError message={error} onDismiss={() => setError(null)} /> : null}

      <section className="settings-section">
        <div className="settings-section-head">
          <div>
            <h2>
              <Languages size={18} />
              {t("Language & appearance", "语言与外观")}
            </h2>
          </div>
        </div>
        <div className="language-picker">
          <button
            className={lang === "zh" ? "is-active" : ""}
            onClick={() => void onLanguageChange("zh")}
          >
            <strong>简体中文</strong>
            <span>Simplified Chinese</span>
            {lang === "zh" ? <Check size={16} /> : null}
          </button>
          <button
            className={lang === "en" ? "is-active" : ""}
            onClick={() => void onLanguageChange("en")}
          >
            <strong>English</strong>
            <span>English (US)</span>
            {lang === "en" ? <Check size={16} /> : null}
          </button>
        </div>
      </section>

      {bootstrap.platform === "macos" ? (
        <section className="settings-section">
          <div className="settings-section-head">
            <div>
              <h2>
                <LockKeyhole size={18} />
                {t("System permissions", "系统权限")}
              </h2>
            </div>
          </div>
          <div className="permission-check-row">
            <span
              className={`permission-check-icon is-${
                permission?.fullDiskAccess === "granted" ? "granted" : "optional"
              }`}
            >
              <LockKeyhole size={18} />
            </span>
            <div className="permission-check-copy">
              <strong>
                {t("Full Disk Access", "完全磁盘访问权限")}
                <span
                  className={`permission-state is-${
                    checkingPermission
                      ? "checking"
                      : permission?.fullDiskAccess === "granted"
                        ? "granted"
                        : permission?.fullDiskAccess === "denied"
                          ? "denied"
                          : "unknown"
                  }`}
                >
                  {checkingPermission
                    ? t("Checking…", "检查中…")
                    : permission?.fullDiskAccess === "granted"
                      ? t("Granted", "已授权")
                      : permission?.fullDiskAccess === "denied"
                        ? t("Needs attention", "需要授权")
                        : t("Not confirmed", "未确认")}
                </span>
              </strong>
              <span>
                {t(
                  "Enables scans of TCC-protected caches, mail data and app leftovers.",
                  "用于扫描受 TCC 保护的缓存、邮件数据与应用卸载残留。",
                )}
              </span>
            </div>
            <div className="permission-check-actions">
              <Button
                variant="ghost"
                size="sm"
                icon={<RefreshCw size={13} className={checkingPermission ? "spin" : ""} />}
                onClick={() => void checkPermissions()}
                disabled={checkingPermission}
              >
                {t("Recheck", "重新检查")}
              </Button>
              <Button
                variant="secondary"
                size="sm"
                onClick={() => void api.openFullDiskAccessSettings()}
              >
                {t("Open Settings", "打开系统设置")}
              </Button>
            </div>
          </div>
        </section>
      ) : null}

      <section className="settings-section">
        <div className="settings-section-head">
          <div>
            <h2>
              <Power size={18} />
              {t("Startup", "启动")}
            </h2>
          </div>
        </div>
        <div className="settings-row is-first">
          <div>
            <strong>{t("Launch Squid at login", "登录时启动 Squid")}</strong>
            <span>
              {t(
                "Only Squid itself; other apps are managed under Startup items.",
                "仅 Squid 自身，其他程序请到启动项页面管理。",
              )}
            </span>
          </div>
          <Toggle
            checked={autostart ?? false}
            onChange={(enabled) => void setLaunchAtLogin(enabled)}
            disabled={autostart === null || savingAutostart}
            label={t("Launch Squid at login", "登录时启动 Squid")}
          />
        </div>
      </section>

      {security.status ? (
        <SecuritySettingsCard security={security} status={security.status} />
      ) : null}

      <section className="settings-section">
        <div className="settings-section-head">
          <div>
            <h2>
              <Trash2 size={18} />
              {t("Deletion safety", "删除安全")}
            </h2>
          </div>
        </div>
        <div className="delete-mode-cards">
          <button
            className={settings.deleteMode === "trash" ? "is-active" : ""}
            onClick={() => void setMode("trash")}
            disabled={saving}
          >
            <span className="delete-mode-icon">
              <FolderOpen size={20} />
            </span>
            <span>
              <strong>{t(`Move to ${bin}`, `移入${bin}`)}</strong>
              <small>
                {t(
                  "Safer. Restore files later from History.",
                  "更安全，之后可从历史记录还原。",
                )}
              </small>
            </span>
            <span className="radio-dot" />
          </button>
          <button
            className={settings.deleteMode === "permanent" ? "is-active" : ""}
            onClick={() => void setMode("permanent")}
            disabled={saving}
          >
            <span className="delete-mode-icon">
              <Trash2 size={20} />
            </span>
            <span>
              <strong>{t("Remove permanently", "永久删除")}</strong>
              <small>
                {t(
                  "No recovery for caches and rebuildable files.",
                  "缓存与可重建文件无法恢复。",
                )}
              </small>
            </span>
            <span className="radio-dot" />
          </button>
        </div>
        <div className="settings-row">
          <div>
            <strong>{t("Report allocated size", "显示磁盘实际占用")}</strong>
            <span>
              {t(
                "Prefer on-disk bytes for sparse and compressed files.",
                "对稀疏和压缩文件优先显示磁盘实际占用。",
              )}
            </span>
          </div>
          <button
            className={`toggle ${settings.showAllocated ? "is-on" : ""}`}
            aria-label={t("Report allocated size", "显示磁盘实际占用")}
            onClick={() => void save({ showAllocated: !settings.showAllocated })}
          >
            <span />
          </button>
        </div>
        <div className="settings-row">
          <div>
            <strong>{t("History records to keep", "保留历史记录数量")}</strong>
          </div>
          <Select
            value={settings.historyKeep}
            label={t("History records to keep", "保留历史记录数量")}
            options={[20, 60, 100, 200, 500].map((value) => ({ value, label: String(value) }))}
            onChange={(next) => void save({ historyKeep: next })}
          />
        </div>
      </section>

      <section className="settings-section">
        <div className="settings-section-head">
          <div>
            <h2>
              <ShieldCheck size={18} />
              {t("Protected locations", "受保护的位置")}
            </h2>
            <p>
              {t(
                "Never deleted here, even when a cleanup rule matches. The malware scan still reports inside them.",
                "这里的内容绝不会被删除。恶意软件扫描仍会检查其中的发现。",
              )}
            </p>
          </div>
        </div>
        <div className="whitelist-add">
          <input
            value={whitelistInput}
            onChange={(event) => setWhitelistInput(event.target.value)}
            placeholder={t("Folder path to protect", "要保护的文件夹路径")}
            onKeyDown={(event) => {
              if (event.key === "Enter") void addWhitelist();
            }}
          />
          <Button variant="secondary" icon={<FolderOpen size={15} />} onClick={() => void chooseWhitelist()}>
            {t("Choose", "选择")}
          </Button>
          <Button
            variant="primary"
            icon={<Plus size={15} />}
            onClick={() => void addWhitelist()}
            disabled={!whitelistInput.trim() || saving}
          >
            {t("Add", "添加")}
          </Button>
        </div>
        <div className="whitelist-list">
          {settings.whitelist.length === 0 ? (
            <p className="settings-empty">
              {t("No protected folders yet.", "尚未设置受保护的文件夹。")}
            </p>
          ) : (
            settings.whitelist.map((pattern) => (
              <div className="whitelist-row" key={pattern}>
                <span className="whitelist-icon">
                  <ShieldCheck size={15} />
                </span>
                <code title={pattern}>{shortPath(pattern, 88)}</code>
                <button
                  className="row-reveal"
                  aria-label={t("Remove protection", "移除保护")}
                  onClick={() => void removeWhitelist(pattern)}
                  disabled={saving}
                >
                  <X size={14} />
                </button>
              </div>
            ))
          )}
        </div>
      </section>

      <UpdatePanel
        flow={flow}
        autoCheck={autoCheck}
        onAutoCheckChange={onAutoCheckChange}
        bootstrap={bootstrap}
      />

      <section className="settings-section">
        <div className="settings-section-head">
          <div>
            <h2>
              <FileCog size={18} />
              {t("Files & information", "文件与信息")}
            </h2>
            <p>
              {t(
                "Configuration and history stay local to this computer.",
                "配置与历史记录仅保存在这台电脑上。",
              )}
            </p>
          </div>
        </div>
        <div className="path-settings-list">
          <div className="path-setting-row">
            <span className="metric-icon tone-primary">
              <FileCog size={17} />
            </span>
            <div>
              <strong>{t("Configuration", "配置文件")}</strong>
              <code title={bootstrap.configPath}>{bootstrap.configPath}</code>
            </div>
            <Button
              variant="ghost"
              size="sm"
              onClick={() =>
                void api
                  .revealPath(bootstrap.configPath)
                  .catch((reason) => push(messageFrom(reason), "danger"))
              }
            >
              {t("Show", "显示")}
            </Button>
          </div>
          <div className="path-setting-row">
            <span className="metric-icon tone-neutral">
              <FolderClock size={17} />
            </span>
            <div>
              <strong>{t("History", "历史记录")}</strong>
              <code title={bootstrap.historyPath}>{bootstrap.historyPath}</code>
            </div>
            <Button
              variant="ghost"
              size="sm"
              onClick={() =>
                void api
                  .revealPath(bootstrap.historyPath)
                  .catch((reason) => push(messageFrom(reason), "danger"))
              }
            >
              {t("Show", "显示")}
            </Button>
          </div>
          <div className="path-setting-row">
            <span className="metric-icon tone-neutral">
              <HardDrive size={17} />
            </span>
            <div>
              <strong>{t("Platform", "平台")}</strong>
              <code>
                {bootstrap.osName} {bootstrap.osVersion} · {bootstrap.arch} ·{" "}
                {bootstrap.hardware}
              </code>
            </div>
            <span className="settings-version">v{bootstrap.version}</span>
          </div>
        </div>
      </section>
    </div>
  );
}
