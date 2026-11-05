import {
  Check,
  Download,
  LoaderCircle,
  Monitor,
  RefreshCw,
  Sparkles,
  X,
} from "lucide-react";
import { useI18n } from "../i18n";
import { formatBytes } from "../lib/format";
import type { UpdateFlow } from "../hooks/useUpdateFlow";
import type { Bootstrap } from "../types";
import { Button, Toggle } from "./ui";

function architectureLabel(arch: string): string {
  if (arch === "aarch64") return "ARM64";
  if (arch === "x86_64") return "x64";
  return arch;
}

export function UpdatePanel({
  flow,
  autoCheck,
  onAutoCheckChange,
  bootstrap,
}: {
  flow: UpdateFlow;
  autoCheck: boolean;
  onAutoCheckChange: (enabled: boolean) => void;
  bootstrap: Bootstrap;
}) {
  const { t } = useI18n();
  const status = flow.status;
  const verified = status?.state === "ready";
  const available = status?.state === "available" || verified;
  const updateStateLabel = verified
    ? t("Downloaded and verified", "已下载并校验")
    : t("Update available", "发现新版本");
  const statusError =
    status?.message &&
    (status.state === "error" || status.state === "unavailable")
      ? status.message
      : null;

  return (
    <section className="settings-section update-section">
      <div className="settings-section-head">
        <div>
          <h2>
            <Sparkles size={18} />
            {t("Software updates", "软件更新")}
          </h2>
          <p>
            {t(
              "Check for a new version, then download and verify it before installing.",
              "发现新版本后，下载并自动校验，确认无误再安装。",
            )}
          </p>
        </div>
        <Button
          variant="secondary"
          size="sm"
          icon={<RefreshCw size={14} className={flow.checking ? "spin" : ""} />}
          onClick={() => void flow.check()}
          disabled={flow.checking || flow.downloading || !bootstrap.updateFeedUrl}
        >
          {flow.checking ? t("Checking…", "检查中…") : t("Check now", "立即检查")}
        </Button>
      </div>

      <div className="update-system-panel">
        <span className="update-system-icon">
          <Monitor size={16} />
        </span>
        <div className="update-system-copy">
          <small>{t("Current system", "当前系统")}</small>
          <strong>
            {bootstrap.osName} {bootstrap.osVersion}
          </strong>
        </div>
        <div className="update-system-facts">
          <span>{architectureLabel(bootstrap.arch)}</span>
          <span className="is-app">Squid {bootstrap.version}</span>
        </div>
      </div>

      {available && status ? (
        <div className="update-status">
          <div className="update-status-copy">
            <span className={`update-state is-${status.state}`}>
              {updateStateLabel}
            </span>
            <span className="update-version-line">
              <strong>Squid {status.currentVersion}</strong>
              <span aria-hidden="true">→</span>
              <b>{status.availableVersion}</b>
            </span>
          </div>
          {status.date ? <time>{status.date}</time> : null}
        </div>
      ) : null}

      {available && status?.notes ? (
        <p className="update-notes">{status.notes}</p>
      ) : null}

      {flow.percent !== null ? (
        <div className="update-progress">
          <div className="update-progress-head">
            <span>
              {flow.progress?.state === "verifying"
                ? t("Verifying download…", "正在校验下载…")
                : t("Downloading update…", "正在下载更新…")}
            </span>
            <b>{flow.percent}%</b>
          </div>
          <div
            className="progress-track"
            role="progressbar"
            aria-valuemin={0}
            aria-valuemax={100}
            aria-valuenow={flow.percent}
          >
            <span style={{ width: `${flow.percent}%` }} />
          </div>
          <small>
            {flow.progress
              ? `${formatBytes(flow.progress.downloaded)} / ${formatBytes(
                  flow.progress.total,
                )}`
              : ""}
          </small>
        </div>
      ) : null}

      {statusError ? (
        <div className="update-message is-error" role="alert">
          {statusError}
        </div>
      ) : null}
      {flow.installError ? (
        <div className="update-message is-error" role="alert">
          {flow.installError}
        </div>
      ) : null}

      <div className="update-actions">
        {available && !verified ? (
          <Button
            variant="primary"
            icon={
              flow.downloading ? (
                <LoaderCircle size={15} className="spin" />
              ) : (
                <Download size={15} />
              )
            }
            onClick={() => void flow.download()}
            disabled={flow.downloading}
          >
            {flow.downloading
              ? t("Downloading…", "下载中…")
              : t("Download and verify", "下载并校验")}
          </Button>
        ) : null}
        {verified ? (
          <Button
            variant="primary"
            icon={
              flow.installing ? (
                <LoaderCircle size={15} className="spin" />
              ) : (
                <Check size={15} />
              )
            }
            onClick={() => void flow.install()}
            disabled={flow.installing}
          >
            {flow.installing
              ? t("Restarting…", "正在重启…")
              : t("Install and restart", "安装并重启")}
          </Button>
        ) : null}
        {verified && !flow.installing ? (
          <Button
            variant="secondary"
            className="update-cancel"
            icon={<X size={14} />}
            onClick={() => void flow.clear()}
          >
            {t("Discard", "放弃")}
          </Button>
        ) : null}
      </div>

      <div className="settings-row">
        <div>
          <strong>{t("Check for updates daily", "每天自动检查更新")}</strong>
          <span>
            {t(
              "Checks at most once a day when Squid starts.",
              "启动 Squid 时每天最多检查一次。",
            )}
          </span>
        </div>
        <Toggle
          checked={autoCheck}
          onChange={onAutoCheckChange}
          label={t("Check for updates automatically", "自动检查更新")}
        />
      </div>
    </section>
  );
}
