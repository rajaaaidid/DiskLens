import type { Drive } from "../api";
import { formatBytes } from "../data/format";
import { IconFolder, IconLens, IconShield } from "./Icons";
import { UsageRing } from "./UsageRing";

interface Props {
  drives: Drive[];
  elevated: boolean;
  onScan: (path: string) => void;
  onPickFolder: () => void;
}

/** Start screen: every local drive as a glass card. */
export function DrivePicker({ drives, elevated, onScan, onPickFolder }: Props) {
  return (
    <div className="picker">
      <div className="picker-head">
        <IconLens size={40} />
        <div>
          <h1>What's using your space?</h1>
          <p>Pick a drive or folder to scan. Nothing is changed on disk without your confirmation.</p>
        </div>
      </div>
      <div className="picker-grid">
        {drives.map((d) => {
          const used = d.total - d.free;
          return (
            <button key={d.letter} className="picker-card glass" onClick={() => onScan(d.letter + "\\")}>
              <UsageRing used={used} total={d.total} size={88} stroke={7} />
              <div className="picker-meta">
                <div className="picker-title">
                  <span className="drive-letter big">{d.letter}</span>
                  <span className="drive-label">{d.label || (d.kind === "removable" ? "Removable" : "Local Disk")}</span>
                </div>
                <dl className="picker-stats">
                  <div><dt>Used</dt><dd>{formatBytes(used)}</dd></div>
                  <div><dt>Free</dt><dd>{formatBytes(d.free)}</dd></div>
                  <div><dt>Total</dt><dd>{formatBytes(d.total, 0)}</dd></div>
                </dl>
                <div className="drive-bar"><span style={{ width: `${(used / Math.max(1, d.total)) * 100}%` }} /></div>
                <div className="picker-fs">{d.fs} · {d.kind}</div>
              </div>
            </button>
          );
        })}
        <button className="picker-card glass folder" onClick={onPickFolder}>
          <span className="folder-icon"><IconFolder /></span>
          <div className="picker-meta">
            <div className="picker-title"><span className="drive-label">Scan a folder…</span></div>
            <p className="picker-fs">Any folder, network share or external drive.</p>
          </div>
        </button>
      </div>
      {!elevated && (
        <p className="picker-note">
          <IconShield /> Running without admin rights, so scans use the parallel directory walk. The much faster NTFS Master File Table
          scan needs administrator rights (coming in a later build).
        </p>
      )}
    </div>
  );
}
