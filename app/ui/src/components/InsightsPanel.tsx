import { useState } from "react";
import type { ScanSummary } from "../api";
import { CATEGORY_LABELS, categoryForName, cssColor } from "../data/categories";
import { formatBytes, formatCount } from "../data/format";

type Tab = "files" | "folders" | "types";

interface Props {
  summary: ScanSummary | null;
  scanning: boolean;
  onReveal: (path: string) => void;
}

function splitPath(p: string) {
  const i = p.lastIndexOf("\\");
  return i > 0 ? { name: p.slice(i + 1), dir: p.slice(0, i) } : { name: p, dir: "" };
}

export function InsightsPanel({ summary, scanning, onReveal }: Props) {
  const [tab, setTab] = useState<Tab>("files");
  const types = summary?.extensions ?? [];
  const maxType = types[0]?.size ?? 1;
  return (
    <aside className="insights glass">
      <div className="panel-head">
        <h2>Insights</h2>
        <div className="segmented" role="tablist">
          {(["files", "folders", "types"] as Tab[]).map((t) => (
            <button key={t} role="tab" aria-selected={tab === t} className={tab === t ? "on" : ""} onClick={() => setTab(t)}>
              {t === "files" ? "Largest files" : t === "folders" ? "Folders" : "Types"}
            </button>
          ))}
        </div>
      </div>

      <div className="panel-scroll">
        {!summary && <p className="empty-note">{scanning ? "Insights appear when the scan finishes." : "Scan a drive to see insights."}</p>}

        {summary && tab === "files" && (
          <ol className="file-rank">
            {summary.largestFiles.map((f, i) => {
              const { name, dir } = splitPath(f.path);
              const cat = categoryForName(name);
              return (
                <li key={f.node} onClick={() => onReveal(f.path)} title={f.path}>
                  <span className="rank">{i + 1}</span>
                  <span className="swatch" style={{ background: cssColor(cat, 0.85) }} />
                  <span className="file-main">
                    <span className="file-name">{name}</span>
                    <span className="file-path">{dir}</span>
                  </span>
                  <span className="file-size">{formatBytes(f.size)}</span>
                </li>
              );
            })}
          </ol>
        )}

        {summary && tab === "folders" && (
          <ol className="file-rank">
            {summary.largestFolders.map((f, i) => (
              <li key={f.node} onClick={() => onReveal(f.path)} title={`${f.path}\n${formatBytes(f.size)} in files directly inside this folder`}>
                <span className="rank">{i + 1}</span>
                <span className="swatch" style={{ background: cssColor("folder", 0.7) }} />
                <span className="file-main">
                  <span className="file-name">{splitPath(f.path).name || f.path}</span>
                  <span className="file-path">{f.path}</span>
                </span>
                <span className="file-size">{formatBytes(f.size)}</span>
              </li>
            ))}
          </ol>
        )}

        {summary && tab === "types" && (
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
        )}
      </div>
    </aside>
  );
}
