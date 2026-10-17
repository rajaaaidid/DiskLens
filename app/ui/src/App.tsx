import { useCallback, useEffect, useRef, useState } from "react";
import { backend, BF_AGGREGATE, BF_DIR, type ActionInfo, type CacheEntry, type Crumb, type Drive, type ExportKind, type Layout, type ScanProgress, type ScanSummary, type ViewBlock } from "./api";
import { ExportMenu } from "./components/ExportMenu";
import { mockFindId } from "./api/mockBackend";
import { ConfirmRecycle } from "./components/ConfirmRecycle";
import { ContextMenu, type MenuTarget } from "./components/ContextMenu";
import { DrivePicker } from "./components/DrivePicker";
import { IconHome, IconRescan } from "./components/Icons";
import { Toasts, type Toast } from "./components/Toasts";
import { InsightsPanel } from "./components/InsightsPanel";
import { Sidebar } from "./components/Sidebar";
import { TitleBar } from "./components/TitleBar";
import { Tooltip } from "./components/Tooltip";
import { TreeList } from "./components/TreeList";
import { formatBytes } from "./data/format";
import { stage } from "./treemap/stage";
import { Treemap, type HoverInfo } from "./treemap/Treemap";

const params = new URLSearchParams(location.search);
/** While scanning, refresh the streamed treemap at most this often. */
const STREAM_REFRESH_MS = 450;

/** Clipboard write that also works when the async Clipboard API is refused (e.g. unfocused webview). */
async function writeClipboard(text: string) {
  try {
    await navigator.clipboard.writeText(text);
    return;
  } catch {
    const ta = document.createElement("textarea");
    ta.value = text;
    ta.style.position = "fixed";
    ta.style.opacity = "0";
    document.body.appendChild(ta);
    ta.select();
    const ok = document.execCommand("copy");
    ta.remove();
    if (!ok) throw new Error("The clipboard is not available.");
  }
}

function usePrefersReducedMotion() {
  const [v, setV] = useState(() => matchMedia("(prefers-reduced-motion: reduce)").matches);
  useEffect(() => {
    const m = matchMedia("(prefers-reduced-motion: reduce)");
    const on = () => setV(m.matches);
    m.addEventListener("change", on);
    return () => m.removeEventListener("change", on);
  }, []);
  return v;
}

export default function App() {
  const bgRef = useRef<HTMLCanvasElement>(null);
  const [theme, setTheme] = useState<"dark" | "light">(params.get("theme") === "light" ? "light" : "dark");
  const reduceMotion = usePrefersReducedMotion();

  const [drives, setDrives] = useState<Drive[]>([]);
  const [elevated, setElevated] = useState(false);

  const [generation, setGeneration] = useState(0);
  const genRef = useRef(0);
  const [target, setTarget] = useState<string | null>(null);
  const [scanning, setScanning] = useState(false);
  const [progress, setProgress] = useState<ScanProgress | null>(null);
  const [summary, setSummary] = useState<ScanSummary | null>(null);
  const [error, setError] = useState<string | null>(null);

  const [rootId, setRootId] = useState(0);
  const [layout, setLayout] = useState<Layout | null>(null);
  const [size, setSize] = useState({ w: 0, h: 0 });
  const [useAlloc, setUseAlloc] = useState(false);
  const [tick, setTick] = useState(0);
  const lastTick = useRef(0);
  const [crumbs, setCrumbs] = useState<Crumb[]>([]);
  const [hover, setHover] = useState<HoverInfo | null>(null);
  const [selected, setSelected] = useState<number | undefined>();
  /** Selected node's ancestors from the scan root down, inclusive. */
  const [selectedChain, setSelectedChain] = useState<number[]>([]);
  /** Row hovered in the tree list (lights up the matching block). */
  const [rowHover, setRowHover] = useState<number | undefined>();
  const [menu, setMenu] = useState<MenuTarget | null>(null);
  const [confirm, setConfirm] = useState<ActionInfo | null>(null);
  const [toasts, setToasts] = useState<Toast[]>([]);
  const [demoHover, setDemoHover] = useState<number | undefined>();
  const [fps, setFps] = useState(0);

  const toast = useCallback((kind: Toast["kind"], text: string, detail?: string) => {
    const id = Date.now() + Math.random();
    setToasts((t) => [...t.slice(-3), { id, kind, text, detail }]);
    setTimeout(() => setToasts((t) => t.filter((x) => x.id !== id)), kind === "error" ? 9000 : 4500);
  }, []);

  // --- startup ---------------------------------------------------------------------------------
  useEffect(() => {
    stage.attach(bgRef.current!, backend.native);
    if (backend.native) document.documentElement.classList.add("native");
    const id = setInterval(() => setFps(stage.fps), 500);
    backend.listDrives().then(setDrives);
    backend.isElevated().then(setElevated);
    if (params.has("bench")) {
      setTimeout(() => {
        const pre = document.createElement("pre");
        pre.id = "bench";
        pre.textContent = JSON.stringify({ ...stage.bench(240), blocks: (window as unknown as { __blocks?: number }).__blocks });
        document.body.appendChild(pre);
      }, 2500);
    }
    return () => {
      clearInterval(id);
      stage.detach();
    };
  }, []);

  useEffect(() => {
    document.documentElement.dataset.theme = theme;
    backend.setBackdrop(theme === "dark");
  }, [theme]);

  // --- scan lifecycle --------------------------------------------------------------------------
  /**
   * A small folder can finish scanning before `startScan` has even returned its generation, so its
   * `scan-done` / `scan-error` event arrives "early". Keep the newest such event and apply it once the
   * generation is known.
   */
  const early = useRef<{ done?: ScanSummary; error?: { generation: number; error: string } }>({});
  const applyDone = useCallback((s: ScanSummary) => {
    setSummary(s);
    setScanning(false);
    setTick((t) => t + 1);
  }, []);
  const applyError = useCallback((error: string) => {
    setScanning(false);
    setError(error);
  }, []);

  /** Start a session: a fresh scan of `path`, or (`fromCache`) its saved scan, opened instantly. */
  const begin = useCallback((path: string, fromCache: boolean) => {
    setError(null);
    setTarget(path);
    setScanning(true);
    setProgress(null);
    setSummary(null);
    setLayout(null);
    setRootId(0);
    setSelected(undefined);
    (fromCache ? backend.openCache(path) : backend.startScan(path))
      .then((g) => {
        if (g < genRef.current) return; // an older request resolving late
        genRef.current = g;
        setGeneration(g);
        // Apply (and drop) only this scan's early event; a newer scan's must survive.
        const { done, error } = early.current;
        if (done && done.generation <= g) early.current.done = undefined;
        if (error && error.generation <= g) early.current.error = undefined;
        if (done?.generation === g) applyDone(done);
        else if (error?.generation === g) applyError(error.error);
      })
      .catch((e) => applyError(String(e)));
  }, [applyDone, applyError]);
  const startScan = useCallback((path: string) => begin(path, false), [begin]);
  const openCached = useCallback((root: string) => begin(root, true), [begin]);

  // Saved scans for the picker; refreshed after each scan (the cache is written in the background).
  const [caches, setCaches] = useState<CacheEntry[]>([]);
  useEffect(() => {
    const load = () => backend.listCaches().then(setCaches).catch(() => {});
    load();
    if (summary && !summary.fromCache) {
      const t = setTimeout(load, 2500);
      return () => clearTimeout(t);
    }
  }, [summary]);

  useEffect(() => {
    const offP = backend.onProgress((p) => {
      if (p.generation !== genRef.current) return;
      setProgress(p);
      const now = performance.now();
      if (now - lastTick.current > STREAM_REFRESH_MS) {
        lastTick.current = now;
        setTick((t) => t + 1);
      }
    });
    const offD = backend.onDone((s) => {
      if (s.generation === genRef.current) applyDone(s);
      else if (s.generation > genRef.current) early.current.done = s;
    });
    const offE = backend.onError((e) => {
      if (e.generation === genRef.current) applyError(e.error);
      else if (e.generation > genRef.current) early.current.error = e;
    });
    const offS = backend.onSummaryUpdated((s) => {
      if (s.generation === genRef.current) setSummary(s);
    });
    return () => {
      offP();
      offD();
      offE();
      offS();
    };
  }, [applyDone, applyError]);

  // `DiskLens.exe D:\Games [--focus D:\Games\Steam] [--theme light]` (or the browser mockup).
  const focusAfterScan = useRef<string | null>(null);
  const launched = useRef(false);
  useEffect(() => {
    if (launched.current) return; // StrictMode runs effects twice in development
    launched.current = true;
    backend.launchOptions().then((o) => {
      if (o.theme) setTheme(o.theme);
      focusAfterScan.current = o.focus;
      if (o.scan) startScan(o.scan);
    });
  }, [startScan]);

  // --- layout fetching (one request in flight; coalesce the rest) --------------------------------
  const want = useRef({ generation, rootId, size, useAlloc, tick });
  want.current = { generation, rootId, size, useAlloc, tick };
  const inFlight = useRef(false);
  const again = useRef(false);
  const fetchLayout = useCallback(() => {
    const w = want.current;
    if (!w.generation || w.size.w < 10 || w.size.h < 10) return;
    if (inFlight.current) {
      again.current = true;
      return;
    }
    inFlight.current = true;
    backend
      .layout(w.generation, w.rootId, w.size.w, w.size.h, w.useAlloc)
      .then((l) => {
        if (w.generation === want.current.generation) setLayout(l);
      })
      .catch(() => {})
      .finally(() => {
        inFlight.current = false;
        if (again.current) {
          again.current = false;
          fetchLayout();
        }
      });
  }, []);
  useEffect(fetchLayout, [generation, rootId, size, useAlloc, tick, fetchLayout]);

  useEffect(() => {
    if (!generation) return;
    backend.ancestors(generation, rootId).then(setCrumbs).catch(() => {});
  }, [generation, rootId, tick]);

  // Mockup helpers: ?zoom=a/b and ?hover=a/b/c once the fake scan is done.
  useEffect(() => {
    if (backend.native || !summary) return;
    const z = params.get("zoom");
    if (z) setRootId(mockFindId(z) ?? 0);
    const h = params.get("hover");
    if (h) setDemoHover(mockFindId(h));
  }, [summary]);

  // --- navigation ------------------------------------------------------------------------------
  const zoomTo = useCallback((id: number) => {
    setHover(null);
    setRootId(id);
  }, []);

  const onActivate = useCallback(
    (top: ViewBlock, hit: ViewBlock) => {
      setSelected(hit.flags & BF_AGGREGATE ? undefined : hit.node);
      if (top.flags & BF_DIR && !(top.flags & BF_AGGREGATE)) zoomTo(top.node);
    },
    [zoomTo],
  );

  const zoomOut = useCallback(() => {
    if (crumbs.length > 1) zoomTo(crumbs[crumbs.length - 2].id);
  }, [crumbs, zoomTo]);

  // The selection's ancestor chain links the treemap ring and the tree list's expanded path.
  useEffect(() => {
    if (selected === undefined || !generation) {
      setSelectedChain([]);
      return;
    }
    let alive = true;
    backend
      .ancestors(generation, selected)
      .then((c) => alive && setSelectedChain(c.map((x) => x.id)))
      .catch(() => alive && setSelectedChain([]));
    return () => {
      alive = false;
    };
  }, [generation, selected]);

  /** Select an item by path (insights) and show it: zoom to the folder that contains it. */
  const showPath = useCallback(
    async (path: string) => {
      const id = await backend.findPath(generation, path);
      if (id === null) return;
      const info = await backend.nodeInfo(generation, id, useAlloc);
      zoomTo(info.isDir ? id : info.parent ?? 0);
      setSelected(id);
    },
    [generation, useAlloc, zoomTo],
  );

  // --- actions (each one is an explicit user choice; recycling also needs the dialog) ------------
  const revealInExplorer = useCallback(
    (id: number) => backend.reveal(generation, id).catch((e) => toast("error", "Couldn't open Explorer", String(e))),
    [generation, toast],
  );

  const copyPath = useCallback(
    async (id: number) => {
      try {
        const info = await backend.nodeInfo(generation, id, false);
        await writeClipboard(info.path);
        toast("ok", "Path copied", info.path);
      } catch (e) {
        toast("error", "Couldn't copy the path", String(e));
      }
    },
    [generation, toast],
  );

  /** Open the confirmation for `id`, or explain why it can't be recycled. */
  const askRecycle = useCallback(
    async (id: number) => {
      const info = await backend.actionInfo(generation, id);
      if (info.recycleBlocked) toast("info", `Can't send “${info.name}” to the Recycle Bin`, info.recycleBlocked);
      else setConfirm(info);
    },
    [generation, toast],
  );

  const doRecycle = useCallback(
    async (info: ActionInfo) => {
      try {
        const r = await backend.recycle(generation, [info.id]);
        if (r.recycled.length) {
          toast("ok", `Moved “${info.name}” to the Recycle Bin`, `${formatBytes(r.freed)} freed · restore it from the Recycle Bin if needed`);
          if (selected !== undefined && selectedChain.includes(info.id)) setSelected(undefined);
          if (crumbs.some((c) => c.id === info.id)) zoomTo(crumbs[Math.max(0, crumbs.findIndex((c) => c.id === info.id) - 1)].id);
          setTick((t) => t + 1);
          backend.listDrives().then(setDrives);
        }
        for (const [path, why] of r.failed) toast("error", `Not moved: ${path.split("\\").pop()}`, why);
        for (const path of r.permanentlyDeleted) toast("error", "Windows deleted an item permanently", path);
      } catch (e) {
        toast("error", "Couldn't send to the Recycle Bin", String(e));
      } finally {
        setConfirm(null);
      }
    },
    [generation, toast, selected, selectedChain, crumbs, zoomTo],
  );

  const openMenu = useCallback((id: number, name: string, isDir: boolean, x: number, y: number) => {
    setHover(null);
    setSelected(id);
    setMenu({ id, name, isDir, x, y });
  }, []);

  // Keyboard: Backspace/Esc zoom out, Enter zooms into the selection, Delete recycles it (with the
  // dialog), Ctrl+C copies its path.
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if ((e.target as HTMLElement).tagName === "INPUT" || menu || confirm) return;
      if (e.key === "Backspace" || e.key === "Escape") zoomOut();
      else if (selected !== undefined && e.key === "Delete") askRecycle(selected);
      else if (selected !== undefined && (e.ctrlKey || e.metaKey) && e.key.toLowerCase() === "c" && !window.getSelection()?.toString()) copyPath(selected);
      else if (selected !== undefined && e.key === "Enter")
        backend.nodeInfo(generation, selected, false).then((i) => i.isDir && zoomTo(i.id));
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [zoomOut, selected, menu, confirm, askRecycle, copyPath, generation, zoomTo]);

  useEffect(() => {
    if (!summary || !focusAfterScan.current) return;
    const path = focusAfterScan.current;
    focusAfterScan.current = null;
    backend.findPath(summary.generation, path).then((id) => id !== null && zoomTo(id));
  }, [summary, zoomTo]);

  const exportView = useCallback(
    async (kind: ExportKind) => {
      const where = crumbs.length ? crumbs[crumbs.length - 1].name.replace(/[\\/:*?"<>|]+/g, "").trim() || "drive" : "scan";
      const stamp = new Date().toISOString().slice(0, 10);
      const name = `DiskLens ${where} ${stamp}${kind === "csv-folders" ? " folders" : ""}.${kind === "json-tree" ? "json" : "csv"}`;
      try {
        const path = await backend.pickSavePath(name, kind);
        if (!path) return;
        const s = await backend.exportTree(generation, rootId, kind, path);
        toast("ok", `Exported ${s.items.toLocaleString()} items`, path);
      } catch (e) {
        toast("error", "Export failed", String(e));
      }
    },
    [crumbs, generation, rootId, toast],
  );

  const restartAsAdmin = useCallback(() => {
    backend.restartAsAdmin(target, theme).catch((e) => toast("info", "Still running without admin rights", String(e)));
  }, [target, theme, toast]);

  const pickFolder = useCallback(async () => {
    const p = await backend.pickFolder();
    if (p) startScan(p);
  }, [startScan]);

  const rootSize = crumbs.length ? crumbs[crumbs.length - 1].size : 0;
  const showPicker = generation === 0 && !scanning;

  return (
    <>
      <canvas ref={bgRef} className="stage" />
      <div className="app">
        <TitleBar
          theme={theme}
          onToggleTheme={() => setTheme((t) => (t === "dark" ? "light" : "dark"))}
          subtitle={target ? `${target} · ${formatBytes(summary?.size ?? progress?.bytes ?? 0)}${scanning ? " scanned so far" : ""}` : ""}
          notice={
            backend.native ? undefined : (
              <>
                Demo data: a browser can't read your disk. Run the DiskLens app (<code>npm run tauri dev</code>) to scan your drives.
              </>
            )
          }
        />
        {showPicker ? (
          <DrivePicker
            drives={drives}
            caches={caches}
            elevated={elevated}
            onScan={startScan}
            onOpenCache={openCached}
            onPickFolder={pickFolder}
            onRestartAsAdmin={restartAsAdmin}
          />
        ) : (
          <div className="workspace">
            <Sidebar
              drives={drives}
              target={target}
              scanning={scanning}
              progress={progress}
              summary={summary}
              reduceMotion={reduceMotion}
              onScan={startScan}
              onPickFolder={pickFolder}
              onCancel={() => backend.cancelScan()}
              elevated={elevated}
              onRestartAsAdmin={restartAsAdmin}
            />
            <main className="main">
              <div className="toolbar glass">
                <nav className="breadcrumbs" aria-label="Breadcrumb">
                  {crumbs.map((c, i) => (
                    <span key={c.id} className="crumb-wrap">
                      {i > 0 && <span className="crumb-sep">›</span>}
                      <button className={`crumb ${i === crumbs.length - 1 ? "current" : ""}`} onClick={() => zoomTo(c.id)}>
                        {i === 0 ? (
                          <>
                            <IconHome /> {c.name.replace(/\\$/, "")}
                          </>
                        ) : (
                          c.name
                        )}
                      </button>
                    </span>
                  ))}
                  {crumbs.length > 0 && <span className="crumb-size">{formatBytes(rootSize)}</span>}
                </nav>
                <div className="toolbar-right">
                  <div className="segmented small">
                    <button className={!useAlloc ? "on" : ""} onClick={() => setUseAlloc(false)}>
                      Size
                    </button>
                    <button className={useAlloc ? "on" : ""} onClick={() => setUseAlloc(true)}>
                      On disk
                    </button>
                  </div>
                  <button className="icon-btn" title="Rescan" disabled={scanning || !target} onClick={() => target && startScan(target)}>
                    <IconRescan />
                  </button>
                  <ExportMenu disabled={scanning || !summary} onExport={exportView} />
                </div>
              </div>
              <div className="treemap-frame">
                <Treemap
                  layout={layout}
                  theme={theme}
                  reduceMotion={reduceMotion}
                  onHover={setHover}
                  onActivate={onActivate}
                  onResize={(w, h) => setSize({ w, h })}
                  onContextMenu={(b, x, y) => {
                    if (!(b.flags & BF_AGGREGATE)) openMenu(b.node, b.name, (b.flags & BF_DIR) !== 0, x, y);
                    // "Small files" block: act on the folder that holds them.
                    else backend.nodeInfo(generation, b.node, false).then((i) => openMenu(i.id, i.name, true, x, y));
                  }}
                  selectedChain={selectedChain}
                  linkedHoverNode={rowHover}
                  demoHoverNode={demoHover}
                />
                {scanning && !layout?.blocks.length && <div className="treemap-empty">Reading {target}…</div>}
                {error && <div className="treemap-empty error">Scan failed: {error}</div>}
                {params.has("fps") && (
                  <div className="fps">
                    {fps.toFixed(0)} fps · {layout?.blocks.length ?? 0} blocks
                  </div>
                )}
              </div>
              {generation > 0 && (
                <TreeList
                  generation={generation}
                  rootId={rootId}
                  rootSize={rootSize}
                  useAlloc={useAlloc}
                  refreshTick={scanning ? Math.floor(tick / 3) : tick}
                  selectedId={selected}
                  selectedChain={selectedChain}
                  linkedHoverId={hover?.block.node}
                  onSelect={setSelected}
                  onOpen={zoomTo}
                  onHoverRow={setRowHover}
                  onContextMenu={(r, x, y) => openMenu(r.id, r.name, r.isDir, x, y)}
                />
              )}
            </main>
            <InsightsPanel summary={summary} scanning={scanning} onReveal={showPath} onOpenInExplorer={revealInExplorer} onItemMenu={openMenu} />
          </div>
        )}
      </div>
      {!menu && !confirm && <Tooltip hover={hover} generation={generation} useAlloc={useAlloc} />}
      {menu && (
        <ContextMenu
          target={menu}
          generation={generation}
          onClose={() => setMenu(null)}
          onZoom={zoomTo}
          onReveal={revealInExplorer}
          onCopyPath={copyPath}
          onRecycle={(info) => setConfirm(info)}
        />
      )}
      {confirm && <ConfirmRecycle info={confirm} onCancel={() => setConfirm(null)} onConfirm={() => doRecycle(confirm)} />}
      <Toasts toasts={toasts} onDismiss={(id) => setToasts((t) => t.filter((x) => x.id !== id))} />
    </>
  );
}
