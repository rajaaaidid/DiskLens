import { useEffect, useRef, useState } from "react";
import type { ActionInfo } from "../api";
import { formatBytes, formatCount } from "../data/format";
import { IconTrash } from "./Icons";

interface Props {
  info: ActionInfo;
  onCancel: () => void;
  onConfirm: () => Promise<void>;
}

/** Modal confirmation for sending one item to the Recycle Bin. Cancel has the focus by default. */
export function ConfirmRecycle({ info, onCancel, onConfirm }: Props) {
  const [busy, setBusy] = useState(false);
  const cancelRef = useRef<HTMLButtonElement>(null);

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

  const confirm = async () => {
    setBusy(true);
    try {
      await onConfirm();
    } finally {
      setBusy(false);
    }
  };

  return (
    <div className="modal-backdrop" onMouseDown={(e) => e.target === e.currentTarget && !busy && onCancel()}>
      <div className="modal glass-strong" role="alertdialog" aria-modal="true" aria-labelledby="confirm-title" aria-describedby="confirm-body">
        <div className="modal-icon">
          <IconTrash />
        </div>
        <h2 id="confirm-title">Send to the Recycle Bin?</h2>
        <div id="confirm-body">
          <div className="confirm-item">
            <div className="confirm-name">{info.name}</div>
            <div className="confirm-path">{info.path}</div>
            <div className="confirm-stats">
              <span>{formatBytes(info.size, 2)}</span>
              {info.isDir && <span>{formatCount(info.files)} files</span>}
            </div>
          </div>
          {info.protection.kind === "warn" && <p className="confirm-warn">{info.protection.reason}</p>}
          <p className="confirm-note">You can restore it from the Recycle Bin. HaveSpace never deletes permanently: if Windows can't recycle it, nothing is removed.</p>
        </div>
        <div className="modal-actions">
          <button ref={cancelRef} className="btn btn-secondary" onClick={onCancel} disabled={busy}>
            Cancel
          </button>
          <button className="btn btn-danger" onClick={confirm} disabled={busy}>
            {busy ? "Moving…" : "Send to Recycle Bin"}
          </button>
        </div>
      </div>
    </div>
  );
}
