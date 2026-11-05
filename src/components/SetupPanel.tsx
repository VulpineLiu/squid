import {
  Check,
  LoaderCircle,
  LockKeyhole,
  RefreshCw,
  ShieldCheck,
  X,
} from "lucide-react";
import { useCallback, useEffect, useState } from "react";
import { useI18n } from "../i18n";
import { api } from "../lib/api";
import type { PermissionStatus } from "../types";
import { Button } from "./ui";

type State = "checking" | "granted" | "denied" | "unknown" | "not-required" | "error";

const SYSTEM_SETTINGS_COPY = {
  en: "System Settings › Privacy & Security › Full Disk Access",
  zh: "系统设置 › 隐私与安全性 › 完全磁盘访问权限",
};

export function SetupPanel({ onDone }: { onDone: () => void }) {
  const { t, lang } = useI18n();
  const [state, setState] = useState<State>("checking");

  const refresh = useCallback(async () => {
    setState("checking");
    try {
      const status: PermissionStatus = await api.permissionStatus();
      setState(status.fullDiskAccess);
    } catch {
      setState("error");
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
            <h2 id="setup-title">{t("Before you start", "开始之前")}</h2>
            <p>
              {t(
                "Squid runs entirely on this computer. One optional macOS permission improves scan coverage.",
                "Squid 完全在这台电脑上运行。macOS 上有一项可选权限可以提升扫描覆盖范围。",
              )}
            </p>
          </div>
        </header>

        <div className={`setup-check is-${checking ? "checking" : ready ? "ok" : "problem"}`}>
          <span className="setup-check-icon">
            <LockKeyhole size={18} />
          </span>
          <div className="setup-check-copy">
            <strong>
              {t("Full Disk Access", "完全磁盘访问权限")}
              <span className="setup-check-state">
                {ready ? <Check size={13} /> : needsAttention ? <X size={13} /> : null}
                {stateLabel}
              </span>
            </strong>
            <small>{detail}</small>
            {state === "denied" || state === "unknown" || state === "error" ? (
              <code>{SYSTEM_SETTINGS_COPY[lang]}</code>
            ) : null}
          </div>
        </div>

        <p className="setup-note">
          {t(
            "You can skip this now. Permissions and the same check remain available in Settings at any time.",
            "现在可以跳过；之后随时都能在设置中检查权限或重新授权。",
          )}
        </p>

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
            {needsAttention ? (
              <Button
                variant="secondary"
                size="sm"
                onClick={() => void api.openFullDiskAccessSettings()}
              >
                {t("Open Settings", "打开系统设置")}
              </Button>
            ) : null}
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
