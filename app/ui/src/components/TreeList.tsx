import { useEffect, useMemo, useState } from "react";
import { backend, type Row } from "../api";
import { CATEGORY_BY_ID, cssColor } from "../data/categories";
import { formatBytes, formatCount, formatDate, formatPercent } from "../data/format";
import { IconChevron } from "./Icons";

type SortKey = "name" | "size" | "files" | "mtime";

interface Props {
  generation: number;
  rootId: number;
  rootSize: number;
  useAlloc: boolean;
  /** Bumped while a scan streams in, to refresh sizes. */
  refreshTick: number;
  selectedId?: number;
  onSelect: (id: number) => void;
  onOpen: (id: number) => void;
}

interface Node {
  row: Row;
  depth: number;
  parentSize: number;
}

/** Folder tree of the current treemap root. Children load lazily when a folder is expanded. */
export function TreeList({ generation, rootId, rootSize, useAlloc, refreshTick, selectedId, onSelect, onOpen }: Props) {
  const [sort, setSort] = useState<{ key: SortKey; desc: boolean }>({ key: "size", desc: true });
  const [open, setOpen] = useState<Set<number>>(new Set());
  const [kids, setKids] = useState<Map<number, Row[]>>(new Map());

  // Reset when the root changes.
  useEffect(() => {
    setOpen(new Set());
    setKids(new Map());
  }, [generation, rootId]);

  // (Re)load the root and every expanded folder.
  useEffect(() => {
    let alive = true;
    const ids = [rootId, ...open];
    Promise.all(ids.map((id) => backend.children(generation, id, useAlloc).then((r) => [id, r] as const).catch(() => [id, [] as Row[]] as const))).then((pairs) => {
      if (alive) setKids(new Map(pairs));
    });
    return () => {
      alive = false;
    };
  }, [generation, rootId, open, useAlloc, refreshTick]);

  const rows = useMemo(() => {
    const out: Node[] = [];
    const sizeOf = (r: Row) => (useAlloc ? r.sizeOnDisk : r.size);
    const cmp = (a: Row, b: Row) => {
      const d = sort.key === "name" ? a.name.localeCompare(b.name) : sort.key === "size" ? sizeOf(a) - sizeOf(b) : sort.key === "files" ? a.files - b.files : a.mtimeMs - b.mtimeMs;
      return sort.desc ? -d : d;
    };
    const walk = (id: number, depth: number, parentSize: number) => {
      const list = [...(kids.get(id) ?? [])].sort(cmp);
      for (const r of list) {
        out.push({ row: r, depth, parentSize });
        if (r.isDir && open.has(r.id)) walk(r.id, depth + 1, sizeOf(r));
      }
    };
    walk(rootId, 0, rootSize);
    return out;
  }, [kids, sort, open, rootId, rootSize, useAlloc]);

  const header = (key: SortKey, label: string, cls = "") => (
    <button className={`th ${cls} ${sort.key === key ? "sorted" : ""}`} onClick={() => setSort((s) => ({ key, desc: s.key === key ? !s.desc : key !== "name" }))}>
      {label}
      {sort.key === key && <span className="sort-arrow">{sort.desc ? "↓" : "↑"}</span>}
    </button>
  );

  const toggle = (id: number) =>
    setOpen((o) => {
      const n = new Set(o);
      n.has(id) ? n.delete(id) : n.add(id);
      return n;
    });

  return (
    <section className="treelist glass">
      <div className="tl-head">
        {header("name", "Name", "c-name")}
        {header("size", useAlloc ? "On disk" : "Size", "c-size")}
        <span className="th c-pct">% of parent</span>
        {header("files", "Files", "c-files")}
        {header("mtime", "Modified", "c-date")}
      </div>
      <div className="tl-body">
        {rows.map(({ row, depth, parentSize }) => {
          const size = useAlloc ? row.sizeOnDisk : row.size;
          const frac = parentSize > 0 ? size / parentSize : 0;
          const cat = CATEGORY_BY_ID[row.category] ?? "other";
          return (
            <div
              key={row.id}
              className={`tl-row ${selectedId === row.id ? "selected" : ""} ${row.inaccessible ? "denied" : ""}`}
              onClick={() => onSelect(row.id)}
              onDoubleClick={() => row.isDir && onOpen(row.id)}
              title={row.inaccessible ? "Access denied" : row.link ? "Link (not followed)" : undefined}
            >
              <span className="c-name" style={{ paddingLeft: 8 + depth * 16 }}>
                {row.isDir && row.hasChildren ? (
                  <button
                    className="twisty"
                    aria-label={open.has(row.id) ? "Collapse" : "Expand"}
                    onClick={(e) => {
                      e.stopPropagation();
                      toggle(row.id);
                    }}
                  >
                    <IconChevron open={open.has(row.id)} />
                  </button>
                ) : (
                  <span className="twisty-spacer" />
                )}
                <span className="swatch" style={{ background: cssColor(cat, row.isDir ? 0.55 : 0.85) }} />
                <span className="name-text">{row.name}</span>
                {row.link && <span className="tag">link</span>}
                {row.inaccessible && <span className="tag warn">no access</span>}
              </span>
              <span className="c-size num">{formatBytes(size)}</span>
              <span className="c-pct">
                <span className="pct-bar">
                  <span style={{ width: `${Math.min(100, frac * 100)}%` }} />
                </span>
                <span className="num pct-text">{formatPercent(size, parentSize)}</span>
              </span>
              <span className="c-files num">{row.isDir ? formatCount(row.files) : "—"}</span>
              <span className="c-date num">{row.mtimeMs > 0 ? formatDate(row.mtimeMs) : "—"}</span>
            </div>
          );
        })}
      </div>
    </section>
  );
}
