import {
  AppWindow,
  ArrowRight,
  Boxes,
  ChevronRight,
  CircleGauge,
  HardDrive,
  ScanSearch,
  Sparkles,
  WandSparkles,
} from "lucide-react";
import { useI18n } from "../i18n";
import { formatBytes, formatCount, percent } from "../lib/format";
import type { Bootstrap, SystemSnapshot } from "../types";
import type { ScanWorkspace } from "../hooks/useScanWorkspace";
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
  onNavigate,
}: {
  bootstrap: Bootstrap;
  snapshot: SystemSnapshot | null;
  cleanup: ScanWorkspace;
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
          "One safe scan for clutter, storage and everyday performance.",
          "一次安全扫描，整理垃圾、空间与日常性能。",
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
              ? t("Review scan", "查看扫描结果")
              : t("Smart scan", "智能扫描")}
          </Button>
        }
      />

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
                : t("Start a smart scan", "开始智能扫描")}
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
