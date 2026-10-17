const UNITS = ["B", "KB", "MB", "GB", "TB"];

export function formatBytes(bytes: number, digits = 1): string {
  if (bytes < 1024) return `${bytes} B`;
  let v = bytes;
  let u = 0;
  while (v >= 1024 && u < UNITS.length - 1) {
    v /= 1024;
    u++;
  }
  return `${v.toFixed(v >= 100 ? 0 : digits)} ${UNITS[u]}`;
}

export function formatCount(n: number): string {
  return n.toLocaleString("en-US");
}

export function formatPercent(part: number, whole: number): string {
  if (whole <= 0) return "0%";
  const p = (part / whole) * 100;
  return p >= 10 ? `${p.toFixed(0)}%` : `${p.toFixed(1)}%`;
}

/** "just now", "5 min ago", "2 h ago", "3 days ago", else a date. */
export function formatAgo(ms: number, now = Date.now()): string {
  const s = Math.max(0, (now - ms) / 1000);
  if (s < 60) return "just now";
  if (s < 3600) return `${Math.round(s / 60)} min ago`;
  if (s < 86_400) return `${Math.round(s / 3600)} h ago`;
  if (s < 14 * 86_400) return `${Math.round(s / 86_400)} days ago`;
  return formatDate(ms);
}

export function formatDate(ms: number): string {
  return new Date(ms).toLocaleDateString("en-GB", { day: "2-digit", month: "short", year: "numeric" });
}
