import { CATEGORY_LABELS, cssColor } from "../data/categories";
import { formatBytes, formatCount } from "../data/format";
import type { Category, Drive } from "../data/types";
import { IconBolt, IconFolder } from "./Icons";

function UsageRing({ used, total, size = 46 }: { used: number; total: number; size?: number }) {
  const r = size / 2 - 4;
  const c = 2 * Math.PI * r;
  const frac = Math.min(1, used / total);
  const hot = frac > 0.9;
  return (
    <svg width={size} height={size} className="usage-ring">
      <defs>
        <linearGradient id={`ring-${hot ? "hot" : "ok"}`} x1="0" y1="0" x2="1" y2="1">
          <stop offset="0" stopColor={hot ? "#fb7185" : "#a78bfa"} />
          <stop offset="1" stopColor={hot ? "#f59e0b" : "#22d3ee"} />
        </linearGradient>
      </defs>
      <circle cx={size / 2} cy={size / 2} r={r} className="ring-track" />
      <circle
        cx={size / 2}
        cy={size / 2}
        r={r}
        className="ring-value"
        stroke={`url(#ring-${hot ? "hot" : "ok"})`}
        strokeDasharray={`${c * frac} ${c}`}
        transform={`rotate(-90 ${size / 2} ${size / 2})`}
      />
      <text x="50%" y="52%" dominantBaseline="middle" textAnchor="middle" className="ring-text">
        {Math.round(frac * 100)}%
      </text>
    </svg>
  );
}

interface Props {
  drives: Drive[];
  active: string;
  scan: { files: number; folders: number; bytes: number; seconds: number; method: "mft" | "walk"; inaccessible: number };
  categories: { category: Category; size: number }[];
  totalBytes: number;
}

export function Sidebar({ drives, active, scan, categories, totalBytes }: Props) {
  return (
    <aside className="sidebar glass">
      <div className="section-label">Drives</div>
      <div className="drive-list">
        {drives.map((d) => {
          const used = d.total - d.free;
          return (
            <button key={d.letter} className={`drive-card ${d.letter === active ? "active" : ""}`}>
              <UsageRing used={used} total={d.total} />
              <div className="drive-meta">
                <div className="drive-title">
                  <span className="drive-letter">{d.letter}</span>
                  <span className="drive-label">{d.label}</span>
                </div>
                <div className="drive-sub">
                  {formatBytes(d.free)} free of {formatBytes(d.total, 0)}
                </div>
                <div className="drive-bar">
                  <span style={{ width: `${(used / d.total) * 100}%` }} />
                </div>
              </div>
            </button>
          );
        })}
      </div>
      <button className="btn btn-ghost wide">
        <IconFolder /> Scan a folder…
      </button>

      <div className="section-label">Last scan</div>
      <div className="scan-card">
        <div className="scan-head">
          <div className="scan-ring done">
            <svg width="38" height="38" viewBox="0 0 38 38">
              <circle cx="19" cy="19" r="15" className="ring-track" />
              <circle cx="19" cy="19" r="15" className="ring-value glow" stroke="url(#ring-ok)" strokeDasharray="94.2 94.2" transform="rotate(-90 19 19)" />
            </svg>
          </div>
          <div>
            <div className="scan-title">Complete in {scan.seconds.toFixed(1)} s</div>
            <div className={`badge ${scan.method}`}>
              <IconBolt /> {scan.method === "mft" ? "MFT fast path" : "Directory walk"}
            </div>
          </div>
        </div>
        <dl className="scan-stats">
          <div><dt>Files</dt><dd>{formatCount(scan.files)}</dd></div>
          <div><dt>Folders</dt><dd>{formatCount(scan.folders)}</dd></div>
          <div><dt>Counted</dt><dd>{formatBytes(scan.bytes)}</dd></div>
          <div><dt>No access</dt><dd className="warn">{scan.inaccessible} folders</dd></div>
        </dl>
      </div>

      <div className="section-label">File types</div>
      <div className="legend">
        {categories.slice(0, 8).map((c) => (
          <div key={c.category} className="legend-row">
            <span className="swatch" style={{ background: cssColor(c.category, 0.85), boxShadow: `0 0 10px ${cssColor(c.category, 0.5)}` }} />
            <span className="legend-name">{CATEGORY_LABELS[c.category]}</span>
            <span className="legend-size">{formatBytes(c.size)}</span>
            <span className="legend-bar">
              <span style={{ width: `${(c.size / totalBytes) * 100}%`, background: cssColor(c.category, 0.8) }} />
            </span>
          </div>
        ))}
      </div>
    </aside>
  );
}
