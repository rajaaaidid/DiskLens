import { useEffect, useMemo, useRef, useState } from "react";
import { InsightsPanel } from "./components/InsightsPanel";
import { IconExport, IconHome, IconRescan, IconSearch } from "./components/Icons";
import { Sidebar } from "./components/Sidebar";
import { TitleBar } from "./components/TitleBar";
import { Tooltip } from "./components/Tooltip";
import { TreeList } from "./components/TreeList";
import { countNodes, FAKE_DRIVES, FAKE_ROOT as BASE_ROOT, makeStressTree } from "./data/fakeTree";
import { formatBytes } from "./data/format";
import { categoryTotals, extensionTotals, findHogs, largestFiles } from "./data/insights";
import type { TreeNode } from "./data/types";
import { stage } from "./treemap/stage";
import { Treemap, type HoverInfo } from "./treemap/Treemap";

const params = new URLSearchParams(location.search);
const FAKE_ROOT = params.get("stress") ? makeStressTree(Number(params.get("stress"))) : BASE_ROOT;

function findByPath(root: TreeNode, path: string): TreeNode | undefined {
  let cur: TreeNode | undefined = root;
  for (const part of path.split("/").filter(Boolean)) {
    cur = cur?.children.find((c) => c.name.toLowerCase() === part.toLowerCase());
  }
  return cur;
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
  const [root, setRoot] = useState<TreeNode>(() => (params.get("zoom") && findByPath(FAKE_ROOT, params.get("zoom")!)) || FAKE_ROOT);
  const [hover, setHover] = useState<HoverInfo | null>(null);
  const [selected, setSelected] = useState<TreeNode | undefined>();
  const [fps, setFps] = useState(0);

  const demoHover = params.get("hover") ? findByPath(FAKE_ROOT, params.get("hover")!) : undefined;

  useEffect(() => {
    stage.attach(bgRef.current!);
    const id = setInterval(() => setFps(stage.fps), 500);
    if (params.has("bench")) {
      // Headless benchmark: wait for layout, then time synchronous frames and publish the result.
      setTimeout(() => {
        const gl = document.createElement("canvas").getContext("webgl2");
        const ext = gl?.getExtension("WEBGL_debug_renderer_info");
        const result = {
          ...stage.bench(240),
          blocks: (window as unknown as { __blocks?: number }).__blocks,
          layoutMs: (window as unknown as { __layoutMs?: number }).__layoutMs,
          gpu: ext ? gl!.getParameter(ext.UNMASKED_RENDERER_WEBGL) : "unknown",
        };
        const pre = document.createElement("pre");
        pre.id = "bench";
        pre.textContent = JSON.stringify(result);
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
  }, [theme]);

  const insights = useMemo(
    () => ({
      hogs: findHogs(FAKE_ROOT),
      files: largestFiles(FAKE_ROOT, 50),
      types: extensionTotals(FAKE_ROOT, 14),
      cats: categoryTotals(FAKE_ROOT),
      nodes: countNodes(FAKE_ROOT),
    }),
    [],
  );

  const crumbs: TreeNode[] = [];
  for (let n: TreeNode | null = root; n; n = n.parent) crumbs.unshift(n);

  const zoomTo = (n: TreeNode) => setRoot(n.isDir ? n : n.parent ?? FAKE_ROOT);
  const drive = FAKE_DRIVES[0];

  return (
    <>
      <canvas ref={bgRef} className="stage" />
      <div className="app">
        <TitleBar theme={theme} onToggleTheme={() => setTheme((t) => (t === "dark" ? "light" : "dark"))} subtitle={`C:\\ · ${formatBytes(FAKE_ROOT.size)} scanned`} />
        <div className="workspace">
          <Sidebar
            drives={FAKE_DRIVES}
            active={drive.letter}
            scan={{ files: FAKE_ROOT.files, folders: insights.nodes - FAKE_ROOT.files, bytes: FAKE_ROOT.size, seconds: 4.2, method: "mft", inaccessible: 3 }}
            categories={insights.cats}
            totalBytes={FAKE_ROOT.size}
          />
          <main className="main">
            <div className="toolbar glass">
              <nav className="breadcrumbs" aria-label="Breadcrumb">
                {crumbs.map((c, i) => (
                  <span key={c.id} className="crumb-wrap">
                    {i > 0 && <span className="crumb-sep">›</span>}
                    <button className={`crumb ${i === crumbs.length - 1 ? "current" : ""}`} onClick={() => setRoot(c)}>
                      {i === 0 ? (
                        <>
                          <IconHome /> {c.name.replace("\\", "")}
                        </>
                      ) : (
                        c.name
                      )}
                    </button>
                  </span>
                ))}
                <span className="crumb-size">{formatBytes(root.size)}</span>
              </nav>
              <div className="toolbar-right">
                <div className="search">
                  <IconSearch />
                  <input placeholder="Filter by name or extension" />
                </div>
                <div className="segmented small">
                  <button className="on">Size</button>
                  <button>On disk</button>
                </div>
                <button className="icon-btn" title="Rescan"><IconRescan /></button>
                <button className="icon-btn" title="Export"><IconExport /></button>
              </div>
            </div>
            <div className="treemap-frame">
              <Treemap root={root} theme={theme} reduceMotion={reduceMotion} onHover={setHover} onZoom={zoomTo} demoHoverId={demoHover?.id} minArea={params.get("stress") ? 2 : undefined} />
              {params.has("fps") && <div className="fps">{fps.toFixed(0)} fps � {(window as unknown as { __blocks?: number }).__blocks ?? 0} blocks</div>}
            </div>
            <TreeList
              root={root}
              initiallyOpen={params.get("open") ? params.get("open")!.split(",").map((p) => findByPath(FAKE_ROOT, p)?.id ?? -1) : []}
              selectedId={selected?.id}
              onSelect={(n) => setSelected(n)}
            />
          </main>
          <InsightsPanel hogs={insights.hogs} files={insights.files} types={insights.types} total={FAKE_ROOT.size} onReveal={(n) => zoomTo(n.parent ?? n)} />
        </div>
      </div>
      <Tooltip hover={hover} />
    </>
  );
}
