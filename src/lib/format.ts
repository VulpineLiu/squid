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

/** RAM is sold in binary units, so 16 GB should not read as 17.2 GB. */
export function formatMemoryBytes(value: number, fractionDigits = 1): string {
  if (!Number.isFinite(value) || value <= 0) return "0 B";
  const units = ["B", "KB", "MB", "GB", "TB", "PB"];
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
