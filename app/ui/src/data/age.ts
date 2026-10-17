// Age bands for the treemap's "Age" colour mode and the Age insights. Must match
// `AGE_BANDS_DAYS` in crates/havespace-core/src/analysis.rs.

export interface AgeBandDef {
  /** Lower bound in days (inclusive). */
  minDays: number;
  label: string;
  /** Linear-ish sRGB 0..1, like CATEGORY_COLORS. */
  color: [number, number, number];
}

/** Fresh data in cool colours, warming up as it ages: the warmest tiles are what nobody has touched. */
export const AGE_BANDS: AgeBandDef[] = [
  { minDays: 0, label: "Under a month", color: [0.13, 0.83, 0.93] },
  { minDays: 30, label: "1–6 months", color: [0.06, 0.73, 0.51] },
  { minDays: 182, label: "6–12 months", color: [0.52, 0.8, 0.09] },
  { minDays: 365, label: "1–2 years", color: [0.96, 0.62, 0.04] },
  { minDays: 730, label: "2–5 years", color: [0.96, 0.42, 0.18] },
  { minDays: 1826, label: "5+ years", color: [0.96, 0.25, 0.37] },
];

export const AGE_UNKNOWN_COLOR: [number, number, number] = [0.61, 0.64, 0.69];

const DAY_MS = 86_400_000;

/** Today as days since 1970. */
export function todayDays(): number {
  return Math.floor(Date.now() / DAY_MS);
}

/** Band index for `newestDays` (days since 1970, 0 = unknown), or -1 if unknown. */
export function ageBand(newestDays: number, today = todayDays()): number {
  if (!newestDays) return -1;
  const age = Math.max(0, today - newestDays);
  let i = 0;
  while (i + 1 < AGE_BANDS.length && age >= AGE_BANDS[i + 1].minDays) i++;
  return i;
}

export function ageColor(newestDays: number, today?: number): [number, number, number] {
  const b = ageBand(newestDays, today);
  return b < 0 ? AGE_UNKNOWN_COLOR : AGE_BANDS[b].color;
}

export function cssRgb([r, g, b]: [number, number, number], alpha = 1): string {
  return `rgba(${Math.round(r * 255)}, ${Math.round(g * 255)}, ${Math.round(b * 255)}, ${alpha})`;
}

/** "3 days ago", "5 months ago", "2.4 years ago" for a days-since-1970 value. */
export function formatAgeDays(newestDays: number, today = todayDays()): string {
  if (!newestDays) return "unknown";
  const d = Math.max(0, today - newestDays);
  if (d === 0) return "today";
  if (d < 45) return `${d} day${d === 1 ? "" : "s"} ago`;
  if (d < 365) return `${Math.round(d / 30.4)} months ago`;
  const y = d / 365.25;
  return `${y < 10 ? y.toFixed(1) : Math.round(y)} years ago`;
}
