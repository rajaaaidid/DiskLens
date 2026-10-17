import { useState } from "react";
import type { Change, Hog, ScanSummary } from "../api";
import { CATEGORY_LABELS, categoryForName, cssColor } from "../data/categories";
import { formatAgo, formatBytes, formatCount, formatPercent } from "../data/format";
import { IconExternal } from "./Icons";

type Tab = "hogs" | "files" | "folders" | "types" | "changes";

const TAB_LABELS: Record<Tab, string> = { hogs: "Hogs", files: "Files", folders: "Folders", types: "Types", changes: "Changes" };

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

/** "+1.2 GB" / "−300 MB" (a real minus sign). */
function formatDelta(delta: number) {
  return `${delta < 0 ? "\u2212" : "+"}${formatBytes(Math.abs(delta))}`;
}

const CHANGE_TAG: Record<Change["kind"], string> = { added: "new", removed: "gone", grew: "grew", shrank: "shrank" };

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

  const changeRow = (c: Change, rank: number) => {
    const { name, dir } = splitPath(c.path);
    const cat = c.isDir ? "folder" : categoryForName(name);
    const node = c.node;
    const sizes = c.kind === "added" ? `Now ${formatBytes(c.after)}` : c.kind === "removed" ? `Was ${formatBytes(c.before)}` : `${formatBytes(c.before)} \u2192 ${formatBytes(c.after)}`;
    const hint = node === null ? "No longer on disk" : "Click to show in the treemap · right-click for actions";
    return (
      <li
        key={c.path}
        className={node === null ? "gone" : undefined}
        onClick={node === null ? undefined : () => onReveal(c.path)}
        onContextMenu={(e) => {
          e.preventDefault();
          if (node !== null) onItemMenu(node, name, c.isDir, e.clientX, e.clientY);
        }}
        title={`${c.path}\n${sizes}\n${hint}`}
      >
        <span className="rank">{rank}</span>
        <span className="swatch" style={{ background: cssColor(cat, c.isDir ? 0.7 : 0.85) }} />
        <span className="file-main">
          <span className="file-name">{name}</span>
          <span className="file-path">
            {(c.kind === "added" || c.kind === "removed") && <span className={`change-tag ${c.kind}`}>{CHANGE_TAG[c.kind]}</span>}
            {dir}
          </span>
        </span>
        <span className={`file-size delta ${c.delta < 0 ? "down" : "up"}`}>{formatDelta(c.delta)}</span>
      </li>
    );
  };

  const changes = summary?.changes;

  return (
    <aside className="insights glass">
      <div className="panel-head">
        <h2>Insights</h2>
        <div className="segmented" role="tablist">
          {(Object.keys(TAB_LABELS) as Tab[]).map((t) => (
            <button key={t} role="tab" aria-selected={tab === t} className={tab === t ? "on" : ""} onClick={() => setTab(t)}>
              {TAB_LABELS[t]}
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

        {summary && tab === "changes" && !changes && (
          <p className="empty-note">
            {summary.fromCache
              ? "This is a saved scan. Rescan to see what changed since then."
              : summary.cancelled
                ? "The scan was cancelled, so there's nothing to compare."
                : `Nothing to compare with yet. The next scan of ${summary.root} will show what changed since this one.`}
          </p>
        )}
        {summary && tab === "changes" && changes && (
          <>
            <div className="change-summary">
              <p className="change-since">Since the scan {formatAgo(changes.sinceMs)}</p>
              <div className="change-stats">
                <div>
                  <span className="stat-label">Grew</span>
                  <span className="delta up">{formatDelta(changes.grew)}</span>
                  <span className="stat-sub">
                    {formatCount(changes.added)} new · {formatCount(changes.changed)} changed
                  </span>
                </div>
                <div>
                  <span className="stat-label">Freed</span>
                  <span className="delta down">{formatDelta(-changes.shrank)}</span>
                  <span className="stat-sub">{formatCount(changes.removed)} gone</span>
                </div>
                <div>
                  <span className="stat-label">Net</span>
                  <span className={`delta ${changes.after < changes.before ? "down" : "up"}`}>{formatDelta(changes.after - changes.before)}</span>
                  <span className="stat-sub">now {formatBytes(changes.after)}</span>
                </div>
              </div>
              {changes.methodDiffers && <p className="tab-note">The earlier scan used a different method (MFT vs folder walk), so small differences are expected.</p>}
            </div>
            {changes.items.length === 0 ? (
              <p className="empty-note">Nothing changed.</p>
            ) : (
              <>
                <div className="section-label">Folders that changed most</div>
                <p className="tab-note">Counting only what changed directly inside each folder.</p>
                <ol className="file-rank">{changes.folders.map((c, i) => changeRow(c, i + 1))}</ol>
                <div className="section-label">Biggest changes</div>
                <ol className="file-rank">{changes.items.map((c, i) => changeRow(c, i + 1))}</ol>
              </>
            )}
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
