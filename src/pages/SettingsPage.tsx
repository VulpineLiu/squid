import {
  FileCog,
  FolderClock,
  FolderOpen,
  Info,
  Languages,
  LockKeyhole,
  Plus,
  Power,
  RefreshCw,
  ShieldCheck,
  SlidersHorizontal,
  Sparkles,
  Trash2,
  X,
} from "lucide-react";
import { useCallback, useEffect, useState } from "react";
import { open } from "@tauri-apps/plugin-dialog";
import brandMarkUrl from "../../src-tauri/icons/brand-mark.svg";
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
  Badge,
  Button,
  ChoiceCard,
  IconButton,
  InlineError,
  PageHeader,
  Row,
  Select,
  Section,
  Tabs,
  Toggle,
  useToast,
} from "../components/ui";

export type SettingsTab = "general" | "cleanup" | "security" | "updates" | "about";

const SETTINGS_TABS: {
  id: SettingsTab;
  labelEn: string;
  labelZh: string;
  icon: typeof SlidersHorizontal;
}[] = [
  { id: "general", labelEn: "General", labelZh: "常规", icon: SlidersHorizontal },
  { id: "cleanup", labelEn: "Cleanup", labelZh: "清理", icon: Trash2 },
  { id: "security", labelEn: "Security", labelZh: "安全", icon: ShieldCheck },
  { id: "updates", labelEn: "Updates", labelZh: "更新", icon: Sparkles },
  { id: "about", labelEn: "About", labelZh: "关于", icon: Info },
];

export function SettingsPage({
  bootstrap,
  settings,
  flow,
  autoCheck,
  security,
  initialTab = "general",
  onSettingsChange,
  onLanguageChange,
  onAutoCheckChange,
}: {
  bootstrap: Bootstrap;
  settings: SettingsView;
  flow: UpdateFlow;
  autoCheck: boolean;
  security: SecurityWorkspace;
  initialTab?: SettingsTab;
  onSettingsChange: (settings: SettingsView) => void;
  onLanguageChange: (language: Language) => Promise<void>;
  onAutoCheckChange: (enabled: boolean) => void;
}) {
  const { t, lang, bin } = useI18n();
  const { push } = useToast();
  const [tab, setTab] = useState<SettingsTab>(initialTab);
  const [whitelistInput, setWhitelistInput] = useState("");
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [permission, setPermission] = useState<PermissionStatus | null>(null);
  const [checkingPermission, setCheckingPermission] = useState(false);
  const [autostart, setAutostart] = useState<boolean | null>(null);
  const [savingAutostart, setSavingAutostart] = useState(false);
  // Whitelist entries are only as good as their last hit: the audit answers
  // "did this pattern ever protect something real?".
  const [audit, setAudit] = useState<Map<string, boolean> | null>(null);
  const [auditing, setAuditing] = useState(false);

  // The sidebar/topbar update prompt opens Settings on its Updates section;
  // a later plain Settings click asks for General and lands there.
  useEffect(() => {
    setTab(initialTab);
  }, [initialTab]);

  const runAudit = useCallback(async () => {
    setAuditing(true);
    try {
      const rows = await api.whitelistAudit();
      setAudit(new Map(rows.map((row) => [row.pattern, row.live])));
    } catch {
      setAudit(null);
    } finally {
      setAuditing(false);
    }
  }, []);

  useEffect(() => {
    if (settings.whitelist.length > 0 && audit === null && !auditing) {
      void runAudit();
    }
  }, [audit, auditing, runAudit, settings.whitelist.length]);

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
        title={t("Make Squid yours", "按您的方式使用 Squid")}
      />

      {error ? <InlineError message={error} onDismiss={() => setError(null)} /> : null}

      <Tabs
        label={t("Settings sections", "设置分区")}
        value={tab}
        onChange={setTab}
        items={SETTINGS_TABS.map((item) => {
          const Icon = item.icon;
          return {
            id: item.id,
            label: t(item.labelEn, item.labelZh),
            icon: <Icon size={15} />,
          };
        })}
      />

      {tab === "general" ? (
        <div className="settings-grid">
          <Section
            icon={<Languages size={16} />}
            title={t("Language & appearance", "语言与外观")}
          >
            <div className="choice-group" role="radiogroup">
              <ChoiceCard
                selected={lang === "zh"}
                onClick={() => void onLanguageChange("zh")}
                icon={<Languages size={18} />}
                title="简体中文"
                description="Simplified Chinese"
              />
              <ChoiceCard
                selected={lang === "en"}
                onClick={() => void onLanguageChange("en")}
                icon={<Languages size={18} />}
                title="English"
                description="English (US)"
              />
            </div>
          </Section>

          <Section icon={<Power size={16} />} title={t("Startup", "启动")}>
            <Row
              className="is-first"
              title={t("Launch Squid at login", "登录时启动 Squid")}
              subtitle={t(
                "Only Squid itself; other apps are managed under Startup items.",
                "仅 Squid 自身，其他程序请到启动项页面管理。",
              )}
              trailing={
              <Toggle
                checked={autostart ?? false}
                onChange={(enabled) => void setLaunchAtLogin(enabled)}
                disabled={autostart === null || savingAutostart}
                label={t("Launch Squid at login", "登录时启动 Squid")}
              />
              }
            />
          </Section>
        </div>
      ) : null}

      {tab === "cleanup" ? (
        <div className="settings-grid">
          <Section icon={<Trash2 size={16} />} title={t("Deletion safety", "删除安全")}>
            <div className="choice-group" role="radiogroup">
              <ChoiceCard
                selected={settings.deleteMode === "trash"}
                onClick={() => void setMode("trash")}
                disabled={saving}
                icon={<FolderOpen size={20} />}
                title={t(`Move to ${bin}`, `移入${bin}`)}
                description={t(
                  "Safer. Restore files later from History.",
                  "更安全，之后可从历史记录还原。",
                )}
              />
              <ChoiceCard
                selected={settings.deleteMode === "permanent"}
                onClick={() => void setMode("permanent")}
                disabled={saving}
                icon={<Trash2 size={20} />}
                title={t("Remove permanently", "永久删除")}
                description={t(
                  "No recovery for caches and rebuildable files.",
                  "缓存与可重建文件无法恢复。",
                )}
              />
            </div>
            <Row
              title={t("Report allocated size", "显示磁盘实际占用")}
              subtitle={t(
                "Prefer on-disk bytes for sparse and compressed files.",
                "对稀疏和压缩文件优先显示磁盘实际占用。",
              )}
              trailing={
              <button
                className={`toggle ${settings.showAllocated ? "is-on" : ""}`}
                aria-label={t("Report allocated size", "显示磁盘实际占用")}
                onClick={() => void save({ showAllocated: !settings.showAllocated })}
              >
                <span />
              </button>
              }
            />
            <Row
              title={t("History records to keep", "保留历史记录数量")}
              trailing={
              <Select
                value={settings.historyKeep}
                label={t("History records to keep", "保留历史记录数量")}
                options={[20, 60, 100, 200, 500].map((value) => ({
                  value,
                  label: String(value),
                }))}
                onChange={(next) => void save({ historyKeep: next })}
              />
              }
            />
          </Section>

          <Section
            icon={<ShieldCheck size={16} />}
            title={t("Protected locations", "受保护的位置")}
            hint={t(
              "Never deleted here, even when a cleanup rule matches. The malware scan still reports inside them.",
              "这里的内容绝不会被删除。恶意软件扫描仍会检查其中的发现。",
            )}
          >
            <div className="whitelist-add">
              <input
                value={whitelistInput}
                onChange={(event) => setWhitelistInput(event.target.value)}
                placeholder={t("Folder path to protect", "要保护的文件夹路径")}
                onKeyDown={(event) => {
                  if (event.key === "Enter") void addWhitelist();
                }}
              />
              <Button
                variant="secondary"
                icon={<FolderOpen size={15} />}
                onClick={() => void chooseWhitelist()}
              >
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
            {settings.whitelist.length > 0 ? (
              <div className="whitelist-audit-bar">
                <span>
                  {audit === null
                    ? t(
                        "Checking whether each pattern still matches anything…",
                        "正在核对每条规则是否还命中过内容…",
                      )
                    : t(
                        `Consumption audit: ${[...audit.values()].filter(Boolean).length} of ${audit.size} pattern(s) matched known cache locations`,
                        `消费审计：${audit.size} 条规则中 ${[...audit.values()].filter(Boolean).length} 条命中过已知缓存位置`,
                      )}
                </span>
                <Button
                  variant="ghost"
                  size="sm"
                  icon={<RefreshCw size={13} className={auditing ? "spin" : undefined} />}
                  onClick={() => void runAudit()}
                  disabled={auditing}
                >
                  {t("Re-check", "重新检查")}
                </Button>
              </div>
            ) : null}
            <div className="row-list">
              {settings.whitelist.length === 0 ? (
                <p className="settings-empty">
                  {t("No protected folders yet.", "尚未设置受保护的文件夹。")}
                </p>
              ) : (
                settings.whitelist.map((pattern) => (
                  <Row
                    key={pattern}
                    leading={
                      <span className="whitelist-icon">
                        <ShieldCheck size={15} />
                      </span>
                    }
                    title={<code title={pattern}>{shortPath(pattern, 88)}</code>}
                    tags={
                      audit ? (
                        <Badge
                          tone={audit.get(pattern) ? "success" : "neutral"}
                          title={t(
                            audit.get(pattern)
                              ? "This pattern matched paths Squid knows about on the last scan"
                              : "This pattern has never matched anything — it may be stale",
                            audit.get(pattern)
                              ? "最近扫描命中过这条规则保护的位置"
                              : "从未命中任何已知位置，规则可能已经过期",
                          )}
                        >
                          {audit.get(pattern)
                            ? t("Recently matched", "最近命中")
                            : t("Never matched", "从未命中")}
                        </Badge>
                      ) : undefined
                    }
                    trailing={
                      <IconButton
                        label={t("Remove protection", "移除保护")}
                        onClick={() => void removeWhitelist(pattern)}
                        disabled={saving}
                      >
                        <X size={14} />
                      </IconButton>
                    }
                  />
                ))
              )}
            </div>
          </Section>
        </div>
      ) : null}

      {tab === "security" ? (
        <>
          {bootstrap.platform === "macos" ? (
            <Section
              icon={<LockKeyhole size={16} />}
              title={t("System permissions", "系统权限")}
            >
              <Row
                className="is-first"
                leading={
                <span
                  className={`permission-check-icon is-${
                    permission?.fullDiskAccess === "granted" ? "granted" : "optional"
                  }`}
                >
                  <LockKeyhole size={18} />
                </span>
                }
                title={t("Full Disk Access", "完全磁盘访问权限")}
                tags={
                  <Badge
                    tone={
                      permission?.fullDiskAccess === "granted"
                        ? "success"
                        : permission?.fullDiskAccess === "denied"
                          ? "warning"
                          : "neutral"
                    }
                  >
                    {checkingPermission
                      ? t("Checking…", "检查中…")
                      : permission?.fullDiskAccess === "granted"
                        ? t("Granted", "已授权")
                        : permission?.fullDiskAccess === "denied"
                          ? t("Needs attention", "需要授权")
                          : t("Not confirmed", "未确认")}
                  </Badge>
                }
                subtitle={t(
                  "Enables scans of TCC-protected caches, mail data and app leftovers.",
                  "用于扫描受 TCC 保护的缓存、邮件数据与应用卸载残留。",
                )}
                trailing={
                  <div className="row-actions">
                  <Button
                    variant="ghost"
                    size="sm"
                    icon={
                      <RefreshCw
                        size={13}
                        className={checkingPermission ? "spin" : ""}
                      />
                    }
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
                }
              />
            </Section>
          ) : null}

          {security.status ? (
            <SecuritySettingsCard security={security} status={security.status} />
          ) : (
            <Section title={t("Security settings", "安全设置")}>
              <p className="settings-empty">
                {t("Security settings are loading…", "安全设置加载中…")}
              </p>
            </Section>
          )}
        </>
      ) : null}

      {tab === "updates" ? (
        <UpdatePanel
          flow={flow}
          autoCheck={autoCheck}
          onAutoCheckChange={onAutoCheckChange}
          bootstrap={bootstrap}
        />
      ) : null}

      {tab === "about" ? (
        <Section
          className="about-section"
          icon={<Info size={16} />}
          title={t("About Squid", "关于 Squid")}
          hint={t(
            "Configuration and history stay local to this computer.",
            "配置与历史记录仅保存在这台电脑上。",
          )}
        >
          <div className="about-brand">
            <img className="about-mark" src={brandMarkUrl} alt="" aria-hidden="true" />
            <div className="about-brand-copy">
              <strong>Squid</strong>
              <span>{t(`Version ${bootstrap.version}`, `版本 ${bootstrap.version}`)}</span>
            </div>
            <span
              className="about-platform"
              title={`${bootstrap.osVersion || bootstrap.osName} · ${bootstrap.arch} · ${bootstrap.hardware}`}
            >
              {bootstrap.osVersion || bootstrap.osName} · {bootstrap.arch}
            </span>
          </div>
          <div className="row-list">
            <Row
              leading={
                <span className="metric-icon tone-primary">
                  <FileCog size={17} />
                </span>
              }
              title={t("Configuration", "配置文件")}
              subtitle={<code title={bootstrap.configPath}>{bootstrap.configPath}</code>}
              trailing={
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
              }
            />
            <Row
              leading={
                <span className="metric-icon tone-neutral">
                  <FolderClock size={17} />
                </span>
              }
              title={t("History", "历史记录")}
              subtitle={<code title={bootstrap.historyPath}>{bootstrap.historyPath}</code>}
              trailing={
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
              }
            />
          </div>
        </Section>
      ) : null}
    </div>
  );
}
