// Draws the chrome every screen shares, so each page only writes its own main area.
const THEMES = ["graphite", "light", "warm", "cool", "neon"];
const ICON = { album: "●", playlist: "▤", local: "◆", custom: "≡" };
const SETTINGS = 'format <b>opus</b> · lookup <b>on</b> · cover <b>on</b> · m3u8 <b>on</b> · rename <b>on</b>';
const hasArt = st => !["queued", "fetching", "fetched", "tagging", "failed", "missing"].includes(st);
const pad = n => String(n).padStart(2, "0");

function theme(t) {
  if (t) document.documentElement.dataset.theme = t; else delete document.documentElement.dataset.theme;
  try { t ? localStorage.setItem("theme", t) : localStorage.removeItem("theme"); } catch {}
  const sel = document.getElementById("theme"); if (sel) sel.value = current();
}
function current() {
  return document.documentElement.dataset.theme || (matchMedia("(prefers-color-scheme: light)").matches ? "light" : "graphite");
}
addEventListener("keydown", e => {
  if (e.ctrlKey && e.key === "t") { e.preventDefault(); theme(THEMES[(THEMES.indexOf(current()) + 1) % THEMES.length]); }
});
try { const t = localStorage.getItem("theme"); if (t) document.documentElement.dataset.theme = t; } catch {}

function header({ folder, stage, count, right = SETTINGS, spin = false } = {}) {
  const parts = [`<span class="brand">earworm</span>`];
  if (folder) parts.push(`<span class="sep">·</span><span>${esc(folder)}</span>`);
  if (stage) parts.push(`<span class="sep">·</span>${spin ? '<span class="spin" id="spin">⠋</span>' : ""}<span class="stage" id="stage">${esc(stage)}</span>`);
  if (count) parts.push(`<span class="sep">·</span><span id="count">${esc(count)}</span>`);
  return `<header class="hd">${parts.join("")}<span class="right">${right}</span></header>`;
}

function rail({ tab = "folders", current: cur, focus, filter, sort = "name", kind = "all", folders = FOLDERS, list } = {}) {
  const tabs = `<div class="tabs"><span ${tab === "folders" ? 'aria-current="true"' : ""}>folders</span><span ${tab === "artists" ? 'aria-current="true"' : ""}><span class="k">b</span> artists</span></div>`;
  const q = filter ? `<div class="q on"><span class="k">/</span>${esc(filter)}<span class="caret">&nbsp;</span></div>` : `<div class="q"><span class="k">/</span> filter ${tab}</div>`;
  let body = list;
  if (!body) {
    body = folders.length ? `<ul>${folders.map(f => `<li ${f.name === cur ? 'aria-current="true"' : ""} class="${f.name === focus ? "focus" : ""}">
      <span class="ic ${f.kind}">${ICON[f.kind]}</span><span class="nm">${esc(f.name)}</span>
      <span class="ct ${f.missing ? "miss" : f.note === "syncing" ? "live" : ""}">${f.missing ? f.missing + "!" : f.note === "syncing" ? "⟳" : f.count}</span></li>`).join("")}</ul>`
      : `<p class="empty">No folders yet. A synced playlist lands here.</p>`;
  }
  const opts = tab === "folders" && folders.length ? `<div class="opts"><span><span class="k">s</span> <b>${sort}</b></span><span><span class="k">a</span> <b>${kind}</b></span></div>` : "";
  return `<nav class="rail" aria-label="Library">${tabs}${q}${opts}${body}
    <label class="themes"><span><span class="k">^t</span> theme</span><select id="theme">${THEMES.map(t => `<option value="${t}">${t}</option>`).join("")}</select></label></nav>`;
}

function footer({ tally = "", hints = [] } = {}) {
  return `<footer class="ft"><span class="tally" id="tally">${tally}</span><span class="hints">${hints.map(h => Array.isArray(h) ? `<span><span class="k">${esc(h[0])}</span> ${esc(h[1])}</span>` : h).join("")}</span></footer>`;
}

function tallyOf(states) {
  const t = {}; states.forEach(s => t[s] = (t[s] || 0) + 1);
  return Object.entries(t).map(([k, v]) => `<span class="${TONE[k] || "warn"}">${v} ${k}</span>`).join("");
}

function tile(t, st, { cur = false, mark = false, size = 16 } = {}) {
  const tone = TONE[st] || "warn";
  const cls = tone === "live" ? "live" : st === "queued" ? "queued" : st === "failed" ? "failed" : st === "missing" ? "missing" : st === "gone" ? "gone" : "";
  const inner = hasArt(st) ? `<img alt="" src="${pixelSleeve(t.album, size)}">` : st === "failed" || st === "missing" ? "!" : tone === "live" ? "" : pad(t.n);
  return `<div class="tile ${cur ? "cur" : ""}" data-i="${t.n - 1}"><div class="frame ${cls}">${inner}${mark ? '<span class="mark">✓</span>' : ""}</div>
    <div class="cap"><span class="n">${pad(t.n)}</span><span class="w ${tone}">${st}</span></div><div class="tt">${esc(t.title)}</div><div class="ar">${esc(t.artist)}</div></div>`;
}

function row(t, st, { cur = false } = {}) {
  const tone = TONE[st] || "warn";
  return `<div class="r ${cur ? "cur" : ""}" data-i="${t.n - 1}"><span class="n">${t.n}</span><span class="w ${tone}">${st}</span>
    <span class="t">${esc(t.title)} <span class="a">- ${esc(t.artist)}</span></span><span class="src">${tone === "live" || st === "queued" ? "" : esc(t.source)}</span></div>`;
}

// What the pane says under the status. Plain words, and what to do next where there is something to do.
const SAY = {
  ok: t => `Confirmed by ${t.source}.`, manual: () => "You chose this match.", "on disk": () => "Already in the folder. Tags read from the file.",
  guessed: () => "Read off the video title. Nothing confirmed it.", unsure: t => `${t.source} found a match that didn't fit.`,
  failed: () => "The download failed: this video is not available in your country. The log has the full error.",
  gone: () => "This video has left the playlist. The file stays until you delete it.",
  missing: () => "The manifest lists this file but it isn't in the folder. A sync fetches it again.",
  queued: () => "Waiting its turn.", fetching: () => "Downloading.", fetched: () => "Downloaded. Looking it up.", tagging: () => "Writing tags and art.",
};
function detail(t, st) {
  const tone = TONE[st] || "warn";
  const act = tone === "warn" && st !== "missing" ? `<button><span class="kc">T</span>search again</button><button><span class="kc">e</span>edit</button>`
    : st === "failed" ? `<button><span class="kc">r</span>retry failed</button><button><span class="kc">l</span>log</button>`
    : st === "gone" ? `<button><span class="kc">D</span>delete departed</button>`
    : st === "missing" ? `<button><span class="kc">S</span>sync</button>`
    : tone === "live" || st === "queued" ? "" : `<button><span class="kc">e</span>edit</button><button><span class="kc">c</span>cover</button><button><span class="kc">u</span>undo</button>`;
  return `${hasArt(st) ? `<img class="cover" src="${pixelSleeve(t.album, 16)}" alt="">` : `<div class="none">${st === "failed" ? "no sleeve, the download failed" : st === "missing" ? "file missing" : "no sleeve yet"}</div>`}
    <h2>${esc(t.title)} - ${esc(t.artist)}</h2><p class="say">${(SAY[st] || (() => ""))(t)}</p>
    <dl class="kv"><dt>status</dt><dd class="${tone}">${st}</dd><dt>artist</dt><dd>${esc(t.artist)}</dd><dt>title</dt><dd>${esc(t.title)}</dd>
    <dt>album</dt><dd>${esc(t.album)}</dd><dt>year</dt><dd>${t.year}</dd><dt>source</dt><dd>${esc(t.source)}</dd><dt>file</dt><dd>${esc(t.file)}</dd></dl>
    <div class="acts">${act}</div>`;
}

// Mount a screen: chrome from the options, the page's own HTML in the middle.
function mount({ head, railOpts, main, side, foot, noDetail = false }) {
  document.body.innerHTML = header(head) + `<main class="app ${noDetail ? "no-detail" : ""}">${rail(railOpts)}${main}${noDetail ? "" : `<aside class="detail" id="detail">${side || ""}</aside>`}</main>` + footer(foot);
  document.getElementById("theme").onchange = e => theme(e.target.value);
  theme(document.documentElement.dataset.theme);
  if (document.getElementById("spin")) { const s = "⠋⠙⠹⠸⠼⠴⠦⠧⠇⠏"; let i = 0; setInterval(() => { const el = document.getElementById("spin"); if (el) el.textContent = s[i++ % s.length]; }, 90); }
}
