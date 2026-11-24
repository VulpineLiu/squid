import {
  Archive,
  Disc3,
  Cloud,
  FileVideo,
  HardDrive,
  Lock,
  MapPin,
  Package,
  ShieldAlert,
  Trash2,
} from "lucide-react";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import * as apiClient from "../lib/api";
import { subscribeJobProgress } from "../lib/api";
import { useI18n } from "../i18n";
import { agoLabel, formatBytes, formatCount, messageFrom } from "../lib/format";
import {
  Button,
  Checkbox,
  EmptyState,
  InlineError,
  PageHeader,
  ProgressCard,
  useToast,
} from "../components/ui";
import type { JobProgress, LargeFile } from "../types";

const THRESHOLDS = [
  { value: 100 * 1024 * 1024, label: "≥100MB" },
  { value: 500 * 1024 * 1024, label: "≥500MB" },
  { value: 1024 * 1024 * 1024, label: "≥1GB" },
];

const CLASSES = [
  { id: "all", en: "All", zh: "全部", icon: HardDrive },
  { id: "video", en: "Video", zh: "视频", icon: FileVideo },
  { id: "archive", en: "Archives", zh: "归档", icon: Archive },
  { id: "installer", en: "Installers", zh: "安装包", icon: Package },
  { id: "image", en: "Disk images", zh: "镜像/VHD", icon: Disc3 },
];

const CLASS_ICON: Record<LargeFile["class"], typeof HardDrive> = {
  video: FileVideo,
  archive: Archive,
  installer: Package,
  image: Disc3,
  other: HardDrive,
};

/** 休眠判定沿用清理页口径：180 天无修改即值得标记。 */
const DORMANT_SECS = 180 * 86_400;

export function LargeFilesPage({
  embedded = false,
  onBadge,
}: {
  /** Rendered inside 空间清理 hub: page title hides, stats flow to onBadge. */
  embedded?: boolean;
  onBadge?: (value: string | null) => void;
} = {}) {
  const { t, lang } = useI18n();
  const { push } = useToast();
  const [files, setFiles] = useState<LargeFile[] | null>(null);
  const [scope, setScope] = useState<string[]>([]);
  const [threshold, setThreshold] = useState(500 * 1024 * 1024);
  const [klass, setKlass] = useState("all");
  const [hideCritical, setHideCritical] = useState(true);
  const [scanning, setScanning] = useState(false);
  const [deleting, setDeleting] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [progress, setProgress] = useState<JobProgress | null>(null);
  const [picked, setPicked] = useState<Set<string>>(new Set());
  const [confirming, setConfirming] = useState(false);
  const job = useRef<string | null>(null);

  const run = useCallback(async () => {
    if (scanning) return;
    const id = crypto.randomUUID?.() ?? String(Date.now());
    job.current = id;
    setScanning(true);
    setError(null);
    setFiles(null);
    setPicked(new Set());
    setProgress(null);
    const stop = await subscribeJobProgress(id, (event) => {
      if (event.event === "metered") {
        setProgress(event);
      } else if (event.event === "tick" || event.event === "phase") {
        setProgress(event);
      }
    });
    try {
      const found = await apiClient.api.largeFilesScan(
        id,
        scope,
        threshold,
        klass === "all" ? null : klass,
      );
      setFiles(found);
    } catch (reason) {
      if (!messageFrom(reason).toLowerCase().includes("cancel")) {
        setError(messageFrom(reason));
      }
    } finally {
      stop();
      setScanning(false);
      job.current = null;
    }
  }, [klass, scanning, scope, threshold]);

  const visible = useMemo(() => {
    if (!files) return null;
    return files.filter((file) => !hideCritical || !file.maybeMounted);
  }, [files, hideCritical]);

  const totalBytes = visible?.reduce((sum, file) => sum + file.size, 0) ?? 0;
  const pickedBytes = visible
    ?.filter((file) => picked.has(file.path))
    .reduce((sum, file) => sum + file.size, 0) ?? 0;
  const pickedCount = picked.size;

  // Hub badge: found files, or null before a scan.
  useEffect(() => {
    if (!embedded || !onBadge) return;
    onBadge(
      visible && visible.length > 0
        ? `${formatCount(visible.length)} 个 · ${formatBytes(totalBytes)}`
        : null,
    );
  }, [embedded, onBadge, totalBytes, visible]);

  const toggle = (path: string) => {
    setPicked((current) => {
      const next = new Set(current);
      if (next.has(path)) next.delete(path);
      else next.add(path);
      return next;
    });
  };

  const confirmDelete = async () => {
    const id = crypto.randomUUID?.() ?? String(Date.now());
    setDeleting(true);
    try {
      const outcome = await apiClient.api.largeFilesDelete(id, [...picked], "trash");
      setFiles((current) => current?.filter((f) => !picked.has(f.path)) ?? null);
      setPicked(new Set());
      setConfirming(false);
      push(
        lang === "zh"
          ? `已将 ${outcome.removed} 个大文件移入回收站（${formatBytes(outcome.freed)}），可在「历史记录」撤销`
          : `Moved ${outcome.removed} files to the Recycle Bin (${formatBytes(outcome.freed)})`,
        "success",
      );
    } catch (reason) {
      setError(messageFrom(reason));
    } finally {
      setDeleting(false);
    }
  };

  const now = Date.now();

  return (
    <div className={`page bigfiles-page ${embedded ? "is-embedded" : ""}`}>
      {embedded ? null : (
        <PageHeader
          title={t("Large files", "大文件")}
          description={t(
            "Everything heavy across your drives, with dormancy and lock hints before anything goes to the Recycle Bin.",
            "全盘范围的大文件清单，附休眠与占用提示；删除同样进回收站、可撤销。",
          )}
        />
      )}
      {error ? <InlineError message={error} onDismiss={() => setError(null)} /> : null}

      <section className="bigfiles-config">
        <span className="dup-config-label">{t("Scope", "范围")}</span>
        <div className="dup-seg" role="group">
          <button
            type="button"
            className={scope.length === 0 ? "is-active" : ""}
            onClick={() => setScope([])}
          >
            {t("All local drives", "全部本地磁盘")}
          </button>
          {["C:", "D:", "E:"].map((drive) => (
            <button
              key={drive}
              type="button"
              className={scope.length === 1 && scope[0] === `${drive}\\` ? "is-active" : ""}
              onClick={() => setScope([`${drive}\\`])}
            >
              {drive}
            </button>
          ))}
        </div>
        <span className="dup-config-label">{t("At least", "阈值")}</span>
        <div className="dup-seg" role="group">
          {THRESHOLDS.map((option) => (
            <button
              key={option.value}
              type="button"
              className={threshold === option.value ? "is-active" : ""}
              onClick={() => setThreshold(option.value)}
            >
              {option.label}
            </button>
          ))}
        </div>
        <span className="dup-config-label">{t("Type", "类型")}</span>
        <div className="dup-seg is-classes" role="group">
          {CLASSES.map((option) => {
            const Icon = option.icon;
            return (
              <button
                key={option.id}
                type="button"
                className={klass === option.id ? "is-active" : ""}
                onClick={() => setKlass(option.id)}
              >
                <Icon size={12} /> {lang === "zh" ? option.zh : option.en}
              </button>
            );
          })}
        </div>
        <Button variant="primary" onClick={() => void run()} disabled={scanning || deleting}>
          {scanning ? t("Scanning…", "扫描中…") : t("Scan", "开始扫描")}
        </Button>
      </section>

      {scanning ? (
        <ProgressCard
          progress={progress}
          title={t("Scanning for large files", "正在查找大文件")}
          onCancel={() => {
            if (job.current) void apiClient.api.cancelJob(job.current);
          }}
        />
      ) : null}

      {visible && visible.length === 0 && !scanning ? (
        <EmptyState
          icon={<HardDrive size={26} />}
          title={t("No files above the threshold", "没有超过阈值的大文件")}
          description={t(
            "Try a lower size floor or another scope.",
            "可以调低体积门槛或更换扫描范围再试。",
          )}
        />
      ) : null}

      {visible && visible.length > 0 ? (
        <>
          <section className="bigfiles-list-head">
            <div>
              <b>
                {formatCount(visible.length)} {t("large files", "个大文件")} ·{" "}
                <span className="is-accent">{formatBytes(totalBytes)}</span>
              </b>
              <small>
                {pickedCount > 0
                  ? t(
                      `${pickedCount} selected · ${formatBytes(pickedBytes)} → Recycle Bin`,
                      `已选 ${pickedCount} 项 · ${formatBytes(pickedBytes)} → 移入回收站`,
                    )
                  : t(
                      "Dormant files (180 days untouched) are tagged; disk images held by Hyper-V/WSL stay locked.",
                      "长期未修改（180 天+）的文件会带休眠标记；被 Hyper-V/WSL 使用的镜像会锁定。",
                    )}
              </small>
            </div>
            <label className="bigfiles-hide-critical">
              <Checkbox
                checked={hideCritical}
                onChange={() => setHideCritical((v) => !v)}
                label={t("Hide system-critical images", "隐藏系统关键镜像")}
              />
              <span>{t("Hide system-critical", "隐藏系统关键文件")}</span>
            </label>
            <Button
              variant="danger"
              disabled={pickedCount === 0 || deleting}
              onClick={() => setConfirming(true)}
            >
              <Trash2 size={14} />
              <span>{t("Clean selected", "清理已选")}</span>
            </Button>
          </section>
          {confirming ? (
            <section className="dup-confirm">
              <ShieldAlert size={15} />
              <div>
                <b>
                  {lang === "zh"
                    ? `将 ${pickedCount} 个大文件（${formatBytes(pickedBytes)}）移入回收站？`
                    : `Move ${pickedCount} files (${formatBytes(pickedBytes)}) to the Recycle Bin?`}
                </b>
                <small>
                  {t(
                    "Undo from History while the Recycle Bin holds them.",
                    "回收站清空前，可随时在「历史记录」撤销。",
                  )}
                </small>
              </div>
              <Button onClick={() => setConfirming(false)}>{t("Cancel", "取消")}</Button>
              <Button variant="danger" disabled={deleting} onClick={() => void confirmDelete()}>
                {deleting ? t("Cleaning…", "清理中…") : t("Confirm", "确认清理")}
              </Button>
            </section>
          ) : null}
          <ul className="bigfiles-list">
            {visible.map((file, index) => {
              const Icon = CLASS_ICON[file.class] ?? HardDrive;
              const dormant =
                file.modifiedMs > 0 && (now - file.modifiedMs) / 1000 > DORMANT_SECS;
              return (
                <li key={file.path} className={`bigfile-row ${picked.has(file.path) ? "is-picked" : ""}`}>
                  <Checkbox
                    checked={picked.has(file.path)}
                    onChange={() => toggle(file.path)}
                    label={file.displayPath}
                  />
                  <span className="bigfile-index">{String(index + 1).padStart(2, "0")}</span>
                  <span className="bigfile-icon">
                    <Icon size={15} />
                  </span>
                  <div className="bigfile-main">
                    <b>{file.path.split(/[\\/]/).pop()}</b>
                    <span className="bigfile-path">{file.displayPath}</span>
                  </div>
                  {dormant ? (
                    <span className="space-rule-dormant">
                      {agoLabel(file.modifiedMs, lang)} {t("untouched", "未修改")}
                    </span>
                  ) : null}
                  {file.cloudHint ? (
                    <span className="bigfile-cloud-chip">
                      <Cloud size={11} /> {t("cloud copy?", "网盘可能有副本？")}
                    </span>
                  ) : null}
                  {file.maybeMounted ? (
                    <span className="dup-lock-chip" title={t("May be in use by Hyper-V/WSL", "可能正被 Hyper-V/WSL 使用")}>
                      <Lock size={11} /> {t("check in use", "占用检查")}
                    </span>
                  ) : null}
                  <span className="bigfile-size">{formatBytes(file.size)}</span>
                  <button
                    type="button"
                    className="dup-copy-reveal"
                    onClick={() => void apiClient.api.revealPath(file.path).catch(() => undefined)}
                    title={t("Reveal in Explorer", "在文件夹中显示")}
                  >
                    <MapPin size={12} />
                    {t("Reveal", "定位")}
                  </button>
                </li>
              );
            })}
          </ul>
        </>
      ) : null}
    </div>
  );
}
