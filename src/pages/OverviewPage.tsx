import {
  AlertTriangle,
  AppWindow,
  ArrowRight,
  Boxes,
  CheckCircle2,
  ChevronRight,
  CircleGauge,
  HardDrive,
  ScanSearch,
  ShieldCheck,
  ShieldQuestion,
  Sparkles,
  WandSparkles,
} from "lucide-react";
import { useI18n } from "../i18n";
import { formatBytes, formatCount, percent } from "../lib/format";
import type { Bootstrap, SystemSnapshot } from "../types";
import type { ScanWorkspace } from "../hooks/useScanWorkspace";
import type { SecurityWorkspace } from "../hooks/useSecurityWorkspace";
import type { PageId } from "../types";
import { CategorySummary } from "../components/CleanupView";
import {
  Button,
  Gauge,
  PageHeader,
  ProgressCard,
  SectionTitle,
  UsageBar,
} from "../components/ui";

/**
 * The security strip: what protects the machine, and whether anything is waiting.
 *
 * It says "not scanned yet" rather than showing a green tick. The whole point of
 * the roadmap's rule — never render an unknown as healthy — is that a strip which
 * defaults to calm teaches people to trust a colour that has not earned it.
 */
function SecurityStrip({
  security,
  onNavigate,
}: {
  security: SecurityWorkspace;
  onNavigate: (page: PageId) => void;
}) {
  const { t } = useI18n();
  const status = security.status;
  if (!status) {
    return (
      <section className="security-strip tone-neutral">
        <span className="metric-icon">
          <ShieldQuestion size={18} />
        </span>
        <span className="quick-copy">
          <strong>{t("Reading this machine's protection…", "正在读取本机防护状态…")}</strong>
        </span>
      </section>
    );
  }
  const scan = status.scan;
  const alarmed = status.confirmed > 0;
  const tone = alarmed ? "danger" : scan ? (scan.coverageComplete ? "success" : "warning") : "neutral";
  const headline = alarmed
    ? t(
        `${status.confirmed} confirmed finding${status.confirmed === 1 ? "" : "s"} need a decision`,
        `有 ${status.confirmed} 项已确认发现需要处理`,
      )
    : !scan
      ? t("No scan of this machine yet", "尚未进行过扫描")
      : !scan.coverageComplete
        ? t("Nothing found — but coverage was limited", "未发现威胁，但覆盖范围受限")
        : t("Nothing outstanding from the last scan", "上次扫描没有待处理项");
  // One line, in the order someone asks about it: what guards the machine now,
  // what squid last looked at, and whether it could look at everything.
  const provider = !status.protection.known
    ? t("protection unknown", "防护状态未知")
    : status.protection.provider === "xprotect"
      ? "XProtect"
      : status.protection.productName || "Antivirus";
  const detail = scan
    ? t(
        `${provider} · ${formatCount(scan.examined)} files checked · ${
          scan.coverageComplete ? "full coverage" : "coverage limited"
        }`,
        `${provider} · 已检查 ${formatCount(scan.examined)} 个文件 · ${
          scan.coverageComplete ? "覆盖完整" : "覆盖受限"
        }`,
      )
    : t(
        `${provider} · quick scan covers login items, apps and downloads`,
        `${provider} · 快速扫描覆盖登录项、应用与下载目录`,
      );

  return (
    <section className={`security-strip tone-${tone}`}>
      <span className="metric-icon">
        {alarmed ? <AlertTriangle size={18} /> : scan ? <CheckCircle2 size={18} /> : <ShieldQuestion size={18} />}
      </span>
      <div className="quick-copy">
        <strong>{headline}</strong>
        <small>{detail}</small>
      </div>
      <Button
        variant={alarmed ? "danger" : "ghost"}
        size="sm"
        icon={<ArrowRight size={14} />}
        onClick={() => onNavigate("security")}
      >
        {alarmed ? t("Review findings", "查看发现") : t("Open security", "打开安全")}
      </Button>
    </section>
  );
}

function greeting(lang: "zh" | "en"): string {
  const hour = new Date().getHours();
  if (lang === "zh") {
    if (hour < 6) return "夜深了";
    if (hour < 12) return "早上好";
    if (hour < 18) return "下午好";
    return "晚上好";
  }
  if (hour < 12) return "Good morning";
  if (hour < 18) return "Good afternoon";
  return "Good evening";
}

export function OverviewPage({
  bootstrap,
  snapshot,
  cleanup,
  security,
  onNavigate,
}: {
  bootstrap: Bootstrap;
  snapshot: SystemSnapshot | null;
  cleanup: ScanWorkspace;
  security: SecurityWorkspace;
  onNavigate: (page: PageId) => void;
}) {
  const { t, lang } = useI18n();
  const volume =
    snapshot?.volumes.find((candidate) => candidate.is_root) ?? snapshot?.volumes[0] ?? null;
  const used = volume ? volume.total - volume.free : 0;
  const usedPercent = volume ? percent(used, volume.total) : 0;
  const memoryPercent = snapshot
    ? percent(snapshot.memory.used, snapshot.memory.total)
    : 0;
  const found = cleanup.result
    ? cleanup.result.items
        .filter((item) => !item.lock)
        .reduce((sum, item) => sum + item.onDisk, 0)
    : 0;

  return (
    <div className="page overview-page">
      <PageHeader
        eyebrow={`${greeting(lang)} · ${bootstrap.osName}`}
        title={t("Your computer, at its best.", "让电脑始终保持在最佳状态。")}
        description={t(
          "Cleaning and security are two different scans here — one frees space, the other checks for threats.",
          "清理与安全是两种不同的扫描：一个释放空间，另一个检查威胁。",
        )}
        actions={
          <Button
            variant="primary"
            size="lg"
            icon={<Sparkles size={17} />}
            onClick={() => {
              if (cleanup.result) onNavigate("cleanup");
              else void cleanup.runScan();
            }}
            disabled={cleanup.scanning}
          >
            {cleanup.result
              ? t("Review cleanup scan", "查看清理结果")
              : t("Cleanup scan", "清理扫描")}
          </Button>
        }
      />

      {security.scanning ? (
        <ProgressCard
          progress={security.progress}
          title={t("Scanning for malware", "正在扫描恶意软件")}
          compact
          onCancel={() => void security.cancel()}
        />
      ) : null}

      <SecurityStrip security={security} onNavigate={onNavigate} />

      <section className="overview-hero">
        <div className="overview-hero-copy">
          <div className="hero-kicker">
            <WandSparkles size={15} />
            {t("Smart Care", "智能维护")}
          </div>
          <h2>
            {cleanup.result
              ? t(
                  `${formatBytes(found)} found safely.`,
                  `安全发现 ${formatBytes(found)} 可清理空间。`,
                )
              : t("A lighter, faster machine.", "更轻盈，更顺畅。")}
          </h2>
          <p>
            {cleanup.result
              ? t(
                  "Review each category before anything is removed. Locked files stay untouched.",
                  "清理前可逐项检查，锁定文件始终不会被处理。",
                )
              : t(
                  "Squid looks for rebuildable caches, old logs and build leftovers without touching your documents.",
                  "Squid 会查找可重建的缓存、旧日志与构建残留，不会触碰您的文档。",
                )}
          </p>
          <div className="hero-actions">
            <Button
              variant="primary"
              icon={cleanup.result ? <ArrowRight size={16} /> : <ScanSearch size={16} />}
              onClick={() => {
                if (cleanup.result) onNavigate("cleanup");
                else void cleanup.runScan();
              }}
              disabled={cleanup.scanning}
            >
              {cleanup.result
                ? t("Choose what to clean", "选择清理项目")
                : t("Start a cleanup scan", "开始清理扫描")}
            </Button>
            {/* Two scans, two names, one screen. Merging them would let a user
                believe the clutter sweep also looked for threats, and it does
                not; this is the line the roadmap drew on purpose. */}
            <Button
              variant="secondary"
              icon={<ShieldCheck size={16} />}
              onClick={() => void security.start("quick")}
              disabled={security.scanning}
            >
              {security.scanning
                ? t("Security scan…", "安全扫描中…")
                : t("Quick security scan", "安全快速扫描")}
            </Button>
            <Button
              variant="ghost"
              icon={<CircleGauge size={16} />}
              onClick={() => onNavigate("system")}
            >
              {t("Live system", "实时监控")}
            </Button>
          </div>
          <div className="hero-assurances">
            <span>{t("Local only", "仅在本地")}</span>
            <span>{t("Policy checked", "安全策略校验")}</span>
            <span>{t("Undo available", "支持还原")}</span>
          </div>
        </div>

        <div className="overview-gauge-wrap">
          <div className="gauge-halo" />
          <Gauge value={usedPercent} size={226} stroke={16} tone={usedPercent > 85 ? "danger" : "primary"}>
            <span className="gauge-caption">{t("Disk in use", "磁盘已用")}</span>
            <strong className="gauge-number">{Math.round(usedPercent)}%</strong>
            <span className="gauge-sub">
              {volume
                ? t(
                    `${formatBytes(volume.free)} free`,
                    `剩余 ${formatBytes(volume.free)}`,
                  )
                : t("Reading volume…", "正在读取磁盘…")}
            </span>
          </Gauge>
          <div className="hero-device">
            <HardDrive size={14} />
            <span>{bootstrap.hardware}</span>
          </div>
        </div>
      </section>

      {cleanup.scanning ? (
        <ProgressCard
          progress={cleanup.progress}
          title={t("Looking for safe things to clean…", "正在查找可安全清理的项目…")}
          onCancel={() => void cleanup.cancel()}
        />
      ) : null}

      {cleanup.result && !cleanup.scanning ? (
        <section className="overview-found">
          <SectionTitle
            title={t("What the scan found", "扫描发现")}
            subtitle={t(
              "Uncheck anything you want to keep.",
              "不想清理的项目可以随时取消勾选。",
            )}
            action={
              <Button variant="ghost" size="sm" onClick={() => onNavigate("cleanup")}>
                {t("Open cleanup", "打开清理")}
                <ChevronRight size={14} />
              </Button>
            }
          />
          <CategorySummary
            result={cleanup.result}
            selected={cleanup.selected}
            onToggle={cleanup.toggleCategory}
            onInspect={() => onNavigate("cleanup")}
          />
        </section>
      ) : null}

      <section className="quick-actions">
        <SectionTitle
          title={t("Everyday care", "日常维护")}
          subtitle={t("The right tool for the next five minutes.", "用几分钟完成最常用的事。")}
        />
        <div className="quick-grid">
          <button className="quick-card tone-primary" onClick={() => onNavigate("security")}>
            <span className="quick-icon">
              <ShieldCheck size={22} />
            </span>
            <span className="quick-copy">
              <strong>{t("Security", "安全")}</strong>
              <small>{t("Scan, review findings, quarantine", "扫描、查看发现、隔离")}</small>
            </span>
            <ChevronRight size={17} />
          </button>
          <button className="quick-card tone-primary" onClick={() => onNavigate("applications")}>
            <span className="quick-icon">
              <AppWindow size={22} />
            </span>
            <span className="quick-copy">
              <strong>{t("Uninstall apps", "卸载应用")}</strong>
              <small>{t("Remove apps and their leftovers", "连同残留一起彻底移除")}</small>
            </span>
            <ChevronRight size={17} />
          </button>
          <button className="quick-card tone-neutral" onClick={() => onNavigate("storage")}>
            <span className="quick-icon">
              <Boxes size={22} />
            </span>
            <span className="quick-copy">
              <strong>{t("Explore storage", "浏览存储空间")}</strong>
              <small>{t("See what is taking up space", "查看空间都被什么占用")}</small>
            </span>
            <ChevronRight size={17} />
          </button>
          <button className="quick-card tone-primary" onClick={() => onNavigate("purge")}>
            <span className="quick-icon">
              <Sparkles size={22} />
            </span>
            <span className="quick-copy">
              <strong>{t("Deep clean", "深度清理")}</strong>
              <small>{t("Clear build artefacts safely", "安全清理项目构建产物")}</small>
            </span>
            <ChevronRight size={17} />
          </button>
        </div>
      </section>

      <section className="overview-metrics">
        <div className="metric-panel">
          <div className="metric-panel-head">
            <span className="metric-icon tone-primary">
              <CircleGauge size={18} />
            </span>
            <div>
              <strong>{t("Processor", "处理器")}</strong>
              <span>
                {snapshot
                  ? `${snapshot.cpu.logical_cores} ${t("cores", "核心")}`
                  : t("Sampling…", "正在采样…")}
              </span>
            </div>
            <b>{Math.round(snapshot?.cpu.total ?? 0)}%</b>
          </div>
          <UsageBar used={snapshot?.cpu.total ?? 0} total={100} tone="primary" />
          <small>
            {snapshot?.cpu.brand || t("Processor details unavailable", "暂无处理器信息")}
          </small>
        </div>
        <div className="metric-panel">
          <div className="metric-panel-head">
            <span className="metric-icon tone-neutral">
              <Boxes size={18} />
            </span>
            <div>
              <strong>{t("Memory", "内存")}</strong>
              <span>
                {snapshot
                  ? `${formatBytes(snapshot.memory.used)} / ${formatBytes(
                      snapshot.memory.total,
                    )}`
                  : t("Sampling…", "正在采样…")}
              </span>
            </div>
            <b>{Math.round(memoryPercent)}%</b>
          </div>
          <UsageBar used={memoryPercent} total={100} tone="primary" />
          <small>
            {snapshot
              ? t(
                  `${formatCount(snapshot.processes)} processes running`,
                  `${formatCount(snapshot.processes)} 个进程正在运行`,
                )
              : t("Reading activity", "正在读取活动")}
          </small>
        </div>
        <div className="metric-panel">
          <div className="metric-panel-head">
            <span
              className={`metric-icon ${
                usedPercent > 85 ? "tone-warning" : "tone-success"
              }`}
            >
              <HardDrive size={18} />
            </span>
            <div>
              <strong>{t("Storage", "存储空间")}</strong>
              <span>
                {volume
                  ? `${formatBytes(volume.free)} ${t("available", "可用")}`
                  : t("Reading volume…", "正在读取磁盘…")}
              </span>
            </div>
            <b>{Math.round(usedPercent)}%</b>
          </div>
          <UsageBar
            used={usedPercent}
            total={100}
            tone={usedPercent > 85 ? "danger" : "success"}
          />
          <small>{volume?.mount_point || bootstrap.configPath}</small>
        </div>
      </section>
    </div>
  );
}
