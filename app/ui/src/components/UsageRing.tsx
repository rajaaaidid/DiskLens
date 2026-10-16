/** Circular used/free gauge for a drive. */
export function UsageRing({ used, total, size = 46, stroke = 4 }: { used: number; total: number; size?: number; stroke?: number }) {
  const r = size / 2 - stroke;
  const c = 2 * Math.PI * r;
  const frac = total > 0 ? Math.min(1, used / total) : 0;
  const hot = frac > 0.9;
  const id = hot ? "ring-hot" : "ring-ok";
  return (
    <svg width={size} height={size} className="usage-ring" aria-label={`${Math.round(frac * 100)}% used`}>
      <defs>
        <linearGradient id={id} x1="0" y1="0" x2="1" y2="1">
          <stop offset="0" stopColor={hot ? "#fb7185" : "#a78bfa"} />
          <stop offset="1" stopColor={hot ? "#f59e0b" : "#22d3ee"} />
        </linearGradient>
      </defs>
      <circle cx={size / 2} cy={size / 2} r={r} className="ring-track" strokeWidth={stroke} />
      <circle
        cx={size / 2}
        cy={size / 2}
        r={r}
        className="ring-value"
        strokeWidth={stroke}
        stroke={`url(#${id})`}
        strokeDasharray={`${c * frac} ${c}`}
        transform={`rotate(-90 ${size / 2} ${size / 2})`}
      />
      <text x="50%" y="52%" dominantBaseline="middle" textAnchor="middle" className="ring-text" style={{ fontSize: size * 0.24 }}>
        {Math.round(frac * 100)}%
      </text>
    </svg>
  );
}
