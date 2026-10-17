// In-browser stand-in for the Tauri backend, over the fake C:\ tree. Lets the UI run in a plain browser
// (design review, screenshots, renderer benchmarks) with the same code paths as the desktop app.

import { CATEGORY_BY_ID } from "../data/categories";
import { FAKE_DRIVES, FAKE_ROOT, makeStressTree, pathOf } from "../data/fakeTree";
import { categoryTotals, extensionTotals, findHogs, largestFiles } from "../data/insights";
import type { TreeNode } from "../data/types";
import { layoutTreemap } from "../treemap/layout";
import { BF_AGGREGATE, BF_DIR, type AgeSummary, type Backend, type Change, type DupeProgress, type DupeReport, type Layout, type Protection, type Ranked, type Row, type ScanChanges, type ScanProgress, type ScanSummary, type ViewBlock } from "./types";

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
  dupesProgress: new Set<(p: DupeProgress) => void>(),
  dupesDone: new Set<(r: DupeReport) => void>(),
};

const DAY_MS = 86_400_000;
/** Fake folders' mtime is already the newest file inside them. */
const daysOf = (n: TreeNode) => (n.mtime > 0 ? Math.floor(n.mtime / DAY_MS) : 0);

function allFiles(n: TreeNode, out: TreeNode[] = []): TreeNode[] {
  if (!n.isDir) out.push(n);
  else n.children.forEach((c) => allFiles(c, out));
  return out;
}

const ranked = (n: TreeNode, size = n.size): Ranked => ({ node: idOf(n), path: pathOf(n), size, alloc: n.onDisk, files: n.files });

/** Same bands and stale rule as `age_summary` in analysis.rs. */
function mockAge(): AgeSummary {
  const bandsDays = [0, 30, 182, 365, 730, 1826];
  const bands = bandsDays.map((minDays) => ({ minDays, size: 0, files: 0 }));
  const today = Math.floor(Date.now() / DAY_MS);
  const stale: TreeNode[] = [];
  const ownStale = new Map<TreeNode, number>();
  for (const f of allFiles(ROOT)) {
    const age = today - daysOf(f);
    let i = 0;
    while (i + 1 < bandsDays.length && age >= bandsDays[i + 1]) i++;
    bands[i].size += f.size;
    bands[i].files++;
    if (age >= 730) {
      stale.push(f);
      if (f.parent) ownStale.set(f.parent, (ownStale.get(f.parent) ?? 0) + f.size);
    }
  }
  return {
    bands,
    unknownSize: 0,
    unknownFiles: 0,
    staleDays: 730,
    staleSize: stale.reduce((s, f) => s + f.size, 0),
    staleFiles: stale.length,
    staleLargest: stale.sort((a, b) => b.size - a.size).slice(0, 50).map((f) => ranked(f)),
    staleFolders: [...ownStale].sort((a, b) => b[1] - a[1]).slice(0, 50).map(([n, size]) => ranked(n, size)),
  };
}

/**
 * Plausible duplicate sets over the fake tree (it has no real duplicates): big media, archives and
 * installers paired up across folders. Demo only; "copies" can differ slightly in size.
 */
function mockDupes(rootId: number, minSize: number): DupeReport {
  const root = byId.get(rootId) ?? ROOT;
  const files = allFiles(root).filter((f) => f.size >= minSize && ["video", "archive", "image", "executable"].includes(f.category));
  files.sort((a, b) => b.size - a.size);
  const sets: DupeReport["sets"] = [];
  const used = new Set<TreeNode>();
  for (const f of files.slice(0, 120)) {
    if (used.has(f) || sets.length >= 14) continue;
    const mates = files.filter((g) => !used.has(g) && g !== f && g.category === f.category && g.parent !== f.parent).slice(0, sets.length % 3 === 0 ? 2 : 1);
    if (!mates.length) continue;
    const group = [f, ...mates];
    group.forEach((g) => used.add(g));
    sets.push({ size: f.size, files: group.map((g) => ({ node: idOf(g), path: pathOf(g), mtimeMs: g.mtime })).sort((a, b) => a.path.localeCompare(b.path)) });
  }
  sets.sort((a, b) => b.size * (b.files.length - 1) - a.size * (a.files.length - 1));
  return {
    generation,
    root: rootId,
    rootPath: pathOf(root),
    minSize,
    sets,
    totalSets: sets.length,
    wasted: sets.reduce((s, x) => s + x.size * (x.files.length - 1), 0),
    extraCopies: sets.reduce((s, x) => s + x.files.length - 1, 0),
    candidates: files.length,
    bytesRead: files.reduce((s, f) => s + f.size, 0),
    skippedCloud: 0,
    unreadable: 0,
    cancelled: false,
    elapsedMs: 1400,
  };
}

let dupes: DupeReport | null = null;
let mockSettings = { explorerMenu: false, tray: false, startWithWindows: false, lowDiskPercent: 10 };
/** Ids of nodes "sent to the Recycle Bin" in the demo. */
const removed = new Set<number>();
let dupeTimer: number | undefined;
let timer: number | undefined;

function catId(c: string) {
  return Math.max(0, CATEGORY_BY_ID.indexOf(c as never));
}

/**
 * Plausible "since the last scan" changes over the fake tree, for the Changes tab: a new download, a
 * virtual disk and a few files that grew, one that shrank, and a backup that is gone.
 */
function mockChanges(): ScanChanges | null {
  if (ROOT !== FAKE_ROOT) return null;
  const files = largestFiles(ROOT, 300);
  const pickFile = (re: RegExp, skip: TreeNode[]) => files.find((f) => re.test(pathOf(f)) && !skip.includes(f));
  const used: TreeNode[] = [];
  const raw: { node: TreeNode | null; parent: TreeNode; path: string; kind: Change["kind"]; before: number; after: number }[] = [];
  const add = (re: RegExp, kind: Change["kind"], factor: number) => {
    const f = pickFile(re, used);
    if (!f?.parent) return;
    used.push(f);
    const before = kind === "added" ? 0 : Math.round(kind === "grew" ? f.size / factor : f.size * factor);
    raw.push({ node: f, parent: f.parent, path: pathOf(f), kind, before, after: f.size });
  };
  add(/\\downloads\\/i, "added", 1);
  add(/\.vhdx$/i, "grew", 1.6);
  add(/\\downloads\\/i, "added", 1);
  add(/\\appdata\\/i, "grew", 1.3);
  add(/\\videos\\/i, "shrank", 1.8);
  add(/\\documents\\/i, "grew", 1.15);
  const dl = byId.get(mockFindId("Users/Alex/Downloads") ?? -1);
  if (dl) raw.push({ node: null, parent: dl, path: `${pathOf(dl)}\\old-backup.zip`, kind: "removed", before: 3.2 * 1024 ** 3, after: 0 });

  const toChange = (r: (typeof raw)[number]): Change => ({ path: r.path, kind: r.kind, isDir: false, before: r.before, after: r.after, delta: r.after - r.before, node: r.node ? idOf(r.node) : null });
  const items = raw.map(toChange).sort((a, b) => Math.abs(b.delta) - Math.abs(a.delta));
  const byFolder = new Map<TreeNode, number>();
  raw.forEach((r) => byFolder.set(r.parent, (byFolder.get(r.parent) ?? 0) + r.after - r.before));
  const folders = [...byFolder]
    .filter(([, delta]) => delta !== 0)
    .map(([n, delta]): Change => ({ path: pathOf(n), kind: delta > 0 ? "grew" : "shrank", isDir: true, before: n.size - delta, after: n.size, delta, node: idOf(n) }))
    .sort((a, b) => Math.abs(b.delta) - Math.abs(a.delta));
  const grew = items.reduce((s, c) => s + Math.max(0, c.delta), 0);
  const shrank = items.reduce((s, c) => s + Math.max(0, -c.delta), 0);
  const count = (k: Change["kind"]) => items.filter((c) => c.kind === k).length;
  return {
    sinceMs: Date.now() - 3 * 86_400_000,
    methodDiffers: false,
    before: ROOT.size - grew + shrank,
    after: ROOT.size,
    filesBefore: ROOT.files - count("added") + count("removed"),
    filesAfter: ROOT.files,
    grew,
    shrank,
    added: count("added"),
    removed: count("removed"),
    changed: count("grew") + count("shrank"),
    items,
    folders,
  };
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
    scannedAtMs: Date.now(),
    fromCache: false,
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
    age: mockAge(),
    changes: mockChanges(),
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
  (function forget(x: TreeNode) {
    removed.add(idOf(x));
    x.children.forEach(forget);
  })(n);
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
  restartAsAdmin: async () => {
    throw new Error("Only the desktop app can restart as administrator.");
  },
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
          newestDays: daysOf(b.node),
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
  pickFolder: async () => "C:\\Users\\Alex",
  launchOptions: async () => ({ scan: params.has("picker") ? null : "C:\\", saved: false, focus: null, theme: null }),
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
  listCaches: async () => [
    { root: "C:\\", scannedAtMs: Date.now() - 2 * 3600_000, method: "walk", elapsedMs: 4200, size: ROOT.size, files: ROOT.files, dirs: byId.size - ROOT.files },
  ],
  openCache: async () => {
    generation++;
    const g = generation;
    setTimeout(() => {
      const s = { ...summary(), generation: g, fromCache: true, scannedAtMs: Date.now() - 2 * 3600_000, changes: null };
      listeners.done.forEach((cb) => cb(s));
    }, 0);
    return g;
  },
  pickSavePath: async (name) => name,
  exportTree: async () => {
    throw new Error("Exporting needs the desktop app.");
  },
  startDuplicates: async (g, root, minSize) => {
    clearInterval(dupeTimer);
    const report = mockDupes(root, minSize);
    const start = performance.now();
    dupeTimer = window.setInterval(() => {
      const t = params.has("instant") ? 1 : Math.min(1, (performance.now() - start) / 1400);
      const p: DupeProgress = {
        generation: g,
        phase: t < 0.4 ? 1 : 2,
        filesDone: Math.round(report.candidates * t),
        filesTotal: report.candidates,
        bytesDone: Math.round(report.bytesRead * t),
        bytesTotal: report.bytesRead,
      };
      listeners.dupesProgress.forEach((cb) => cb(p));
      if (t >= 1) {
        clearInterval(dupeTimer);
        dupes = { ...report, generation: g };
        listeners.dupesDone.forEach((cb) => cb(dupes!));
      }
    }, 120);
  },
  cancelDuplicates: async () => {
    clearInterval(dupeTimer);
    dupes = { ...mockDupes(0, Infinity), cancelled: true };
    listeners.dupesDone.forEach((cb) => cb(dupes!));
  },
  duplicates: async () => {
    if (!dupes) return null;
    const sets = dupes.sets.map((s) => ({ ...s, files: s.files.filter((f) => !removed.has(f.node)) })).filter((s) => s.files.length > 1);
    return { ...dupes, sets, totalSets: sets.length, wasted: sets.reduce((a, s) => a + s.size * (s.files.length - 1), 0), extraCopies: sets.reduce((a, s) => a + s.files.length - 1, 0) };
  },
  onDupesProgress: (cb) => (listeners.dupesProgress.add(cb), () => listeners.dupesProgress.delete(cb)),
  onDupesDone: (cb) => (listeners.dupesDone.add(cb), () => listeners.dupesDone.delete(cb)),
  getIntegration: async () => ({ settings: mockSettings, trayActive: false, trayElsewhere: false }),
  setIntegration: async (s) => {
    // Demo: remembered for this page only; nothing touches Windows.
    mockSettings = { ...s, startWithWindows: s.tray && s.startWithWindows };
    return { settings: mockSettings, trayActive: false, trayElsewhere: false };
  },
  testNotification: async () => {
    throw new Error("Notifications need the desktop app.");
  },
  onOpenPath: () => () => {},
  setBackdrop: async () => {},
  windowControl: async () => {},
};

/** Resolve a `/`-separated path under the fake root to a node id (for `?zoom=` / `?hover=`). */
export function mockFindId(path: string): number | undefined {
  let cur: TreeNode | undefined = ROOT;
  for (const part of path.split("/").filter(Boolean)) cur = cur?.children.find((c) => c.name.toLowerCase() === part.toLowerCase());
  return cur ? idOf(cur) : undefined;
}
