import {
  AppWindow,
  ArrowRight,
  Boxes,
  ChevronRight,
  CircuitBoard,
  CircleGauge,
  Cpu,
  HardDrive,
  MemoryStick,
  MonitorCog,
  ScanSearch,
  Sparkles,
  WandSparkles,
  Waypoints,
} from "lucide-react";
import { useI18n } from "../i18n";
import {
  formatBytes,
  formatCount,
  formatMemoryBytes,
  percent,
} from "../lib/format";
import type { Bootstrap, PageId, SystemSnapshot, Volume } from "../types";
import type { ScanWorkspace } from "../hooks/useScanWorkspace";
import { CategorySummary } from "../components/CleanupView";
import {
  Button,
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

function coreLabel(snapshot: SystemSnapshot | null, t: (en: string, zh: string) => string) {
  if (!snapshot) return t("Reading processor…", "正在读取处理器…");
  const { physical_cores: physical, logical_cores: logical } = snapshot.cpu;
  if (physical === logical) {
    return t(`${physical} cores`, `${physical} 核`);
  }
  return t(`${physical} cores / ${logical} threads`, `${physical} 核 / ${logical} 线程`);
}

function machineRows(
  bootstrap: Bootstrap,
  snapshot: SystemSnapshot | null,
  volume: Volume | null,
  t: (en: string, zh: string) => string,
) {
  const hardware = bootstrap.hardwareInfo;
  const model = bootstrap.hardware.split(" · ")[0]?.trim() || bootstrap.hardware;
  const installed = snapshot
    ? Math.max(snapshot.memory.installed, snapshot.memory.total)
    : 0;
  const ramBrands = [
    ...new Set(hardware.memory_sticks.map((stick) => stick.manufacturer).filter(Boolean)),
  ].join(" / ");
  const ramSpeed = hardware.memory_sticks.reduce(
    (top, stick) => Math.max(top, stick.speed_mhz),
    0,
  );
  const memory = [
    installed ? formatMemoryBytes(installed) : "",
    ramBrands,
    ramSpeed ? `${ramSpeed} MT/s` : "",
  ]
    .filter(Boolean)
    .join(" · ");
  const disk = [
    [hardware.disk_vendor, hardware.disk_model].filter(Boolean).join(" "),
    hardware.disk_size_bytes ? formatBytes(hardware.disk_size_bytes, 0) : "",
    hardware.disk_bus,
  ]
    .filter(Boolean)
    .join(" · ");
  const gpu = [
    hardware.gpu_model &&
    hardware.gpu_model.toLowerCase().startsWith(hardware.gpu_vendor.toLowerCase())
      ? hardware.gpu_model
      : [hardware.gpu_vendor, hardware.gpu_model].filter(Boolean).join(" "),
    hardware.gpu_vram_bytes ? formatBytes(hardware.gpu_vram_bytes, 0) : "",
  ]
    .filter(Boolean)
    .join(" · ");
  const rows = [
    {
      key: "os",
      icon: MonitorCog,
      label: t("Operating system", "操作系统"),
      value: bootstrap.osVersion || bootstrap.osName,
    },
    {
      key: "model",
      icon: CircuitBoard,
      label: t("Computer", "本机型号"),
      value: hardware.board || model,
    },
    {
      key: "cpu",
      icon: Cpu,
      label: t("Processor", "处理器"),
      value: snapshot
        ? snapshot.cpu.brand || t("Unknown processor", "未知处理器")
        : t("Reading processor…", "正在读取处理器…"),
    },
    {
      key: "memory",
      icon: MemoryStick,
      label: t("Memory", "内存"),
      value: memory || t("Reading memory…", "正在读取内存…"),
    },
    {
      key: "disk",
      icon: HardDrive,
      label: t("Primary disk", "主硬盘"),
      value:
        disk ||
        (volume
          ? `${volume.name || volume.mount_point} · ${formatBytes(volume.total, 0)} · ${volume.filesystem.toUpperCase()}`
          : t("Reading disks…", "正在读取磁盘…")),
    },
  ];
  if (gpu || snapshot?.host.gpu) {
    rows.push({
      key: "gpu",
      icon: Waypoints,
      label: t("Graphics", "显卡"),
      value: gpu || snapshot?.host.gpu || "",
    });
  }
  return rows;
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
  const volumes = snapshot?.volumes ?? [];
  const rootVolume = volumes.find((candidate) => candidate.is_root) ?? volumes[0] ?? null;
  const used = rootVolume ? rootVolume.total - rootVolume.free : 0;
  const usedPercent = rootVolume ? percent(used, rootVolume.total) : 0;
  const memoryPercent = snapshot
    ? percent(snapshot.memory.used, snapshot.memory.total)
    : 0;
  const memoryInstalled = snapshot
    ? Math.max(snapshot.memory.installed, snapshot.memory.total)
    : 0;
  const memoryReserved = snapshot
    ? Math.max(0, memoryInstalled - snapshot.memory.total)
    : 0;
  const found = cleanup.result
    ? cleanup.result.items
        .filter((item) => !item.lock)
        .reduce((sum, item) => sum + item.onDisk, 0)
    : 0;
  const visibleVolumes = (snapshot?.volumes ?? []).slice(0, 2);
  const startScan = () => {
    if (cleanup.result) onNavigate("cleanup");
    else void cleanup.runScan();
  };

  return (
    <div className="page overview-page">
      <PageHeader
        eyebrow={`${greeting(lang)} · ${bootstrap.osName}`}
        title={t("Your computer, at its best.", "让电脑始终保持在最佳状态。")}
        description={t(
          "One safe scan tidies clutter, space and everyday performance.",
          "一次安全扫描，整理垃圾、空间与日常性能。",
        )}
        actions={
          <Button
            variant="primary"
            size="lg"
            icon={<Sparkles size={17} />}
            onClick={startScan}
            disabled={cleanup.scanning}
          >
            {cleanup.scanning
              ? t("Scanning…", "扫描中…")
              : cleanup.result
                ? t("Review smart scan", "查看智能扫描")
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
              onClick={startScan}
              disabled={cleanup.scanning}
            >
              {cleanup.result
                ? t("Choose what to clean", "选择清理项目")
                : t("Start smart scan", "开始智能扫描")}
            </Button>
            <Button
              variant="secondary"
              icon={<CircleGauge size={16} />}
              onClick={() => onNavigate("system")}
            >
              {t("Live monitor", "实时监控")}
            </Button>
          </div>
          <div className="hero-assurances">
            <span>{t("Local only", "仅在本地")}</span>
            <span>{t("Policy checked", "安全策略校验")}</span>
            <span>{t("Undo available", "支持还原")}</span>
          </div>
        </div>

        <aside className="overview-machine">
          <span className="machine-title">{t("This machine", "本机配置")}</span>
          <div className="machine-list">
            {machineRows(bootstrap, snapshot, rootVolume, t).map((row) => {
              const Icon = row.icon;
              return (
                <div className="machine-row" key={row.key}>
                  <span className="machine-icon">
                    <Icon size={15} />
                  </span>
                  <span className="machine-label">{row.label}</span>
                  <strong title={row.value}>{row.value}</strong>
                </div>
              );
            })}
          </div>
        </aside>
      </section>

      {cleanup.scanning ? (
        <ProgressCard
          progress={cleanup.progress}
          title={t("Looking for safe things to clean…", "正在查找可安全清理的项目…")}
          onCancel={() => void cleanup.cancel()}
        />
      ) : null}

      <section className="overview-metrics">
        <article className="metric-panel">
          <div className="metric-panel-head">
            <span className="metric-icon tone-primary">
              <Cpu size={18} />
            </span>
            <div>
              <strong>{t("Processor", "处理器")}</strong>
              <span>{coreLabel(snapshot, t)}</span>
            </div>
            <b>{Math.round(snapshot?.cpu.total ?? 0)}%</b>
          </div>
          <UsageBar used={snapshot?.cpu.total ?? 0} total={100} tone="primary" />
          <small>
            {snapshot?.cpu.brand || t("Processor details unavailable", "暂无处理器信息")}
          </small>
        </article>

        <article className="metric-panel">
          <div className="metric-panel-head">
            <span className="metric-icon tone-violet">
              <MemoryStick size={18} />
            </span>
            <div>
              <strong>{t("Memory", "内存")}</strong>
              <span>
                {snapshot
                  ? `${formatMemoryBytes(snapshot.memory.used)} / ${formatMemoryBytes(
                      memoryInstalled,
                    )}`
                  : t("Sampling…", "正在采样…")}
              </span>
            </div>
            <b>{Math.round(memoryPercent)}%</b>
          </div>
          <UsageBar used={memoryPercent} total={100} tone="violet" />
          <small>
            {memoryReserved > 0
              ? t(
                  `Hardware reserved (integrated graphics): ${formatMemoryBytes(memoryReserved)}`,
                  `硬件保留（含核显）：${formatMemoryBytes(memoryReserved)}`,
                )
              : snapshot?.memory.compressed !== null &&
            snapshot?.memory.compressed !== undefined
              ? t(
                  `Compressed ${formatMemoryBytes(snapshot.memory.compressed)} · Wired ${formatMemoryBytes(
                    snapshot.memory.wired ?? 0,
                  )}`,
                  `压缩 ${formatMemoryBytes(snapshot.memory.compressed)} · 固定 ${formatMemoryBytes(
                    snapshot.memory.wired ?? 0,
                  )}`,
                )
              : snapshot
                ? t(
                    `${formatCount(snapshot.processes)} processes running`,
                    `${formatCount(snapshot.processes)} 个进程正在运行`,
                  )
                : t("Reading activity", "正在读取活动")}
          </small>
        </article>

        <article className="metric-panel overview-storage-card">
          <div className="metric-panel-head">
            <span className="metric-icon tone-success">
              <HardDrive size={18} />
            </span>
            <div>
              <strong>{t("Storage", "存储空间")}</strong>
              <span>
                {snapshot
                  ? t(
                      `${formatCount(snapshot.volumes.length)} partitions`,
                      `${formatCount(snapshot.volumes.length)} 个分区`,
                    )
                  : t("Reading volume…", "正在读取磁盘…")}
              </span>
            </div>
            <b>{Math.round(usedPercent)}%</b>
          </div>
          {visibleVolumes.length > 0 ? (
            <div className="storage-volume-list">
              {visibleVolumes.map((candidate) => {
                const candidateUsed = candidate.total - candidate.free;
                const candidatePercent = percent(candidateUsed, candidate.total);
                return (
                  <div className="storage-volume" key={candidate.mount_point}>
                    <div className="storage-volume-head">
                      <strong>
                        {candidate.name || candidate.mount_point}
                        {candidate.is_root ? ` · ${t("System", "系统")}` : ""}
                      </strong>
                      <span>
                        {t(
                          `Total ${formatBytes(candidate.total)} · ${formatBytes(candidate.free)} free`,
                          `总 ${formatBytes(candidate.total)} · 可用 ${formatBytes(candidate.free)}`,
                        )}
                      </span>
                    </div>
                    <UsageBar
                      used={candidateUsed}
                      total={candidate.total}
                      tone={candidatePercent > 88 ? "danger" : "success"}
                    />
                  </div>
                );
              })}
            </div>
          ) : (
            <UsageBar used={0} total={100} tone="success" />
          )}
        </article>
      </section>

      {cleanup.result && !cleanup.scanning ? (
        <section className="overview-found">
          <SectionTitle
            title={t("What the scan found", "扫描发现")}
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
          <button className="quick-card tone-info" onClick={() => onNavigate("applications")}>
            <span className="quick-icon">
              <AppWindow size={22} />
            </span>
            <span className="quick-copy">
              <strong>{t("Uninstall apps", "卸载应用")}</strong>
              <small>{t("Remove apps and their leftovers", "连同残留一起彻底移除")}</small>
            </span>
            <ChevronRight size={17} />
          </button>
          <button className="quick-card tone-violet" onClick={() => onNavigate("storage")}>
            <span className="quick-icon">
              <Boxes size={22} />
            </span>
            <span className="quick-copy">
              <strong>{t("Explore storage", "浏览存储空间")}</strong>
              <small>{t("See what is taking up space", "查看空间都被什么占用")}</small>
            </span>
            <ChevronRight size={17} />
          </button>
          <button className="quick-card tone-violet" onClick={() => onNavigate("purge")}>
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
    </div>
  );
}
