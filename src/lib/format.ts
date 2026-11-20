export function formatBytes(value: number, fractionDigits = 1): string {
  if (!Number.isFinite(value) || value <= 0) return "0 B";
  const units = ["B", "KB", "MB", "GB", "TB", "PB"];
  let remaining = value;
  let unit = 0;
  while (remaining >= 1000 && unit < units.length - 1) {
    remaining /= 1000;
    unit += 1;
  }
  if (unit === 0) return `${Math.round(remaining)} B`;
  const digits = remaining >= 100 ? 0 : fractionDigits;
  return `${remaining.toFixed(digits)} ${units[unit]}`;
}

/// RAM is sold in powers of two (a "32 GB" stick holds 32 GiB); disks are not.
/// Memory readouts must divide by 1024 or a 32 GiB machine shows "34.4 GB".
export function formatMemory(value: number, fractionDigits = 1): string {
  if (!Number.isFinite(value) || value <= 0) return "0 B";
  const units = ["B", "KB", "MB", "GB", "TB"];
  let remaining = value;
  let unit = 0;
  while (remaining >= 1024 && unit < units.length - 1) {
    remaining /= 1024;
    unit += 1;
  }
  if (unit === 0) return `${Math.round(remaining)} B`;
  const digits = remaining >= 100 ? 0 : fractionDigits;
  return `${remaining.toFixed(digits)} ${units[unit]}`;
}

export function formatBytesExact(value: number): string {
  if (value <= 0) return "0 B";
  const formatted = formatBytes(value, 2);
  return `${formatted} (${Math.round(value).toLocaleString()} bytes)`;
}

export function formatRate(value: number): string {
  return `${formatBytes(value)}/s`;
}

export function formatCount(value: number): string {
  return Math.max(0, Math.round(value)).toLocaleString();
}

export function formatDate(iso: string, language: "zh" | "en"): string {
  const date = new Date(iso);
  if (Number.isNaN(date.getTime())) return iso;
  return new Intl.DateTimeFormat(language === "zh" ? "zh-CN" : "en-US", {
    month: "short",
    day: "numeric",
    hour: "2-digit",
    minute: "2-digit",
  }).format(date);
}

export function formatUptime(seconds: number, language: "zh" | "en"): string {
  const days = Math.floor(seconds / 86_400);
  const hours = Math.floor((seconds % 86_400) / 3_600);
  const minutes = Math.floor((seconds % 3_600) / 60);
  if (language === "zh") {
    if (days > 0) return `${days} 天 ${hours} 小时`;
    if (hours > 0) return `${hours} 小时 ${minutes} 分钟`;
    return `${minutes} 分钟`;
  }
  if (days > 0) return `${days}d ${hours}h`;
  if (hours > 0) return `${hours}h ${minutes}m`;
  return `${minutes}m`;
}

export function percent(part: number, total: number): number {
  if (!Number.isFinite(part) || !Number.isFinite(total) || total <= 0) return 0;
  return Math.max(0, Math.min(100, (part / total) * 100));
}

export function shortPath(path: string, length = 48): string {
  if (path.length <= length) return path;
  const head = Math.max(8, Math.floor(length * 0.35));
  const tail = Math.max(10, length - head - 1);
  return `${path.slice(0, head)}…${path.slice(-tail)}`;
}

export function basename(path: string): string {
  const pieces = path.split(/[\\/]/).filter(Boolean);
  return pieces[pieces.length - 1] || path;
}

export function messageFrom(error: unknown): string {
  if (typeof error === "string") return error;
  if (error instanceof Error) return error.message;
  try {
    return JSON.stringify(error);
  } catch {
    return String(error);
  }
}

export function isCancelled(error: unknown): boolean {
  const text = messageFrom(error).toLowerCase();
  return text.includes("cancel") || text.includes("取消");
}

/* "最后写入 3 个月前" / "written into 3 months ago" — the Mole decision
 * aid: size alone is not actionable, dormancy is. */
export function ageLabel(secs: number, lang: "zh" | "en"): string {
  const days = Math.floor(secs / 86_400);
  if (lang === "zh") {
    if (days < 1) return "今天有改动";
    if (days < 30) return `${days} 天未修改`;
    if (days < 365) return `${Math.floor(days / 30)} 个月未修改`;
    return `${(days / 365).toFixed(days < 730 ? 1 : 0)} 年未修改`;
  }
  if (days < 1) return "written today";
  if (days < 30) return `${days} days untouched`;
  if (days < 365) return `${Math.floor(days / 30)} months untouched`;
  return `${(days / 365).toFixed(days < 730 ? 1 : 0)} years untouched`;
}

/* "12 分钟前" for cache freshness lines. */
export function agoLabel(ms: number, lang: "zh" | "en"): string {
  const mins = Math.floor(ms / 60_000);
  if (lang === "zh") {
    if (mins < 1) return "刚刚";
    if (mins < 60) return `${mins} 分钟前`;
    return `${Math.floor(mins / 60)} 小时前`;
  }
  if (mins < 1) return "just now";
  if (mins < 60) return `${mins} min ago`;
  return `${Math.floor(mins / 60)} h ago`;
}

/* Throttled "128,431 文件 · 38.2 GB" live scan counter (A①). */
export function countLabel(files: number, bytes: number, lang: "zh" | "en"): string {
  const f = formatThousands(files);
  const b = formatBytes(bytes);
  return lang === "zh" ? `已统计 ${f} 个文件 · ${b}` : `${f} files · ${b} measured`;
}

function formatThousands(value: number): string {
  return Math.round(value)
    .toString()
    .replace(/\B(?=(\d{3})+(?!\d))/g, ",");
}
