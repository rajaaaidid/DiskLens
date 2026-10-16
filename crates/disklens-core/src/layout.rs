//! Nested squarified treemap layout (Bruls, Huizing, van Wijk) over a `Tree`.
//!
//! Output is a flat list of blocks in draw order (parents before children). Children too small to see
//! are merged into one "small files" aggregate per folder. Folders only nest their children when there
//! is room for a header label; smaller folders are drawn as solid tiles.

use serde::Serialize;

use crate::tree::{flags, NodeId, Tree};

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

impl Rect {
    pub fn area(&self) -> f32 {
        self.w * self.h
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
pub struct Block {
    /// Node for this block; for an aggregate, the folder whose small children were merged.
    pub node: NodeId,
    pub aggregate: bool,
    /// Bytes represented (the merged total for aggregates).
    pub size: u64,
    /// Items merged into an aggregate (0 otherwise).
    pub agg_count: u32,
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
    pub depth: u16,
    /// Index of the enclosing block, or -1.
    pub parent: i32,
    /// Header height reserved for the folder label (0 = not nested).
    pub header: f32,
}

#[derive(Debug, Clone, Copy)]
pub struct LayoutOptions {
    /// Children whose area would be below this many px² are merged.
    pub min_area: f32,
    pub max_blocks: usize,
    pub header_height: f32,
    /// Folder levels below the root that may nest.
    pub max_depth: u16,
    /// Gap between sibling blocks at depth 0 (shrinks with depth and for tiny blocks).
    pub gap: f32,
    /// Use size on disk instead of logical size.
    pub use_alloc: bool,
}

impl Default for LayoutOptions {
    fn default() -> Self {
        LayoutOptions { min_area: 30.0, max_blocks: 60_000, header_height: 18.0, max_depth: 6, gap: 3.0, use_alloc: false }
    }
}

fn gap_for_depth(base: f32, depth: u16) -> f32 {
    match depth {
        0 => base,
        1 => base * 0.83,
        2 => base * 0.67,
        _ => base * 0.5,
    }
}

/// Squarify `sizes` (sorted descending, all > 0) into `r`. Returns one rect per size, in order.
pub fn squarify(sizes: &[f64], r: Rect) -> Vec<Rect> {
    let mut out = vec![Rect { x: r.x, y: r.y, w: 0.0, h: 0.0 }; sizes.len()];
    let total: f64 = sizes.iter().sum();
    if total <= 0.0 || r.w <= 0.0 || r.h <= 0.0 {
        return out;
    }
    let scale = (r.w as f64 * r.h as f64) / total;
    let (mut x, mut y, mut w, mut h) = (r.x as f64, r.y as f64, r.w as f64, r.h as f64);
    let mut i = 0;
    while i < sizes.len() {
        let side = w.min(h);
        let s2 = side * side;
        let (mut sum, mut min, mut max) = (0.0f64, f64::INFINITY, 0.0f64);
        let mut worst = f64::INFINITY;
        let mut j = i;
        while j < sizes.len() {
            let a = sizes[j] * scale;
            let (ns, nmin, nmax) = (sum + a, min.min(a), max.max(a));
            let nw = (s2 * nmax / (ns * ns)).max(ns * ns / (s2 * nmin));
            if j > i && nw > worst {
                break;
            }
            worst = nw;
            sum = ns;
            min = nmin;
            max = nmax;
            j += 1;
        }
        // On the last row use all remaining space so float error never leaves a sliver.
        let last = j == sizes.len();
        if w >= h {
            let cw = if last { w } else if h > 0.0 { sum / h } else { 0.0 };
            let mut cy = y;
            for k in i..j {
                let ih = if k + 1 == j { y + h - cy } else if sum > 0.0 { sizes[k] * scale / sum * h } else { 0.0 };
                out[k] = Rect { x: x as f32, y: cy as f32, w: cw as f32, h: ih as f32 };
                cy += ih;
            }
            x += cw;
            w -= cw;
        } else {
            let rh = if last { h } else if w > 0.0 { sum / w } else { 0.0 };
            let mut cx = x;
            for k in i..j {
                let iw = if k + 1 == j { x + w - cx } else if sum > 0.0 { sizes[k] * scale / sum * w } else { 0.0 };
                out[k] = Rect { x: cx as f32, y: y as f32, w: iw as f32, h: rh as f32 };
                cx += iw;
            }
            y += rh;
            h -= rh;
        }
        i = j;
    }
    out
}

struct Item {
    node: NodeId,
    size: u64,
    aggregate: bool,
    agg_count: u32,
}

fn node_size(t: &Tree, id: NodeId, o: &LayoutOptions) -> u64 {
    if o.use_alloc {
        t.alloc[id as usize]
    } else {
        t.size[id as usize]
    }
}

fn items_for(t: &Tree, dir: NodeId, area: f32, o: &LayoutOptions) -> Vec<Item> {
    let total = node_size(t, dir, o);
    if total == 0 {
        return Vec::new();
    }
    let mut kids: Vec<(NodeId, u64)> = t
        .children(dir)
        .filter(|&c| !t.has_flag(c, flags::DELETED))
        .map(|c| (c, node_size(t, c, o)))
        .filter(|&(_, s)| s > 0)
        .collect();
    kids.sort_unstable_by(|a, b| b.1.cmp(&a.1));
    let per_byte = area as f64 / total as f64;
    let mut cut = kids.len();
    while cut > 0 && (kids[cut - 1].1 as f64 * per_byte) < o.min_area as f64 {
        cut -= 1;
    }
    let mut items: Vec<Item> = kids[..cut].iter().map(|&(node, size)| Item { node, size, aggregate: false, agg_count: 0 }).collect();
    if cut < kids.len() {
        let size: u64 = kids[cut..].iter().map(|k| k.1).sum();
        let count: u32 = kids[cut..].iter().map(|&(c, _)| if t.is_dir(c) { t.files[c as usize] } else { 1 }).sum();
        if size as f64 * per_byte >= o.min_area as f64 * 0.5 {
            items.push(Item { node: dir, size, aggregate: true, agg_count: count });
            items.sort_by(|a, b| b.size.cmp(&a.size));
        }
    }
    items
}

/// Lay out the subtree under `root` into a `width × height` viewport.
pub fn layout(t: &Tree, root: NodeId, width: f32, height: f32, o: &LayoutOptions) -> Vec<Block> {
    let mut blocks = Vec::new();
    place(t, root, Rect { x: 0.0, y: 0.0, w: width, h: height }, 0, -1, o, &mut blocks);
    blocks
}

fn place(t: &Tree, dir: NodeId, inner: Rect, depth: u16, parent: i32, o: &LayoutOptions, blocks: &mut Vec<Block>) {
    if inner.w < 4.0 || inner.h < 4.0 {
        return;
    }
    let items = items_for(t, dir, inner.area(), o);
    if items.is_empty() {
        return;
    }
    let sizes: Vec<f64> = items.iter().map(|i| i.size as f64).collect();
    let rects = squarify(&sizes, inner);
    let depth_gap = gap_for_depth(o.gap, depth);
    let mut placed = Vec::with_capacity(items.len());
    for (it, r) in items.iter().zip(rects) {
        if blocks.len() >= o.max_blocks {
            break;
        }
        // Tiny tiles get proportionally smaller gaps so they stay visible.
        let gap = depth_gap.min((r.w.min(r.h) * 0.3).max(0.5));
        let b = Block {
            node: it.node,
            aggregate: it.aggregate,
            size: it.size,
            agg_count: it.agg_count,
            x: r.x + gap / 2.0,
            y: r.y + gap / 2.0,
            w: (r.w - gap).max(0.0),
            h: (r.h - gap).max(0.0),
            depth,
            parent,
            header: 0.0,
        };
        if b.w < 1.0 || b.h < 1.0 {
            continue;
        }
        placed.push(blocks.len());
        blocks.push(b);
    }
    for idx in placed {
        let b = blocks[idx];
        if b.aggregate || !t.is_dir(b.node) || depth + 1 >= o.max_depth || b.w < 64.0 || b.h < 40.0 {
            continue;
        }
        let pad = gap_for_depth(o.gap, depth + 1);
        let before = blocks.len();
        let inner = Rect { x: b.x + pad / 2.0, y: b.y + o.header_height + pad / 2.0, w: b.w - pad, h: b.h - o.header_height - pad };
        place(t, b.node, inner, depth + 1, idx as i32, o, blocks);
        if blocks.len() > before {
            blocks[idx].header = o.header_height;
        }
    }
}

/// Deepest block containing (x, y).
pub fn hit_test(blocks: &[Block], x: f32, y: f32) -> Option<usize> {
    blocks.iter().rposition(|b| x >= b.x && x < b.x + b.w && y >= b.y && y < b.y + b.h)
}
