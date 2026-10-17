import type { Category, Drive } from "../data/types";

export type { Drive };

/** Block flag bits (match `encode.rs`). */
export const BF_AGGREGATE = 1;
export const BF_DIR = 2;
export const BF_LINK = 4;
export const BF_INACCESSIBLE = 8;
export const BF_PENDING = 16;

/** One treemap block as delivered by the backend. Draw order = array order. */
export interface ViewBlock {
  /** Stable key for morphing: node id, or -(node id + 1) for a folder's "small files" aggregate. */
  key: number;
  node: number;
  name: string;
  size: number;
  files: number;
  aggCount: number;
  x: number;
  y: number;
  w: number;
  h: number;
  header: number;
  parent: number;
  depth: number;
  category: Category;
  flags: number;
}

export interface Layout {
  root: number;
  blocks: ViewBlock[];
}

export interface ScanProgress {
  generation: number;
  files: number;
  dirs: number;
  bytes: number;
  elapsedMs: number;
}

export interface Ranked {
  node: number;
  path: string;
  size: number;
  alloc: number;
  files: number;
}

export interface ScanSummary {
  generation: number;
  root: string;
  method: "auto" | "walk" | "findfirst" | "mft";
  fallbackReason: string | null;
  cancelled: boolean;
  elapsedMs: number;
  /** When the scan finished (Unix ms). */
  scannedAtMs: number;
  /** Opened from the scan cache rather than scanned just now. */
  fromCache: boolean;
  files: number;
  dirs: number;
  size: number;
  sizeOnDisk: number;
  nodes: number;
  treeBytes: number;
  inaccessible: { path: string; error: string }[];
  largestFiles: Ranked[];
  largestFolders: Ranked[];
  extensions: { ext: string; size: number; files: number }[];
  categories: { category: Category; size: number; files: number }[];
  hogs: Hog[];
}

export interface Hog {
  id: string;
  title: string;
  why: string;
  advice: string;
  severity: "reclaimable" | "system" | "info";
  recyclable: boolean;
  size: number;
  count: number;
  items: { node: number; path: string; size: number }[];
}

export type Protection = { kind: "none" } | { kind: "warn"; reason: string } | { kind: "blocked"; reason: string };

export interface ActionInfo {
  id: number;
  name: string;
  path: string;
  isDir: boolean;
  size: number;
  files: number;
  protection: Protection;
  /** Why "Send to Recycle Bin" is unavailable, if it is. */
  recycleBlocked: string | null;
}

export interface RecycleResponse {
  recycled: number[];
  failed: [string, string][];
  permanentlyDeleted: string[];
  freed: number;
}

export interface NodeInfo {
  id: number;
  name: string;
  path: string;
  size: number;
  sizeOnDisk: number;
  files: number;
  dirs: number;
  mtimeMs: number;
  isDir: boolean;
  category: number;
  parent: number | null;
  parentSize: number;
  link: boolean;
  inaccessible: boolean;
  hardlinkDup: boolean;
}

export interface Row {
  id: number;
  name: string;
  size: number;
  sizeOnDisk: number;
  files: number;
  mtimeMs: number;
  isDir: boolean;
  hasChildren: boolean;
  category: number;
  link: boolean;
  inaccessible: boolean;
}

export interface Crumb {
  id: number;
  name: string;
  size: number;
}

export interface CacheEntry {
  root: string;
  scannedAtMs: number;
  method: ScanSummary["method"];
  elapsedMs: number;
  size: number;
  files: number;
  dirs: number;
}

export type ExportKind = "csv-all" | "csv-folders" | "json-tree";

export interface LaunchOptions {
  scan: string | null;
  focus: string | null;
  theme: "light" | "dark" | null;
}

export interface Backend {
  /** True when running inside the Tauri shell. */
  native: boolean;
  listDrives(): Promise<Drive[]>;
  isElevated(): Promise<boolean>;
  /** Relaunch elevated (UAC prompt) so drive scans can read the MFT; this instance then exits. */
  restartAsAdmin(scan: string | null, theme: string): Promise<void>;
  startScan(path: string, method?: string): Promise<number>;
  cancelScan(): Promise<void>;
  layout(generation: number, root: number, width: number, height: number, useAlloc: boolean): Promise<Layout>;
  nodeInfo(generation: number, id: number, useAlloc: boolean): Promise<NodeInfo>;
  children(generation: number, id: number, useAlloc: boolean): Promise<Row[]>;
  ancestors(generation: number, id: number): Promise<Crumb[]>;
  findPath(generation: number, path: string): Promise<number | null>;
  pickFolder(): Promise<string | null>;
  /** Command-line options: folder to scan at launch, folder to focus afterwards, theme. */
  launchOptions(): Promise<LaunchOptions>;
  onProgress(cb: (p: ScanProgress) => void): () => void;
  onDone(cb: (s: ScanSummary) => void): () => void;
  onError(cb: (e: { generation: number; error: string }) => void): () => void;
  /** Insights rebuilt after items were sent to the Recycle Bin. */
  onSummaryUpdated(cb: (s: ScanSummary) => void): () => void;
  actionInfo(generation: number, id: number): Promise<ActionInfo>;
  /** Send to the Recycle Bin. Only call after the user confirmed. */
  recycle(generation: number, ids: number[]): Promise<RecycleResponse>;
  reveal(generation: number, id: number): Promise<void>;
  /** Cached scans, newest first. */
  listCaches(): Promise<CacheEntry[]>;
  /** Open a cached scan instantly; resolves with its generation, then `scan-done` follows. */
  openCache(root: string): Promise<number>;
  /** Ask where to save; null if cancelled. */
  pickSavePath(defaultName: string, kind: ExportKind): Promise<string | null>;
  exportTree(generation: number, id: number, kind: ExportKind, path: string): Promise<{ items: number }>;
  setBackdrop(dark: boolean): Promise<void>;
  windowControl(action: "minimize" | "maximize" | "close"): Promise<void>;
}
