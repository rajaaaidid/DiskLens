import { useCallback, useEffect, useRef, useState } from "react";
import { backend, BF_AGGREGATE, BF_DIR, type Crumb, type Drive, type Layout, type ScanProgress, type ScanSummary, type ViewBlock } from "./api";
import { mockFindId } from "./api/mockBackend";
import { DrivePicker } from "./components/DrivePicker";
import { IconHome, IconRescan, IconSearch } from "./components/Icons";
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
  const [demoHover, setDemoHover] = useState<number | undefined>();
  const [fps, setFps] = useState(0);

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
  const startScan = useCallback((path: string) => {
    setError(null);
    setTarget(path);
    setScanning(true);
    setProgress(null);
    setSummary(null);
    setLayout(null);
    setRootId(0);
    setSelected(undefined);
    backend
      .startScan(path)
      .then((g) => {
        genRef.current = g;
        setGeneration(g);
      })
      .catch((e) => {
        setScanning(false);
        setError(String(e));
      });
  }, []);

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
      if (s.generation !== genRef.current) return;
      setSummary(s);
      setScanning(false);
      setTick((t) => t + 1);
    });
    const offE = backend.onError((e) => {
      if (e.generation !== genRef.current) return;
      setScanning(false);
      setError(e.error);
    });
    return () => {
      offP();
      offD();
      offE();
    };
  }, []);

  // `DiskLens.exe D:\Games [--focus D:\Games\Steam] [--theme light]` (or the browser mockup).
  const focusAfterScan = useRef<string | null>(null);
  useEffect(() => {
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

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if ((e.target as HTMLElement).tagName === "INPUT") return;
      if (e.key === "Backspace" || e.key === "Escape") zoomOut();
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [zoomOut]);

  const reveal = useCallback(
    async (path: string) => {
      const id = await backend.findPath(generation, path);
      if (id === null) return;
      const info = await backend.nodeInfo(generation, id, useAlloc);
      zoomTo(info.isDir ? id : info.parent ?? 0);
      setSelected(id);
    },
    [generation, useAlloc, zoomTo],
  );

  useEffect(() => {
    if (!summary || !focusAfterScan.current) return;
    const path = focusAfterScan.current;
    focusAfterScan.current = null;
    backend.findPath(summary.generation, path).then((id) => id !== null && zoomTo(id));
  }, [summary, zoomTo]);

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
        />
        {showPicker ? (
          <DrivePicker drives={drives} elevated={elevated} onScan={startScan} onPickFolder={pickFolder} />
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
                  <div className="search" title="Filtering arrives with the tree list linking">
                    <IconSearch />
                    <input placeholder="Filter by name or extension" disabled />
                  </div>
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
                  onSelect={setSelected}
                  onOpen={zoomTo}
                />
              )}
            </main>
            <InsightsPanel summary={summary} scanning={scanning} onReveal={reveal} />
          </div>
        )}
      </div>
      <Tooltip hover={hover} generation={generation} useAlloc={useAlloc} />
    </>
  );
}
