import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { open, save } from "@tauri-apps/plugin-dialog";
import { CATEGORY_BY_ID } from "../data/categories";
import { BF_AGGREGATE, type Backend, type Layout, type ViewBlock } from "./types";

const BLOCK_BYTES = 56;
const decoder = new TextDecoder();

/** Decode the binary layout payload described in `src-tauri/src/encode.rs`. */
export function decodeLayout(buf: ArrayBuffer): Layout {
  const v = new DataView(buf);
  const magic = String.fromCharCode(v.getUint8(0), v.getUint8(1), v.getUint8(2), v.getUint8(3));
  if (magic !== "DLLY") throw new Error(`bad layout payload (${magic})`);
  const count = v.getUint32(4, true);
  const namesLen = v.getUint32(8, true);
  const root = v.getUint32(12, true);
  const namesStart = 16 + count * BLOCK_BYTES;
  const names = decoder.decode(new Uint8Array(buf, namesStart, namesLen)).split("\0");
  const blocks: ViewBlock[] = new Array(count);
  for (let i = 0; i < count; i++) {
    const o = 16 + i * BLOCK_BYTES;
    const node = v.getUint32(o + 32, true);
    const flags = v.getUint8(o + 43);
    blocks[i] = {
      key: flags & BF_AGGREGATE ? -(node + 1) : node,
      node,
      size: v.getFloat64(o, true),
      x: v.getFloat32(o + 8, true),
      y: v.getFloat32(o + 12, true),
      w: v.getFloat32(o + 16, true),
      h: v.getFloat32(o + 20, true),
      header: v.getFloat32(o + 24, true),
      parent: v.getInt32(o + 28, true),
      aggCount: v.getUint32(o + 36, true),
      depth: v.getUint16(o + 40, true),
      category: CATEGORY_BY_ID[v.getUint8(o + 42)] ?? "other",
      flags,
      files: v.getUint32(o + 44, true),
      newestDays: v.getUint32(o + 48, true),
      name: names[i] ?? "",
    };
  }
  return { root, blocks };
}

export const tauriBackend: Backend = {
  native: true,
  listDrives: () => invoke("list_drives"),
  isElevated: () => invoke("is_elevated"),
  restartAsAdmin: (scan, theme) => invoke("restart_as_admin", { scan, theme }),
  startScan: (path, method) => invoke("start_scan", { path, method }),
  cancelScan: () => invoke("cancel_scan"),
  layout: async (generation, root, width, height, useAlloc) =>
    decodeLayout(await invoke<ArrayBuffer>("treemap_layout", { generation, root, width, height, useAlloc })),
  nodeInfo: (generation, id, useAlloc) => invoke("node_info", { generation, id, useAlloc }),
  children: (generation, id, useAlloc) => invoke("children", { generation, id, useAlloc, limit: 500 }),
  ancestors: (generation, id) => invoke("ancestors", { generation, id }),
  findPath: (generation, path) => invoke("find_path", { generation, path }),
  pickFolder: async () => {
    const r = await open({ directory: true, multiple: false, title: "Choose a folder to scan" });
    return typeof r === "string" ? r : null;
  },
  launchOptions: () => invoke("launch_options"),
  onProgress: (cb) => subscribe("scan-progress", cb),
  onDone: (cb) => subscribe("scan-done", cb),
  onError: (cb) => subscribe("scan-error", cb),
  onSummaryUpdated: (cb) => subscribe("summary-updated", cb),
  actionInfo: (generation, id) => invoke("action_info", { generation, id }),
  recycle: (generation, ids) => invoke("recycle_nodes", { generation, ids }),
  reveal: (generation, id) => invoke("reveal_node", { generation, id }),
  listCaches: () => invoke("list_caches"),
  openCache: (root) => invoke("open_cache", { root }),
  pickSavePath: async (defaultName, kind) => {
    const json = kind === "json-tree";
    const r = await save({
      title: "Export scan",
      defaultPath: defaultName,
      filters: [json ? { name: "JSON", extensions: ["json"] } : { name: "CSV", extensions: ["csv"] }],
    });
    return r ?? null;
  },
  exportTree: (generation, id, kind, path) =>
    invoke("export_tree", {
      generation,
      id,
      path,
      format: kind === "json-tree" ? "json" : "csv",
      foldersOnly: kind !== "csv-all",
      maxDepth: kind === "json-tree" ? 6 : null,
    }),
  startDuplicates: (generation, root, minSize) => invoke("start_duplicates", { generation, root, minSize }),
  cancelDuplicates: () => invoke("cancel_duplicates"),
  duplicates: (generation) => invoke("duplicates", { generation }),
  onDupesProgress: (cb) => subscribe("dupes-progress", cb),
  onDupesDone: (cb) => subscribe("dupes-done", cb),
  getIntegration: () => invoke("get_integration"),
  setIntegration: (settings) => invoke("set_integration", { settings }),
  testNotification: () => invoke("test_notification"),
  onOpenPath: (cb) => subscribe("open-path", cb),
  setBackdrop: (dark) => invoke("set_backdrop", { dark }),
  windowControl: async (action) => {
    const w = getCurrentWindow();
    if (action === "minimize") await w.minimize();
    else if (action === "maximize") await w.toggleMaximize();
    else await w.close();
  },
};

function subscribe<T>(event: string, cb: (p: T) => void): () => void {
  let unlisten: (() => void) | null = null;
  let cancelled = false;
  listen<T>(event, (e) => cb(e.payload)).then((u) => {
    if (cancelled) u();
    else unlisten = u;
  });
  return () => {
    cancelled = true;
    unlisten?.();
  };
}
