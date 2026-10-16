import { useEffect, useRef } from "react";
import { BF_AGGREGATE, BF_DIR, BF_PENDING, type Layout, type ViewBlock } from "../api";
import { CATEGORY_COLORS } from "../data/categories";
import { formatBytes, formatCount } from "../data/format";
import { FLAG_AGGREGATE, FLAG_DIR, FLAG_LEAF, FLAG_OVERLAY, FLAG_PENDING, INSTANCE_FLOATS } from "./glRenderer";
import { stage } from "./stage";

export interface HoverInfo {
  block: ViewBlock;
  /** Mouse position in window CSS px. */
  clientX: number;
  clientY: number;
}

interface Props {
  layout: Layout | null;
  theme: "dark" | "light";
  reduceMotion: boolean;
  onHover: (h: HoverInfo | null) => void;
  /** Click on a block; `top` is the top-level block under the cursor (the zoom target). */
  onActivate: (top: ViewBlock, hit: ViewBlock) => void;
  onResize: (width: number, height: number) => void;
  /** Screenshot helper: pin the hover state on this node. */
  demoHoverNode?: number;
}

const ZOOM_MS = 300;
const REFRESH_MS = 240;
const HOVER_MS = 140;

interface Anim {
  start: number;
  duration: number;
  from: Float32Array; // per new block: x, y, w, h, alpha
  outgoing: ViewBlock[];
  outFrom: Float32Array;
  outTo: Float32Array;
}

const easeInOutCubic = (t: number) => (t < 0.5 ? 4 * t * t * t : 1 - Math.pow(-2 * t + 2, 3) / 2);

export function hitTest(blocks: ViewBlock[], x: number, y: number): number {
  for (let i = blocks.length - 1; i >= 0; i--) {
    const b = blocks[i];
    if (x >= b.x && x < b.x + b.w && y >= b.y && y < b.y + b.h) return i;
  }
  return -1;
}

function blockFlags(b: ViewBlock): number {
  let f = 0;
  if (b.flags & BF_AGGREGATE) f |= FLAG_AGGREGATE | FLAG_LEAF;
  else if (b.flags & BF_DIR && b.header > 0) f |= FLAG_DIR;
  else f |= FLAG_LEAF; // files, and folders drawn as solid tiles
  if (b.flags & BF_PENDING) f |= FLAG_PENDING;
  return f;
}

function writeInstance(out: Float32Array, i: number, b: ViewBlock, x: number, y: number, w: number, h: number, alpha: number, flags: number) {
  const o = i * INSTANCE_FLOATS;
  const c = CATEGORY_COLORS[b.category];
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

export function Treemap({ layout, theme, reduceMotion, onHover, onActivate, onResize, demoHoverNode }: Props) {
  const hostRef = useRef<HTMLDivElement>(null);
  const labelRef = useRef<HTMLCanvasElement>(null);
  const state = useRef({
    blocks: [] as ViewBlock[],
    root: -1,
    size: { w: 0, h: 0 },
    anim: null as Anim | null,
    hovered: -1,
    hoverT: 0,
    hoverStart: 0,
    buf: new Float32Array(0),
    dirty: true,
    labelsDirty: true,
    theme,
    pending: false,
  });
  const cb = useRef({ onHover, onActivate, onResize });
  cb.current = { onHover, onActivate, onResize };

  // Track the host size; the parent requests a layout for it.
  useEffect(() => {
    const host = hostRef.current!;
    const s = state.current;
    const sync = () => {
      const r = host.getBoundingClientRect();
      stage.setViewport({ x: r.left, y: r.top, w: r.width, h: r.height });
      const w = Math.round(r.width);
      const h = Math.round(r.height);
      if (w !== s.size.w || h !== s.size.h) {
        s.size = { w, h };
        cb.current.onResize(w, h);
      }
    };
    const ro = new ResizeObserver(sync);
    ro.observe(host);
    window.addEventListener("resize", sync);
    sync();
    return () => {
      ro.disconnect();
      window.removeEventListener("resize", sync);
    };
  }, []);

  // New layout: morph from the previous one.
  useEffect(() => {
    const s = state.current;
    if (!layout) {
      s.blocks = [];
      s.root = -1;
      s.anim = null;
      s.dirty = s.labelsDirty = true;
      return;
    }
    const prev = s.blocks;
    const animate = prev.length > 0 && !stage.reduceMotion;
    // start = -1: the first frame stamps it with its own clock (rAF time can differ from performance.now()).
    s.anim = animate ? buildMorph(prev, s.root, layout.blocks, layout.root, s.size.w, s.size.h) : null;
    // Keep hovering the same node across streamed refreshes.
    const hoveredKey = s.hovered >= 0 ? prev[s.hovered]?.key : undefined;
    s.blocks = layout.blocks;
    s.hovered = hoveredKey !== undefined && layout.root === s.root ? layout.blocks.findIndex((b) => b.key === hoveredKey) : -1;
    if (s.hovered < 0) cb.current.onHover(null);
    s.root = layout.root;
    s.pending = layout.blocks.some((b) => b.flags & BF_PENDING);
    s.dirty = s.labelsDirty = true;
  }, [layout]);

  useEffect(() => {
    state.current.theme = theme;
    state.current.labelsDirty = true;
    if (stage.renderer) stage.renderer.theme = theme === "light" ? 1 : 0;
  }, [theme]);

  useEffect(() => {
    stage.reduceMotion = reduceMotion;
  }, [reduceMotion]);

  // Per frame: rebuild instances while animating; redraw labels when settled.
  useEffect(() => {
    const s = state.current;
    return stage.addHook((now) => {
      const r = stage.renderer;
      if (!r) return;
      r.theme = s.theme === "light" ? 1 : 0;

      const targetHover = s.hovered >= 0 ? 1 : 0;
      if (s.hoverT !== targetHover) {
        if (s.hoverStart < 0) s.hoverStart = now;
        const k = stage.reduceMotion ? 1 : Math.min(1, Math.max(0, (now - s.hoverStart) / HOVER_MS));
        s.hoverT = targetHover ? k : 1 - k;
        if (k >= 1) s.hoverT = targetHover;
        s.dirty = true;
      }
      r.hover = easeInOutCubic(s.hoverT);

      let t = 1;
      if (s.anim) {
        if (s.anim.start < 0) s.anim.start = now;
        t = Math.min(1, Math.max(0, (now - s.anim.start) / s.anim.duration));
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
      if (s.buf.length < total * INSTANCE_FLOATS) s.buf = new Float32Array(Math.ceil(total * 1.5) * INSTANCE_FLOATS);
      const buf = s.buf;
      let n = 0;
      const e = easeInOutCubic(t);
      if (anim) {
        const a = anim.outFrom;
        const z = anim.outTo;
        for (let i = 0; i < anim.outgoing.length; i++) {
          const o = i * 4;
          const b = anim.outgoing[i];
          writeInstance(buf, n++, b, a[o] + (z[o] - a[o]) * e, a[o + 1] + (z[o + 1] - a[o + 1]) * e, a[o + 2] + (z[o + 2] - a[o + 2]) * e, a[o + 3] + (z[o + 3] - a[o + 3]) * e, 1 - e, blockFlags(b));
        }
        const f = anim.from;
        for (let i = 0; i < blocks.length; i++) {
          const b = blocks[i];
          const o = i * 5;
          writeInstance(buf, n++, b, f[o] + (b.x - f[o]) * e, f[o + 1] + (b.y - f[o + 1]) * e, f[o + 2] + (b.w - f[o + 2]) * e, f[o + 3] + (b.h - f[o + 3]) * e, f[o + 4] + (1 - f[o + 4]) * e, blockFlags(b));
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
      // Pending folders shimmer, which needs continuous redraws.
      if (s.pending && !stage.reduceMotion) s.dirty = true;

      const lc = labelRef.current;
      if (lc) {
        const zooming = anim !== null && anim.duration === ZOOM_MS;
        lc.style.opacity = zooming ? "0" : "1";
        if (anim && !zooming) {
          // Streamed refresh: labels ride along with their gliding tiles.
          const f = anim.from;
          drawLabels(lc, blocks, s.size.w, s.size.h, s.theme, (i, b) => {
            const o = i * 5;
            return [f[o] + (b.x - f[o]) * e, f[o + 1] + (b.y - f[o + 1]) * e, f[o + 2] + (b.w - f[o + 2]) * e, f[o + 3] + (b.h - f[o + 3]) * e];
          });
          s.labelsDirty = true;
        } else if (!anim && s.labelsDirty) {
          s.labelsDirty = false;
          drawLabels(lc, blocks, s.size.w, s.size.h, s.theme);
        }
      }
    });
  }, []);

  // Screenshot helper: pin hover to a node once it is laid out.
  useEffect(() => {
    if (demoHoverNode === undefined || !layout) return;
    const s = state.current;
    const idx = s.blocks.findIndex((b) => b.node === demoHoverNode && !(b.flags & BF_AGGREGATE));
    if (idx < 0) return;
    s.hovered = idx;
    s.hoverT = 1;
    s.dirty = true;
    const b = s.blocks[idx];
    const r = hostRef.current!.getBoundingClientRect();
    cb.current.onHover({ block: b, clientX: r.left + b.x + b.w * 0.6, clientY: r.top + b.y + b.h * 0.45 });
  }, [demoHoverNode, layout]);

  const localPoint = (ev: React.MouseEvent) => {
    const r = hostRef.current!.getBoundingClientRect();
    return [ev.clientX - r.left, ev.clientY - r.top] as const;
  };

  const onMove = (ev: React.MouseEvent) => {
    const s = state.current;
    if (s.anim && s.anim.duration === ZOOM_MS) return;
    const [x, y] = localPoint(ev);
    const idx = hitTest(s.blocks, x, y);
    if (idx !== s.hovered) {
      const wasHover = s.hovered >= 0;
      s.hovered = idx;
      if (!wasHover || idx < 0) s.hoverStart = -1;
      else s.hoverT = 1;
      s.dirty = true;
    }
    cb.current.onHover(idx >= 0 ? { block: s.blocks[idx], clientX: ev.clientX, clientY: ev.clientY } : null);
  };

  const onLeave = () => {
    const s = state.current;
    if (s.hovered >= 0) {
      s.hovered = -1;
      s.hoverStart = -1;
      s.dirty = true;
    }
    cb.current.onHover(null);
  };

  const onClick = (ev: React.MouseEvent) => {
    const s = state.current;
    if (s.anim && s.anim.duration === ZOOM_MS) return;
    const [x, y] = localPoint(ev);
    const idx = hitTest(s.blocks, x, y);
    if (idx < 0) return;
    let top = idx;
    while (s.blocks[top].parent >= 0) top = s.blocks[top].parent;
    cb.current.onActivate(s.blocks[top], s.blocks[idx]);
  };

  return (
    <div ref={hostRef} className="treemap-host" onMouseMove={onMove} onMouseLeave={onLeave} onClick={onClick}>
      <canvas ref={labelRef} className="treemap-labels" />
    </div>
  );
}

/**
 * Start rects for the new layout and paths for blocks that leave.
 * Zoom (root changed): a camera move between the old and new views. Refresh (same root, e.g. streamed
 * scan data): blocks glide to their new rects and new ones fade in.
 */
function buildMorph(prev: ViewBlock[], prevRoot: number, next: ViewBlock[], nextRoot: number, W: number, H: number): Anim {
  const prevMap = new Map<number, ViewBlock>();
  for (const b of prev) prevMap.set(b.key, b);
  const nextKeys = new Set(next.map((b) => b.key));
  const zoom = prevRoot !== nextRoot;

  let cam = { x: 0, y: 0, w: W, h: H };
  if (zoom) {
    const inOld = prevMap.get(nextRoot);
    if (inOld) cam = { x: inOld.x, y: inOld.y, w: inOld.w, h: inOld.h };
    else {
      const inNew = next.find((b) => b.key === prevRoot);
      if (inNew) {
        const sx = W / inNew.w;
        const sy = H / inNew.h;
        cam = { x: -inNew.x * sx, y: -inNew.y * sy, w: W * sx, h: H * sy };
      }
    }
  }
  const toOld = (x: number, y: number, w: number, h: number) => [cam.x + (x / W) * cam.w, cam.y + (y / H) * cam.h, (w / W) * cam.w, (h / H) * cam.h];
  const toNew = (x: number, y: number, w: number, h: number) => [((x - cam.x) / cam.w) * W, ((y - cam.y) / cam.h) * H, (w / cam.w) * W, (h / cam.h) * H];

  const from = new Float32Array(next.length * 5);
  next.forEach((b, i) => {
    const p = prevMap.get(b.key);
    if (p) from.set([p.x, p.y, p.w, p.h, 1], i * 5);
    else from.set([...toOld(b.x, b.y, b.w, b.h), 0], i * 5);
  });
  const outgoing = prev.filter((b) => !nextKeys.has(b.key));
  const outFrom = new Float32Array(outgoing.length * 4);
  const outTo = new Float32Array(outgoing.length * 4);
  outgoing.forEach((b, i) => {
    outFrom.set([b.x, b.y, b.w, b.h], i * 4);
    outTo.set(zoom ? toNew(b.x, b.y, b.w, b.h) : [b.x, b.y, b.w, b.h], i * 4);
  });
  return { start: -1, duration: zoom ? ZOOM_MS : REFRESH_MS, from, outgoing, outFrom, outTo };
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

type RectOf = (i: number, b: ViewBlock) => [number, number, number, number];
const finalRect: RectOf = (_i, b) => [b.x, b.y, b.w, b.h];

function drawLabels(canvas: HTMLCanvasElement, blocks: ViewBlock[], w: number, h: number, theme: "dark" | "light", rectOf: RectOf = finalRect) {
  const dpr = window.devicePixelRatio || 1;
  const cw = Math.round(w * dpr);
  const ch = Math.round(h * dpr);
  if (canvas.width !== cw || canvas.height !== ch) {
    canvas.width = cw;
    canvas.height = ch;
    canvas.style.width = `${w}px`;
    canvas.style.height = `${h}px`;
  }
  const ctx = canvas.getContext("2d")!;
  ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
  ctx.clearRect(0, 0, w, h);
  ctx.textBaseline = "middle";

  const light = theme === "light";
  const primary = light ? "rgba(10, 16, 30, 0.94)" : "rgba(255, 255, 255, 0.96)";
  const secondary = light ? "rgba(10, 16, 30, 0.70)" : "rgba(235, 240, 255, 0.76)";
  ctx.shadowColor = light ? "rgba(255, 255, 255, 0.7)" : "rgba(0, 0, 10, 0.55)";
  ctx.shadowBlur = 3;

  const candidates: number[] = [];
  for (let i = 0; i < blocks.length; i++) {
    const b = blocks[i];
    if (b.header > 0 || (b.w >= 46 && b.h >= 22)) candidates.push(i);
  }
  candidates.sort((a, b) => blocks[b].w * blocks[b].h - blocks[a].w * blocks[a].h);
  const LIMIT = 700;
  for (let k = 0; k < Math.min(LIMIT, candidates.length); k++) {
    const src = blocks[candidates[k]];
    const [x, y, bw, bh] = rectOf(candidates[k], src);
    const b = { ...src, x, y, w: bw, h: bh };
    const agg = b.flags & BF_AGGREGATE;
    const name = agg ? `${formatCount(b.aggCount)} small files` : b.name;
    const size = formatBytes(b.size);
    if (b.header > 0) {
      ctx.font = `600 11.5px ${FONT_STACK}`;
      const sizeW = ctx.measureText(size).width;
      const cy = b.y + b.header / 2 + 1;
      const showSize = b.w > sizeW + 70;
      ctx.fillStyle = primary;
      ctx.fillText(fitText(ctx, name, b.w - 14 - (showSize ? sizeW + 10 : 0)), b.x + 7, cy);
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
