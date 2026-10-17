import { useEffect, useRef, useState } from "react";
import { backend, type IntegrationInfo, type IntegrationSettings } from "../api";
import type { Toast } from "./Toasts";

interface Props {
  onClose: () => void;
  onToast: (kind: Toast["kind"], text: string, detail?: string) => void;
}

const THRESHOLDS = [5, 10, 15, 20];

/** Windows integration settings. Each change is applied (registry, tray) as soon as it's toggled. */
export function SettingsDialog({ onClose, onToast }: Props) {
  const [info, setInfo] = useState<IntegrationInfo | null>(null);
  const [busy, setBusy] = useState(false);
  const closeRef = useRef<HTMLButtonElement>(null);

  useEffect(() => {
    backend.getIntegration().then(setInfo).catch((e) => onToast("error", "Couldn't read the settings", String(e)));
  }, [onToast]);

  useEffect(() => {
    closeRef.current?.focus();
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") {
        e.stopPropagation();
        onClose();
      }
    };
    window.addEventListener("keydown", onKey, true);
    return () => window.removeEventListener("keydown", onKey, true);
  }, [onClose]);

  const change = async (patch: Partial<IntegrationSettings>) => {
    if (!info) return;
    setBusy(true);
    try {
      setInfo(await backend.setIntegration({ ...info.settings, ...patch }));
    } catch (e) {
      onToast("error", "Couldn't change the setting", String(e));
    } finally {
      setBusy(false);
    }
  };

  const s = info?.settings;
  const toggle = (key: keyof IntegrationSettings, label: string, hint: React.ReactNode, disabled = false) => (
    <label className={`setting ${disabled ? "disabled" : ""}`}>
      <input type="checkbox" className="switch" checked={Boolean(s?.[key])} disabled={!s || busy || disabled} onChange={(e) => change({ [key]: e.target.checked })} />
      <span className="setting-text">
        <span className="setting-label">{label}</span>
        <span className="setting-hint">{hint}</span>
      </span>
    </label>
  );

  return (
    <div className="modal-backdrop" onMouseDown={(e) => e.target === e.currentTarget && onClose()}>
      <div className="modal glass-strong settings" role="dialog" aria-modal="true" aria-labelledby="settings-title">
        <h2 id="settings-title">Settings</h2>
        {!backend.native && <p className="confirm-warn">Demo: these settings only change Windows when you run the HaveSpace app.</p>}

        <div className="section-label">Explorer</div>
        {toggle(
          "explorerMenu",
          "Add “Analyze with HaveSpace” to the right-click menu",
          <>For folders and drives. On Windows 11 it's under <i>Show more options</i>. Only your user account is changed.</>,
        )}

        <div className="section-label">Low disk space</div>
        {toggle("tray", "Keep HaveSpace in the notification area", "Closing the window keeps it running there. It checks your drives every 5 minutes and warns you when one runs low.")}
        <div className={`setting setting-row ${!s?.tray ? "disabled" : ""}`}>
          <span className="setting-label">Warn when free space drops below</span>
          <select value={s?.lowDiskPercent ?? 10} disabled={!s?.tray || busy} onChange={(e) => change({ lowDiskPercent: Number(e.target.value) })}>
            {THRESHOLDS.map((t) => (
              <option key={t} value={t}>
                {t}%
              </option>
            ))}
          </select>
        </div>
        {toggle("startWithWindows", "Start with Windows", "Starts hidden in the notification area when you sign in.", !s?.tray)}
        {info?.trayElsewhere && <p className="setting-hint">Another HaveSpace window is keeping watch in the notification area.</p>}
        <div className="settings-actions">
          <button
            className="chip"
            disabled={!s?.tray}
            onClick={() => backend.testNotification().catch((e) => onToast("info", "No notification sent", String(e)))}
          >
            Show a sample warning
          </button>
        </div>

        <div className="section-label">Taskbar</div>
        <p className="setting-hint">Right-click HaveSpace on the taskbar to reopen your recent scans. This is always on.</p>

        <div className="modal-actions">
          <button ref={closeRef} className="btn btn-secondary" onClick={onClose}>
            Done
          </button>
        </div>
      </div>
    </div>
  );
}
