import { IconLens, IconMoon, IconSun } from "./Icons";

interface Props {
  theme: "dark" | "light";
  onToggleTheme: () => void;
  subtitle: string;
}

/** Custom frameless title bar. In Tauri the drag region is `data-tauri-drag-region`. */
export function TitleBar({ theme, onToggleTheme, subtitle }: Props) {
  return (
    <header className="titlebar" data-tauri-drag-region>
      <div className="titlebar-brand" data-tauri-drag-region>
        <IconLens />
        <span className="brand-name">DiskLens</span>
        <span className="brand-sub">{subtitle}</span>
      </div>
      <div className="titlebar-actions">
        <button className="icon-btn" onClick={onToggleTheme} aria-label="Toggle theme" title="Toggle theme">
          {theme === "dark" ? <IconSun /> : <IconMoon />}
        </button>
        <div className="win-controls">
          <button aria-label="Minimize">
            <svg width="10" height="10" viewBox="0 0 10 10"><path d="M0 5h10" stroke="currentColor" /></svg>
          </button>
          <button aria-label="Maximize">
            <svg width="10" height="10" viewBox="0 0 10 10"><rect x="0.5" y="0.5" width="9" height="9" rx="1.5" fill="none" stroke="currentColor" /></svg>
          </button>
          <button aria-label="Close" className="close">
            <svg width="10" height="10" viewBox="0 0 10 10"><path d="M0 0l10 10M10 0 0 10" stroke="currentColor" /></svg>
          </button>
        </div>
      </div>
    </header>
  );
}
