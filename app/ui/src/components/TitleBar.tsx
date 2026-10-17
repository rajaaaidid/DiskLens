import { backend } from "../api";
import { IconLens, IconMoon, IconSun } from "./Icons";

interface Props {
  theme: "dark" | "light";
  onToggleTheme: () => void;
  subtitle: string;
  /** Short notice shown in the middle of the bar (e.g. demo mode). */
  notice?: React.ReactNode;
}

/** Custom frameless title bar. `data-tauri-drag-region` makes it drag the window. */
export function TitleBar({ theme, onToggleTheme, subtitle, notice }: Props) {
  return (
    <header className="titlebar" data-tauri-drag-region onDoubleClick={() => backend.windowControl("maximize")}>
      <div className="titlebar-brand" data-tauri-drag-region>
        <IconLens />
        <span className="brand-name" data-tauri-drag-region>DiskLens</span>
        {subtitle && <span className="brand-sub" data-tauri-drag-region>{subtitle}</span>}
      </div>
      {notice && (
        <div className="titlebar-notice" role="note">
          {notice}
        </div>
      )}
      <div className="titlebar-actions">
        <button className="icon-btn" onClick={onToggleTheme} aria-label="Toggle theme" title="Toggle theme">
          {theme === "dark" ? <IconSun /> : <IconMoon />}
        </button>
        <div className="win-controls">
          <button aria-label="Minimize" onClick={() => backend.windowControl("minimize")}>
            <svg width="10" height="10" viewBox="0 0 10 10"><path d="M0 5h10" stroke="currentColor" /></svg>
          </button>
          <button aria-label="Maximize" onClick={() => backend.windowControl("maximize")}>
            <svg width="10" height="10" viewBox="0 0 10 10"><rect x="0.5" y="0.5" width="9" height="9" rx="1.5" fill="none" stroke="currentColor" /></svg>
          </button>
          <button aria-label="Close" className="close" onClick={() => backend.windowControl("close")}>
            <svg width="10" height="10" viewBox="0 0 10 10"><path d="M0 0l10 10M10 0 0 10" stroke="currentColor" /></svg>
          </button>
        </div>
      </div>
    </header>
  );
}
