import { useEffect, useRef, useState } from "react";
import type { ExportKind } from "../api";
import { IconExport } from "./Icons";

const ITEMS: { kind: ExportKind; label: string; hint: string }[] = [
  { kind: "csv-all", label: "CSV: every file and folder", hint: "One row per item, largest first. Opens in Excel." },
  { kind: "csv-folders", label: "CSV: folders only", hint: "Smaller; good for spotting big folders." },
  { kind: "json-tree", label: "JSON: folder tree", hint: "Nested, 6 levels deep, for scripts and tools." },
];

/** Toolbar button with the export choices; exports what is currently shown (the zoomed folder). */
export function ExportMenu({ disabled, onExport }: { disabled: boolean; onExport: (kind: ExportKind) => void }) {
  const [open, setOpen] = useState(false);
  const ref = useRef<HTMLDivElement>(null);

  useEffect(() => {
    if (!open) return;
    const close = (e: Event) => {
      if (e instanceof KeyboardEvent && e.key !== "Escape") return;
      if (e instanceof MouseEvent && ref.current?.contains(e.target as Node)) return;
      e.stopPropagation();
      setOpen(false);
    };
    window.addEventListener("mousedown", close, true);
    window.addEventListener("keydown", close, true);
    return () => {
      window.removeEventListener("mousedown", close, true);
      window.removeEventListener("keydown", close, true);
    };
  }, [open]);

  return (
    <div className="export-wrap" ref={ref}>
      <button className="icon-btn" title="Export this view" aria-haspopup="menu" aria-expanded={open} disabled={disabled} onClick={() => setOpen((o) => !o)}>
        <IconExport />
      </button>
      {open && (
        <div className="ctx-menu glass-strong export-menu" role="menu">
          <div className="ctx-title">Export the current view</div>
          {ITEMS.map((it) => (
            <button
              key={it.kind}
              role="menuitem"
              onClick={() => {
                setOpen(false);
                onExport(it.kind);
              }}
            >
              <span className="export-item">
                <span>{it.label}</span>
                <span className="export-hint">{it.hint}</span>
              </span>
            </button>
          ))}
        </div>
      )}
    </div>
  );
}
