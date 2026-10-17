import { useEffect, useRef, useState } from "react";
import { backend, type ActionInfo, type DupeSet } from "../api";
import { formatBytes, formatDate } from "../data/format";
import { IconTrash } from "./Icons";

interface Props {
  set: DupeSet;
  generation: number;
  onCancel: () => void;
  /** Send `ids` to the Recycle Bin (the user has confirmed). */
  onConfirm: (ids: number[], keptName: string) => Promise<void>;
}

/**
 * Choose which copy of a duplicate set to keep; the others go to the Recycle Bin. The oldest copy is
 * picked by default (usually the original). Copies HaveSpace refuses to recycle always stay. Cancel has
 * the focus, as in the single-item confirmation.
 */
export function KeepOneDialog({ set, generation, onCancel, onConfirm }: Props) {
  const [infos, setInfos] = useState<(ActionInfo | null)[] | null>(null);
  const [keep, setKeep] = useState(() => {
    let best = 0;
    set.files.forEach((f, i) => {
      if (f.mtimeMs && (!set.files[best].mtimeMs || f.mtimeMs < set.files[best].mtimeMs)) best = i;
    });
    return set.files[best].node;
  });
  const [busy, setBusy] = useState(false);
  const cancelRef = useRef<HTMLButtonElement>(null);

  useEffect(() => {
    let alive = true;
    Promise.all(set.files.map((f) => backend.actionInfo(generation, f.node).catch(() => null))).then((r) => alive && setInfos(r));
    return () => {
      alive = false;
    };
  }, [set, generation]);

  useEffect(() => {
    cancelRef.current?.focus();
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape" && !busy) {
        e.stopPropagation();
        onCancel();
      }
    };
    window.addEventListener("keydown", onKey, true);
    return () => window.removeEventListener("keydown", onKey, true);
  }, [busy, onCancel]);

  const blockedWhy = (i: number) => (infos ? (infos[i] ? infos[i]!.recycleBlocked : "Couldn't check this item.") : null);
  const recycle = set.files.filter((f, i) => f.node !== keep && infos && !blockedWhy(i));
  const warnings = [...new Set(set.files.flatMap((f, i) => (f.node !== keep && infos?.[i]?.protection.kind === "warn" ? [infos[i]!.protection.reason] : [])))];
  const keptName = set.files.find((f) => f.node === keep)?.path.split("\\").pop() ?? "";

  const confirm = async () => {
    setBusy(true);
    try {
      await onConfirm(
        recycle.map((f) => f.node),
        keptName,
      );
    } finally {
      setBusy(false);
    }
  };

  return (
    <div className="modal-backdrop" onMouseDown={(e) => e.target === e.currentTarget && !busy && onCancel()}>
      <div className="modal glass-strong keep-one" role="alertdialog" aria-modal="true" aria-labelledby="keep-title" aria-describedby="keep-body">
        <div className="modal-icon">
          <IconTrash />
        </div>
        <h2 id="keep-title">Keep one copy</h2>
        <div id="keep-body">
          <p className="confirm-note">Choose the copy to keep. The others go to the Recycle Bin.</p>
          <div className="keep-list" role="radiogroup" aria-label="Copy to keep">
            {set.files.map((f, i) => {
              const why = blockedWhy(i);
              const name = f.path.split("\\").pop();
              return (
                <label key={f.node} className={`keep-row ${f.node === keep ? "kept" : ""}`}>
                  <input type="radio" name="keep" checked={f.node === keep} onChange={() => setKeep(f.node)} disabled={busy} />
                  <span className="keep-main">
                    <span className="confirm-name">{name}</span>
                    <span className="confirm-path">{f.path}</span>
                    <span className="keep-meta">
                      {f.mtimeMs ? `Modified ${formatDate(f.mtimeMs)}` : "Modified at an unknown time"}
                      {f.node === keep ? " · kept" : why ? ` · stays: ${why}` : " · to the Recycle Bin"}
                    </span>
                  </span>
                </label>
              );
            })}
          </div>
          {warnings.map((w) => (
            <p key={w} className="confirm-warn">
              {w}
            </p>
          ))}
          <p className="confirm-note">
            {infos
              ? `${recycle.length} ${recycle.length === 1 ? "copy" : "copies"} · ${formatBytes(recycle.length * set.size)} freed. You can restore them from the Recycle Bin. HaveSpace never deletes permanently.`
              : "Checking the copies…"}
          </p>
        </div>
        <div className="modal-actions">
          <button ref={cancelRef} className="btn btn-secondary" onClick={onCancel} disabled={busy}>
            Cancel
          </button>
          <button className="btn btn-danger" onClick={confirm} disabled={busy || !infos || recycle.length === 0}>
            {busy ? "Moving…" : `Send ${recycle.length} to Recycle Bin`}
          </button>
        </div>
      </div>
    </div>
  );
}
