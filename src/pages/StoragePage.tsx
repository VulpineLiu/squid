import {
  ArrowLeft,
  ChevronRight,
  ExternalLink,
  FileArchive,
  FolderOpen,
  FolderSearch,
  HardDrive,
  LoaderCircle,
  RefreshCw,
  ScanSearch,
} from "lucide-react";
import { useEffect, useMemo, useState } from "react";
import { open } from "@tauri-apps/plugin-dialog";
import * as apiClient from "../lib/api";
import { useI18n } from "../i18n";
import {
  basename,
  formatBytes,
  formatCount,
  percent,
  shortPath,
} from "../lib/format";
import type { Bootstrap, StorageNode } from "../types";
import { useStorageWorkspace } from "../hooks/useStorageWorkspace";
import {
  Button,
  InlineError,
  PageHeader,
  ProgressCard,
  SectionTitle,
} from "../components/ui";

const BAR_TONES = ["primary", "neutral"] as const;

function nodeSize(node: { allocated: number; logical: number }): number {
  return node.allocated || node.logical;
}

function parentDirectory(path: string): string | null {
  const normalized = path.replace(/[\\/]+$/, "");
  const index = Math.max(normalized.lastIndexOf("/"), normalized.lastIndexOf("\\"));
  if (index <= 0) return null;
  if (/^[A-Za-z]:[\\/]/.test(normalized) && index === 2) {
    return `${normalized.slice(0, 2)}\\`;
  }
  return normalized.slice(0, index);
}

export function StoragePage({ bootstrap }: { bootstrap: Bootstrap }) {
  const { t } = useI18n();
  const workspace = useStorageWorkspace();
  const [pathInput, setPathInput] = useState("");
  const [manualPath, setManualPath] = useState(false);

  useEffect(() => {
    if (workspace.node && !manualPath) setPathInput(workspace.node.path);
  }, [manualPath, workspace.node]);

  const children = useMemo(
    () =>
      workspace.node
        ? [...workspace.node.children].sort((a, b) => nodeSize(b) - nodeSize(a))
        : [],
    [workspace.node],
  );
  const nodeTotal = workspace.node
    ? Math.max(nodeSize(workspace.node), 1)
    : 1;
  const visibleShare = children.reduce((sum, child) => sum + nodeSize(child), 0);
  const rest = percent(visibleShare, nodeTotal);

  const explore = (path?: string | null) => {
    setManualPath(false);
    void workspace.explore(path);
  };

  const chooseFolder = async () => {
    const chosen = await open({
      directory: true,
      multiple: false,
      title: t("Choose a folder to analyse", "选择要分析的文件夹"),
    });
    if (typeof chosen === "string") explore(chosen);
  };

  const reveal = (path: string) => {
    void apiClient.api.revealPath(path).catch(() => undefined);
  };

  const drill = (node: StorageNode) => {
    if (!node.is_dir || node.path === workspace.node?.path) return;
    explore(node.path);
  };

  const parent = workspace.node ? parentDirectory(workspace.node.path) : null;

  return (
    <div className="page storage-page">
      <PageHeader
        title={t("See space clearly.", "看清每一寸空间。")}
        description={t(
          "Drill into folders, find the heavyweight files, and open anything in Finder.",
          "逐层查看文件夹，找到占用最大的文件，并直接在访达中打开。",
        )}
        actions={
          <>
            {parent ? (
              <Button variant="ghost" icon={<ArrowLeft size={15} />} onClick={() => explore(parent)}>
                {t("Up", "上一级")}
              </Button>
            ) : null}
            <Button variant="secondary" icon={<FolderOpen size={15} />} onClick={() => void chooseFolder()}>
              {t("Choose folder", "选择文件夹")}
            </Button>
          </>
        }
      />

      <div className="storage-pathbar">
        <span className="pathbar-icon">
          <HardDrive size={17} />
        </span>
        <input
          value={pathInput}
          onChange={(event) => {
            setPathInput(event.target.value);
            setManualPath(true);
          }}
          onKeyDown={(event) => {
            if (event.key === "Enter" && pathInput.trim()) explore(pathInput.trim());
          }}
          spellCheck={false}
          aria-label={t("Folder path", "文件夹路径")}
        />
        <Button
          variant="primary"
          size="sm"
          icon={<ScanSearch size={14} />}
          onClick={() => explore(pathInput.trim() || null)}
          disabled={workspace.exploring || workspace.scanningFiles}
        >
          {t("Analyse", "分析")}
        </Button>
      </div>

      {workspace.error ? (
        <InlineError message={workspace.error} onDismiss={workspace.clearError} />
      ) : null}

      {workspace.exploring ? (
        <ProgressCard
          progress={workspace.progress}
          title={t("Measuring folders…", "正在测量文件夹…")}
          onCancel={() => void workspace.cancel()}
        />
      ) : null}

      {workspace.node && !workspace.exploring ? (
        <>
          <section className="storage-overview">
            <div className="storage-overview-head">
              <div>
                <span className="field-label">{t("Current folder", "当前文件夹")}</span>
                <h2 title={workspace.node.path}>{shortPath(workspace.node.path, 72)}</h2>
              </div>
              <div className="storage-total">
                <strong>{formatBytes(nodeSize(workspace.node))}</strong>
                <span>
                  {t(
                    `${formatCount(workspace.node.files)} files · ${formatCount(
                      workspace.node.dirs,
                    )} folders`,
                    `${formatCount(workspace.node.files)} 个文件 · ${formatCount(
                      workspace.node.dirs,
                    )} 个文件夹`,
                  )}
                </span>
              </div>
            </div>

            {children.length > 0 ? (
              <>
                <div className="storage-map" aria-label={t("Folder distribution", "文件夹占比")}>
                  {children.map((child, index) => (
                    <span
                      key={`${child.path}-${index}`}
                      className={`tone-${BAR_TONES[index % BAR_TONES.length]}`}
                      style={{ width: `${percent(nodeSize(child), nodeTotal)}%` }}
                      title={`${basename(child.path)} · ${formatBytes(nodeSize(child))}`}
                    />
                  ))}
                  <span className="storage-map-rest" />
                </div>
                {workspace.node.folded > 0 ? (
                  <p className="storage-folded">
                    {t(
                      `${workspace.node.folded} smaller folders (${formatBytes(
                        workspace.node.folded_size,
                      )}) are folded into the remainder.`,
                      `${workspace.node.folded} 个较小文件夹（${formatBytes(
                        workspace.node.folded_size,
                      )}）已合并显示。`,
                    )}
                  </p>
                ) : null}

                <div className="storage-children">
                  {children.map((child, index) => {
                    const share = percent(nodeSize(child), nodeTotal);
                    return (
                      <button
                        key={`${child.path}-${index}`}
                        className={`storage-child ${
                          child.is_dir && child.path !== workspace.node?.path
                            ? "is-navigable"
                            : ""
                        }`}
                        onClick={() => drill(child)}
                      >
                        <span className={`storage-child-dot tone-${BAR_TONES[index % BAR_TONES.length]}`} />
                        <span className="storage-child-icon">
                          {child.is_dir ? <FolderOpen size={19} /> : <FileArchive size={19} />}
                        </span>
                        <span className="storage-child-copy">
                          <strong>{basename(child.path)}</strong>
                          <small title={child.path}>
                            {shortPath(child.path, 62)}
                            {!child.readable ? ` · ${t("partially unreadable", "部分不可读")}` : ""}
                          </small>
                          <span className="storage-mini-track">
                            <span
                              className={`tone-${BAR_TONES[index % BAR_TONES.length]}`}
                              style={{ width: `${share}%` }}
                            />
                          </span>
                        </span>
                        <span className="storage-child-size">
                          <strong>{formatBytes(nodeSize(child))}</strong>
                          <small>{share.toFixed(1)}%</small>
                        </span>
                        <span className="storage-child-actions">
                          <span
                            role="button"
                            tabIndex={0}
                            className="row-reveal"
                            title={t("Show in Finder", "在访达中显示")}
                            onClick={(event) => {
                              event.stopPropagation();
                              reveal(child.path);
                            }}
                            onKeyDown={(event) => {
                              if (event.key === "Enter") reveal(child.path);
                            }}
                          >
                            <ExternalLink size={14} />
                          </span>
                          {child.is_dir && child.path !== workspace.node?.path ? (
                            <ChevronRight size={15} />
                          ) : null}
                        </span>
                      </button>
                    );
                  })}
                  {rest < 100 ? (
                    <div className="storage-child is-muted">
                      <span className="storage-child-dot" />
                      <span className="storage-child-icon">
                        <BoxesIcon />
                      </span>
                      <span className="storage-child-copy">
                        <strong>{t("Other folders", "其他文件夹")}</strong>
                        <small>{t("Not among the largest entries", "不在最大项目之列")}</small>
                      </span>
                      <span className="storage-child-size">
                        <strong>
                          {formatBytes(Math.max(0, nodeSize(workspace.node) - visibleShare))}
                        </strong>
                        <small>{(100 - rest).toFixed(1)}%</small>
                      </span>
                    </div>
                  ) : null}
                </div>
              </>
            ) : (
              <div className="empty-inline">
                <FolderSearch size={21} />
                <span>{t("This folder has no measurable children.", "此文件夹没有可测量的子项。")}</span>
              </div>
            )}
          </section>

          <section className="storage-large">
            <SectionTitle
              title={t("Largest files", "最大文件")}
              subtitle={t(
                "Over 1 MB in the current folder, largest first.",
                "当前文件夹中大于 1 MB 的文件，从大到小。",
              )}
              action={
                <Button
                  variant="secondary"
                  size="sm"
                  icon={
                    workspace.scanningFiles ? (
                      <LoaderCircle size={14} className="spin" />
                    ) : (
                      <ScanSearch size={14} />
                    )
                  }
                  onClick={() =>
                    workspace.files.length > 0
                      ? workspace.clearFiles()
                      : void workspace.largestFiles(workspace.node!.path)
                  }
                  disabled={workspace.exploring || workspace.scanningFiles}
                >
                  {workspace.files.length > 0
                    ? t("Hide", "隐藏")
                    : t("Find large files", "查找大文件")}
                </Button>
              }
            />
            {workspace.scanningFiles ? (
              <ProgressCard
                progress={workspace.progress}
                title={t("Searching for the largest files…", "正在查找最大文件…")}
                compact
                onCancel={() => void workspace.cancel()}
              />
            ) : null}
            {workspace.files.length > 0 ? (
              <div className="large-file-list">
                {workspace.files.map((file, index) => (
                  <div className="large-file-row" key={`${file.path}-${index}`}>
                    <span className="large-file-rank">{String(index + 1).padStart(2, "0")}</span>
                    <span className="large-file-icon">
                      <FileArchive size={16} />
                    </span>
                    <span className="large-file-copy">
                      <strong>{basename(file.path)}</strong>
                      <small title={file.path}>{shortPath(file.path, 76)}</small>
                    </span>
                    <span className="large-file-size">
                      {formatBytes(file.allocated || file.logical)}
                    </span>
                    <button className="row-reveal" onClick={() => reveal(file.path)}>
                      <ExternalLink size={14} />
                    </button>
                  </div>
                ))}
              </div>
            ) : null}
          </section>
        </>
      ) : null}

      {!workspace.node && !workspace.exploring ? (
        <button className="storage-start" onClick={() => explore(null)}>
          <span>
            <RefreshCw size={25} />
          </span>
          <strong>{t("Start with your home folder", "从用户主目录开始")}</strong>
          <small>{bootstrap.settings.purgeRoots[0] || t("Home", "用户主目录")}</small>
        </button>
      ) : null}
    </div>
  );
}

function BoxesIcon() {
  return (
    <svg width="18" height="18" viewBox="0 0 24 24" fill="none" aria-hidden="true">
      <path d="m12 3 8 4.5v9L12 21l-8-4.5v-9L12 3Z" stroke="currentColor" strokeWidth="1.7" />
      <path d="m4.5 7.8 7.5 4.3 7.5-4.3M12 12.1V21" stroke="currentColor" strokeWidth="1.7" />
    </svg>
  );
}
