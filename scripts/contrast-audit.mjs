// WCAG contrast audit for HaveSpace text on glass. Text sits on translucent panels over a moving aurora,
// so we composite each text token over each panel over the extreme aurora colours the shader can
// produce (see AURORA_FN in app/ui/src/treemap/shaders.ts), and report the worst case.
// Usage: node scripts/contrast-audit.mjs

const css = (await import("node:fs")).readFileSync(new URL("../app/ui/src/styles.css", import.meta.url), "utf8");

function tokens(selector) {
  const block = css.match(new RegExp(selector.replace(/[[\]"()]/g, "\\$&") + "\\s*\\{([^}]*)\\}"))[1];
  const out = {};
  for (const m of block.matchAll(/--([\w-]+):\s*rgba?\(([^)]+)\)/g)) {
    const [r, g, b, a = "1"] = m[2].split(",").map((s) => s.trim());
    out[m[1]] = [+r / 255, +g / 255, +b / 255, +a];
  }
  return out;
}

const over = (fg, bg) => [0, 1, 2].map((i) => fg[i] * fg[3] + bg[i] * (1 - fg[3]));
const lin = (c) => (c <= 0.04045 ? c / 12.92 : ((c + 0.055) / 1.055) ** 2.4);
const lum = ([r, g, b]) => 0.2126 * lin(r) + 0.7152 * lin(g) + 0.0722 * lin(b);
const ratio = (a, b) => {
  const [x, y] = [lum(a), lum(b)].sort((p, q) => q - p);
  return (x + 0.05) / (y + 0.05);
};

// Aurora extremes (base colour plus the strongest blob), per theme, as the shader computes them.
const auroraDark = [
  [0.027, 0.035, 0.075], // between blobs
  [0.027 + 0.3 * 0.85, 0.035 + 0.16 * 0.85, 0.075 + 0.62 * 0.85], // violet peak
  [0.027 + 0.05 * 0.85, 0.035 + 0.36 * 0.85, 0.075 + 0.52 * 0.85], // teal peak
  [0.027 + 0.45 * 0.85, 0.035 + 0.1 * 0.85, 0.075 + 0.38 * 0.85], // magenta peak
];
const mixL = (base, k, g) => base.map((c, i) => c + (k[i] - c) * g * 0.55);
const baseL = [0.925, 0.937, 0.965];
const auroraLight = [baseL, mixL(baseL, [0.66, 0.6, 0.98], 1), mixL(baseL, [0.55, 0.85, 0.95], 1), mixL(baseL, [0.98, 0.7, 0.86], 1)];

let failures = 0;
for (const [theme, sel, aurora] of [
  ["dark", ":root", auroraDark],
  ["light", ':root[data-theme="light"]', auroraLight],
]) {
  const t = { ...tokens(":root"), ...(theme === "light" ? tokens(sel) : {}) };
  console.log(`\n${theme} theme (worst case over the aurora)`);
  for (const panel of ["glass", "glass-strong"]) {
    for (const text of ["text", "text-2", "text-3", "grow", "shrink"]) {
      let worst = Infinity;
      for (const a of aurora) {
        const bg = over(t[panel], a);
        // Inset wells (cards inside panels) darken/lighten further; include them.
        for (const surface of [bg, over(t["inset"], bg)]) {
          worst = Math.min(worst, ratio(over(t[text], surface), surface));
        }
      }
      // text-3 is used for small secondary labels: hold it to 4.5 like body text. grow/shrink colour the
      // size changes in the Changes tab.
      const ok = worst >= 4.5;
      if (!ok) failures++;
      console.log(`  --${text.padEnd(7)} on --${panel.padEnd(12)} ${worst.toFixed(2)}:1 ${ok ? "AA" : "FAILS AA (4.5:1)"}`);
    }
  }
}

// Treemap labels sit on the glass tiles themselves (see BLOCK_FS): tint mixed over the blurred aurora,
// frosted, brightened at the top. Check label colours from Treemap.tsx over every category tint.
const cats = {
  folder: [0.74, 0.79, 0.92], video: [0.55, 0.36, 0.96], image: [0.13, 0.83, 0.93], audio: [0.52, 0.8, 0.09], archive: [0.96, 0.62, 0.04],
  executable: [0.96, 0.25, 0.37], document: [0.06, 0.73, 0.51], code: [0.23, 0.51, 0.96], system: [0.39, 0.45, 0.55], other: [0.61, 0.64, 0.69],
};
const labels = {
  dark: { primary: [1, 1, 1, 0.96], secondary: [235 / 255, 240 / 255, 1, 0.76] },
  light: { primary: [10 / 255, 16 / 255, 30 / 255, 0.94], secondary: [10 / 255, 16 / 255, 30 / 255, 0.7] },
};
// Age colours (app/ui/src/data/age.ts) are drawn with AGE_TINT_BOOST (Treemap.tsx): leaf tint alpha x 1.8,
// and in the light theme x 1.3 more, capped at 0.8 (see BLOCK_FS).
const ages = {
  "< 1 month": [0.13, 0.83, 0.93], "1-6 months": [0.06, 0.73, 0.51], "6-12 months": [0.52, 0.8, 0.09], "1-2 years": [0.96, 0.62, 0.04],
  "2-5 years": [0.96, 0.42, 0.18], "5+ years": [0.96, 0.25, 0.37], "no date": [0.61, 0.64, 0.69],
};
const modes = [
  // [name, tints, tint alphas per theme, dark-theme label pill alpha (LABEL_PILL / LABEL_PILL_AGE)]
  ["file types", cats, { dark: [0.27, 0.4], light: [0.35, 0.52] }, 0.55],
  ["age", ages, { dark: [0.27 * 1.8, 0.4 * 1.8], light: [Math.min(0.8, 0.27 * 1.8 * 1.3), 0.8] }, 0.64],
];
for (const [mode, tints, alphas, pill] of modes)
for (const [theme, aurora] of [["dark", auroraDark], ["light", auroraLight]]) {
  console.log(`\n${theme} treemap labels, ${mode} (worst tile)`);
  for (const which of ["primary", "secondary"]) {
    let worst = Infinity;
    let at = "";
    for (const [name, tint] of Object.entries(tints)) {
      for (const a of aurora) {
        // Saturation boost on the blurred background, as in the shader.
        const l = 0.299 * a[0] + 0.587 * a[1] + 0.114 * a[2];
        const bg = a.map((c) => Math.min(1, Math.max(0, l + (c - l) * 1.35)));
        for (const tintA of alphas[theme]) {
          let g = bg.map((c, i) => c + (tint[i] - c) * tintA);
          g = g.map((c) => c + (1 - c) * (theme === "light" ? 0.2 : 0.045));
          g = g.map((c) => Math.min(1, c * 1.14 + 0.06)); // brightest spot: top edge + sheen
          // Dark theme: labels sit on a frosted dark pill (LABEL_PILL in Treemap.tsx).
          if (theme === "dark") g = over([8 / 255, 10 / 255, 24 / 255, pill], g);
          const r = ratio(over(labels[theme][which], g), g);
          if (r < worst) [worst, at] = [r, name];
        }
      }
    }
    const ok = worst >= 4.5;
    if (!ok) failures++;
    console.log(`  ${which.padEnd(9)} ${worst.toFixed(2)}:1 (on ${at}) ${ok ? "AA" : "FAILS AA (4.5:1)"}`);
  }
}
process.exitCode = failures ? 1 : 0;