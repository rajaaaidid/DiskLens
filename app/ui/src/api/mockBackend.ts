// In-browser stand-in for the Tauri backend, over the fake C:\ tree. Lets the UI run in a plain browser
// (design review, screenshots, renderer benchmarks) with the same code paths as the desktop app.

import { CATEGORY_BY_ID } from "../data/categories";
import { FAKE_DRIVES, FAKE_ROOT, makeStressTree, pathOf } from "../data/fakeTree";
import { categoryTotals, extensionTotals, findHogs, largestFiles } from "../data/insights";
import type { TreeNode } from "../data/types";
import { layoutTreemap } from "../treemap/layout";
import { BF_AGGREGATE, BF_DIR, type Backend, type Layout, type Protection, type Row, type ScanProgress, type ScanSummary, type ViewBlock } from "./types";

const params = new URLSearchParams(location.search);
const ROOT: TreeNode = params.get("stress") ? makeStressTree(Number(params.get("stress"))) : FAKE_ROOT;
const MIN_AREA = params.get("stress") ? 2 : undefined;

const byId = new Map<number, TreeNode>();
(function index(n: TreeNode) {
  byId.set(n === ROOT ? 0 : n.id, n);
  n.children.forEach(index);
})(ROOT);
const idOf = (n: TreeNode) => (n === ROOT ? 0 : n.id);

let generation = 0;
const listeners = {
  progress: new Set<(p: ScanProgress) => void>(),
  done: new Set<(s: ScanSummary) => void>(),
  summary: new Set<(s: ScanSummary) => void>(),
};
let timer: number | undefined;

function catId(c: string) {
  return Math.max(0, CATEGORY_BY_ID.indexOf(c as never));
}

function summary(): ScanSummary {
  const nodes = byId.size;
  return {
    generation,
    root: "C:\\",
    method: "walk",
    fallbackReason: "MFT reader not implemented yet",
    cancelled: false,
    elapsedMs: 4200,
    files: ROOT.files,
    dirs: nodes - ROOT.files - 1,
    size: ROOT.size,
    sizeOnDisk: ROOT.onDisk,
    nodes,
    treeBytes: nodes * 70,
    inaccessible: [{ path: "C:\\System Volume Information", error: "Access is denied. (os error 5)" }],
    largestFiles: largestFiles(ROOT, 50).map((f) => ({ node: idOf(f), path: pathOf(f), size: f.size, alloc: f.onDisk, files: 1 })),
    largestFolders: [],
    extensions: extensionTotals(ROOT, 40).map(({ ext, size, files }) => ({ ext, size, files })),
    categories: categoryTotals(ROOT),
    hogs: findHogs(ROOT).map((h) => ({
      id: h.id,
      title: h.title,
      why: h.why,
      advice: h.advice,
      severity: h.severity,
      recyclable: ["temp", "browsercache", "node_modules"].includes(h.id),
      size: h.size,
      count: 1,
      items: [{ node: idOf(h.node), path: pathOf(h.node), size: h.node.size }],
    })),
  };
}

/** Mock version of the Rust protection rules (enough for the demo). */
function mockProtection(path: string): Protection {
  const p = path.toLowerCase().replace(/\\$/, "");
  if (/^[a-z]:$/.test(p)) return { kind: "blocked", reason: "This is the root of a drive." };
  if (/^c:\\windows(\\|$)/.test(p)) return { kind: "blocked", reason: "Part of Windows. Use Disk Cleanup or Storage settings to reclaim space here." };
  if (/^c:\\(pagefile|hiberfil|swapfile)\.sys$/.test(p)) return { kind: "blocked", reason: "Used by Windows while it runs. Change it through Windows settings instead." };
  if (/^c:\\(program files( \(x86\))?|programdata|users(\\[^\\]+)?)$/.test(p)) return { kind: "blocked", reason: "A Windows system folder." };
  if (/^c:\\(system volume information|\$recycle\.bin)/.test(p)) return { kind: "blocked", reason: "Managed by Windows." };
  if (/^c:\\program files/.test(p)) return { kind: "warn", reason: "Installed apps live here. Uninstalling from Settings › Apps is safer than deleting files." };
  if (/\\appdata\\/.test(p)) return { kind: "warn", reason: "App data: programs may lose settings or caches. Close the app first." };
  return { kind: "none" };
}

function removeNode(n: TreeNode) {
  const p = n.parent;
  if (!p) return;
  p.children = p.children.filter((c) => c !== n);
  for (let a: TreeNode | null = p; a; a = a.parent) {
    a.size -= n.size;
    a.onDisk -= n.onDisk;
    a.files -= n.files;
  }
}

export const mockBackend: Backend = {
  native: false,
  listDrives: async () => FAKE_DRIVES.map((d) => ({ ...d, kind: "fixed" })),
  isElevated: async () => false,
  startScan: async () => {
    generation++;
    const g = generation;
    const start = performance.now();
    clearInterval(timer);
    const instant = params.has("instant");
    timer = window.setInterval(
      () => {
        const t = instant ? 1 : Math.min(1, (performance.now() - start) / 1800);
        const p = { generation: g, files: Math.round(ROOT.files * t), dirs: Math.round((byId.size - ROOT.files) * t), bytes: Math.round(ROOT.size * t), elapsedMs: performance.now() - start };
        listeners.progress.forEach((cb) => cb(p));
        if (t >= 1) {
          clearInterval(timer);
          const s = summary();
          listeners.done.forEach((cb) => cb(s));
        }
      },
      instant ? 0 : 120,
    );
    return g;
  },
  cancelScan: async () => clearInterval(timer),
  layout: async (_g, root, width, height): Promise<Layout> => {
    const node = byId.get(root) ?? ROOT;
    const blocks = layoutTreemap(node, width, height, MIN_AREA ? { minArea: MIN_AREA } : {});
    (window as unknown as { __blocks: number }).__blocks = blocks.length;
    return {
      root,
      blocks: blocks.map(
        (b): ViewBlock => ({
          key: b.isAggregate ? -(idOf(b.node) + 1) : idOf(b.node),
          node: idOf(b.node),
          name: b.isAggregate ? "" : b.node.name,
          size: b.isAggregate ? b.aggSize : b.node.size,
          files: b.isAggregate ? b.aggCount : b.node.files,
          aggCount: b.aggCount,
          x: b.x,
          y: b.y,
          w: b.w,
          h: b.h,
          header: b.header,
          parent: b.parent,
          depth: b.depth,
          category: b.isAggregate ? "other" : b.node.isDir ? (b.expanded ? "folder" : b.node.dominant) : b.node.category,
          flags: (b.isAggregate ? BF_AGGREGATE : 0) | (!b.isAggregate && b.node.isDir ? BF_DIR : 0),
        }),
      ),
    };
  },
  nodeInfo: async (_g, id) => {
    const n = byId.get(id)!;
    return {
      id,
      name: n.name,
      path: pathOf(n),
      size: n.size,
      sizeOnDisk: n.onDisk,
      files: n.files,
      dirs: 0,
      mtimeMs: n.mtime,
      isDir: n.isDir,
      category: catId(n.category),
      parent: n.parent ? idOf(n.parent) : null,
      parentSize: n.parent?.size ?? n.size,
      link: false,
      inaccessible: false,
      hardlinkDup: false,
    };
  },
  children: async (_g, id) => {
    const n = byId.get(id)!;
    return [...n.children]
      .sort((a, b) => b.size - a.size)
      .slice(0, 500)
      .map(
        (c): Row => ({
          id: idOf(c),
          name: c.name,
          size: c.size,
          sizeOnDisk: c.onDisk,
          files: c.files,
          mtimeMs: c.mtime,
          isDir: c.isDir,
          hasChildren: c.children.length > 0,
          category: catId(c.isDir ? c.dominant : c.category),
          link: false,
          inaccessible: false,
        }),
      );
  },
  ancestors: async (_g, id) => {
    const out = [];
    for (let n: TreeNode | null = byId.get(id) ?? ROOT; n; n = n.parent) out.unshift({ id: idOf(n), name: n.name, size: n.size });
    return out;
  },
  findPath: async (_g, path) => mockFindId(path.replace(/^C:\\/i, "").split("\\").join("/")) ?? null,
  pickFolder: async () => "C:\\Users\\Aidid",
  launchOptions: async () => ({ scan: params.has("picker") ? null : "C:\\", focus: null, theme: null }),
  onProgress: (cb) => (listeners.progress.add(cb), () => listeners.progress.delete(cb)),
  onDone: (cb) => (listeners.done.add(cb), () => listeners.done.delete(cb)),
  onError: () => () => {},
  onSummaryUpdated: (cb) => (listeners.summary.add(cb), () => listeners.summary.delete(cb)),
  actionInfo: async (_g, id) => {
    const n = byId.get(id)!;
    const path = pathOf(n);
    const protection = id === 0 ? ({ kind: "blocked", reason: "This is the folder you scanned." } as const) : mockProtection(path);
    return {
      id,
      name: n.name,
      path,
      isDir: n.isDir,
      size: n.size,
      files: n.files,
      protection,
      recycleBlocked: protection.kind === "blocked" ? protection.reason : null,
    };
  },
  recycle: async (_g, ids) => {
    // Demo only: removes the items from the fake tree. Nothing on disk is touched.
    let freed = 0;
    const recycled: number[] = [];
    for (const id of ids) {
      const n = byId.get(id);
      if (!n || id === 0 || mockProtection(pathOf(n)).kind === "blocked") continue;
      freed += n.size;
      removeNode(n);
      recycled.push(id);
    }
    const s = summary();
    listeners.summary.forEach((cb) => cb(s));
    return { recycled, failed: [], permanentlyDeleted: [], freed };
  },
  reveal: async () => {},
  setBackdrop: async () => {},
  windowControl: async () => {},
};

/** Resolve a `/`-separated path under the fake root to a node id (for `?zoom=` / `?hover=`). */
export function mockFindId(path: string): number | undefined {
  let cur: TreeNode | undefined = ROOT;
  for (const part of path.split("/").filter(Boolean)) cur = cur?.children.find((c) => c.name.toLowerCase() === part.toLowerCase());
  return cur ? idOf(cur) : undefined;
}
