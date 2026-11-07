import {
  AlertTriangle,
  Ban,
  CheckCircle2,
  Download,
  Eye,
  EyeOff,
  FolderOpen,
  FolderSearch,
  FolderX,
  HardDriveDownload,
  RefreshCw,
  ShieldAlert,
  ShieldCheck,
  ShieldQuestion,
  Trash2,
  Undo2,
} from "lucide-react";
import { useMemo, useState } from "react";
import { open } from "@tauri-apps/plugin-dialog";
import { useI18n } from "../i18n";
import { api } from "../lib/api";
import { formatBytes, formatCount, messageFrom, shortPath } from "../lib/format";
import type { Confidence, Finding, SecurityScan, SecurityStatus } from "../types";
import type { SecurityWorkspace } from "../hooks/useSecurityWorkspace";
import {
  Button,
  ConfirmDialog,
  EmptyState,
  InlineError,
  Modal,
  PageHeader,
  ProgressCard,
  SectionTitle,
  useToast,
} from "../components/ui";

type Filter = "all" | Confidence | "quarantined";

const CONFIDENCE_META: Record<Confidence, { tone: string; en: string; zh: string }> = {
  confirmed: { tone: "danger", en: "Confirmed", zh: "已确认" },
  suspicious: { tone: "warning", en: "Suspicious", zh: "可疑" },
  advisory: { tone: "neutral", en: "Advisory", zh: "提示" },
};

/**
 * Security: what protects this machine, what squid found, and what it is holding.
 *
 * The page never says "safe" on the strength of an empty list. `clean` is only
 * true when nothing was found *and* nothing was skipped, so a machine where
 * Full Disk Access was refused shows the gap beside the result rather than
 * underneath it.
 */
export function SecurityPage({
  security,
  onOpenStartup,
}: {
  security: SecurityWorkspace;
  onOpenStartup: () => void;
}) {
  const { t } = useI18n();
  const { push } = useToast();
  const [filter, setFilter] = useState<Filter>("all");
  const [detail, setDetail] = useState<Finding | null>(null);
  const [quarantining, setQuarantining] = useState<Finding | null>(null);
  const [deleting, setDeleting] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const [showScope, setShowScope] = useState(false);
  const status = security.status;
  const scan = status?.scan ?? null;

  const findings = useMemo(() => {
    const all = scan?.findings ?? [];
    if (filter === "all") return all.filter((finding) => !finding.ignored);
    if (filter === "quarantined") return all.filter((finding) => finding.quarantined);
    return all.filter((finding) => finding.confidence === filter && !finding.ignored);
  }, [filter, scan]);

  /**
   * Per-filter counts, counted the same way the list itself is filtered so a
   * chip's number is exactly what clicking it shows. The header used to spell out
   * "3 confirmed · 2 suspicious" beside chips that said the same words; one of the
   * two had to go, and the number belongs where the click is.
   */
  const counts = useMemo(() => {
    const all = scan?.findings ?? [];
    const open = all.filter((finding) => !finding.ignored);
    return {
      all: open.length,
      confirmed: open.filter((f) => f.confidence === "confirmed").length,
      suspicious: open.filter((f) => f.confidence === "suspicious").length,
      advisory: open.filter((f) => f.confidence === "advisory").length,
      quarantined: all.filter((f) => f.quarantined).length,
    };
  }, [scan]);

  const run = async (mode: "quick" | "full") => {
    try {
      await security.start(mode);
    } catch (reason) {
      push(messageFrom(reason), "danger");
    }
  };

  const chooseCustom = async () => {
    try {
      const picked = await open({
        multiple: true,
        directory: true,
        title: t("Choose what to scan", "选择要扫描的内容"),
      });
      if (!picked) return;
      const paths = (Array.isArray(picked) ? picked : [picked]).filter(Boolean) as string[];
      if (paths.length) await security.start("custom", paths);
    } catch (reason) {
      push(messageFrom(reason), "danger");
    }
  };

  const act = async (action: () => Promise<string>) => {
    setBusy(true);
    try {
      push(await action(), "success");
    } catch (reason) {
      push(messageFrom(reason), "danger");
    } finally {
      setBusy(false);
      setQuarantining(null);
      setDeleting(null);
    }
  };

  if (!status) {
    return (
      <div className="page is-stacked">
        <PageHeader
          eyebrow={t("Security", "安全")}
          title={t("Checking this machine…", "正在检查这台电脑…")}
          description={t(
            "Reading what protects the machine and what ran at the last login.",
            "正在读取系统防护状态与上次的启动项。",
          )}
        />
        {security.error ? <InlineError message={security.error} /> : null}
      </div>
    );
  }

  return (
    <div className="page is-stacked">
      <PageHeader
        eyebrow={t("Security", "安全")}
        title={
          scan
            ? t("The last scan, and what to do about it.", "上次扫描的结果，以及下一步。")
            : t("Nothing has been scanned yet.", "还没有扫描过这台电脑。")
        }
        description={t(
          "Scans run when you ask. Real-time protection stays with the system.",
          "扫描只在你要求时运行，实时防护仍由系统负责。",
        )}
        actions={
          <>
            <Button
              variant="ghost"
              size="sm"
              icon={<FolderSearch size={15} />}
              onClick={() => void chooseCustom()}
              disabled={security.scanning}
            >
              {t("Custom…", "自定义…")}
            </Button>
            <Button
              variant="ghost"
              size="sm"
              icon={<ShieldAlert size={15} />}
              onClick={() => void run("full")}
              disabled={security.scanning}
            >
              {t("Full scan", "全盘扫描")}
            </Button>
            <Button
              variant="primary"
              icon={<ShieldCheck size={16} />}
              onClick={() => void run("quick")}
              disabled={security.scanning}
            >
              {security.scanning
                ? t("Scanning…", "正在扫描…")
                : t("Quick scan", "快速扫描")}
            </Button>
          </>
        }
      />

      {security.scanning ? (
        <ProgressCard
          progress={security.progress}
          title={t("Scanning for malware", "正在扫描恶意软件")}
          onCancel={() => void security.cancel()}
        />
      ) : null}

      {security.error ? <InlineError message={security.error} /> : null}

      <ProtectionCard status={status} onRefresh={() => void security.refresh()} />

      <CoverageCard
        status={status}
        scan={scan}
        security={security}
        showScope={showScope}
        onToggleScope={() => setShowScope((current) => !current)}
      />

      <section className="panel">
        <SectionTitle
          title={t("Findings", "发现")}
          subtitle={
            scan
              ? undefined
              : t("Nothing has been scanned on this machine yet.", "这台电脑还没有扫描记录。")
          }
          action={
            scan ? (
              <div className="chip-row" role="tablist">
                {(
                  [
                    ["all", t("All", "全部"), counts.all],
                    ["confirmed", t("Confirmed", "已确认"), counts.confirmed],
                    ["suspicious", t("Suspicious", "可疑"), counts.suspicious],
                    ["advisory", t("Advisory", "提示"), counts.advisory],
                    ["quarantined", t("Quarantined", "已隔离"), counts.quarantined],
                  ] as [Filter, string, number][]
                ).map(([key, label, count]) => (
                  <button
                    key={key}
                    role="tab"
                    aria-selected={filter === key}
                    className={`filter-chip ${filter === key ? "is-active" : ""}`}
                    onClick={() => setFilter(key)}
                  >
                    {label}
                    <b>{count}</b>
                  </button>
                ))}
              </div>
            ) : null
          }
        />
        {!scan ? (
          <EmptyState
            icon={<ShieldQuestion size={26} />}
            title={t("No scan yet", "尚未扫描")}
            description={t(
              "A quick scan covers login items, applications, browser extensions and downloads.",
              "快速扫描会检查登录项、应用程序、浏览器扩展与下载目录。",
            )}
          />
        ) : findings.length === 0 ? (
          <EmptyState
            icon={<CheckCircle2 size={26} />}
            title={
              scan.coverageComplete
                ? t("Nothing found in the range this scan covers", "在已检查范围内未发现威胁")
                : t("Nothing found in what could be read", "在可读取范围内未发现威胁")
            }
            description={
              scan.coverageComplete
                ? t(
                    "This is not a promise about files a scan never opened — the coverage card lists those.",
                    "这不代表未打开的文件也是安全的——未检查范围见上方覆盖说明。",
                  )
                : t(
                    "Part of this scan could not be read. See the coverage card above.",
                    "本次扫描有部分目录无法读取，请查看上方覆盖范围。",
                  )
            }
          />
        ) : (
          <ul className="item-list">
            {findings.map((finding) => (
              <FindingRow
                key={finding.id}
                finding={finding}
                busy={busy}
                onOpen={() => setDetail(finding)}
                onQuarantine={() => setQuarantining(finding)}
                onDismiss={(ignored) => void act(() => security.dismiss(finding.id, ignored))}
                onExclude={() =>
                  finding.path &&
                  void act(() => security.exclude("add", parentFolder(finding.path as string)))
                }
                onReveal={() =>
                  finding.path &&
                  api
                    .revealPath(finding.path)
                    .catch((reason) => push(messageFrom(reason), "danger"))
                }
              />
            ))}
          </ul>
        )}
      </section>

      <QuarantineCard
        status={status}
        busy={busy}
        onRestore={(id) => void act(() => security.restore(id))}
        onDelete={(id) => setDeleting(id)}
      />

      <ActivityCard status={status} onOpenStartup={onOpenStartup} />

      <Modal
        open={Boolean(detail)}
        onClose={() => setDetail(null)}
        size="drawer"
        title={detail?.name ?? ""}
        subtitle={detail?.displayPath ?? undefined}
        footer={
          detail && !detail.quarantined && detail.confidence === "confirmed" ? (
            <Button
              variant="danger"
              onClick={() => {
                setDetail(null);
                setQuarantining(detail);
              }}
            >
              {t("Move to quarantine", "移入隔离区")}
            </Button>
          ) : null
        }
      >
        {detail ? <FindingDetail finding={detail} /> : null}
      </Modal>

      <ConfirmDialog
        open={Boolean(quarantining)}
        title={t("Quarantine this file?", "要隔离这个文件吗？")}
        tone="danger"
        busy={busy}
        icon={<Ban size={24} />}
        confirmLabel={t("Quarantine", "隔离")}
        cancelLabel={t("Cancel", "取消")}
        message={quarantining?.title ?? ""}
        note={t(
          "The copy cannot run, and the original path is kept so it can be restored.",
          "隔离后的副本不可执行，并保留原路径以便恢复。",
        )}
        onConfirm={() =>
          quarantining && void act(() => security.quarantine(quarantining.id))
        }
        onClose={() => setQuarantining(null)}
      >
        <div className="confirm-path">{quarantining?.displayPath}</div>
      </ConfirmDialog>

      <ConfirmDialog
        open={Boolean(deleting)}
        title={t("Delete permanently?", "要永久删除吗？")}
        tone="danger"
        busy={busy}
        icon={<Trash2 size={24} />}
        confirmLabel={t("Delete for good", "永久删除")}
        cancelLabel={t("Cancel", "取消")}
        message={t(
          "This removes the only copy squid is holding. It does not go to the Trash.",
          "这会删除 Squid 保存的唯一副本，且不会进入废纸篓。",
        )}
        onConfirm={() => deleting && void act(() => security.purge(deleting))}
        onClose={() => setDeleting(null)}
      />
    </div>
  );
}

function flagLabel(
  value: boolean | null,
  t: (en: string, zh: string) => string,
): { text: string; tone: string } {
  if (value === true) return { text: t("On", "已开启"), tone: "success" };
  if (value === false) return { text: t("Off", "已关闭"), tone: "danger" };
  return { text: t("Unknown", "无法确认"), tone: "neutral" };
}

function ProtectionCard({
  status,
  onRefresh,
}: {
  status: SecurityStatus;
  onRefresh: () => void;
}) {
  const { t } = useI18n();
  const protection = status.protection;
  const unknown = !protection.known;
  const providerName =
    protection.provider === "xprotect"
      ? "macOS XProtect"
      : protection.provider === "defender"
        ? protection.productName || "Windows Defender"
        : protection.productName || t("No product reported", "未读到防护产品");

  return (
    <section className={`panel is-protection ${unknown ? "is-unknown" : ""}`}>
      <span className={`metric-icon ${unknown ? "tone-warning" : "tone-primary"}`}>
        {unknown ? <ShieldQuestion size={22} /> : <ShieldCheck size={22} />}
      </span>
      <div className="protection-copy">
        <div className="protection-head">
          <strong>{providerName}</strong>
          <span className={`status-chip is-${unknown ? "neutral" : "info"}`}>
            {unknown
              ? t("Could not be confirmed", "无法确认")
              : t("Read from the system", "来自系统")}
          </span>
        </div>
        <p>
          {t(
            "Read only. Squid never starts, stops or updates it.",
            "只读。Squid 不会开启、关闭或更新它。",
          )}
        </p>
        {protection.notes.length ? (
          <ul className="protection-notes">
            {protection.notes.map((note) => (
              <li key={note}>{note}</li>
            ))}
          </ul>
        ) : null}
      </div>
      <div className="protection-facts">
        {protection.definitions ? (
          <div className="fact">
            <span>{t("Definitions", "定义版本")}</span>
            <strong>
              {protection.definitions}
              {protection.definitionsDate ? ` · ${protection.definitionsDate}` : ""}
            </strong>
          </div>
        ) : null}
        {protection.realtime !== null ? (
          <Fact
            label={t("Real-time protection", "实时防护")}
            value={flagLabel(protection.realtime, t)}
          />
        ) : null}
        {protection.backgroundScans !== null ? (
          <Fact
            label={t("Background scanning", "后台扫描")}
            value={flagLabel(protection.backgroundScans, t)}
          />
        ) : null}
        {protection.launchScans !== null ? (
          <Fact
            label={t("Scan at launch", "启动时扫描")}
            value={flagLabel(protection.launchScans, t)}
          />
        ) : null}
        {protection.gatekeeper !== null ? (
          <Fact label="Gatekeeper" value={flagLabel(protection.gatekeeper, t)} />
        ) : null}
        {protection.lastScanDate ? (
          <div className="fact">
            <span>{t("Last system scan", "上次系统扫描")}</span>
            <strong>{protection.lastScanDate}</strong>
          </div>
        ) : null}
        <button className="text-button" onClick={onRefresh}>
          <RefreshCw size={13} />
          {t("Read again", "重新读取")}
        </button>
      </div>
    </section>
  );
}

function Fact({ label, value }: { label: string; value: { text: string; tone: string } }) {
  return (
    <div className="fact">
      <span>{label}</span>
      <strong className={`is-${value.tone}`}>{value.text}</strong>
    </div>
  );
}

function CoverageCard({
  status,
  scan,
  showScope,
  security,
  onToggleScope,
}: {
  status: SecurityStatus;
  scan: SecurityScan | null;
  showScope: boolean;
  security: SecurityWorkspace;
  onToggleScope: () => void;
}) {
  const { t, lang } = useI18n();
  const { push } = useToast();
  const [updating, setUpdating] = useState(false);
  const intel = status.intel;
  const access = status.permissions.fullDiskAccess;
  const restricted = scan ? !scan.coverageComplete : false;
  const tone = !scan ? "neutral" : restricted ? "warning" : "success";
  const title = !scan
    ? t("This machine has not been scanned", "这台电脑尚未扫描")
    : restricted
      ? t("Coverage was limited", "覆盖范围受限")
      : t("Coverage was complete", "覆盖范围完整");

  return (
    <section className={`panel is-coverage is-${tone}`}>
      <div className="coverage-head">
        <div>
          <strong>{title}</strong>
          <p>
            {scan
              ? t(
                  `${formatCount(scan.examined)} files checked · ${formatCount(
                    scan.cached,
                  )} reused from last time · ${formatCount(scan.skipped)} skipped${
                    scan.cancelled ? " · this scan was cancelled before it finished" : ""
                  }.`,
                  `已检查 ${formatCount(scan.examined)} 个文件 · 复用上次结果 ${formatCount(
                    scan.cached,
                  )} 个 · 跳过 ${formatCount(scan.skipped)} 个${
                    scan.cancelled ? " · 本次扫描在完成前被取消" : ""
                  }。`,
                )
              : t(
                  "Squid has read nothing yet, so there is no result that could be called clean.",
                  "Squid 还没有读取过任何内容，因此不能称其为安全。",
                )}
          </p>
        </div>
        <span className={`status-chip is-${tone}`}>
          {lang === "zh"
            ? !scan
              ? "未扫描"
              : restricted
                ? "受限"
                : "完整"
            : !scan
              ? "not scanned"
              : restricted
                ? "limited"
                : "complete"}
        </span>
      </div>

      {access === "denied" || access === "unknown" ? (
        <div className="notice">
          <span>
            <AlertTriangle size={14} />
            {t(
              "Without Full Disk Access, protected folders are skipped and reported as unchecked.",
              "没有完全磁盘访问权限时，受保护目录会被跳过并标注为未检查。",
            )}
          </span>
          <Button
            variant="ghost"
            size="sm"
            onClick={() => void api.openFullDiskAccessSettings().catch(() => undefined)}
          >
            {t("Open System Settings", "打开系统设置")}
          </Button>
        </div>
      ) : null}

      {scan?.gaps.length ? (
        <ul className="coverage-gaps">
          {scan.gaps.slice(0, 8).map((gap) => (
            <li key={`${gap.path}-${gap.reason}`}>
              <FolderX size={13} />
              <span title={gap.path}>{shortPath(gap.path, 56)}</span>
              <small>{gap.reason}</small>
            </li>
          ))}
          {scan.gaps.length > 8 ? (
            <li className="is-more">
              {t(`${scan.gaps.length - 8} more`, `另有 ${scan.gaps.length - 8} 项`)}
            </li>
          ) : null}
        </ul>
      ) : null}

      {/* The rule actions live here as well as in Settings: someone reading a
          stale coverage card is exactly the person who wants to refresh the
          rules, and sending them off to find a setting first loses the thread. */}
      <div className="coverage-foot">
        <div className="coverage-rules">
          <span>
            {intel.source === "pack"
              ? t(
                  `Rule pack r${intel.revision} · ${formatCount(intel.installedRules)} rules · ${intel.published}`,
                  `规则包 r${intel.revision} · ${formatCount(intel.installedRules)} 条规则 · ${intel.published}`,
                )
              : t(
                  `${formatCount(intel.builtinRules)} built-in rules · no pack downloaded`,
                  `内置 ${formatCount(intel.builtinRules)} 条规则 · 未下载规则包`,
                )}
          </span>
          {/* A pack is only ever described by how it was actually checked. Saying
              "signed" because a signature would have been accepted if one had been
              configured is the kind of green that costs trust later. */}
          {intel.source === "pack" ? (
            <span
              className={`status-chip is-${intel.signed ? "success" : "warning"}`}
              title={
                intel.signed
                  ? t(
                      "This build verified the manifest against its rule-pack key.",
                      "本构建已用规则包密钥校验清单签名。",
                    )
                  : t(
                      "SHA-256 checked against the manifest. This build has no rule-pack key, so no signature was verified.",
                      "已按清单核对 SHA-256；本构建未内置规则包公钥，未校验签名。",
                    )
              }
            >
              {intel.signed
                ? t("signature verified", "已校验签名")
                : t("hash only · not signed", "仅校验哈希 · 未验签")}
            </span>
          ) : null}
        </div>
        <div className="coverage-actions">
          <button className="text-button" onClick={onToggleScope}>
            <Eye size={13} />
            {showScope ? t("Hide scope", "收起范围") : t("What is scanned", "扫描范围")}
          </button>
          <button
            className="text-button"
            disabled={updating}
            onClick={() => {
              setUpdating(true);
              void security
                .updateRules()
                .then((message) => push(message, "success"))
                .catch((reason) => push(messageFrom(reason), "danger"))
                .finally(() => setUpdating(false));
            }}
          >
            <Download size={13} />
            {updating ? t("Checking rules…", "正在检查规则…") : t("Update rules", "更新规则包")}
          </button>
          <button
            className="text-button"
            onClick={() =>
              void api
                .revealPath(intel.path)
                .catch(() => push(t("The rules folder does not exist yet.", "规则目录尚未创建。"), "danger"))
            }
          >
            <FolderOpen size={13} />
            {t("Open rules folder", "打开规则目录")}
          </button>
        </div>
      </div>

      {showScope ? (
        <div className="coverage-scope">
          <div>
            <h4>{t("Quick scan covers", "快速扫描覆盖")}</h4>
            <ul>
              {status.settings.quickRoots.map((root) => (
                <li key={root}>{root}</li>
              ))}
            </ul>
          </div>
          <div>
            <h4>{t("Full scan adds", "全盘扫描额外包含")}</h4>
            <ul>
              {status.settings.fullRoots
                .filter((root) => !status.settings.quickRoots.includes(root))
                .map((root) => (
                  <li key={root}>{root}</li>
                ))}
            </ul>
          </div>
          {status.settings.scanExclusions.length ? (
            <div>
              <h4>{t("Excluded by you", "你已排除")}</h4>
              <ul>
                {status.settings.scanExclusions.map((root) => (
                  <li key={root}>{root}</li>
                ))}
              </ul>
            </div>
          ) : null}
        </div>
      ) : null}
    </section>
  );
}

/// The folder a finding lives in. Excluding a whole tree is a bigger promise than
/// ignoring one finding, so it is offered as its own action rather than folded in.
function parentFolder(path: string): string {
  const cut = Math.max(path.lastIndexOf("/"), path.lastIndexOf("\\"));
  return cut > 0 ? path.slice(0, cut) : path;
}

function FindingRow({
  finding,
  busy,
  onOpen,
  onQuarantine,
  onDismiss,
  onExclude,
  onReveal,
}: {
  finding: Finding;
  busy: boolean;
  onOpen: () => void;
  onQuarantine: () => void;
  onDismiss: (ignored: boolean) => void;
  onExclude: () => void;
  onReveal: () => void;
}) {
  const { t } = useI18n();
  const meta = CONFIDENCE_META[finding.confidence];
  return (
    <li className={`item-row is-${meta.tone} ${finding.quarantined ? "is-handled" : ""}`}>
      <button className="item-main" onClick={onOpen}>
        <span className={`level-dot is-${meta.tone}`} />
        <span className="item-copy">
          <strong>{finding.name}</strong>
          <small title={finding.displayPath ?? undefined}>
            {finding.displayPath ?? t("Machine state", "系统状态")}
          </small>
          <em>{finding.title}</em>
        </span>
      </button>
      <span className={`status-chip is-${meta.tone}`}>
        {t(meta.en, meta.zh)}
        {finding.quarantined ? ` · ${t("held", "已隔离")}` : ""}
      </span>
      <div className="item-actions">
        {finding.path ? (
          <button
            className="icon-button"
            title={t("Show in folder", "在文件夹中显示")}
            onClick={onReveal}
          >
            <FolderOpen size={15} />
          </button>
        ) : null}
        {finding.quarantined ? null : (
          <>
            <button
              className="icon-button"
              title={t("Ignore this finding", "忽略此发现")}
              onClick={() => onDismiss(true)}
            >
              <EyeOff size={15} />
            </button>
            {finding.path ? (
              <button
                className="icon-button"
                title={t("Exclude this folder from future scans", "今后扫描时排除该文件夹")}
                onClick={onExclude}
              >
                <FolderX size={15} />
              </button>
            ) : null}
            {finding.confidence === "confirmed" ? (
              <Button
                size="sm"
                variant="danger"
                disabled={busy}
                icon={<Ban size={14} />}
                onClick={onQuarantine}
              >
                {t("Quarantine", "隔离")}
              </Button>
            ) : null}
          </>
        )}
      </div>
    </li>
  );
}

function FindingDetail({ finding }: { finding: Finding }) {
  const { t } = useI18n();
  const meta = CONFIDENCE_META[finding.confidence];
  const rows: [string, string][] = [
    [t("Confidence", "级别"), t(meta.en, meta.zh)],
    [t("Size", "大小"), formatBytes(finding.size)],
    [
      t("Signed", "签名"),
      finding.signed ? finding.publisher || t("Yes", "是") : t("No", "否"),
    ],
  ];
  if (finding.sha256) rows.push(["SHA-256", finding.sha256]);
  return (
    <div className="item-detail">
      <p className="item-detail-lead">{finding.description || finding.title}</p>
      <dl className="detail-grid">
        {rows.map(([label, value]) => (
          <div key={label}>
            <dt>{label}</dt>
            <dd title={value}>{value}</dd>
          </div>
        ))}
      </dl>
      <h4>{t("Why this was reported", "报告原因")}</h4>
      <ul className="evidence-list">
        {finding.evidence.map((item) => (
          <li key={item.key}>
            <span className="evidence-key">{item.key.split(":")[0]}</span>
            {item.summary}
          </li>
        ))}
      </ul>
      <p className="detail-note">
        {t(
          "Restoring puts the bytes and the permissions back exactly as they were.",
          "恢复时文件内容与权限会原样返回。",
        )}
      </p>
    </div>
  );
}

function QuarantineCard({
  status,
  busy,
  onRestore,
  onDelete,
}: {
  status: SecurityStatus;
  busy: boolean;
  onRestore: (id: string) => void;
  onDelete: (id: string) => void;
}) {
  const { t } = useI18n();
  const items = status.quarantine;
  return (
    <section className="panel">
      <SectionTitle
        title={t("Quarantine", "隔离区")}
        subtitle={
          items.length
            ? t(
                `${formatCount(items.length)} items · ${formatBytes(
                  status.quarantineSize,
                )} · held where they cannot run`,
                `${formatCount(items.length)} 项 · ${formatBytes(
                  status.quarantineSize,
                )} · 存放位置无法执行`,
              )
            : t("Empty — nothing has been moved out of the way.", "空的——没有文件被移走。")
        }
        action={
          <Button
            variant="ghost"
            size="sm"
            icon={<FolderOpen size={14} />}
            onClick={() => void api.openQuarantineFolder().catch(() => undefined)}
          >
            {t("Open folder", "打开目录")}
          </Button>
        }
      />
      {items.length === 0 ? (
        <p className="empty-inline">
          {t(
            "Only files squid moved are held here. This is not the Trash.",
            "这里只保存 Squid 移动的文件，不是废纸篓。",
          )}
        </p>
      ) : (
        <ul className="item-list">
          {items.map((item) => (
            <li key={item.id} className="item-row">
              <div className="item-copy">
                <strong title={item.originalPath}>{item.displayPath}</strong>
                <small>
                  {item.title} · {formatBytes(item.size)} ·{" "}
                  {new Date(item.quarantinedMs).toLocaleString()}
                </small>
              </div>
              <div className="item-actions">
                <Button
                  size="sm"
                  variant="ghost"
                  disabled={busy}
                  icon={<Undo2 size={14} />}
                  onClick={() => onRestore(item.id)}
                >
                  {t("Restore", "恢复")}
                </Button>
                <Button
                  size="sm"
                  variant="ghost"
                  disabled={busy}
                  icon={<Trash2 size={14} />}
                  onClick={() => onDelete(item.id)}
                >
                  {t("Delete", "删除")}
                </Button>
              </div>
            </li>
          ))}
        </ul>
      )}
    </section>
  );
}

function ActivityCard({
  status,
  onOpenStartup,
}: {
  status: SecurityStatus;
  onOpenStartup: () => void;
}) {
  const { t } = useI18n();
  const activity = status.activity;
  if (!activity.length) return null;
  return (
    <section className="panel">
      <SectionTitle
        title={t("Recent security activity", "最近的安全活动")}
        subtitle={t("Scans, quarantines and startup changes.", "本机的扫描、隔离与启动项修改。")}
      />
      <ul className="activity-list">
        {activity.slice(0, 10).map((entry, index) => (
          <li key={`${entry.atMs}-${index}`}>
            <span className="activity-kind">{entry.kind}</span>
            <span className="activity-detail">{entry.detail}</span>
            <time>{new Date(entry.atMs).toLocaleString()}</time>
          </li>
        ))}
      </ul>
      <button className="text-button" onClick={onOpenStartup}>
        <HardDriveDownload size={13} />
        {t("Manage startup items", "管理启动项")}
      </button>
    </section>
  );
}
