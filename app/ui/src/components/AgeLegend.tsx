import { AGE_BANDS, AGE_UNKNOWN_COLOR, cssRgb } from "../data/age";

/** Key for the treemap's age colours, pinned to the bottom-left of the treemap. */
export function AgeLegend() {
  return (
    <div className="age-legend glass-strong" aria-label="Tile colours: last modified">
      <span className="age-legend-title">Last modified</span>
      {AGE_BANDS.map((b) => (
        <span key={b.minDays} className="age-legend-item">
          <span className="swatch" style={{ background: cssRgb(b.color, 0.9) }} />
          {b.label}
        </span>
      ))}
      <span className="age-legend-item">
        <span className="swatch" style={{ background: cssRgb(AGE_UNKNOWN_COLOR, 0.9) }} />
        No date
      </span>
    </div>
  );
}
