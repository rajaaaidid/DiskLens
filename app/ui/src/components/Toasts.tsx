export interface Toast {
  id: number;
  kind: "ok" | "error" | "info";
  text: string;
  detail?: string;
}

export function Toasts({ toasts, onDismiss }: { toasts: Toast[]; onDismiss: (id: number) => void }) {
  return (
    <div className="toasts" role="status" aria-live="polite">
      {toasts.map((t) => (
        <div key={t.id} className={`toast glass-strong ${t.kind}`} onClick={() => onDismiss(t.id)}>
          <div className="toast-text">{t.text}</div>
          {t.detail && <div className="toast-detail">{t.detail}</div>}
        </div>
      ))}
    </div>
  );
}
