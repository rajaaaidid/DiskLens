import { CATEGORY_LABELS, cssColor } from "../data/categories";
import { pathOf } from "../data/fakeTree";
import { formatBytes, formatCount, formatPercent } from "../data/format";
import type { HoverInfo } from "../treemap/Treemap";

export function Tooltip({ hover }: { hover: HoverInfo | null }) {
  if (!hover) return null;
  const { block } = hover;
  const n = block.node;
  const agg = block.isAggregate;
  const size = agg ? block.aggSize : n.size;
  const parentSize = agg ? n.size : n.parent?.size ?? n.size;
  const title = agg ? `${formatCount(block.aggCount)} small items` : n.name;
  const path = agg ? `${pathOf(n)}\\…` : pathOf(n);
  const cat = agg ? "other" : n.category;

  // Keep the card on screen: flip left/up near the edges.
  const W = 300;
  const x = hover.clientX + 18 + W > window.innerWidth ? hover.clientX - W - 18 : hover.clientX + 18;
  const y = Math.min(hover.clientY + 18, window.innerHeight - 170);

  return (
    <div className="tooltip glass-strong" style={{ left: x, top: y, width: W }}>
      <div className="tt-title">
        <span className="swatch" style={{ background: cssColor(cat, 0.9), boxShadow: `0 0 12px ${cssColor(cat, 0.6)}` }} />
        <span>{title}</span>
      </div>
      <div className="tt-path">{path}</div>
      <div className="tt-size">{formatBytes(size, 2)}</div>
      <div className="tt-grid">
        <span>of parent</span>
        <b>{formatPercent(size, parentSize)}</b>
        <span>{n.isDir || agg ? "files" : "type"}</span>
        <b>{n.isDir || agg ? formatCount(agg ? block.aggCount : n.files) : CATEGORY_LABELS[cat]}</b>
        <span>on disk</span>
        <b>{formatBytes(agg ? size : n.onDisk)}</b>
      </div>
      <div className="tt-pct"><span style={{ width: `${Math.min(100, (size / parentSize) * 100)}%`, background: cssColor(cat, 0.85) }} /></div>
    </div>
  );
}
