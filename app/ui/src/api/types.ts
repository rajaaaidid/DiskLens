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
  /** Newest modification of a file inside, as days since 1970 (0 = unknown). */
  newestDays: number;
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
  age: AgeSummary;
  /** What changed since the previous saved scan of this root; null until compared, or with nothing to compare. */
  changes: ScanChanges | null;
}

export interface AgeSummary {
  /** Six bands (see data/age.ts), newest first. */
  bands: { minDays: number; size: number; files: number }[];
  unknownSize: number;
  unknownFiles: number;
  /** Files untouched for at least this many days count as stale. */
  staleDays: number;
  staleSize: number;
  staleFiles: number;
  staleLargest: Ranked[];
  /** Folders ranked by stale data in their own files. */
  staleFolders: Ranked[];
}

export interface DupeFile {
  node: number;
  path: string;
  /** Unix ms, 0 if unknown. */
  mtimeMs: number;
}

export interface DupeSet {
  /** Size of one copy. */
  size: number;
  files: DupeFile[];
}

/** A finished duplicate search (re-filtered for anything recycled since). */
export interface DupeReport {
  generation: number;
  /** Folder that was searched. */
  root: number;
  rootPath: string;
  minSize: number;
  /** Largest waste first; capped, see totalSets. */
  sets: DupeSet[];
  totalSets: number;
  wasted: number;
  extraCopies: number;
  candidates: number;
  bytesRead: number;
  skippedCloud: number;
  unreadable: number;
  cancelled: boolean;
  elapsedMs: number;
}

export interface DupeProgress {
  generation: number;
  /** 1 = sampling starts and ends, 2 = comparing whole files. */
  phase: number;
  filesDone: number;
  filesTotal: number;
  bytesDone: number;
  bytesTotal: number;
}

export interface Change {
  path: string;
  kind: "added" | "removed" | "grew" | "shrank";
  isDir: boolean;
  /** Size in the earlier scan and now (for folders, the whole folder). */
  before: number;
  after: number;
  /** Bytes gained, negative when freed. For folders, only the change directly inside them. */
  delta: number;
  /** Node in the current scan; null for removed items. */
  node: number | null;
}

export interface ScanChanges {
  /** When the earlier scan finished (Unix ms). */
  sinceMs: number;
  /** The earlier scan used another method (MFT vs walk), so small differences are expected. */
  methodDiffers: boolean;
  before: number;
  after: number;
  filesBefore: number;
  filesAfter: number;
  /** Totals over every changed item, not just the listed ones. */
  grew: number;
  shrank: number;
  added: number;
  removed: number;
  changed: number;
  /** Largest changes first: changed files, and new or gone files and folders. */
  items: Change[];
  /** Folders ranked by the change directly inside them. */
  folders: Change[];
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
  /** Open the saved scan of `scan` instead of scanning (jump list). */
  saved: boolean;
  focus: string | null;
  theme: "light" | "dark" | null;
}

export interface IntegrationSettings {
  /** "Analyze with HaveSpace" in Explorer's right-click menu. */
  explorerMenu: boolean;
  /** Keep running in the notification area and warn when a drive runs low. */
  tray: boolean;
  /** Start hidden in the tray at sign-in (needs `tray`). */
  startWithWindows: boolean;
  /** Warn below this share of free space. */
  lowDiskPercent: number;
}

export interface IntegrationInfo {
  settings: IntegrationSettings;
  /** This window owns the tray icon. */
  trayActive: boolean;
  /** The tray is on, but another HaveSpace window owns it. */
  trayElsewhere: boolean;
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
  /** Look for duplicate files under `root` in the background; progress and the result arrive as events. */
  startDuplicates(generation: number, root: number, minSize: number): Promise<void>;
  cancelDuplicates(): Promise<void>;
  /** The last finished search, without anything recycled since; null if none. */
  duplicates(generation: number): Promise<DupeReport | null>;
  onDupesProgress(cb: (p: DupeProgress) => void): () => void;
  onDupesDone(cb: (r: DupeReport) => void): () => void;
  getIntegration(): Promise<IntegrationInfo>;
  setIntegration(settings: IntegrationSettings): Promise<IntegrationInfo>;
  /** Show the low-disk notification once, as a preview. */
  testNotification(): Promise<void>;
  /** A notification was clicked: show this drive (its saved scan when `saved`). */
  onOpenPath(cb: (p: { path: string; saved: boolean }) => void): () => void;
  setBackdrop(dark: boolean): Promise<void>;
  windowControl(action: "minimize" | "maximize" | "close"): Promise<void>;
}
