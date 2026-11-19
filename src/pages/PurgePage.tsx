import {
  AlertTriangle,
  FolderOpen,
  FolderSearch,
  GitBranch,
  ScanSearch,
  ShieldAlert,
} from "lucide-react";
import { open } from "@tauri-apps/plugin-dialog";
import { useState } from "react";
import { useI18n } from "../i18n";
import type { Bootstrap, DeleteMode } from "../types";
import type { ScanWorkspace } from "../hooks/useScanWorkspace";
import { CleanupInventory } from "../components/CleanupView";
import { Button, PageHeader, Toggle } from "../components/ui";

export function PurgePage({
  workspace,
  bootstrap,
  deleteMode,
  onHistory,
}: {
  workspace: ScanWorkspace;
  bootstrap: Bootstrap;
  deleteMode: DeleteMode;
  onHistory: () => void;
}) {
  const { t } = useI18n();
  const [roots, setRoots] = useState(bootstrap.settings.purgeRoots.join("\n"));
  const [minAge, setMinAge] = useState(bootstrap.settings.purgeMinAgeDays);
  const [force, setForce] = useState(false);

  const browse = async () => {
    const chosen = await open({
      directory: true,
      multiple: true,
      title: t("Choose project folders to inspect", "选择要检查的项目目录"),
    });
    if (!chosen) return;
    const picked = (Array.isArray(chosen) ? chosen : [chosen]).filter(
      (path) => !roots.split(/\r?\n/).some((line) => line.trim() === path),
    );
    if (picked.length > 0) {
      setRoots((current) =>
        current.trim().length > 0
          ? `${current.trimEnd()}\n${picked.join("\n")}`
          : picked.join("\n"),
      );
    }
  };

  const run = () =>
    void workspace.runScan({
      roots: roots
        .split(/\r?\n/)
        .map((root) => root.trim())
        .filter(Boolean),
      minAgeDays: minAge > 0 ? minAge : null,
      force,
    });

  return (
    <div className="page">
      <PageHeader
        eyebrow={t("Deep Clean", "深度清理")}
        title={t("Find the space normal cleaning misses.", "找到普通清理遗漏的空间。")}
        description={t(
          "Search projects for rebuildable build folders and dependency caches. Git history and source files are never selected.",
          "在项目中查找可重建的构建目录与依赖缓存。Git 历史和源文件绝不会被选中。",
        )}
        actions={
          <Button variant="primary" icon={<ScanSearch size={16} />} onClick={run}>
            {t("Start deep scan", "开始深度扫描")}
          </Button>
        }
      />

      {workspace.error && !workspace.scanning ? (
        <div className="error-banner">{workspace.error}</div>
      ) : null}

      <div className="deep-options">
        <div className="deep-option-main">
          <label className="field-label" htmlFor="purge-roots">
            {t("Project folders to inspect", "要检查的项目目录")}
            <button
              type="button"
              className="text-button"
              style={{ marginLeft: 10 }}
              onClick={() => void browse()}
            >
              <FolderOpen size={13} />
              {t("Browse…", "浏览…")}
            </button>
          </label>
          <textarea
            id="purge-roots"
            value={roots}
            onChange={(event) => setRoots(event.target.value)}
            placeholder={t(
              "One folder per line. Leave empty to use your home folder.",
              "每行一个目录；留空则使用用户主目录。",
            )}
            rows={3}
          />
          <span className="field-help">
            {t(
              "Squid uses these as search roots, not as delete targets.",
              "这些路径只用于查找，不会作为删除目标。",
            )}
          </span>
        </div>
        <div className="deep-option-side">
          <label className="field-label" htmlFor="purge-age">
            {t("Only files older than", "仅处理早于")}
          </label>
          <div className="number-field">
            <input
              id="purge-age"
              type="number"
              min={0}
              max={3650}
              value={minAge}
              onChange={(event) => setMinAge(Math.max(0, Number(event.target.value) || 0))}
            />
            <span>{t("days", "天")}</span>
          </div>
          <div className="option-toggle-row">
            <div>
              <strong>{t("Force mode", "强制模式")}</strong>
              <span>{t("Include folders inside a Git work tree", "包含 Git 工作树内的目录")}</span>
            </div>
            <Toggle
              checked={force}
              onChange={setForce}
              label={t("Force mode", "强制模式")}
            />
          </div>
        </div>
      </div>

      <div className="safety-strip">
        <GitBranch size={18} />
        <div>
          <strong>{t("Git-aware by default", "默认感知 Git 状态")}</strong>
          <span>
            {t(
              "Uncommitted work is protected. Force mode is for throwaway build folders only.",
              "存在未提交改动时会自动保护。强制模式仅建议用于可丢弃的构建目录。",
            )}
          </span>
        </div>
        <ShieldAlert size={19} />
      </div>
      <div className="warning-strip">
        <AlertTriangle size={18} />
        <span>
          {t(
            "node_modules, target, dist and similar folders can be regenerated, but rebuilding may take time or require network access.",
            "node_modules、target、dist 等目录可以重新生成，但重新构建可能需要时间或网络。",
          )}
        </span>
      </div>

      <CleanupInventory
        workspace={workspace}
        defaultMode={deleteMode}
        onHistory={onHistory}
      />

      {!workspace.result && !workspace.scanning && !workspace.summary ? (
        <div className="empty-inline">
          <FolderSearch size={22} />
          <span>
            {t(
              "No deep scan yet. Choose your project roots above and start when ready.",
              "尚未进行深度扫描。选择项目目录后即可开始。",
            )}
          </span>
        </div>
      ) : null}
    </div>
  );
}
