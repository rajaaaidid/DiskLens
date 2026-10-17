import { useEffect, useMemo, useRef, useState } from "react";
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
  /** Bumped while a scan streams in or after deletes, to refresh sizes. */
  refreshTick: number;
  selectedId?: number;
  /** Selected node's ancestors from the scan root down (inclusive); folders on the way get expanded. */
  selectedChain?: number[];
  /** Node hovered in the treemap. */
  linkedHoverId?: number;
  onSelect: (id: number) => void;
  onOpen: (id: number) => void;
  onHoverRow: (id: number | undefined) => void;
  onContextMenu: (row: Row, clientX: number, clientY: number) => void;
}

interface Node {
  row: Row;
  depth: number;
  parentSize: number;
}

/** Folder tree of the current treemap root, linked both ways with the treemap. Children load lazily. */
export function TreeList(props: Props) {
  const { generation, rootId, rootSize, useAlloc, refreshTick, selectedId, selectedChain, linkedHoverId, onSelect, onOpen, onHoverRow, onContextMenu } = props;
  const [sort, setSort] = useState<{ key: SortKey; desc: boolean }>({ key: "size", desc: true });
  const [open, setOpen] = useState<Set<number>>(new Set());
  const [kids, setKids] = useState<Map<number, Row[]>>(new Map());
  const rowRefs = useRef(new Map<number, HTMLDivElement>());
  const scrollPending = useRef(false);

  useEffect(() => {
    setOpen(new Set());
    setKids(new Map());
  }, [generation, rootId]);

  // Reveal the selection: expand every folder between the current root and the selected item.
  useEffect(() => {
    if (!selectedChain?.length) return;
    const at = selectedChain.indexOf(rootId);
    if (at < 0) return;
    const between = selectedChain.slice(at + 1, -1);
    scrollPending.current = true;
    if (between.length) setOpen((o) => (between.every((id) => o.has(id)) ? o : new Set([...o, ...between])));
  }, [selectedChain, rootId]);

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

  // Scroll the selected row into view once it has been rendered.
  useEffect(() => {
    if (!scrollPending.current || selectedId === undefined) return;
    const el = rowRefs.current.get(selectedId);
    if (el) {
      el.scrollIntoView({ block: "nearest", behavior: "smooth" });
      scrollPending.current = false;
    }
  }, [rows, selectedId]);

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
    <section className="treelist glass" onMouseLeave={() => onHoverRow(undefined)}>
      <div className="tl-head">
        {header("name", "Name", "c-name")}
        {header("size", useAlloc ? "On disk" : "Size", "c-size")}
        <span className="th c-pct">% of parent</span>
        {header("files", "Files", "c-files")}
        {header("mtime", "Modified", "c-date")}
      </div>
      <div className="tl-body" role="tree" aria-label="Folder tree">
        {rows.map(({ row, depth, parentSize }) => {
          const size = useAlloc ? row.sizeOnDisk : row.size;
          const frac = parentSize > 0 ? size / parentSize : 0;
          const cat = CATEGORY_BY_ID[row.category] ?? "other";
          const cls = ["tl-row", selectedId === row.id && "selected", linkedHoverId === row.id && "linked", row.inaccessible && "denied"].filter(Boolean).join(" ");
          return (
            <div
              key={row.id}
              ref={(el) => (el ? rowRefs.current.set(row.id, el) : rowRefs.current.delete(row.id))}
              role="treeitem"
              aria-selected={selectedId === row.id}
              aria-expanded={row.isDir && row.hasChildren ? open.has(row.id) : undefined}
              className={cls}
              onClick={() => onSelect(row.id)}
              onDoubleClick={() => row.isDir && onOpen(row.id)}
              onMouseEnter={() => onHoverRow(row.id)}
              onContextMenu={(e) => {
                e.preventDefault();
                onSelect(row.id);
                onContextMenu(row, e.clientX, e.clientY);
              }}
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
