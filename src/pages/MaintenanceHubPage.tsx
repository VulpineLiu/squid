import { EyeOff, SlidersHorizontal, Wrench } from "lucide-react";
import { useEffect, useState } from "react";
import { useI18n } from "../i18n";
import { EmptyState, PageHeader, Tabs } from "../components/ui";
import { MaintenancePage } from "./MaintenancePage";
import { PrivacyPage } from "./PrivacyPage";
import { OptimizationPage } from "./OptimizationPage";

/* 系统维护 hub（方案②）：一键修复 / 隐私清理 / 系统优化 三页签。
 * 面板全部保持挂载（display 切换），修复任务运行中切页签不会打断
 * 日志流；徽标由面板回报（隐私=痕迹数、优化=已更改数）。 */

export type MaintTab = "repair" | "privacy" | "optimize";

const TABS: { id: MaintTab; icon: typeof Wrench; en: string; zh: string }[] = [
  { id: "repair", icon: Wrench, en: "One-click repairs", zh: "一键修复" },
  { id: "privacy", icon: EyeOff, en: "Privacy cleaning", zh: "隐私清理" },
  { id: "optimize", icon: SlidersHorizontal, en: "System optimisation", zh: "系统优化" },
];

export function MaintenanceHubPage({
  initialTab = "repair",
  platform,
}: {
  initialTab?: MaintTab;
  platform?: string;
}) {
  const { t } = useI18n();
  const [tab, setTab] = useState<MaintTab>(initialTab);
  useEffect(() => setTab(initialTab), [initialTab]);
  const [badges, setBadges] = useState<Partial<Record<MaintTab, string | null>>>({});
  const report = (id: MaintTab) => (value: string | null) =>
    setBadges((current) => (current[id] === value ? current : { ...current, [id]: value }));

  // Repairs, privacy traces and optimisation switches are all Windows
  // registry / command surfaces. Say so instead of offering buttons that
  // can only answer "not available on this platform".
  if (platform !== "windows") {
    return (
      <div className="page hub-page">
        <PageHeader title={t("System maintenance", "系统维护")} />
        <EmptyState
          icon={<Wrench size={30} />}
          title={t("Windows only", "目前仅支持 Windows")}
          description={t(
            "Repairs, privacy traces and optimisation switches rely on Windows services and the registry. macOS keeps the security scan, startup items and cleanup tools.",
            "一键修复、隐私清理与系统优化依赖 Windows 服务与注册表。macOS 版保留安全扫描、启动项与清理工具。",
          )}
        />
      </div>
    );
  }

  return (
    <div className="page hub-page">
      <PageHeader
        title={t("System maintenance", "系统维护")}
      />
      <Tabs
        label={t("System maintenance sections", "系统维护分区")}
        value={tab}
        onChange={setTab}
        items={TABS.map((item) => {
          const Icon = item.icon;
          return {
            id: item.id,
            label: t(item.en, item.zh),
            icon: <Icon size={15} />,
            badge: badges[item.id] ?? undefined,
          };
        })}
      />
      <div className="hub-pane" style={{ display: tab === "repair" ? undefined : "none" }}>
        <MaintenancePage embedded onBadge={report("repair")} />
      </div>
      <div className="hub-pane" style={{ display: tab === "privacy" ? undefined : "none" }}>
        <PrivacyPage embedded onBadge={report("privacy")} />
      </div>
      <div className="hub-pane" style={{ display: tab === "optimize" ? undefined : "none" }}>
        <OptimizationPage embedded onBadge={report("optimize")} />
      </div>
    </div>
  );
}
