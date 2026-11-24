import { CopyX, Eraser, FileArchive } from "lucide-react";
import { useEffect, useState } from "react";
import { useI18n } from "../i18n";
import { PageHeader } from "../components/ui";
import { SpacePage } from "./SpacePage";
import { DuplicateFilesPage } from "./DuplicateFilesPage";
import { LargeFilesPage } from "./LargeFilesPage";
import type { Bootstrap, DeleteMode } from "../types";
import type { ScanWorkspace } from "../hooks/useScanWorkspace";

/* 空间清理 hub（方案①）：垃圾清理 / 重复文件 / 大文件 三页签。
 * 三个面板全部保持挂载（display 切换），扫描结果与勾选状态在切页签
 * 时不丢失；每个面板自己决定何时扫描（懒扫描），页签徽标由面板回报。 */

export type SpaceTab = "junk" | "dups" | "big";

const TABS: { id: SpaceTab; icon: typeof Eraser; en: string; zh: string }[] = [
  { id: "junk", icon: Eraser, en: "Junk cleaning", zh: "垃圾清理" },
  { id: "dups", icon: CopyX, en: "Duplicate files", zh: "重复文件" },
  { id: "big", icon: FileArchive, en: "Large files", zh: "大文件" },
];

export function SpaceHubPage({
  cleanup,
  purge,
  bootstrap,
  deleteMode,
  onHistory,
  initialTab = "junk",
}: {
  cleanup: ScanWorkspace;
  purge: ScanWorkspace;
  bootstrap: Bootstrap;
  deleteMode: DeleteMode;
  onHistory: () => void;
  /** Deep-link from the overview quick cards (重复文件 / 大文件). */
  initialTab?: SpaceTab;
}) {
  const { t } = useI18n();
  const [tab, setTab] = useState<SpaceTab>(initialTab);
  // A fresh deep-link wins over the last manually chosen tab.
  useEffect(() => setTab(initialTab), [initialTab]);
  const [badges, setBadges] = useState<Partial<Record<SpaceTab, string | null>>>({});
  const report = (id: SpaceTab) => (value: string | null) =>
    setBadges((current) => (current[id] === value ? current : { ...current, [id]: value }));

  return (
    <div className="page hub-page">
      <PageHeader
        title={t("Space cleaning", "空间清理")}
        description={t(
          "Junk, duplicates, heavyweights — three views of the same disk. Everything deleted lands in the Recycle Bin, undoable.",
          "垃圾、重复、大块头——三个视角看清同一块磁盘，删除都走回收站、可撤销。",
        )}
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
      <div className="hub-pane" style={{ display: tab === "junk" ? undefined : "none" }}>
        <SpacePage
          embedded
          onBadge={report("junk")}
          cleanup={cleanup}
          purge={purge}
          bootstrap={bootstrap}
          deleteMode={deleteMode}
          onHistory={onHistory}
        />
      </div>
      <div className="hub-pane" style={{ display: tab === "dups" ? undefined : "none" }}>
        <DuplicateFilesPage embedded onBadge={report("dups")} />
      </div>
      <div className="hub-pane" style={{ display: tab === "big" ? undefined : "none" }}>
        <LargeFilesPage embedded onBadge={report("big")} />
      </div>
    </div>
  );
}
