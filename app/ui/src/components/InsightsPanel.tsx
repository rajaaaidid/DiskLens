import { useState } from "react";
import { CATEGORY_LABELS, cssColor } from "../data/categories";
import { pathOf } from "../data/fakeTree";
import { formatBytes, formatCount } from "../data/format";
import type { Hog } from "../data/insights";
import type { Category, TreeNode } from "../data/types";
import { IconExternal } from "./Icons";

type Tab = "hogs" | "files" | "types";

interface Props {
  hogs: Hog[];
  files: TreeNode[];
  types: { ext: string; size: number; files: number; category: Category }[];
  total: number;
  onReveal: (n: TreeNode) => void;
}

const SEVERITY_LABEL: Record<Hog["severity"], string> = {
  reclaimable: "Reclaimable",
  system: "System",
  info: "Info",
};

export function InsightsPanel({ hogs, files, types, total, onReveal }: Props) {
  const [tab, setTab] = useState<Tab>("hogs");
  const maxType = types[0]?.size ?? 1;
  return (
    <aside className="insights glass">
      <div className="panel-head">
        <h2>Insights</h2>
        <div className="segmented" role="tablist">
          {(["hogs", "files", "types"] as Tab[]).map((t) => (
            <button key={t} role="tab" aria-selected={tab === t} className={tab === t ? "on" : ""} onClick={() => setTab(t)}>
              {t === "hogs" ? "Space hogs" : t === "files" ? "Largest" : "Types"}
            </button>
          ))}
        </div>
      </div>

      <div className="panel-scroll">
        {tab === "hogs" &&
          hogs.map((h) => (
            <article key={h.id} className="hog-card">
              <header>
                <span className={`sev ${h.severity}`}>{SEVERITY_LABEL[h.severity]}</span>
                <span className="hog-size">{formatBytes(h.size)}</span>
              </header>
              <h3>{h.title}</h3>
              <div className="hog-path" title={pathOf(h.node)}>{pathOf(h.node)}</div>
              <p>{h.why}</p>
              <p className="advice">{h.advice}</p>
              <div className="hog-meter">
                <span style={{ width: `${Math.max(1.5, (h.size / total) * 100)}%` }} />
              </div>
              <button className="chip" onClick={() => onReveal(h.node)}>
                <IconExternal /> Show in treemap
              </button>
            </article>
          ))}

        {tab === "files" && (
          <ol className="file-rank">
            {files.map((f, i) => (
              <li key={f.id}>
                <span className="rank">{i + 1}</span>
                <span className="swatch" style={{ background: cssColor(f.category, 0.85) }} />
                <span className="file-main">
                  <span className="file-name">{f.name}</span>
                  <span className="file-path">{pathOf(f.parent!)}</span>
                </span>
                <span className="file-size">{formatBytes(f.size)}</span>
              </li>
            ))}
          </ol>
        )}

        {tab === "types" && (
          <div className="type-list">
            {types.map((t) => (
              <div key={t.ext} className="type-row">
                <span className="ext" style={{ color: cssColor(t.category, 1), borderColor: cssColor(t.category, 0.4), background: cssColor(t.category, 0.12) }}>.{t.ext}</span>
                <span className="type-cat">{CATEGORY_LABELS[t.category]} · {formatCount(t.files)} files</span>
                <span className="type-size">{formatBytes(t.size)}</span>
                <span className="type-bar"><span style={{ width: `${(t.size / maxType) * 100}%`, background: cssColor(t.category, 0.75) }} /></span>
              </div>
            ))}
          </div>
        )}
      </div>
    </aside>
  );
}
