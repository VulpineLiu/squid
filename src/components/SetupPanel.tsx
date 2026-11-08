import {
  BellRing,
  Check,
  Download,
  LoaderCircle,
  LockKeyhole,
  Power,
  RefreshCw,
  ShieldCheck,
  X,
} from "lucide-react";
import { useCallback, useEffect, useState } from "react";
import { useI18n } from "../i18n";
import { api } from "../lib/api";
import { messageFrom } from "../lib/format";
import type { PermissionStatus, SecurityStatus } from "../types";
import type { SecurityWorkspace } from "../hooks/useSecurityWorkspace";
import { Button } from "./ui";

type State = "checking" | "granted" | "denied" | "unknown" | "not-required" | "error";

const SYSTEM_SETTINGS_COPY = {
  en: "System Settings › Privacy & Security › Full Disk Access",
  zh: "系统设置 › 隐私与安全性 › 完全磁盘访问权限",
};

/**
 * The security-readiness card, shown once before the first scan.
 *
 * A list rather than one gate, because these five things have different weights:
 * Full Disk Access only widens coverage, notifications are a convenience, and the
 * system's own protection is something squid can neither change nor claim credit
 * for. Each row says whether it is optional; closing the card changes nothing, and
 * no permission is requested here that the user did not ask for.
 *
 * The copy matters more than the layout. An unsigned build is trusted by the user
 * releasing it in Gatekeeper, not by the system vouching for a publisher, so
 * nothing here is allowed to read as "the system has certified this".
 */
export function SetupPanel({
  security,
  autostart,
  onDone,
}: {
  security: SecurityWorkspace;
  autostart: boolean;
  onDone: () => void;
}) {
  const { t, lang } = useI18n();
  const [state, setState] = useState<State>("checking");
  const [status, setStatus] = useState<SecurityStatus | null>(security.status);
  const [note, setNote] = useState<string | null>(null);
  const [launching, setLaunching] = useState(false);

  const refresh = useCallback(async () => {
    setState("checking");
    try {
      const permissions: PermissionStatus = await api.permissionStatus();
      setState(permissions.fullDiskAccess);
    } catch {
      setState("error");
    }
    try {
      setStatus(await api.securityStatus());
      setNote(null);
    } catch (reason) {
      setNote(messageFrom(reason));
    }
  }, []);

  useEffect(() => {
    void refresh();
    const onFocus = () => void refresh();
    const onKey = (event: KeyboardEvent) => {
      if (event.key === "Escape") onDone();
    };
    window.addEventListener("focus", onFocus);
    window.addEventListener("keydown", onKey);
    return () => {
      window.removeEventListener("focus", onFocus);
      window.removeEventListener("keydown", onKey);
    };
  }, [onDone, refresh]);

  const checking = state === "checking";
  const ready = state === "granted" || state === "not-required";
  const needsAttention = state === "denied" || state === "unknown" || state === "error";
  const stateLabel = checking
    ? t("Checking…", "检查中…")
    : ready
      ? t("Ready", "已就绪")
      : state === "denied"
        ? t("Needs you", "需要你处理")
        : t("Not confirmed", "未确认");
  const detail = checking
    ? t("Reading this Mac's privacy state…", "正在读取这台 Mac 的隐私权限状态…")
    : state === "granted"
      ? t(
          "Squid can include protected app caches and leftovers in its scans.",
          "Squid 可以扫描受保护的 App 缓存与卸载残留。",
        )
      : state === "not-required"
        ? t(
            "Windows needs no extra privacy permission for Squid.",
            "Windows 上使用 Squid 不需要额外隐私权限。",
          )
        : state === "denied"
          ? t(
              "Optional. Without it Squid still works, but skips protected app data.",
              "这是可选项。未授权时 Squid 仍可正常工作，只会跳过受保护的 App 数据。",
            )
          : t(
              "Open System Settings to review access, then come back and recheck.",
              "请到系统设置中确认权限，然后返回重新检查。",
            );

  const protection = status?.protection ?? null;
  const intel = status?.intel ?? null;

  /** One switch on the security half of settings, then re-read: this card reports
   *  what the backend believes, not what was last clicked. */
  const toggle = async (key: "notifyOnFindings" | "updateRules") => {
    const next = !status?.settings[key];
    try {
      await security.saveSettings({ [key]: next });
      setStatus((current) =>
        current ? { ...current, settings: { ...current.settings, [key]: next } } : current,
      );
    } catch (reason) {
      setNote(messageFrom(reason));
    }
  };

  const flipAutostart = async () => {
    setLaunching(true);
    try {
      await api.setAutostart(!autostart);
    } catch (reason) {
      setNote(messageFrom(reason));
    } finally {
      setLaunching(false);
    }
  };

  return (
    <div
      className="setup-scrim"
      role="dialog"
      aria-modal="true"
      aria-labelledby="setup-title"
    >
      <div className="setup-card">
        <header className="setup-head">
          <span className="setup-brand">
            <ShieldCheck size={22} />
          </span>
          <div>
            <div className="setup-eyebrow">NEX · SQUID</div>
            <h2 id="setup-title">{t("Security readiness", "安全准备")}</h2>
            <p>
              {t(
                "Nothing here is switched on for you. Nothing leaves this computer.",
                "这里不会替你打开任何开关，也不会上传任何内容。",
              )}
            </p>
          </div>
        </header>

        <SetupRow
          icon={<ShieldCheck size={18} />}
          title={t("System protection", "系统防护")}
          state={
            !protection
              ? { tone: "checking", label: t("Reading…", "读取中…") }
              : protection.known
                ? { tone: "ok", label: t("Read from the system", "已读取") }
                : { tone: "problem", label: t("Cannot be confirmed", "无法确认") }
          }
          note={
            protection?.known
              ? t(
                  `${
                    protection.productName ||
                    (protection.provider === "xprotect" ? "XProtect" : protection.provider)
                  }${protection.definitions ? ` · ${protection.definitions}` : ""}. Read-only.`,
                  `${
                    protection.productName ||
                    (protection.provider === "xprotect" ? "XProtect" : protection.provider)
                  }${protection.definitions ? ` · ${protection.definitions}` : ""}。只读取，不改动。`,
                )
              : protection?.notes[0] ??
                t("Squid reports this instead of guessing.", "读不到时会直接说明，不会显示为正常。")
          }
        />

        <SetupRow
          icon={<LockKeyhole size={18} />}
          title={t("Scan access", "扫描访问")}
          optional
          state={{
            tone: checking ? "checking" : ready ? "ok" : "problem",
            label: stateLabel,
          }}
          note={detail}
          code={needsAttention ? SYSTEM_SETTINGS_COPY[lang] : undefined}
          action={
            needsAttention ? (
              <Button
                variant="secondary"
                size="sm"
                onClick={() =>
                  void api.openFullDiskAccessSettings().catch(() => undefined)
                }
              >
                {t("Open Settings", "打开系统设置")}
              </Button>
            ) : null
          }
        />

        <SetupRow
          icon={<BellRing size={18} />}
          title={t("Notifications", "通知")}
          optional
          state={{
            tone: status?.settings.notifyOnFindings ? "ok" : "problem",
            label: status?.settings.notifyOnFindings
              ? t("On", "已开启")
              : t("Off", "已关闭"),
          }}
          note={t(
            "Confirmed findings only. The banner and tray still report without it.",
            "仅针对已确认发现，应用内提示与托盘不受影响。",
          )}
          action={
            <Button
              variant="ghost"
              size="sm"
              disabled={!status}
              onClick={() => void toggle("notifyOnFindings")}
            >
              {status?.settings.notifyOnFindings
                ? t("Turn off", "关闭")
                : t("Turn on", "开启")}
            </Button>
          }
        />

        <SetupRow
          icon={<Power size={18} />}
          title={t("Start at login", "登录时启动")}
          optional
          state={{
            tone: autostart ? "ok" : "problem",
            label: autostart ? t("On", "已开启") : t("Off", "已关闭"),
          }}
          note={t(
            "No background service, so a scheduled scan only runs from the tray.",
            "不安装后台服务，定时扫描只在驻留托盘时运行。",
          )}
          action={
            <Button
              variant="ghost"
              size="sm"
              disabled={launching}
              onClick={() => void flipAutostart()}
            >
              {autostart ? t("Turn off", "关闭") : t("Turn on", "开启")}
            </Button>
          }
        />

        <SetupRow
          icon={<Download size={18} />}
          title={t("Rule updates", "规则更新")}
          optional
          state={{
            tone: status?.settings.updateRules ? "ok" : "problem",
            label: status?.settings.updateRules
              ? t("On", "已开启")
              : t("Built-in only", "仅使用内置"),
          }}
          note={
            intel
              ? t(
                  `${
                    intel.source === "pack"
                      ? `Rule pack r${intel.revision} · ${intel.installedRules} rules`
                      : `${intel.builtinRules} built-in rules, no pack downloaded`
                  }. Rules are the only download.`,
                  `${
                    intel.source === "pack"
                      ? `规则包 r${intel.revision} · ${intel.installedRules} 条规则`
                      : `内置 ${intel.builtinRules} 条规则，未下载规则包`
                  }。只会下载规则。`,
                )
              : t("Reading the installed rules…", "正在读取已安装规则…")
          }
          action={
            <Button
              variant="ghost"
              size="sm"
              disabled={!status}
              onClick={() => void toggle("updateRules")}
            >
              {status?.settings.updateRules
                ? t("Turn off", "关闭")
                : t("Turn on", "开启")}
            </Button>
          }
        />

        {note ? <p className="setup-error">{note}</p> : null}


        <footer className="setup-foot">
          <div className="setup-actions-secondary">
            <Button
              variant="ghost"
              size="sm"
              icon={<RefreshCw size={13} className={checking ? "spin" : ""} />}
              onClick={() => void refresh()}
              disabled={checking}
            >
              {t("Recheck", "重新检查")}
            </Button>
          </div>
          <div className="setup-actions-primary">
            <Button variant="ghost" onClick={onDone}>
              {t("Not now", "暂不设置")}
            </Button>
            <Button
              variant="primary"
              onClick={onDone}
              icon={checking ? <LoaderCircle size={14} className="spin" /> : <Check size={14} />}
            >
              {t("Start using Squid", "开始使用 Squid")}
            </Button>
          </div>
        </footer>
      </div>
    </div>
  );
}

function SetupRow({
  icon,
  title,
  state,
  note,
  code,
  action,
  optional,
}: {
  icon: React.ReactNode;
  title: string;
  state: { tone: "checking" | "ok" | "problem"; label: string };
  note: string;
  code?: string;
  action?: React.ReactNode;
  optional?: boolean;
}) {
  const { t } = useI18n();
  return (
    <div className={`setup-check is-${state.tone}`}>
      <span className="setup-check-icon">{icon}</span>
      <div className="setup-check-copy">
        <strong>
          {title}
          {optional ? <span className="setup-check-optional">{t("optional", "可选")}</span> : null}
          <span className="setup-check-state">
            {state.tone === "ok" ? <Check size={13} /> : null}
            {state.tone === "problem" ? <X size={13} /> : null}
            {state.label}
          </span>
        </strong>
        <small>{note}</small>
        {code ? <code>{code}</code> : null}
      </div>
      {action ? <div className="setup-check-action">{action}</div> : null}
    </div>
  );
}
