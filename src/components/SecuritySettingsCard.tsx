import {
  AlertTriangle,
  Download,
  FolderOpen,
  RotateCcw,
  ShieldCheck,
  X,
} from "lucide-react";
import { useState } from "react";
import { open } from "@tauri-apps/plugin-dialog";
import { useI18n } from "../i18n";
import { api } from "../lib/api";
import { messageFrom, shortPath } from "../lib/format";
import type { ScanSchedule, SecurityStatus } from "../types";
import type { SecurityWorkspace } from "../hooks/useSecurityWorkspace";
import {
  Button,
  ConfirmDialog,
  IconButton,
  Row,
  Section,
  Select,
  Subhead,
  Toggle,
  useToast,
} from "./ui";

const SCHEDULES: [ScanSchedule, string, string][] = [
  ["manual", "Only when I ask", "仅在我要求时"],
  ["on-launch", "Once each time Squid starts", "每次启动 Squid 后一次"],
  ["daily", "Daily while Squid is in the tray", "驻留托盘时每天一次"],
  ["weekly", "Weekly while Squid is in the tray", "驻留托盘时每周一次"],
];

const EVIDENCE_LABELS: Record<string, [string, string]> = {
  startup: ["Startup items", "登录启动项"],
  unsigned: ["Unsigned files", "未签名文件"],
  writable: ["Writable locations", "可写位置"],
  missing: ["Missing targets", "目标缺失"],
};

/**
 * The security half of Settings.
 *
 * The schedule wording is deliberate. There is no daemon, so "daily" means daily
 * while Squid is running in the tray and not at all otherwise — a promise about a
 * timer that exists, rather than about a watchdog that does not.
 *
 * Exclusions live here and not next to the cleanup whitelist, because they mean
 * different things: one says "do not delete in here", the other says "do not look
 * in here", and a user protecting a folder from cleaning must not accidentally
 * hide a threat from being reported.
 */
export function SecuritySettingsCard({
  security,
  status,
}: {
  security: SecurityWorkspace;
  status: SecurityStatus;
}) {
  const { t } = useI18n();
  const { push } = useToast();
  const [draft, setDraft] = useState("");
  const [busy, setBusy] = useState(false);
  const [confirmAuto, setConfirmAuto] = useState(false);
  const [pendingRemove, setPendingRemove] = useState<string | null>(null);
  const [rulesNote, setRulesNote] = useState<string | null>(null);
  const settings = status.settings;
  const intel = status.intel;

  const save = async (patch: Parameters<SecurityWorkspace["saveSettings"]>[0]) => {
    setBusy(true);
    try {
      await security.saveSettings(patch);
    } catch (reason) {
      push(messageFrom(reason), "danger");
    } finally {
      setBusy(false);
    }
  };

  const add = async (path: string) => {
    setBusy(true);
    try {
      await security.exclude("add", path);
      setDraft("");
    } catch (reason) {
      push(messageFrom(reason), "danger");
    } finally {
      setBusy(false);
    }
  };

  const choose = async () => {
    const picked = await open({ directory: true, title: t("Exclude folder", "排除文件夹") });
    if (typeof picked === "string") void add(picked);
  };

  const runRules = async (action: () => Promise<string>) => {
    setBusy(true);
    setRulesNote(null);
    try {
      setRulesNote(await action());
    } catch (reason) {
      setRulesNote(messageFrom(reason));
    } finally {
      setBusy(false);
    }
  };

  const untrust = async (kind: "publisher" | "path" | "evidence", value: string) => {
    setBusy(true);
    try {
      push(await security.untrust(kind, value), "success");
    } catch (reason) {
      push(messageFrom(reason), "danger");
    } finally {
      setBusy(false);
    }
  };

  return (
    <Section icon={<ShieldCheck size={16} />} title={t("Security", "安全")}>
      <div className="row-list">
        <Row
          title={t("Scan schedule", "扫描计划")}
          subtitle={
            settings.scanSchedule === "manual"
              ? t(
                  "Nothing is scanned unless you start it.",
                  "不主动扫描，只在你点开始时才扫描。",
                )
              : t(
                  "No background service, so a scheduled scan runs only while Squid is in the tray.",
                  "Squid 无后台服务，定时扫描只在驻留托盘时才会运行。",
                )
          }
          trailing={
            <Select
              value={settings.scanSchedule}
              disabled={busy}
              label={t("Scan schedule", "扫描计划")}
              options={SCHEDULES.map(([value, en, zh]) => ({ value, label: t(en, zh) }))}
              onChange={(next) => void save({ scanSchedule: next })}
            />
          }
        />

        <Row
          title={t("Notify on confirmed findings", "发现高危时通知")}
          subtitle={t(
            "The banner and the tray report the same thing without it.",
            "关闭后，应用内提示与托盘仍会显示相同状态。",
          )}
          trailing={
            <Toggle
              checked={settings.notifyOnFindings}
              disabled={busy}
              label={t("Notify on confirmed findings", "发现高危时通知")}
              onChange={(next) => void save({ notifyOnFindings: next })}
            />
          }
        />

        <Row
          title={t("Auto-quarantine confirmed findings", "自动隔离已确认项")}
          subtitle={t(
            "Confirmed findings only. A suspicious file is never moved on its own.",
            "仅针对已确认发现，可疑项不会被自动移动。",
          )}
          trailing={
            <Toggle
              checked={settings.autoQuarantine}
              disabled={busy}
              label={t("Auto-quarantine confirmed findings", "自动隔离已确认项")}
              onChange={(next) =>
                next ? setConfirmAuto(true) : void save({ autoQuarantine: false })
              }
            />
          }
        />

        <Row
          title={t("Update rules automatically", "自动更新规则")}
          subtitle={
            intel.source === "pack"
              ? t(
                  `Rule pack r${intel.revision} · ${intel.installedRules} rules · published ${intel.published} · ${
                    intel.signed
                      ? "signature verified"
                      : "hash only — this build has no rule-pack key"
                  }.`,
                  `规则包 r${intel.revision} · ${intel.installedRules} 条规则 · 发布于 ${intel.published} · ${
                    intel.signed ? "已校验签名" : "仅校验哈希，本构建未内置规则包公钥"
                  }。`,
                )
              : t(
                  `${intel.builtinRules} built-in rules in this build; nothing downloaded yet.`,
                  `当前构建内置 ${intel.builtinRules} 条规则，尚未下载规则包。`,
                )
          }
          trailing={
            <Toggle
              checked={settings.updateRules}
              disabled={busy}
              label={t("Update rules automatically", "自动更新规则")}
              onChange={(next) => void save({ updateRules: next })}
            />
          }
        />

        <Row
          title={t("Rule pack", "规则包")}
          subtitle={
            rulesNote ??
            t(
              "Rules are downloaded to this machine; nothing is ever sent out.",
              "规则只下载到本机，不上传任何内容。",
            )
          }
          trailing={
            <div className="row-actions">
              <Button
                variant="ghost"
                size="sm"
                icon={<Download size={14} />}
                disabled={busy || !settings.updateRules}
                onClick={() => void runRules(security.updateRules)}
              >
                {t("Check for rules", "检查规则更新")}
              </Button>
              {intel.fallbackAvailable ? (
                <Button
                  variant="ghost"
                  size="sm"
                  icon={<RotateCcw size={14} />}
                  disabled={busy}
                  onClick={() => void runRules(security.rollbackRules)}
                >
                  {t("Roll back one pack", "回滚上一版规则")}
                </Button>
              ) : null}
            </div>
          }
        />
      </div>

      <div>
        <Subhead
          title={t("Scan exclusions", "安全扫描排除项")}
          hint={t(
            "Separate from the cleanup whitelist. Nothing inside is reported.",
            "与清理白名单彼此独立。被排除目录内的威胁不会上报。",
          )}
        />
        <div className="whitelist-add">
          <input
            value={draft}
            onChange={(event) => setDraft(event.target.value)}
            placeholder={t("Folder a scan must not read", "扫描不读取的文件夹")}
            onKeyDown={(event) => {
              if (event.key === "Enter" && draft.trim()) void add(draft.trim());
            }}
          />
          <Button variant="secondary" icon={<FolderOpen size={15} />} onClick={() => void choose()}>
            {t("Choose", "选择")}
          </Button>
          <Button
            variant="primary"
            disabled={!draft.trim() || busy}
            onClick={() => void add(draft.trim())}
          >
            {t("Add", "添加")}
          </Button>
        </div>
        <div className="row-list">
          {settings.scanExclusions.length === 0 ? (
            <p className="row-empty">
              {t("No folders are excluded from scanning.", "没有排除扫描的文件夹。")}
            </p>
          ) : (
            settings.scanExclusions.map((pattern) => (
              <Row
                key={pattern}
                leading={
                  <span className="whitelist-icon is-warning">
                    <AlertTriangle size={15} />
                  </span>
                }
                title={<code title={pattern}>{shortPath(pattern, 88)}</code>}
                trailing={
                  <IconButton
                    label={t("Stop excluding", "取消排除")}
                    onClick={() => setPendingRemove(pattern)}
                    disabled={busy}
                  >
                    <X size={14} />
                  </IconButton>
                }
              />
            ))
          )}
        </div>
      </div>

      {settings.allowlist.length ? (
        <div>
          <Subhead
            title={t("Trusted software", "已信任的软件")}
            hint={t(
              `${settings.allowlist.length} rules quiet ordinary advisories. Confirmed threats are never trusted.`,
              `${settings.allowlist.length} 条规则用于静默普通提示，已确认威胁永远不会被信任。`,
            )}
          />
          <div className="row-list">
            {settings.allowlist.map((entry) => (
              <Row
                key={`${entry.kind}:${entry.value}`}
                leading={
                  <span className="whitelist-icon">
                    <ShieldCheck size={15} />
                  </span>
                }
                title={
                  <code title={entry.value}>
                    {entry.kind === "evidence"
                      ? t(
                          EVIDENCE_LABELS[entry.value]?.[0] ?? entry.value,
                          EVIDENCE_LABELS[entry.value]?.[1] ?? entry.value,
                        )
                      : entry.kind === "path"
                        ? shortPath(entry.value, 88)
                        : entry.value}
                  </code>
                }
                tags={
                  <span className="allowlist-kind">
                    {entry.kind === "publisher"
                      ? t("Publisher", "发布者")
                      : entry.kind === "path"
                        ? t("Folder", "文件夹")
                        : t("Note type", "提示类型")}
                  </span>
                }
                trailing={
                  <IconButton
                    label={t("Stop trusting", "取消信任")}
                    onClick={() => void untrust(entry.kind, entry.value)}
                    disabled={busy}
                  >
                    <X size={14} />
                  </IconButton>
                }
              />
            ))}
          </div>
        </div>
      ) : null}

      {settings.ignoredFindings.length ? (
        <div>
          <Subhead
            title={t("Ignored findings", "已忽略的发现")}
            hint={t(
              `${settings.ignoredFindings.length} dismissed. Future scans hide them until you restore them here.`,
              `已忽略 ${settings.ignoredFindings.length} 项。在恢复之前，后续扫描不会再次报告。`,
            )}
          />
          <div className="row-list">
            {settings.ignoredFindings.map((fingerprint) => (
              <Row
                key={fingerprint}
                title={<code title={fingerprint}>{shortPath(fingerprint, 88)}</code>}
                trailing={
                  <Button
                    variant="ghost"
                    size="sm"
                    disabled={busy}
                    onClick={() => {
                      const match = status.scan?.findings.find(
                        (finding) =>
                          `${finding.confidence}|${finding.path}` === fingerprint,
                      );
                      if (match) {
                        void security
                          .dismiss(match.id, false)
                          .then((message) => push(message, "success"));
                      } else {
                        push(
                          t(
                            "This finding is not in the last scan — run a scan to restore it.",
                            "该发现不在上次扫描结果中，请先扫描再恢复。",
                          ),
                          "danger",
                        );
                      }
                    }}
                  >
                    {t("Report again", "重新报告")}
                  </Button>
                }
              />
            ))}
          </div>
        </div>
      ) : null}

      <ConfirmDialog
        open={confirmAuto}
        title={t("Let Squid move files on its own?", "允许 Squid 自动移动文件吗？")}
        tone="danger"
        busy={busy}
        icon={<ShieldCheck size={22} />}
        confirmLabel={t("Turn on", "开启")}
        cancelLabel={t("Cancel", "取消")}
        message={t(
          "Confirmed findings move to the quarantine without asking. Everything stays restorable.",
          "已确认发现将未经确认直接移入隔离区，仍可恢复。",
        )}
        onConfirm={() => {
          setConfirmAuto(false);
          void save({ autoQuarantine: true });
        }}
        onClose={() => setConfirmAuto(false)}
      />

      <ConfirmDialog
        open={Boolean(pendingRemove)}
        title={t("Scan this folder again?", "要恢复扫描这个文件夹吗？")}
        tone="default"
        busy={busy}
        icon={<AlertTriangle size={22} />}
        confirmLabel={t("Remove exclusion", "取消排除")}
        cancelLabel={t("Keep excluded", "保持排除")}
        message={pendingRemove ?? ""}
        note={t(
          "The next scan will read it again and may report findings inside it.",
          "下次扫描会重新读取该目录，并可能报告其中的发现。",
        )}
        onConfirm={() => {
          const path = pendingRemove;
          setPendingRemove(null);
          if (path) {
            void security
              .exclude("remove", path)
              .then((message) => push(message, "success"))
              .catch((reason) => push(messageFrom(reason), "danger"));
          }
        }}
        onClose={() => setPendingRemove(null)}
      />
    </Section>
  );
}
