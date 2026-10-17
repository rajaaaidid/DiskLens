import type { Drive, ScanProgress, ScanSummary } from "../api";
import { CATEGORY_LABELS, cssColor } from "../data/categories";
import { formatAgo, formatBytes, formatCount } from "../data/format";
import { AnimatedNumber } from "./AnimatedNumber";
import { IconBolt, IconFolder } from "./Icons";
import { UsageRing } from "./UsageRing";

interface Props {
  drives: Drive[];
  /** Root path of the current scan, e.g. `C:\`. */
  target: string | null;
  scanning: boolean;
  progress: ScanProgress | null;
  summary: ScanSummary | null;
  reduceMotion: boolean;
  onScan: (path: string) => void;
  onPickFolder: () => void;
  onCancel: () => void;
  elevated: boolean;
  onRestartAsAdmin: () => void;
}

const METHOD_LABEL: Record<string, string> = { mft: "MFT fast path", walk: "Directory walk", findfirst: "FindFirstFile walk", auto: "Auto" };

export function Sidebar({ drives, target, scanning, progress, summary, reduceMotion, onScan, onPickFolder, onCancel, elevated, onRestartAsAdmin }: Props) {
  // A whole-drive scan that couldn't use the MFT only because of missing admin rights.
  const couldUseMft = !elevated && !scanning && summary?.method === "walk" && /^[A-Za-z]:\\?$/.test(target ?? "") && /administrator/i.test(summary?.fallbackReason ?? "");
  const files = progress?.files ?? summary?.files ?? 0;
  const dirs = progress?.dirs ?? summary?.dirs ?? 0;
  const bytes = summary && !scanning ? summary.size : progress?.bytes ?? 0;
  const seconds = (scanning ? progress?.elapsedMs ?? 0 : summary?.elapsedMs ?? 0) / 1000;
  const total = summary?.size ?? 1;

  return (
    <aside className="sidebar glass">
      <div className="section-label">Drives</div>
      <div className="drive-list">
        {drives.map((d) => {
          const used = d.total - d.free;
          const active = target?.toUpperCase().startsWith(d.letter.toUpperCase()) ?? false;
          return (
            <button key={d.letter} className={`drive-card ${active ? "active" : ""}`} onClick={() => onScan(d.letter + "\\")} title={`Scan ${d.letter}`}>
              <UsageRing used={used} total={d.total} />
              <div className="drive-meta">
                <div className="drive-title">
                  <span className="drive-letter">{d.letter}</span>
                  <span className="drive-label">{d.label || "Local Disk"}</span>
                </div>
                <div className="drive-sub">
                  {formatBytes(d.free)} free of {formatBytes(d.total, 0)}
                </div>
                <div className="drive-bar">
                  <span style={{ width: `${(used / Math.max(1, d.total)) * 100}%` }} />
                </div>
              </div>
            </button>
          );
        })}
      </div>
      <button className="btn btn-ghost wide" onClick={onPickFolder}>
        <IconFolder /> Scan a folder…
      </button>

      {(scanning || summary) && (
        <>
          <div className="section-label">{scanning ? "Scanning" : "Last scan"}</div>
          <div className="scan-card">
            <div className="scan-head">
              <div className={`scan-ring ${scanning ? "spinning" : "done"}`}>
                <svg width="38" height="38" viewBox="0 0 38 38">
                  <circle cx="19" cy="19" r="15" className="ring-track" strokeWidth="4" />
                  <circle cx="19" cy="19" r="15" className="ring-value glow" strokeWidth="4" stroke="url(#ring-ok)" strokeDasharray={scanning ? "30 94.2" : "94.2 94.2"} transform="rotate(-90 19 19)" />
                </svg>
              </div>
              <div className="scan-head-text">
                <div className="scan-title">
                  {scanning ? (
                    <>Scanning {target}</>
                  ) : summary?.cancelled ? (
                    <>Cancelled after {seconds.toFixed(1)} s</>
                  ) : summary?.fromCache ? (
                    <span title={new Date(summary.scannedAtMs).toLocaleString()}>Saved scan · {formatAgo(summary.scannedAtMs)}</span>
                  ) : (
                    <>Complete in {seconds.toFixed(1)} s</>
                  )}
                </div>
                {summary && !scanning ? (
                  <div className={`badge ${summary.method}`} title={summary.fallbackReason ?? undefined}>
                    <IconBolt /> {METHOD_LABEL[summary.method] ?? summary.method}
                  </div>
                ) : (
                  <div className="scan-elapsed num">{seconds.toFixed(1)} s</div>
                )}
              </div>
              {scanning && (
                <button className="chip cancel" onClick={onCancel}>
                  Cancel
                </button>
              )}
              {!scanning && summary?.fromCache && target && (
                <button className="chip" onClick={() => onScan(target)} title="Scan again for up-to-date numbers">
                  Rescan
                </button>
              )}
            </div>
            <dl className="scan-stats">
              <div>
                <dt>Files</dt>
                <dd><AnimatedNumber value={files} format={formatCount} reduceMotion={reduceMotion} /></dd>
              </div>
              <div>
                <dt>Folders</dt>
                <dd><AnimatedNumber value={dirs} format={formatCount} reduceMotion={reduceMotion} /></dd>
              </div>
              <div>
                <dt>Counted</dt>
                <dd><AnimatedNumber value={bytes} format={(n) => formatBytes(n)} reduceMotion={reduceMotion} /></dd>
              </div>
              <div>
                <dt>No access</dt>
                <dd className={summary && summary.inaccessible.length ? "warn" : ""}>
                  {summary && !scanning ? `${formatCount(summary.inaccessible.length)} folders` : "—"}
                </dd>
              </div>
            </dl>
            {couldUseMft && (
              <button className="mft-hint" onClick={onRestartAsAdmin} title={summary?.fallbackReason ?? undefined}>
                <IconBolt /> Scan much faster by reading the MFT: restart as administrator
              </button>
            )}
          </div>
        </>
      )}

      {summary && !scanning && summary.categories.length > 0 && (
        <>
          <div className="section-label">File types</div>
          <div className="legend">
            {summary.categories.slice(0, 9).map((c) => (
              <div key={c.category} className="legend-row">
                <span className="swatch" style={{ background: cssColor(c.category, 0.85), boxShadow: `0 0 10px ${cssColor(c.category, 0.5)}` }} />
                <span className="legend-name">{CATEGORY_LABELS[c.category]}</span>
                <span className="legend-size">{formatBytes(c.size)}</span>
                <span className="legend-bar">
                  <span style={{ width: `${(c.size / total) * 100}%`, background: cssColor(c.category, 0.8) }} />
                </span>
              </div>
            ))}
          </div>
        </>
      )}
    </aside>
  );
}
