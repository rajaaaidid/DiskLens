import type { CacheEntry, Drive } from "../api";
import { formatAgo, formatBytes, formatCount } from "../data/format";
import { IconBolt, IconFolder, IconLens, IconShield } from "./Icons";
import { UsageRing } from "./UsageRing";

interface Props {
  drives: Drive[];
  caches: CacheEntry[];
  elevated: boolean;
  onScan: (path: string) => void;
  onOpenCache: (root: string) => void;
  onPickFolder: () => void;
  onRestartAsAdmin: () => void;
}

/** Start screen: every local drive as a glass card. */
export function DrivePicker({ drives, caches, elevated, onScan, onOpenCache, onPickFolder, onRestartAsAdmin }: Props) {
  return (
    <div className="picker">
      <div className="picker-head">
        <IconLens size={40} />
        <div>
          <h1>What's using your space?</h1>
          <p>Pick a drive or folder to scan. Nothing is changed on disk without your confirmation.</p>
        </div>
      </div>
      {caches.length > 0 && (
        <section className="recent" aria-label="Recent scans">
          <div className="section-label">Recent scans · open instantly</div>
          <div className="recent-list">
            {caches.slice(0, 4).map((c) => (
              <div key={c.root} className="recent-row glass">
                <div className="recent-main">
                  <span className="recent-root">{c.root}</span>
                  <span className="recent-meta">
                    {formatBytes(c.size)} · {formatCount(c.files)} files · scanned {formatAgo(c.scannedAtMs)}
                  </span>
                </div>
                <button className="btn btn-primary" onClick={() => onOpenCache(c.root)}>
                  Open
                </button>
                <button className="btn btn-secondary" onClick={() => onScan(c.root)} title="Scan again for up-to-date numbers">
                  Rescan
                </button>
              </div>
            ))}
          </div>
        </section>
      )}
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
      {elevated ? (
        <p className="picker-note good">
          <IconBolt /> Running as administrator: NTFS drives are scanned by reading the Master File Table directly.
        </p>
      ) : (
        <div className="picker-note">
          <IconShield />
          <span>
            Without admin rights, scans walk every folder. As administrator, HaveSpace reads the NTFS Master File Table directly, which is
            several times faster and also sees folders your account can't open.
          </span>
          <button className="btn btn-secondary admin-btn" onClick={onRestartAsAdmin}>
            <IconShield /> Restart as administrator
          </button>
        </div>
      )}
    </div>
  );
}
