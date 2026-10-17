import { useEffect, useLayoutEffect, useRef, useState } from "react";
import { backend, type ActionInfo } from "../api";
import { IconCopy, IconExternal, IconFolder, IconTrash } from "./Icons";

export interface MenuTarget {
  id: number;
  name: string;
  isDir: boolean;
  x: number;
  y: number;
}

interface Props {
  target: MenuTarget;
  generation: number;
  onClose: () => void;
  onZoom: (id: number) => void;
  onReveal: (id: number) => void;
  onCopyPath: (id: number) => void;
  onRecycle: (info: ActionInfo) => void;
}

/** Frosted right-click menu. Nothing happens until an item is chosen; recycling opens a confirmation. */
export function ContextMenu({ target, generation, onClose, onZoom, onReveal, onCopyPath, onRecycle }: Props) {
  const ref = useRef<HTMLDivElement>(null);
  const [info, setInfo] = useState<ActionInfo | null>(null);
  const [pos, setPos] = useState({ left: target.x, top: target.y });

  useEffect(() => {
    let alive = true;
    backend.actionInfo(generation, target.id).then((i) => alive && setInfo(i)).catch(() => {});
    return () => {
      alive = false;
    };
  }, [generation, target.id]);

  // Keep on screen.
  useLayoutEffect(() => {
    const el = ref.current;
    if (!el) return;
    const r = el.getBoundingClientRect();
    setPos({ left: Math.min(target.x, window.innerWidth - r.width - 8), top: Math.min(target.y, window.innerHeight - r.height - 8) });
    el.querySelector<HTMLButtonElement>("button:not(:disabled)")?.focus();
  }, [target.x, target.y, info]);

  useEffect(() => {
    const close = (e: Event) => {
      if (e instanceof KeyboardEvent) {
        if (e.key !== "Escape") return;
        e.stopPropagation(); // don't also zoom out
      }
      if (e instanceof MouseEvent && ref.current?.contains(e.target as Node)) return;
      onClose();
    };
    window.addEventListener("mousedown", close, true);
    window.addEventListener("keydown", close, true);
    window.addEventListener("blur", onClose);
    return () => {
      window.removeEventListener("mousedown", close, true);
      window.removeEventListener("keydown", close, true);
      window.removeEventListener("blur", onClose);
    };
  }, [onClose]);

  const run = (f: () => void) => () => {
    onClose();
    f();
  };

  return (
    <div ref={ref} className="ctx-menu glass-strong" style={pos} role="menu" aria-label={`Actions for ${target.name}`}>
      <div className="ctx-title" title={info?.path}>{target.name}</div>
      {target.isDir && (
        <button role="menuitem" onClick={run(() => onZoom(target.id))}>
          <IconFolder /> Zoom into folder
        </button>
      )}
      <button role="menuitem" onClick={run(() => onReveal(target.id))}>
        <IconExternal /> Open in Explorer
      </button>
      <button role="menuitem" onClick={run(() => onCopyPath(target.id))}>
        <IconCopy /> Copy path
      </button>
      <div className="ctx-sep" />
      <button
        role="menuitem"
        className="danger"
        disabled={!info || info.recycleBlocked !== null}
        title={info?.recycleBlocked ?? undefined}
        onClick={run(() => info && onRecycle(info))}
      >
        <IconTrash /> Send to Recycle Bin…
      </button>
      {info?.recycleBlocked && <div className="ctx-note">{info.recycleBlocked}</div>}
    </div>
  );
}
