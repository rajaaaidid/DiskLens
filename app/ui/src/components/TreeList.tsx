import { useMemo, useState } from "react";
import { cssColor } from "../data/categories";
import { formatBytes, formatCount, formatDate, formatPercent } from "../data/format";
import type { TreeNode } from "../data/types";
import { IconChevron } from "./Icons";

type SortKey = "name" | "size" | "files" | "mtime";

interface Props {
  root: TreeNode;
  /** Folders expanded by default (mockup). */
  initiallyOpen?: number[];
  selectedId?: number;
  onSelect: (n: TreeNode) => void;
}

export function TreeList({ root, initiallyOpen = [], selectedId, onSelect }: Props) {
  const [sort, setSort] = useState<{ key: SortKey; desc: boolean }>({ key: "size", desc: true });
  const [open, setOpen] = useState<Set<number>>(() => new Set(initiallyOpen));

  const rows = useMemo(() => {
    const out: { node: TreeNode; depth: number }[] = [];
    const cmp = (a: TreeNode, b: TreeNode) => {
      const d =
        sort.key === "name" ? a.name.localeCompare(b.name) : sort.key === "size" ? a.size - b.size : sort.key === "files" ? a.files - b.files : a.mtime - b.mtime;
      return sort.desc ? -d : d;
    };
    const walk = (n: TreeNode, depth: number) => {
      const kids = [...n.children].sort(cmp).slice(0, 200);
      for (const k of kids) {
        out.push({ node: k, depth });
        if (k.isDir && open.has(k.id)) walk(k, depth + 1);
      }
    };
    walk(root, 0);
    return out;
  }, [root, sort, open]);

  const header = (key: SortKey, label: string, cls = "") => (
    <button className={`th ${cls} ${sort.key === key ? "sorted" : ""}`} onClick={() => setSort((s) => ({ key, desc: s.key === key ? !s.desc : key !== "name" }))}>
      {label}
      {sort.key === key && <span className="sort-arrow">{sort.desc ? "↓" : "↑"}</span>}
    </button>
  );

  return (
    <section className="treelist glass">
      <div className="tl-head">
        {header("name", "Name", "c-name")}
        {header("size", "Size", "c-size")}
        <span className="th c-pct">% of parent</span>
        {header("files", "Files", "c-files")}
        {header("mtime", "Modified", "c-date")}
      </div>
      <div className="tl-body">
        {rows.map(({ node, depth }) => {
          const parentSize = node.parent?.size ?? node.size;
          const frac = parentSize > 0 ? node.size / parentSize : 0;
          return (
            <div key={node.id} className={`tl-row ${selectedId === node.id ? "selected" : ""}`} onClick={() => onSelect(node)}>
              <span className="c-name" style={{ paddingLeft: 8 + depth * 16 }}>
                {node.isDir ? (
                  <button
                    className="twisty"
                    onClick={(e) => {
                      e.stopPropagation();
                      setOpen((o) => {
                        const n = new Set(o);
                        n.has(node.id) ? n.delete(node.id) : n.add(node.id);
                        return n;
                      });
                    }}
                  >
                    <IconChevron open={open.has(node.id)} />
                  </button>
                ) : (
                  <span className="twisty-spacer" />
                )}
                <span className="swatch" style={{ background: cssColor(node.category, node.isDir ? 0.55 : 0.85) }} />
                <span className="name-text">{node.name}</span>
              </span>
              <span className="c-size num">{formatBytes(node.size)}</span>
              <span className="c-pct">
                <span className="pct-bar">
                  <span style={{ width: `${frac * 100}%` }} />
                </span>
                <span className="num pct-text">{formatPercent(node.size, parentSize)}</span>
              </span>
              <span className="c-files num">{node.isDir ? formatCount(node.files) : "—"}</span>
              <span className="c-date num">{formatDate(node.mtime)}</span>
            </div>
          );
        })}
      </div>
    </section>
  );
}
