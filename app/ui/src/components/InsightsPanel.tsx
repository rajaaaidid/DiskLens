import { useState } from "react";
import type { Change, DupeProgress, DupeReport, DupeSet, Hog, ScanSummary } from "../api";
import { AGE_BANDS, AGE_UNKNOWN_COLOR, cssRgb } from "../data/age";
import { CATEGORY_LABELS, categoryForName, cssColor } from "../data/categories";
import { formatAgo, formatBytes, formatCount, formatDate, formatPercent } from "../data/format";
import type { ColorMode } from "../treemap/Treemap";
import { IconExternal } from "./Icons";

type Tab = "hogs" | "files" | "folders" | "types" | "age" | "dupes" | "changes";

const TAB_LABELS: Record<Tab, string> = { hogs: "Hogs", files: "Files", folders: "Folders", types: "Types", age: "Age", dupes: "Dupes", changes: "Changes" };

/** Smallest file the duplicate search looks at. */
export const DUPE_MIN_SIZES = [
  { bytes: 100 * 1024, label: "100 KB" },
  { bytes: 1024 * 1024, label: "1 MB" },
  { bytes: 10 * 1024 * 1024, label: "10 MB" },
  { bytes: 100 * 1024 * 1024, label: "100 MB" },
];

export interface DupesState {
  report: DupeReport | null;
  /** Latest progress while a search runs. */
  progress: DupeProgress | null;
  running: boolean;
  /** Folder the next search would cover (the current view). */
  scopePath: string;
}

interface Props {
  summary: ScanSummary | null;
  scanning: boolean;
  colorMode: ColorMode;
  onColorMode: (m: ColorMode) => void;
  dupes: DupesState;
  onFindDupes: (minSize: number) => void;
  onCancelDupes: () => void;
  /** Choose which copy to keep and send the others to the Recycle Bin (asks first). */
  onKeepOne: (set: DupeSet) => void;
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

export function InsightsPanel({ summary, scanning, colorMode, onColorMode, dupes, onFindDupes, onCancelDupes, onKeepOne, onReveal, onOpenInExplorer, onItemMenu }: Props) {
  const [tab, setTab] = useState<Tab>("hogs");
  const [minSize, setMinSize] = useState(DUPE_MIN_SIZES[1].bytes);
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
  const age = summary?.age;
  const ageTotal = age ? age.bands.reduce((s, b) => s + b.size, 0) + age.unknownSize : 1;
  const ageRows = age
    ? [
        ...age.bands.map((b, i) => ({ key: String(i), label: AGE_BANDS[i]?.label ?? "", color: AGE_BANDS[i]?.color ?? AGE_UNKNOWN_COLOR, size: b.size, files: b.files })),
        ...(age.unknownFiles ? [{ key: "unknown", label: "No date", color: AGE_UNKNOWN_COLOR, size: age.unknownSize, files: age.unknownFiles }] : []),
      ]
    : [];

  const dupeRow = (f: DupeSet["files"][number]) => {
    const { name, dir } = splitPath(f.path);
    return (
      <li
        key={f.node}
        onClick={() => onReveal(f.path)}
        onContextMenu={(e) => {
          e.preventDefault();
          onItemMenu(f.node, name, false, e.clientX, e.clientY);
        }}
        title={`${f.path}\nModified ${f.mtimeMs ? formatDate(f.mtimeMs) : "at an unknown time"}\nClick to show in the treemap · right-click for actions`}
      >
        <span className="swatch" style={{ background: cssColor(categoryForName(name), 0.85) }} />
        <span className="file-main">
          <span className="file-name">{name}</span>
          <span className="file-path">{dir}</span>
        </span>
        <span className="file-size dupe-date">{f.mtimeMs ? formatDate(f.mtimeMs) : ""}</span>
      </li>
    );
  };

  const report = dupes.report;
  const p = dupes.progress;

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

        {summary && tab === "age" && age && (
          <>
            <div className="age-toggle">
              <span>Treemap colours</span>
              <div className="segmented small">
                <button className={colorMode === "type" ? "on" : ""} onClick={() => onColorMode("type")}>
                  Type
                </button>
                <button className={colorMode === "age" ? "on" : ""} onClick={() => onColorMode("age")}>
                  Age
                </button>
              </div>
            </div>
            <p className="tab-note">When files were last modified. A folder counts as recent as the newest file inside it.</p>
            <div className="cat-stack" aria-label="Space by age">
              {ageRows.map((r) => (
                <span key={r.key} title={`${r.label}: ${formatBytes(r.size)}`} style={{ flexGrow: r.size / Math.max(1, ageTotal), background: cssRgb(r.color, 0.8) }} />
              ))}
            </div>
            <div className="cat-list">
              {ageRows.map((r) => (
                <div key={r.key} className="cat-row">
                  <span className="swatch" style={{ background: cssRgb(r.color, 0.85) }} />
                  <span className="cat-name">{r.label}</span>
                  <span className="cat-files">{formatCount(r.files)} files</span>
                  <span className="cat-pct">{formatPercent(r.size, ageTotal)}</span>
                  <span className="cat-size">{formatBytes(r.size)}</span>
                </div>
              ))}
            </div>
            <div className="stale-callout">
              <span className="stat-label">Not modified in {Math.round(age.staleDays / 365)}+ years</span>
              <span className="stale-size">{formatBytes(age.staleSize)}</span>
              <span className="stat-sub">
                {formatCount(age.staleFiles)} files · {formatPercent(age.staleSize, ageTotal)} of the scanned data
              </span>
            </div>
            {age.staleFolders.length > 0 && (
              <>
                <div className="section-label">Folders with the most stale data</div>
                <p className="tab-note">Counting only stale files directly inside each folder.</p>
                <ol className="file-rank">{age.staleFolders.slice(0, 20).map((f, i) => itemRow(f.node, f.path, f.size, true, i + 1))}</ol>
              </>
            )}
            {age.staleLargest.length > 0 && (
              <>
                <div className="section-label">Largest stale files</div>
                <ol className="file-rank">{age.staleLargest.map((f, i) => itemRow(f.node, f.path, f.size, false, i + 1))}</ol>
              </>
            )}
          </>
        )}

        {summary && tab === "dupes" && (
          <>
            {!dupes.running && (
              <div className="dupe-start">
                <p className="tab-note">
                  Finds files with identical contents in <b>{dupes.scopePath}</b>. Only files that share their size with another one are read, and
                  online-only OneDrive files are skipped so nothing gets downloaded.
                </p>
                <div className="dupe-controls">
                  <label>
                    At least{" "}
                    <select value={minSize} onChange={(e) => setMinSize(Number(e.target.value))}>
                      {DUPE_MIN_SIZES.map((m) => (
                        <option key={m.bytes} value={m.bytes}>
                          {m.label}
                        </option>
                      ))}
                    </select>
                  </label>
                  <button className="btn btn-primary" disabled={scanning} onClick={() => onFindDupes(minSize)}>
                    {report ? "Search again" : "Find duplicates"}
                  </button>
                </div>
              </div>
            )}
            {dupes.running && (
              <div className="dupe-progress">
                <p className="change-since">{!p || p.phase <= 1 ? "Comparing the start and end of each file…" : "Comparing whole files…"}</p>
                <div className="hog-meter">
                  <span style={{ width: `${p && p.bytesTotal > 0 ? Math.min(100, (p.bytesDone / p.bytesTotal) * 100) : 0}%` }} />
                </div>
                <p className="stat-sub">
                  {p ? `${formatCount(p.filesDone)} of ${formatCount(p.filesTotal)} files · ${formatBytes(p.bytesDone)} of ${formatBytes(p.bytesTotal)} read` : "Listing candidates…"}
                </p>
                <button className="btn btn-secondary" onClick={onCancelDupes}>
                  Cancel
                </button>
              </div>
            )}
            {report && !dupes.running && (
              <>
                <div className="change-summary">
                  <p className="change-since">
                    In {report.rootPath} · files of {formatBytes(report.minSize)} or more
                  </p>
                  <div className="change-stats">
                    <div>
                      <span className="stat-label">Could free</span>
                      <span className="delta down">{formatBytes(report.wasted)}</span>
                      <span className="stat-sub">keeping one copy of each</span>
                    </div>
                    <div>
                      <span className="stat-label">Sets</span>
                      <span className="delta">{formatCount(report.totalSets)}</span>
                      <span className="stat-sub">{formatCount(report.extraCopies)} extra copies</span>
                    </div>
                  </div>
                  {report.cancelled && <p className="tab-note">The search was cancelled, so some duplicates may be missing.</p>}
                  {(report.unreadable > 0 || report.skippedCloud > 0) && (
                    <p className="tab-note">
                      Skipped{report.unreadable > 0 && ` ${formatCount(report.unreadable)} files that couldn't be read`}
                      {report.unreadable > 0 && report.skippedCloud > 0 && " and"}
                      {report.skippedCloud > 0 && ` ${formatCount(report.skippedCloud)} online-only files`}.
                    </p>
                  )}
                </div>
                {report.sets.length === 0 && <p className="empty-note">No duplicates found.</p>}
                {report.sets.map((set) => (
                  <article key={set.files[0].node} className="hog-card dupe-card">
                    <header>
                      <span className="dupe-copies">
                        {set.files.length} copies × {formatBytes(set.size)}
                      </span>
                      <span className="hog-size">{formatBytes(set.size * (set.files.length - 1))}</span>
                    </header>
                    <ul className="hog-items">{set.files.map(dupeRow)}</ul>
                    <div className="hog-actions">
                      <button className="chip" onClick={() => onKeepOne(set)}>
                        Keep one…
                      </button>
                    </div>
                  </article>
                ))}
                {report.totalSets > report.sets.length && <p className="tab-note">Showing the {formatCount(report.sets.length)} sets that waste the most space.</p>}
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
