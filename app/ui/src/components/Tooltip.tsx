import { useEffect, useState } from "react";
import { backend, BF_AGGREGATE, BF_DIR, BF_INACCESSIBLE, BF_LINK, BF_PENDING, type NodeInfo } from "../api";
import { formatAgeDays } from "../data/age";
import { CATEGORY_LABELS, cssColor } from "../data/categories";
import { formatBytes, formatCount, formatPercent } from "../data/format";
import type { HoverInfo } from "../treemap/Treemap";

interface Props {
  hover: HoverInfo | null;
  generation: number;
  useAlloc: boolean;
}

/** Frosted tooltip. Shows what the block knows immediately, then fills in the path and parent share. */
export function Tooltip({ hover, generation, useAlloc }: Props) {
  const [info, setInfo] = useState<NodeInfo | null>(null);
  const node = hover?.block.node;

  useEffect(() => {
    setInfo(null);
    if (node === undefined) return;
    let alive = true;
    const t = setTimeout(() => {
      backend.nodeInfo(generation, node, useAlloc).then((i) => alive && setInfo(i)).catch(() => {});
    }, 40);
    return () => {
      alive = false;
      clearTimeout(t);
    };
  }, [node, generation, useAlloc]);

  if (!hover) return null;
  const b = hover.block;
  const agg = (b.flags & BF_AGGREGATE) !== 0;
  const isDir = (b.flags & BF_DIR) !== 0;
  const title = agg ? `${formatCount(b.aggCount)} small items` : b.name;
  const cat = agg ? "other" : b.category;
  const path = info ? (agg ? `${info.path}\\…` : info.path) : "";
  // For an aggregate the "parent" is the folder itself.
  const parentSize = info ? (agg ? (useAlloc ? info.sizeOnDisk : info.size) : info.parentSize) : 0;

  const W = 300;
  const x = hover.clientX + 18 + W > window.innerWidth ? hover.clientX - W - 18 : hover.clientX + 18;
  const y = Math.min(hover.clientY + 18, window.innerHeight - 190);

  return (
    <div className="tooltip glass-strong" style={{ left: x, top: y, width: W }}>
      <div className="tt-title">
        <span className="swatch" style={{ background: cssColor(cat, 0.9), boxShadow: `0 0 12px ${cssColor(cat, 0.6)}` }} />
        <span>{title}</span>
      </div>
      <div className="tt-path">{path || " "}</div>
      <div className="tt-size">{formatBytes(b.size, 2)}</div>
      <div className="tt-grid">
        <span>of parent</span>
        <b>{info ? formatPercent(b.size, parentSize) : "…"}</b>
        <span>{isDir || agg ? "files" : "type"}</span>
        <b>{isDir || agg ? formatCount(b.files) : CATEGORY_LABELS[cat]}</b>
        {!agg && (
          <>
            <span>{useAlloc ? "logical size" : "on disk"}</span>
            <b>{info ? formatBytes(useAlloc ? info.size : info.sizeOnDisk) : "…"}</b>
          </>
        )}
        {b.newestDays > 0 && (
          <>
            <span>{isDir || agg ? "newest file" : "modified"}</span>
            <b>{formatAgeDays(b.newestDays)}</b>
          </>
        )}
      </div>
      {(b.flags & (BF_PENDING | BF_LINK | BF_INACCESSIBLE)) !== 0 && (
        <div className="tt-note">
          {b.flags & BF_PENDING ? "Still scanning…" : b.flags & BF_LINK ? "Junction or symlink: not followed" : "Access denied"}
        </div>
      )}
      <div className="tt-pct">
        <span style={{ width: `${info && parentSize > 0 ? Math.min(100, (b.size / parentSize) * 100) : 0}%`, background: cssColor(cat, 0.85) }} />
      </div>
    </div>
  );
}
