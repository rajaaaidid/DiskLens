import type { TreeNode } from "../data/types";

/** One visible block. Draw order = array order (parents before their children). */
export interface Block {
  /** Stable key used to morph blocks between layouts. Aggregates use -(parent id). */
  key: number;
  /** The node for this block; for an aggregate this is the parent folder. */
  node: TreeNode;
  isAggregate: boolean;
  /** For aggregates: how many items and bytes were merged. */
  aggCount: number;
  aggSize: number;
  x: number;
  y: number;
  w: number;
  h: number;
  depth: number;
  /** Index of the enclosing block, or -1 for top-level blocks. */
  parent: number;
  /** Height reserved at the top for the folder label (0 if none). */
  header: number;
  /** True when this folder's children are drawn inside it. */
  expanded: boolean;
}

export interface LayoutOptions {
  /** Children whose area would be below this (px²) are merged into one "small files" block. */
  minArea: number;
  /** Hard cap on emitted blocks. */
  maxBlocks: number;
  /** Header height for folders big enough to label. */
  headerHeight: number;
  /** Folder levels to nest below the root before drawing folders as solid tiles. */
  maxDepth: number;
}

const DEFAULTS: LayoutOptions = { minArea: 30, maxBlocks: 60_000, headerHeight: 18, maxDepth: 6 };

export const gapForDepth = (depth: number) => (depth <= 0 ? 3 : depth === 1 ? 2.5 : depth === 2 ? 2 : 1.5);

interface Item {
  node: TreeNode;
  size: number;
  agg: boolean;
  aggCount: number;
}

interface Rect {
  x: number;
  y: number;
  w: number;
  h: number;
}

/**
 * Squarified treemap (Bruls, Huizing, van Wijk) for `items` (sorted desc, size>0) into `r`.
 * Returns one rect per item, in item order.
 */
export function squarify(sizes: number[], r: Rect): Rect[] {
  const out: Rect[] = new Array(sizes.length);
  const total = sizes.reduce((a, b) => a + b, 0);
  if (total <= 0 || r.w <= 0 || r.h <= 0) {
    for (let i = 0; i < sizes.length; i++) out[i] = { x: r.x, y: r.y, w: 0, h: 0 };
    return out;
  }
  const scale = (r.w * r.h) / total;
  let { x, y, w, h } = r;
  let i = 0;
  while (i < sizes.length) {
    const side = Math.min(w, h);
    let rowSum = 0;
    let rowMin = Infinity;
    let rowMax = 0;
    let j = i;
    let worst = Infinity;
    while (j < sizes.length) {
      const a = sizes[j] * scale;
      const nSum = rowSum + a;
      const nMin = Math.min(rowMin, a);
      const nMax = Math.max(rowMax, a);
      const s2 = side * side;
      const nWorst = Math.max((s2 * nMax) / (nSum * nSum), (nSum * nSum) / (s2 * nMin));
      if (j > i && nWorst > worst) break;
      worst = nWorst;
      rowSum = nSum;
      rowMin = nMin;
      rowMax = nMax;
      j++;
    }
    // Lay row [i, j) along the short side.
    if (w >= h) {
      const colW = h > 0 ? rowSum / h : 0;
      let cy = y;
      for (let k = i; k < j; k++) {
        const ih = rowSum > 0 ? (sizes[k] * scale) / rowSum * h : 0;
        out[k] = { x, y: cy, w: colW, h: ih };
        cy += ih;
      }
      x += colW;
      w -= colW;
    } else {
      const rowH = w > 0 ? rowSum / w : 0;
      let cx = x;
      for (let k = i; k < j; k++) {
        const iw = rowSum > 0 ? (sizes[k] * scale) / rowSum * w : 0;
        out[k] = { x: cx, y, w: iw, h: rowH };
        cx += iw;
      }
      y += rowH;
      h -= rowH;
    }
    i = j;
  }
  return out;
}

/** Sorted children with the tail of tiny items merged into one aggregate. */
function itemsFor(node: TreeNode, area: number, minArea: number): Item[] {
  const kids = node.children.filter((c) => c.size > 0).sort((a, b) => b.size - a.size);
  if (kids.length === 0 || node.size <= 0) return [];
  const perByte = area / node.size;
  let cut = kids.length;
  while (cut > 0 && kids[cut - 1].size * perByte < minArea) cut--;
  const items: Item[] = kids.slice(0, cut).map((n) => ({ node: n, size: n.size, agg: false, aggCount: 1 }));
  if (cut < kids.length) {
    let s = 0;
    let count = 0;
    for (let k = cut; k < kids.length; k++) {
      s += kids[k].size;
      count += kids[k].isDir ? kids[k].files : 1;
    }
    // A merged tail that is itself too small is dropped (still counted in the parent's size).
    if (s * perByte >= minArea * 0.5) {
      items.push({ node, size: s, agg: true, aggCount: count });
      items.sort((a, b) => b.size - a.size);
    }
  }
  return items;
}

export function layoutTreemap(root: TreeNode, width: number, height: number, opts: Partial<LayoutOptions> = {}): Block[] {
  const o = { ...DEFAULTS, ...opts };
  const blocks: Block[] = [];

  const place = (node: TreeNode, inner: Rect, depth: number, parentIdx: number) => {
    if (inner.w < 4 || inner.h < 4) return;
    const items = itemsFor(node, inner.w * inner.h, o.minArea);
    if (items.length === 0) return;
    const rects = squarify(
      items.map((it) => it.size),
      inner,
    );
    const depthGap = gapForDepth(depth);
    const childIdx: number[] = [];
    for (let k = 0; k < items.length; k++) {
      if (blocks.length >= o.maxBlocks) break;
      const it = items[k];
      const r = rects[k];
      // Tiny tiles get proportionally smaller gaps so they stay visible.
      const gap = Math.min(depthGap, Math.max(0.5, Math.min(r.w, r.h) * 0.3));
      const half = gap / 2;
      const b: Block = {
        key: it.agg ? -it.node.id : it.node.id,
        node: it.node,
        isAggregate: it.agg,
        aggCount: it.aggCount,
        aggSize: it.agg ? it.size : 0,
        x: r.x + half,
        y: r.y + half,
        w: Math.max(0, r.w - gap),
        h: Math.max(0, r.h - gap),
        depth,
        parent: parentIdx,
        header: 0,
        expanded: false,
      };
      if (b.w < 1 || b.h < 1) continue;
      childIdx.push(blocks.length);
      blocks.push(b);
    }
    // Recurse after emitting all siblings so the draw order stays parent-before-child.
    for (const idx of childIdx) {
      const b = blocks[idx];
      if (b.isAggregate || !b.node.isDir || depth + 1 >= o.maxDepth) continue;
      const pad = gapForDepth(depth + 1);
      // Only nest folders that have room for their label; smaller ones stay solid tiles.
      if (b.w < 64 || b.h < 40) continue;
      const header = o.headerHeight;
      b.header = header;
      const before = blocks.length;
      place(b.node, { x: b.x + pad / 2, y: b.y + header + pad / 2, w: b.w - pad, h: b.h - header - pad }, depth + 1, idx);
      b.expanded = blocks.length > before;
      if (!b.expanded) b.header = 0;
    }
  };

  place(root, { x: 0, y: 0, w: width, h: height }, 0, -1);
  return blocks;
}

/** Deepest block containing (x, y), or -1. */
export function hitTest(blocks: Block[], x: number, y: number): number {
  for (let i = blocks.length - 1; i >= 0; i--) {
    const b = blocks[i];
    if (x >= b.x && x < b.x + b.w && y >= b.y && y < b.y + b.h) return i;
  }
  return -1;
}
