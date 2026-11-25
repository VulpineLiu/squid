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
import type { ReactNode } from "react";
import { useI18n } from "../i18n";
import { formatBytes, formatCount, formatMemory, percent } from "../lib/format";
import type { Bootstrap, SystemSnapshot, Volume } from "../types";
import type { ScanWorkspace } from "../hooks/useScanWorkspace";
import type { PageId } from "../types";
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

/// Windows mounts read `C:\`; everything else shows its own label.
function driveLabel(volume: Volume): string {
  return /^[A-Za-z]:/.exec(volume.mount_point)?.[0] ?? volume.mount_point;
}

function SpecRow({
  icon,
  label,
  value,
  t,
}: {
  icon: ReactNode;
  label: string;
  value: string;
  t: (en: string, zh: string) => string;
}) {
  return (
    <div className="spec-row">
      <span className="spec-icon">{icon}</span>
      <span className="spec-label">{label}</span>
      <strong title={value || undefined}>{value || t("—", "—")}</strong>
    </div>
  );
}

export function OverviewPage({
  bootstrap,
  snapshot,
  cleanup,
  purge,
  onNavigate,
  onOpenSpaceTab,
}: {
  bootstrap: Bootstrap;
  snapshot: SystemSnapshot | null;
  cleanup: ScanWorkspace;
  purge: ScanWorkspace;
  onNavigate: (page: PageId) => void;
  /** Deep-link into a 空间清理 hub tab (重复文件 / 大文件). */
  onOpenSpaceTab?: (tab: "junk" | "dups" | "big") => void;
}) {
  const { t, lang } = useI18n();
  const volumes = snapshot?.volumes ?? [];
  const rootVolume = volumes.find((candidate) => candidate.is_root) ?? volumes[0] ?? null;
  const memoryInstalled = snapshot
    ? Math.max(snapshot.memory.installed, snapshot.memory.total)
    : 0;
  const memoryReserved = snapshot
    ? Math.max(0, memoryInstalled - snapshot.memory.total)
    : 0;
  const scanned = Boolean(cleanup.result || purge.result);
  const scanning = cleanup.scanning || purge.scanning;
  const found = [cleanup, purge].reduce(
    (sum, workspace) =>
      sum +
      (workspace.result
        ? workspace.result.items
            .filter((item) => !item.lock)
            .reduce((total, item) => total + item.onDisk, 0)
        : 0),
    0,
  );
  // 智能扫描 is an entry point, not a resident process: the moment it is
  // pressed we hand the page over to 空间清理 and launch both scan stages
  // there (SpacePage owns the settings — age threshold, auto-discovery).
  // A finished scan keeps the button as a shortcut back to the results.
  const smartScan = () => {
    onNavigate("space");
    if (!scanned && !scanning) {
      void cleanup.runScan().then(() =>
        purge.runScan({
          // Empty = engine auto-discovery of project containers.
          roots: [],
          minAgeDays: bootstrap.settings.purgeMinAgeDays || 7,
          force: false,
        }),
      );
    }
  };

  const hardware = bootstrap.hardwareInfo;
  const ramBrands = [
    ...new Set(hardware.memory_sticks.map((stick) => stick.manufacturer).filter(Boolean)),
  ].join(" / ");
  const ramSpeed = hardware.memory_sticks.reduce(
    (top, stick) => Math.max(top, stick.speed_mhz),
    0,
  );
  const specs = {
    os: bootstrap.osVersion,
    board: hardware.board || bootstrap.hardware,
    cpu: snapshot
      ? `${snapshot.cpu.brand} · ${snapshot.cpu.physical_cores} ${t("cores", "核")} / ${
          snapshot.cpu.logical_cores
        } ${t("threads", "线程")}`
      : hardware.cpu_model,
    memory:
      [
        memoryInstalled ? formatMemory(memoryInstalled) : "",
        ramBrands,
        ramSpeed ? `${ramSpeed} MT/s` : "",
      ]
        .filter(Boolean)
        .join(" · ") || (memoryReserved ? formatMemory(memoryInstalled) : ""),
    disk:
      [
        [hardware.disk_vendor, hardware.disk_model].filter(Boolean).join(" "),
        hardware.disk_size_bytes ? formatBytes(hardware.disk_size_bytes) : "",
        hardware.disk_bus,
      ]
        .filter(Boolean)
        .join(" · "),
    gpu:
      [
        hardware.gpu_model &&
        hardware.gpu_model.toLowerCase().startsWith(hardware.gpu_vendor.toLowerCase())
          ? hardware.gpu_model
          : [hardware.gpu_vendor, hardware.gpu_model].filter(Boolean).join(" "),
        hardware.gpu_vram_bytes ? formatBytes(hardware.gpu_vram_bytes) : "",
      ]
        .filter(Boolean)
        .join(" · "),
  };

  return (
    <div className="page overview-page">
      <PageHeader
        eyebrow={`${greeting(lang)} · ${bootstrap.osName}`}
        title={t("Your computer, at its best", "让电脑始终保持在最佳状态")}
      />

      <section className="overview-hero">
        <div className="overview-hero-copy">
          <div className="hero-kicker">
            <WandSparkles size={15} />
            {t("Smart Care", "智能维护")}
          </div>
          <h2>
            {scanned
              ? t(
                  `${formatBytes(found)} found safely`,
                  `安全发现 ${formatBytes(found)} 可清理空间`,
                )
              : t("A lighter, faster machine", "更轻盈，更顺畅")}
          </h2>
          <p>
            {scanned
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
              icon={scanned ? <ArrowRight size={16} /> : <ScanSearch size={16} />}
              onClick={smartScan}
              disabled={scanning}
            >
              {scanned
                ? t("Choose what to clean", "选择清理项目")
                : t("Start a smart scan", "开始智能扫描")}
            </Button>
            <Button
              variant="primary"
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

        <div className="overview-specs">
          <span className="specs-kicker">{t("This machine", "本机配置")}</span>
          <SpecRow
            icon={<MonitorCog size={15} />}
            label={t("System", "操作系统")}
            value={specs.os}
            t={t}
          />
          <SpecRow
            icon={<CircuitBoard size={15} />}
            label={t("Motherboard", "主板")}
            value={specs.board}
            t={t}
          />
          <SpecRow
            icon={<Cpu size={15} />}
            label={t("Processor", "处理器")}
            value={specs.cpu}
            t={t}
          />
          <SpecRow
            icon={<MemoryStick size={15} />}
            label={t("Memory", "内存")}
            value={specs.memory}
            t={t}
          />
          <SpecRow
            icon={<HardDrive size={15} />}
            label={t("Primary disk", "主硬盘")}
            value={specs.disk}
            t={t}
          />
          <SpecRow
            icon={<Waypoints size={15} />}
            label={t("Graphics", "显卡")}
            value={specs.gpu}
            t={t}
          />
        </div>
      </section>

      {scanning ? (
        <ProgressCard
          progress={cleanup.progress ?? purge.progress}
          title={t("Looking for safe things to clean…", "正在查找可安全清理的项目…")}
          onCancel={() => void (cleanup.scanning ? cleanup.cancel() : purge.cancel())}
        />
      ) : null}

      {scanned && !scanning ? (
        <section className="overview-found">
          <SectionTitle
            title={t("What the scan found", "扫描发现")}
            subtitle={t(
              "Uncheck anything you want to keep.",
              "不想清理的项目可以随时取消勾选。",
            )}
            action={
              <Button variant="ghost" size="sm" onClick={() => onNavigate("space")}>
                {t("Open space cleaning", "打开空间清理")}
                <ChevronRight size={14} />
              </Button>
            }
          />
          {cleanup.result ? (
            <CategorySummary
              result={cleanup.result}
              selected={cleanup.selected}
              onToggle={cleanup.toggleCategory}
              onInspect={() => onNavigate("space")}
            />
          ) : null}
        </section>
      ) : null}

      {/* 指标行 30% / 42% / 28%：处理器+内存共用一框上下分布；存储每盘
          一行；日常维护三入口竖排放右列，与左两卡等高。独立的「日常维护」
          区块随此改版取消。 */}
      <section className="overview-metrics">
        <div className="metric-panel metric-combo">
          <div className="m-block">
            <div className="metric-panel-head">
              <span className="metric-icon tone-blue">
                <CircleGauge size={18} />
              </span>
              <div>
                <strong>{t("Processor", "处理器")}</strong>
                <span>
                  {snapshot
                    ? `${snapshot.cpu.physical_cores} ${t("cores", "核")} / ${snapshot.cpu.logical_cores} ${t("threads", "线程")}`
                    : t("Sampling…", "正在采样…")}
                </span>
              </div>
              <b>{Math.round(snapshot?.cpu.total ?? 0)}%</b>
            </div>
            <UsageBar used={snapshot?.cpu.total ?? 0} total={100} tone="blue" />
            <small>
              {snapshot?.cpu.brand || t("Processor details unavailable", "暂无处理器信息")}
            </small>
          </div>
          <div className="m-block">
            <div className="metric-panel-head">
              <span className="metric-icon tone-violet">
                <Boxes size={18} />
              </span>
              <div>
                <strong>{t("Memory", "内存")}</strong>
                <span>
                  {snapshot
                    ? `${formatMemory(snapshot.memory.used)} / ${formatMemory(memoryInstalled)}`
                    : t("Sampling…", "正在采样…")}
                </span>
              </div>
              <b>
                {Math.round(
                  snapshot ? percent(snapshot.memory.used, snapshot.memory.total) : 0,
                )}
                %
              </b>
            </div>
            <UsageBar
              used={snapshot ? percent(snapshot.memory.used, snapshot.memory.total) : 0}
              total={100}
              tone="violet"
            />
            <small>
              {memoryReserved > 0
                ? t(
                    `Hardware reserved (integrated graphics): ${formatMemory(memoryReserved)}`,
                    `硬件保留（含核显）：${formatMemory(memoryReserved)}`,
                  )
                : snapshot
                  ? t(
                      `${formatCount(snapshot.processes)} processes running`,
                      `${formatCount(snapshot.processes)} 个进程正在运行`,
                    )
                  : t("Reading activity", "正在读取活动")}
            </small>
          </div>
        </div>
        <div className="metric-panel metric-drives">
          <div className="metric-panel-head">
            <span className="metric-icon tone-green">
              <HardDrive size={18} />
            </span>
            <div>
              <strong>{t("Storage", "存储空间")}</strong>
              <span>
                {volumes.length > 0
                  ? t(
                      `${volumes.length} drives`,
                      `${volumes.length} 个分区`,
                    )
                  : t("Reading volumes…", "正在读取磁盘…")}
              </span>
            </div>
          </div>
          <div className="drive-list">
            {volumes.slice(0, 4).map((volume) => {
              const usedPercent = percent(volume.total - volume.free, volume.total);
              return (
                <div className="drive-row" key={volume.mount_point}>
                  <strong className="drive-name" title={volume.name || undefined}>
                    {driveLabel(volume)}
                  </strong>
                  <span className="drive-bar">
                    <UsageBar
                      used={usedPercent}
                      total={100}
                      tone={usedPercent > 85 ? "red" : "green"}
                    />
                  </span>
                  <span className="drive-info">
                    <b className="drive-pct">{Math.round(usedPercent)}%</b>
                    {t(
                      `total ${formatBytes(volume.total)} · ${formatBytes(volume.free)} free`,
                      `总 ${formatBytes(volume.total)} · 可用 ${formatBytes(volume.free)}`,
                    )}
                  </span>
                </div>
              );
            })}
          </div>
        </div>
        <div className="quick-col">
          <button className="quick-card tone-cyan" onClick={() => onNavigate("applications")}>
            <span className="quick-icon">
              <AppWindow size={22} />
            </span>
            <span className="quick-copy">
              <strong>{t("Uninstall apps", "卸载应用")}</strong>
              <small>{t("Remove apps and their leftovers", "连同残留一起彻底移除")}</small>
            </span>
            <ChevronRight size={17} />
          </button>
          <button className="quick-card tone-indigo" onClick={() => onOpenSpaceTab?.("big")}>
            <span className="quick-icon">
              <HardDrive size={22} />
            </span>
            <span className="quick-copy">
              <strong>{t("Large files", "大文件")}</strong>
              <small>{t("See what is taking up space", "查看空间都被什么占用")}</small>
            </span>
            <ChevronRight size={17} />
          </button>
          <button className="quick-card tone-violet" onClick={smartScan}>
            <span className="quick-icon">
              <Sparkles size={22} />
            </span>
            <span className="quick-copy">
              <strong>{t("Space cleaning", "空间清理")}</strong>
              <small>
                {t(
                  "Caches, logs and build artefacts in one scan",
                  "一次扫描清掉缓存、垃圾与构建产物",
                )}
              </small>
            </span>
            <ChevronRight size={17} />
          </button>
        </div>
      </section>
    </div>
  );
}
