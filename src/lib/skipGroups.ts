/* Why a skipped item stayed behind — the same taxonomy the 空间清理 result
 * view uses, now shared with the in-place clean banners on every page. */
export interface SkipGroup {
  key: string;
  title: [string, string];
  hint: [string, string];
  action?: "rescan";
}

export function skipGroupOf(reason: string): SkipGroup {
  const lower = reason.toLowerCase();
  if (lower.includes("running") || lower.includes("占用")) {
    return {
      key: "running",
      title: ["应用正在运行", "应用正在运行"],
      hint: ["退出这些应用后重新扫描，即可再次选中。", "退出这些应用后重新扫描，即可再次选中。"],
      action: "rescan",
    };
  }
  if (lower.includes("admin") || lower.includes("管理员")) {
    return {
      key: "admin",
      title: ["需要管理员权限", "需要管理员权限"],
      hint: [
        "以管理员身份重新启动 Squid 后再清理这批项目。",
        "以管理员身份重新启动 Squid 后再清理这批项目。",
      ],
      action: "rescan",
    };
  }
  if (lower.includes("已不存在") || lower.includes("no longer") || lower.includes("missing")) {
    return {
      key: "gone",
      title: ["扫描后已消失", "扫描后已消失"],
      hint: ["这些路径在清理开始前就不在了，无需处理。", "这些路径在清理开始前就不在了，无需处理。"],
    };
  }
  if (lower.includes("whitelist")) {
    return {
      key: "whitelist",
      title: ["命中白名单", "命中白名单"],
      hint: [
        "在 设置 → 白名单 中移除对应规则后才会被清理。",
        "在 设置 → 白名单 中移除对应规则后才会被清理。",
      ],
    };
  }
  return {
    key: "protected",
    title: ["受安全策略保护", "受安全策略保护"],
    hint: [
      "这些目录看似缓存，实际可能存有您自己的文件，Squid 不会删除。",
      "这些目录看似缓存，实际可能存有您自己的文件，Squid 不会删除。",
    ],
  };
}

/** "占用中 5 · 需要管理员 2" — the skipped-reason tally the amber banner
 * shows under its headline. */
export function skipTally(skipped: { reason: string }[]): string {
  const counts = new Map<string, number>();
  for (const row of skipped) {
    const group = skipGroupOf(row.reason);
    counts.set(group.title[1], (counts.get(group.title[1]) ?? 0) + 1);
  }
  return [...counts.entries()].map(([title, count]) => `${title} ${count}`).join(" · ");
}

/** 41 秒 / 1 分 12 秒 — elapsed labels for the done banner. */
export function formatDuration(ms: number): string {
  const seconds = Math.max(1, Math.round(ms / 1000));
  if (seconds < 60) return `${seconds} 秒`;
  return `${Math.floor(seconds / 60)} 分 ${seconds % 60} 秒`;
}
