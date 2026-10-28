// The pairings on trial. ui is names (titles, artists, folders); data is status words, numbers, paths and keys.
const PAIRS = [
  { name: "JetBrains Mono", note: "One mono for everything, as now. Closest to the TUI.",
    ui: '"JetBrains Mono", monospace', data: '"JetBrains Mono", monospace' },
  { name: "IBM Plex Sans + Plex Mono", note: "One superfamily. Engineered, a little corporate, very even.",
    ui: '"IBM Plex Sans", sans-serif', data: '"IBM Plex Mono", monospace', size: "13px", h: "16px" },
  { name: "Geist + Geist Mono", note: "Crisp and modern. The mono is narrow, so more fits on a line.",
    ui: '"Geist", sans-serif', data: '"Geist Mono", monospace', size: "13px", h: "16px" },
  { name: "Atkinson Hyperlegible Next + Mono", note: "Drawn for legibility, which matches the measured palettes.",
    ui: '"Atkinson Hyperlegible Next", sans-serif', data: '"Atkinson Hyperlegible Mono", monospace', size: "13px", h: "16px" },
  { name: "Recursive Linear + Mono", note: "One variable family that slides from sans to mono, so the two roles are siblings.",
    ui: '"Recursive", sans-serif', uiAxes: '"MONO" 0, "CASL" 0', data: '"Recursive", monospace', dataAxes: '"MONO" 1, "CASL" 0', size: "13px", h: "16px" },
  { name: "Schibsted Grotesk + Fragment Mono", note: "A newspaper grotesk over a Helvetica-shaped mono. Warmer, more editorial.",
    ui: '"Schibsted Grotesk", sans-serif', data: '"Fragment Mono", monospace', size: "13px", h: "16px" },
  { name: "Martian Mono condensed", note: "One mono throughout, squeezed to 75% width. Dense and mechanical.",
    ui: '"Martian Mono", monospace', data: '"Martian Mono", monospace', stretch: "75%" },
];
function applyPair(el, p) {
  el.style.setProperty("--ui", p.ui);
  el.style.setProperty("--data", p.data);
  el.style.setProperty("--ui-axes", p.uiAxes || "normal");
  el.style.setProperty("--data-axes", p.dataAxes || "normal");
  el.style.setProperty("--ui-size", p.size || "12px");
  el.style.setProperty("--ui-h", p.h || "14px");
  // Google serves Martian as a width range, so the narrow cut is picked by font-stretch, not by a raw axis setting.
  el.style.fontStretch = p.stretch || "";
}
