import { useEffect, useRef } from "react";
import { CATEGORY_COLORS } from "../data/categories";
import { formatBytes, formatCount } from "../data/format";
import type { TreeNode } from "../data/types";
import { FLAG_AGGREGATE, FLAG_DIR, FLAG_LEAF, FLAG_OVERLAY, INSTANCE_FLOATS } from "./glRenderer";
import { hitTest, layoutTreemap, type Block } from "./layout";
import { stage } from "./stage";

export interface HoverInfo {
  block: Block;
  /** Mouse position in window CSS px. */
  clientX: number;
  clientY: number;
}

interface Props {
  root: TreeNode;
  theme: "dark" | "light";
  reduceMotion: boolean;
  onHover: (h: HoverInfo | null) => void;
  onZoom: (node: TreeNode) => void;
  /** Mockup/screenshot helper: force-hover the block for this node id. */
  demoHoverId?: number;
  /** Override the merge threshold (px�); used by the stress test. */
  minArea?: number;
}

const ZOOM_MS = 300;
const HOVER_MS = 140;

interface Anim {
  start: number;
  from: Float32Array; // per new-block start rects (x,y,w,h) + alpha
  outgoing: Block[]; // old blocks that only fade out
  outFrom: Float32Array;
  outTo: Float32Array;
}

const easeInOutCubic = (t: number) => (t < 0.5 ? 4 * t * t * t : 1 - Math.pow(-2 * t + 2, 3) / 2);

function blockFlags(b: Block): number {
  let f = 0;
  if (b.isAggregate) f |= FLAG_AGGREGATE | FLAG_LEAF;
  else if (b.node.isDir && b.expanded) f |= FLAG_DIR;
  else f |= FLAG_LEAF; // files, and folders drawn as solid tiles
  return f;
}

function writeInstance(out: Float32Array, i: number, b: Block, x: number, y: number, w: number, h: number, alpha: number, flags: number) {
  const o = i * INSTANCE_FLOATS;
  const c = CATEGORY_COLORS[b.isAggregate ? "other" : b.node.isDir && !b.expanded ? b.node.dominant : b.node.category];
  out[o] = x;
  out[o + 1] = y;
  out[o + 2] = w;
  out[o + 3] = h;
  out[o + 4] = c[0];
  out[o + 5] = c[1];
  out[o + 6] = c[2];
  out[o + 7] = b.depth;
  out[o + 8] = alpha;
  out[o + 9] = flags;
  out[o + 10] = b.header;
}

export function Treemap({ root, theme, reduceMotion, onHover, onZoom, demoHoverId, minArea }: Props) {
  const hostRef = useRef<HTMLDivElement>(null);
  const labelRef = useRef<HTMLCanvasElement>(null);
  const state = useRef({
    blocks: [] as Block[],
    root: null as TreeNode | null,
    size: { w: 0, h: 0 },
    anim: null as Anim | null,
    hovered: -1,
    hoverT: 0,
    hoverStart: 0,
    buf: new Float32Array(0),
    dirty: true,
    labelsDirty: true,
    theme,
  });
  const cb = useRef({ onHover, onZoom });
  cb.current = { onHover, onZoom };

  // Relayout on root / size change, with a morph animation from the previous layout.
  useEffect(() => {
    const host = hostRef.current!;
    const s = state.current;

    const relayout = (animate: boolean) => {
      const { w, h } = s.size;
      if (w <= 0 || h <= 0) return;
      const t0 = performance.now();
      const next = layoutTreemap(root, w, h, minArea ? { minArea } : {});
      const ms = performance.now() - t0;
      (window as unknown as { __layoutMs: number; __blocks: number }).__layoutMs = ms;
      (window as unknown as { __layoutMs: number; __blocks: number }).__blocks = next.length;
      const prev = s.blocks;
      const prevRoot = s.root;
      if (animate && prev.length && prevRoot && prevRoot !== root && !stage.reduceMotion) {
        s.anim = buildMorph(prev, prevRoot, next, root, w, h);
        s.anim.start = performance.now();
      } else {
        s.anim = null;
      }
      s.blocks = next;
      s.root = root;
      s.hovered = -1;
      s.dirty = true;
      s.labelsDirty = true;
      cb.current.onHover(null);
    };

    const ro = new ResizeObserver(() => {
      const r = host.getBoundingClientRect();
      const changed = Math.round(r.width) !== s.size.w || Math.round(r.height) !== s.size.h;
      s.size = { w: Math.round(r.width), h: Math.round(r.height) };
      stage.setViewport({ x: r.left, y: r.top, w: r.width, h: r.height });
      if (changed) relayout(false);
    });
    ro.observe(host);
    const onWin = () => {
      const r = host.getBoundingClientRect();
      stage.setViewport({ x: r.left, y: r.top, w: r.width, h: r.height });
    };
    window.addEventListener("resize", onWin);
    if (s.size.w > 0) relayout(s.root !== null);
    return () => {
      ro.disconnect();
      window.removeEventListener("resize", onWin);
    };
  }, [root]);

  useEffect(() => {
    state.current.theme = theme;
    state.current.labelsDirty = true;
    if (stage.renderer) stage.renderer.theme = theme === "light" ? 1 : 0;
  }, [theme]);

  useEffect(() => {
    stage.reduceMotion = reduceMotion;
  }, [reduceMotion]);

  // Per-frame: rebuild instances while animating, upload, and redraw labels when settled.
  useEffect(() => {
    const s = state.current;
    return stage.addHook((now) => {
      const r = stage.renderer;
      if (!r) return;
      r.theme = s.theme === "light" ? 1 : 0;

      // hover fade
      const targetHover = s.hovered >= 0 ? 1 : 0;
      if (s.hoverT !== targetHover) {
        const k = stage.reduceMotion ? 1 : Math.min(1, (now - s.hoverStart) / HOVER_MS);
        s.hoverT = targetHover ? k : 1 - k;
        if (k >= 1) s.hoverT = targetHover;
        s.dirty = true;
      }
      r.hover = easeInOutCubic(s.hoverT);

      let t = 1;
      if (s.anim) {
        t = Math.min(1, (now - s.anim.start) / ZOOM_MS);
        s.dirty = true;
        if (t >= 1) {
          s.anim = null;
          s.labelsDirty = true;
        }
      }
      if (!s.dirty) return;
      s.dirty = false;

      const blocks = s.blocks;
      const anim = s.anim;
      const total = blocks.length + (anim ? anim.outgoing.length : 0) + 1;
      if (s.buf.length < total * INSTANCE_FLOATS) s.buf = new Float32Array(total * INSTANCE_FLOATS * 1.5);
      const buf = s.buf;
      let n = 0;
      const e = easeInOutCubic(t);
      if (anim) {
        for (let i = 0; i < anim.outgoing.length; i++) {
          const a = anim.outFrom;
          const b = anim.outTo;
          const o = i * 4;
          writeInstance(
            buf, n++, anim.outgoing[i],
            a[o] + (b[o] - a[o]) * e, a[o + 1] + (b[o + 1] - a[o + 1]) * e,
            a[o + 2] + (b[o + 2] - a[o + 2]) * e, a[o + 3] + (b[o + 3] - a[o + 3]) * e,
            1 - e, blockFlags(anim.outgoing[i]),
          );
        }
        for (let i = 0; i < blocks.length; i++) {
          const b = blocks[i];
          const f = anim.from;
          const o = i * 5;
          writeInstance(
            buf, n++, b,
            f[o] + (b.x - f[o]) * e, f[o + 1] + (b.y - f[o + 1]) * e,
            f[o + 2] + (b.w - f[o + 2]) * e, f[o + 3] + (b.h - f[o + 3]) * e,
            f[o + 4] + (1 - f[o + 4]) * e, blockFlags(b),
          );
        }
      } else {
        for (let i = 0; i < blocks.length; i++) {
          const b = blocks[i];
          writeInstance(buf, n++, b, b.x, b.y, b.w, b.h, 1, blockFlags(b));
        }
        if (s.hovered >= 0 && s.hoverT > 0) {
          const b = blocks[s.hovered];
          writeInstance(buf, n++, b, b.x, b.y, b.w, b.h, 1, blockFlags(b) | FLAG_OVERLAY);
        }
      }
      r.setInstances(buf, n);

      const lc = labelRef.current;
      if (lc) {
        lc.style.opacity = anim ? "0" : "1";
        if (!anim && s.labelsDirty) {
          s.labelsDirty = false;
          drawLabels(lc, blocks, s.size.w, s.size.h, s.theme);
        }
      }
    });
  }, []);

  // Mockup helper: pin hover to a given node.
  useEffect(() => {
    if (demoHoverId === undefined) return;
    const s = state.current;
    const tryPin = () => {
      const idx = s.blocks.findIndex((b) => b.node.id === demoHoverId && !b.isAggregate);
      if (idx < 0) return false;
      s.hovered = idx;
      s.hoverT = 1;
      s.dirty = true;
      const b = s.blocks[idx];
      const r = hostRef.current!.getBoundingClientRect();
      cb.current.onHover({ block: b, clientX: r.left + b.x + b.w * 0.6, clientY: r.top + b.y + b.h * 0.45 });
      return true;
    };
    const id = setInterval(() => tryPin() && clearInterval(id), 50);
    return () => clearInterval(id);
  }, [demoHoverId, root]);

  const onMove = (ev: React.MouseEvent) => {
    const s = state.current;
    if (s.anim) return;
    const r = hostRef.current!.getBoundingClientRect();
    const idx = hitTest(s.blocks, ev.clientX - r.left, ev.clientY - r.top);
    if (idx !== s.hovered) {
      const wasHover = s.hovered >= 0;
      s.hovered = idx;
      // Moving between blocks keeps the lift; entering from nothing fades it in.
      if (!wasHover || idx < 0) s.hoverStart = performance.now();
      else s.hoverT = 1;
      s.dirty = true;
    }
    cb.current.onHover(idx >= 0 ? { block: s.blocks[idx], clientX: ev.clientX, clientY: ev.clientY } : null);
  };

  const onLeave = () => {
    const s = state.current;
    if (s.hovered >= 0) {
      s.hovered = -1;
      s.hoverStart = performance.now();
      s.dirty = true;
    }
    cb.current.onHover(null);
  };

  const onClick = (ev: React.MouseEvent) => {
    const s = state.current;
    if (s.anim) return;
    const r = hostRef.current!.getBoundingClientRect();
    const idx = hitTest(s.blocks, ev.clientX - r.left, ev.clientY - r.top);
    if (idx < 0) return;
    // Zoom one level: the top-level folder under the cursor.
    let i = idx;
    while (s.blocks[i].parent >= 0) i = s.blocks[i].parent;
    const target = s.blocks[i];
    if (target.node.isDir && !target.isAggregate) cb.current.onZoom(target.node);
  };

  return (
    <div ref={hostRef} className="treemap-host" onMouseMove={onMove} onMouseLeave={onLeave} onClick={onClick}>
      <canvas ref={labelRef} className="treemap-labels" />
    </div>
  );
}

/** Start rects for the new layout and paths for blocks that leave, so the zoom reads as one motion. */
function buildMorph(prev: Block[], prevRoot: TreeNode, next: Block[], nextRoot: TreeNode, W: number, H: number): Anim {
  const prevMap = new Map<number, Block>();
  for (const b of prev) prevMap.set(b.key, b);
  const nextKeys = new Set(next.map((b) => b.key));

  // Camera: the rectangle, in the old view, that the new view corresponds to.
  let cam = { x: 0, y: 0, w: W, h: H }; // new view expressed in old coords
  const inOld = prevMap.get(nextRoot.id);
  if (inOld) cam = { x: inOld.x, y: inOld.y, w: inOld.w, h: inOld.h };
  else {
    const inNew = next.find((b) => b.key === prevRoot.id);
    if (inNew) {
      // Zooming out: the old view sits at inNew inside the new one.
      const sx = W / inNew.w;
      const sy = H / inNew.h;
      cam = { x: -inNew.x * sx, y: -inNew.y * sy, w: W * sx, h: H * sy };
    }
  }
  const toOld = (x: number, y: number, w: number, h: number) => [cam.x + (x / W) * cam.w, cam.y + (y / H) * cam.h, (w / W) * cam.w, (h / H) * cam.h];
  const toNew = (x: number, y: number, w: number, h: number) => [((x - cam.x) / cam.w) * W, ((y - cam.y) / cam.h) * H, (w / cam.w) * W, (h / cam.h) * H];

  const from = new Float32Array(next.length * 5);
  next.forEach((b, i) => {
    const o = i * 5;
    const p = prevMap.get(b.key);
    if (p) {
      from.set([p.x, p.y, p.w, p.h, 1], o);
    } else {
      const r = toOld(b.x, b.y, b.w, b.h);
      from.set([r[0], r[1], r[2], r[3], 0], o);
    }
  });
  const outgoing = prev.filter((b) => !nextKeys.has(b.key));
  const outFrom = new Float32Array(outgoing.length * 4);
  const outTo = new Float32Array(outgoing.length * 4);
  outgoing.forEach((b, i) => {
    outFrom.set([b.x, b.y, b.w, b.h], i * 4);
    outTo.set(toNew(b.x, b.y, b.w, b.h), i * 4);
  });
  return { start: 0, from, outgoing, outFrom, outTo };
}

const FONT_STACK = `"Segoe UI Variable Text", "Segoe UI Variable", "Inter", "Segoe UI", system-ui, sans-serif`;

function fitText(ctx: CanvasRenderingContext2D, text: string, maxW: number): string {
  if (maxW <= 8) return "";
  if (ctx.measureText(text).width <= maxW) return text;
  let lo = 0;
  let hi = text.length;
  while (lo < hi) {
    const mid = (lo + hi + 1) >> 1;
    if (ctx.measureText(text.slice(0, mid) + "…").width <= maxW) lo = mid;
    else hi = mid - 1;
  }
  return lo >= 2 ? text.slice(0, lo) + "…" : "";
}

function drawLabels(canvas: HTMLCanvasElement, blocks: Block[], w: number, h: number, theme: "dark" | "light") {
  const dpr = window.devicePixelRatio || 1;
  canvas.width = Math.round(w * dpr);
  canvas.height = Math.round(h * dpr);
  canvas.style.width = `${w}px`;
  canvas.style.height = `${h}px`;
  const ctx = canvas.getContext("2d")!;
  ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
  ctx.clearRect(0, 0, w, h);
  ctx.textBaseline = "middle";

  const light = theme === "light";
  const primary = light ? "rgba(10, 16, 30, 0.94)" : "rgba(255, 255, 255, 0.96)";
  const secondary = light ? "rgba(10, 16, 30, 0.70)" : "rgba(235, 240, 255, 0.76)";
  ctx.shadowColor = light ? "rgba(255, 255, 255, 0.7)" : "rgba(0, 0, 10, 0.55)";
  ctx.shadowBlur = 3;

  // Biggest blocks first; cap the number of labels.
  const candidates: number[] = [];
  for (let i = 0; i < blocks.length; i++) {
    const b = blocks[i];
    if (b.header > 0 || (b.w >= 46 && b.h >= 22)) candidates.push(i);
  }
  candidates.sort((a, b) => blocks[b].w * blocks[b].h - blocks[a].w * blocks[a].h);
  const LIMIT = 700;
  for (let k = 0; k < Math.min(LIMIT, candidates.length); k++) {
    const b = blocks[candidates[k]];
    const name = b.isAggregate ? `${formatCount(b.aggCount)} small files` : b.node.name;
    const size = formatBytes(b.isAggregate ? b.aggSize : b.node.size);
    if (b.header > 0) {
      ctx.font = `600 11.5px ${FONT_STACK}`;
      const sizeW = ctx.measureText(size).width;
      const cy = b.y + b.header / 2 + 1;
      const showSize = b.w > sizeW + 70;
      const nameText = fitText(ctx, name, b.w - 14 - (showSize ? sizeW + 10 : 0));
      ctx.fillStyle = primary;
      ctx.fillText(nameText, b.x + 7, cy);
      if (showSize) {
        ctx.font = `500 11px ${FONT_STACK}`;
        ctx.fillStyle = secondary;
        ctx.textAlign = "right";
        ctx.fillText(size, b.x + b.w - 7, cy);
        ctx.textAlign = "left";
      }
    } else {
      const big = b.w >= 90 && b.h >= 48;
      ctx.font = `600 ${big ? 12.5 : 11}px ${FONT_STACK}`;
      const nameText = fitText(ctx, name, b.w - 12);
      if (!nameText) continue;
      ctx.fillStyle = primary;
      if (b.h >= 36) {
        ctx.fillText(nameText, b.x + 7, b.y + (big ? 15 : 12));
        ctx.font = `500 ${big ? 11.5 : 10.5}px ${FONT_STACK}`;
        ctx.fillStyle = secondary;
        ctx.fillText(fitText(ctx, size, b.w - 12), b.x + 7, b.y + (big ? 31 : 25));
      } else {
        ctx.fillText(nameText, b.x + 6, b.y + b.h / 2);
      }
    }
  }
}
