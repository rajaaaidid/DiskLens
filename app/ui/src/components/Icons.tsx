// Small inline icon set (1.5px strokes, currentColor).
const P = { fill: "none", stroke: "currentColor", strokeWidth: 1.6, strokeLinecap: "round", strokeLinejoin: "round" } as const;

export const IconLens = ({ size = 18 }: { size?: number }) => (
  <svg width={size} height={size} viewBox="0 0 24 24" aria-hidden>
    <defs>
      <linearGradient id="lensg" x1="0" y1="0" x2="1" y2="1">
        <stop offset="0" stopColor="#a78bfa" />
        <stop offset="1" stopColor="#22d3ee" />
      </linearGradient>
    </defs>
    <circle cx="10.5" cy="10.5" r="6.5" fill="none" stroke="url(#lensg)" strokeWidth="2.4" />
    <path d="M15.5 15.5 20 20" stroke="url(#lensg)" strokeWidth="2.6" strokeLinecap="round" />
    <path d="M7.5 9a3.5 3.5 0 0 1 3-2.5" stroke="#fff" strokeOpacity=".7" strokeWidth="1.4" fill="none" strokeLinecap="round" />
  </svg>
);
export const IconSun = () => (
  <svg width="16" height="16" viewBox="0 0 24 24" {...P}><circle cx="12" cy="12" r="4" /><path d="M12 2v2M12 20v2M4.9 4.9l1.4 1.4M17.7 17.7l1.4 1.4M2 12h2M20 12h2M4.9 19.1l1.4-1.4M17.7 6.3l1.4-1.4" /></svg>
);
export const IconMoon = () => (
  <svg width="16" height="16" viewBox="0 0 24 24" {...P}><path d="M20 14.5A8 8 0 0 1 9.5 4a8 8 0 1 0 10.5 10.5Z" /></svg>
);
export const IconFolder = () => (
  <svg width="16" height="16" viewBox="0 0 24 24" {...P}><path d="M3 7a2 2 0 0 1 2-2h4l2 2h8a2 2 0 0 1 2 2v8a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2Z" /></svg>
);
export const IconChevron = ({ open }: { open?: boolean }) => (
  <svg width="12" height="12" viewBox="0 0 24 24" {...P} style={{ transform: open ? "rotate(90deg)" : undefined, transition: "transform .15s" }}><path d="m9 6 6 6-6 6" /></svg>
);
export const IconSearch = () => (
  <svg width="15" height="15" viewBox="0 0 24 24" {...P}><circle cx="11" cy="11" r="6.5" /><path d="m16 16 4 4" /></svg>
);
export const IconRescan = () => (
  <svg width="15" height="15" viewBox="0 0 24 24" {...P}><path d="M20 11a8 8 0 1 0-2.3 5.7" /><path d="M20 4v7h-7" /></svg>
);
export const IconExport = () => (
  <svg width="15" height="15" viewBox="0 0 24 24" {...P}><path d="M12 3v12M7 8l5-5 5 5M5 15v4a2 2 0 0 0 2 2h10a2 2 0 0 0 2-2v-4" /></svg>
);
export const IconBolt = () => (
  <svg width="13" height="13" viewBox="0 0 24 24" {...P}><path d="M13 2 4 14h7l-1 8 9-12h-7Z" /></svg>
);
export const IconShield = () => (
  <svg width="13" height="13" viewBox="0 0 24 24" {...P}><path d="M12 3 4 6v6c0 4.5 3.4 8.3 8 9 4.6-.7 8-4.5 8-9V6Z" /></svg>
);
export const IconExternal = () => (
  <svg width="13" height="13" viewBox="0 0 24 24" {...P}><path d="M14 4h6v6M20 4l-9 9M18 14v5a1 1 0 0 1-1 1H5a1 1 0 0 1-1-1V7a1 1 0 0 1 1-1h5" /></svg>
);
export const IconCopy = () => (
  <svg width="13" height="13" viewBox="0 0 24 24" {...P}><rect x="8" y="8" width="12" height="12" rx="2" /><path d="M16 8V6a2 2 0 0 0-2-2H6a2 2 0 0 0-2 2v8a2 2 0 0 0 2 2h2" /></svg>
);
export const IconTrash = () => (
  <svg width="13" height="13" viewBox="0 0 24 24" {...P}><path d="M4 7h16M9 7V4h6v3M6 7l1 13h10l1-13" /></svg>
);
export const IconHome = () => (
  <svg width="14" height="14" viewBox="0 0 24 24" {...P}><path d="M4 11 12 4l8 7v9H4Z" /></svg>
);
