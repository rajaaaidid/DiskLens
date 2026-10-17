import { useState } from "react";
import type { Hog, ScanSummary } from "../api";
import { CATEGORY_LABELS, categoryForName, cssColor } from "../data/categories";
import { formatBytes, formatCount, formatPercent } from "../data/format";
import { IconExternal } from "./Icons";

type Tab = "hogs" | "files" | "folders" | "types";

interface Props {
  summary: ScanSummary | null;
  scanning: boolean;
  /** Select the item and show it in the treemap. */
  onReveal: (path: string) => void;
  onOpenInExplorer: (node: number) => void;
  onItemMenu: (node: number, name: string, isDir: boolean, x: number, y: number) => void;
}

function splitPath(p: string) {
  const i = p.lastIndexOf("\\");
  return i > 0 ? { name: p.slice(i + 1) || p, dir: p.slice(0, i) } : { name: p, dir: "" };
}

/** Hog rules whose matches are files; all others match folders. */
const FILE_HOGS = new Set(["hiberfil", "pagefile", "vhdx"]);

const SEVERITY_LABEL: Record<Hog["severity"], string> = { reclaimable: "Reclaimable", system: "Windows-managed", info: "Good to know" };

/** Renders `code` spans in the advice text. */
function Advice({ text }: { text: string }) {
  return (
    <p className="advice">
      {text.split("`").map((part, i) => (i % 2 ? <code key={i}>{part}</code> : <span key={i}>{part}</span>))}
    </p>
  );
}

export function InsightsPanel({ summary, scanning, onReveal, onOpenInExplorer, onItemMenu }: Props) {
  const [tab, setTab] = useState<Tab>("hogs");
  const types = summary?.extensions ?? [];
  const maxType = types[0]?.size ?? 1;
  const catTotal = summary?.categories.reduce((s, c) => s + c.size, 0) ?? 1;

  const itemRow = (node: number, path: string, size: number, isDir: boolean, rank?: number) => {
    const { name, dir } = splitPath(path);
    const cat = isDir ? "folder" : categoryForName(name);
    return (
      <li
        key={node}
        onClick={() => onReveal(path)}
        onContextMenu={(e) => {
          e.preventDefault();
          onItemMenu(node, name, isDir, e.clientX, e.clientY);
        }}
        title={`${path}\nClick to show in the treemap · right-click for actions`}
      >
        {rank !== undefined && <span className="rank">{rank}</span>}
        <span className="swatch" style={{ background: cssColor(cat, isDir ? 0.7 : 0.85) }} />
        <span className="file-main">
          <span className="file-name">{name}</span>
          <span className="file-path">{dir}</span>
        </span>
        <span className="file-size">{formatBytes(size)}</span>
      </li>
    );
  };

  return (
    <aside className="insights glass">
      <div className="panel-head">
        <h2>Insights</h2>
        <div className="segmented" role="tablist">
          {(["hogs", "files", "folders", "types"] as Tab[]).map((t) => (
            <button key={t} role="tab" aria-selected={tab === t} className={tab === t ? "on" : ""} onClick={() => setTab(t)}>
              {t === "hogs" ? "Hogs" : t === "files" ? "Files" : t === "folders" ? "Folders" : "Types"}
            </button>
          ))}
        </div>
      </div>

      <div className="panel-scroll">
        {!summary && <p className="empty-note">{scanning ? "Insights appear when the scan finishes." : "Scan a drive to see insights."}</p>}

        {summary && tab === "hogs" && summary.hogs.length === 0 && <p className="empty-note">No known space hogs found here.</p>}
        {summary &&
          tab === "hogs" &&
          summary.hogs.map((h) => (
            <article key={h.id} className="hog-card">
              <header>
                <span className={`sev ${h.severity}`}>{SEVERITY_LABEL[h.severity]}</span>
                <span className="hog-size">{formatBytes(h.size)}</span>
              </header>
              <h3>
                {h.title}
                {h.count > 1 && <span className="hog-count"> ×{formatCount(h.count)}</span>}
              </h3>
              <p>{h.why}</p>
              <Advice text={h.advice} />
              <div className="hog-meter" title={`${formatPercent(h.size, summary.size)} of the scanned total`}>
                <span style={{ width: `${Math.max(1.5, (h.size / Math.max(1, summary.size)) * 100)}%` }} />
              </div>
              <ul className="hog-items">{h.items.slice(0, h.count > 1 ? 5 : 1).map((it) => itemRow(it.node, it.path, it.size, !FILE_HOGS.has(h.id)))}</ul>
              <div className="hog-actions">
                <button className="chip" onClick={() => onReveal(h.items[0].path)}>
                  Show in treemap
                </button>
                <button className="chip" onClick={() => onOpenInExplorer(h.items[0].node)}>
                  <IconExternal /> Open in Explorer
                </button>
              </div>
            </article>
          ))}

        {summary && tab === "files" && <ol className="file-rank">{summary.largestFiles.map((f, i) => itemRow(f.node, f.path, f.size, false, i + 1))}</ol>}

        {summary && tab === "folders" && (
          <>
            <p className="tab-note">Folders holding the most data in their own files (not counting subfolders).</p>
            <ol className="file-rank">{summary.largestFolders.map((f, i) => itemRow(f.node, f.path, f.size, true, i + 1))}</ol>
          </>
        )}

        {summary && tab === "types" && (
          <>
            <div className="cat-stack" aria-label="Space by category">
              {summary.categories.map((c) => (
                <span key={c.category} title={`${CATEGORY_LABELS[c.category]}: ${formatBytes(c.size)}`} style={{ flexGrow: c.size / Math.max(1, catTotal), background: cssColor(c.category, 0.8) }} />
              ))}
            </div>
            <div className="cat-list">
              {summary.categories.map((c) => (
                <div key={c.category} className="cat-row">
                  <span className="swatch" style={{ background: cssColor(c.category, 0.85) }} />
                  <span className="cat-name">{CATEGORY_LABELS[c.category]}</span>
                  <span className="cat-files">{formatCount(c.files)} files</span>
                  <span className="cat-pct">{formatPercent(c.size, catTotal)}</span>
                  <span className="cat-size">{formatBytes(c.size)}</span>
                </div>
              ))}
            </div>
            <div className="section-label">By extension</div>
            <div className="type-list">
              {types.map((t) => {
                const cat = categoryForName(`x.${t.ext}`);
                return (
                  <div key={t.ext} className="type-row">
                    <span className="ext" style={{ color: cssColor(cat, 1), borderColor: cssColor(cat, 0.4), background: cssColor(cat, 0.12) }}>
                      {t.ext === "(none)" ? "no ext" : `.${t.ext}`}
                    </span>
                    <span className="type-cat">
                      {CATEGORY_LABELS[cat]} · {formatCount(t.files)} files
                    </span>
                    <span className="type-size">{formatBytes(t.size)}</span>
                    <span className="type-bar">
                      <span style={{ width: `${(t.size / maxType) * 100}%`, background: cssColor(cat, 0.75) }} />
                    </span>
                  </div>
                );
              })}
            </div>
          </>
        )}
      </div>
    </aside>
  );
}
