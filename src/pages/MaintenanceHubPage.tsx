import { EyeOff, SlidersHorizontal, Wrench } from "lucide-react";
import { useEffect, useState } from "react";
import { useI18n } from "../i18n";
import { PageHeader } from "../components/ui";
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

export function MaintenanceHubPage({ initialTab = "repair" }: { initialTab?: MaintTab }) {
  const { t } = useI18n();
  const [tab, setTab] = useState<MaintTab>(initialTab);
  useEffect(() => setTab(initialTab), [initialTab]);
  const [badges, setBadges] = useState<Partial<Record<MaintTab, string | null>>>({});
  const report = (id: MaintTab) => (value: string | null) =>
    setBadges((current) => (current[id] === value ? current : { ...current, [id]: value }));

  return (
    <div className="page hub-page">
      <PageHeader
        title={t("System maintenance", "系统维护")}
      />
      <div className="hub-tabs" role="tablist">
        {TABS.map((item) => {
          const Icon = item.icon;
          const badge = badges[item.id];
          return (
            <button
              key={item.id}
              type="button"
              role="tab"
              aria-selected={tab === item.id}
              className={`hub-tab ${tab === item.id ? "is-active" : ""}`}
              onClick={() => setTab(item.id)}
            >
              <Icon size={15} />
              {t(item.en, item.zh)}
              {badge ? <b>{badge}</b> : null}
            </button>
          );
        })}
      </div>
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
