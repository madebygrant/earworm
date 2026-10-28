// WCAG 2.1 contrast for the graphite theme, the same formula as theme::contrast in src/theme.rs.
// The other four themes are the TUI's and its suite measures them.
const graphite = {
  slots: { text: "#F1F2F4", accent: "#9DB7FF", cursor: "#7CDDB2", warn: "#F5B867", error: "#FF8F87", muted: "#A0A6B2", unsure: "#B9BEC8" },
  grounds: { near: "#1D2026", far: "#121418", surface: "#262A31" },
  ink: ["#16181D", "#9DB7FF"],
};
const lin = c => (c /= 255) <= 0.03928 ? c / 12.92 : ((c + 0.055) / 1.055) ** 2.4;
const lum = h => { const [r, g, b] = [1, 3, 5].map(i => parseInt(h.slice(i, i + 2), 16)); return 0.2126 * lin(r) + 0.7152 * lin(g) + 0.0722 * lin(b); };
const ratio = (a, b) => { const [x, y] = [lum(a), lum(b)].sort((p, q) => q - p); return (x + 0.05) / (y + 0.05); };
let worst = Infinity;
for (const [slot, c] of Object.entries(graphite.slots)) {
  const cells = Object.entries(graphite.grounds).map(([g, gc]) => { const r = ratio(c, gc); worst = Math.min(worst, r); return `${g} ${r.toFixed(2)}`; });
  console.log(slot.padEnd(7), cells.join("  "));
}
const ink = ratio(...graphite.ink); worst = Math.min(worst, ink);
console.log("ink on accent", ink.toFixed(2));
console.log(worst >= 4.5 ? `every slot clears 4.5:1 (lowest ${worst.toFixed(2)})` : `FAILS: lowest ${worst.toFixed(2)}`);
process.exit(worst >= 4.5 ? 0 : 1);
