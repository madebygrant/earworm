// Shared fixture and a fake run, so the three directions show identical content.
const FOLDERS = [
  { name: "Krautrock", kind: "playlist", count: 18, note: "syncing", open: true },
  { name: "Zuckerzeit", kind: "album", artist: "Cluster", count: 10, note: "synced yesterday" },
  { name: "Phaedra", kind: "album", artist: "Tangerine Dream", count: 4, note: "synced 3 weeks ago" },
  { name: "Ambient 1975", kind: "playlist", count: 31, note: "synced 2 days ago", missing: 3 },
  { name: "Chill Evenings", kind: "playlist", count: 40, note: "synced last month" },
  { name: "Rips from CDs", kind: "local", count: 112, note: "not downloaded by earworm" },
  { name: "Neu! 75", kind: "album", artist: "Neu!", count: 6, note: "synced in March" },
];

const TRACKS = [
  ["Hallogallo", "Neu!", "Neu!", 1972, "on disk", "on disk"],
  ["Vitamin C", "Can", "Ege Bamyasi", 1972, "on disk", "on disk"],
  ["Autobahn", "Kraftwerk", "Autobahn", 1974, "ok", "AcoustID"],
  ["Watussi", "Harmonia", "Musik von Harmonia", 1974, "ok", "Deezer"],
  ["Sowiesoso", "Cluster", "Sowiesoso", 1976, "ok", "AcoustID"],
  ["It's a Rainy Day, Sunshine Girl", "Faust", "Faust So Far", 1972, "manual", "your answer"],
  ["Traummaschine", "Ash Ra Tempel", "Join Inn", 1973, "guessed", "video title"],
  ["Archangels Thunderbird", "Amon Düül II", "Yeti", 1970, "ok", "AcoustID"],
  ["Aguirre", "Popol Vuh", "Aguirre", 1975, "unsure", "Deezer"],
  ["Phaedra", "Tangerine Dream", "Phaedra", 1974, "ok", "Apple Music"],
  ["Rheinita", "La Düsseldorf", "La Düsseldorf", 1976, "ok", "AcoustID"],
  ["Karussell", "Michael Rother", "Flammende Herzen", 1977, "ok", "Deezer"],
  ["Electric Junk", "Guru Guru", "Känguru", 1972, "failed", "yt-dlp"],
  ["Laila", "Agitation Free", "Malesch", 1972, "ok", "AcoustID"],
  ["Radio Marrakesch", "Embryo", "Embryo's Reise", 1979, "guessed", "video title"],
  ["Bayreuth Return", "Klaus Schulze", "Timewind", 1975, "ok", "AcoustID"],
  ["Auf dem Schwarzen Kanal", "Conrad Schnitzler", "Rot", 1973, "gone", "left the playlist"],
  ["Regenmacher", "Roedelius", "Durch die Wüste", 1978, "gone", "left the playlist"],
].map(([title, artist, album, year, final, source], i) => ({
  n: i + 1, title, artist, album, year, final, source,
  file: `music/Krautrock/${String(i + 1).padStart(2, "0")} - ${artist} - ${title}.opus`,
}));

// How each word is read: confirmed, look at it, untouched, failed, in flight.
const TONE = {
  ok: "good", manual: "good", "on disk": "good",
  guessed: "warn", unsure: "warn", "no match": "warn",
  failed: "error",
  gone: "muted", skipped: "muted", queued: "muted",
  fetching: "live", fetched: "live", tagging: "live",
};

const LOG = [
  ["", "[youtube:tab] Extracting URL: https://youtube.com/playlist?list=PLk7r…"],
  ["", "[youtube:tab] Krautrock: Downloading 18 items"],
  ["", "[download] Destination: .tmp/Autobahn.webm"],
  ["warn", "WARNING: [youtube] 9Hw1… nsig extraction failed: some formats may be missing"],
  ["", "[ExtractAudio] Destination: .tmp/Autobahn.opus"],
  ["", "[download] 100% of 21.42MiB in 00:00:06 at 3.21MiB/s"],
  ["error", "ERROR: [youtube] q3Fz…: Video unavailable. This video is not available in your country"],
];

function hash(s) { let h = 2166136261; for (const c of s) h = Math.imul(h ^ c.charCodeAt(0), 16777619); return h >>> 0; }

// Stand-in cover art: a seeded abstract sleeve, distinct per album.
function sleeve(name) {
  const h = hash(name), a = h % 360, b = (a + 40 + (h >> 9) % 140) % 360;
  const x = 20 + (h >> 4) % 60, y = 20 + (h >> 12) % 60;
  const kinds = [
    `radial-gradient(circle at ${x}% ${y}%, hsl(${b} 70% 62%) 0 22%, transparent 23%), linear-gradient(${h % 180}deg, hsl(${a} 45% 28%), hsl(${a} 55% 16%))`,
    `repeating-linear-gradient(${h % 90}deg, hsl(${a} 50% 30%) 0 8%, hsl(${b} 60% 55%) 8% 12%), hsl(${a} 40% 20%)`,
    `conic-gradient(from ${h % 360}deg at ${x}% ${y}%, hsl(${a} 60% 50%), hsl(${b} 60% 30%), hsl(${a} 30% 15%), hsl(${a} 60% 50%))`,
    `linear-gradient(180deg, transparent 0 ${y}%, hsl(${b} 65% 58%) ${y}% ${y + 6}%, transparent ${y + 6}%), linear-gradient(${h % 180}deg, hsl(${a} 35% 22%), hsl(${b} 35% 12%))`,
  ];
  return kinds[h % kinds.length];
}
function hue(name) { return hash(name) % 360; }

// Walks each new track through the run's stages; the on-disk and gone rows are settled from the start.
function startRun(onUpdate, { step = 420 } = {}) {
  const reduce = matchMedia("(prefers-reduced-motion: reduce)").matches;
  const state = TRACKS.map(t => (t.final === "on disk" || t.final === "gone") ? t.final : "queued");
  const work = TRACKS.filter(t => state[t.n - 1] === "queued").map(t => t.n - 1);
  const stages = ["fetching", "fetching", "fetched", "tagging"];
  if (reduce) { TRACKS.forEach((t, i) => state[i] = t.final); onUpdate(state, work.length, work.length, -1); return; }
  let tick = 0;
  const timer = setInterval(() => {
    const w = Math.floor(tick / (stages.length + 1)), s = tick % (stages.length + 1);
    if (w >= work.length) {
      onUpdate(state, work.length, work.length, -1);
      if (tick > work.length * (stages.length + 1) + 12) { clearInterval(timer); setTimeout(() => startRun(onUpdate, { step }), 0); }
      tick++; return;
    }
    const i = work[w];
    state[i] = s < stages.length ? stages[s] : TRACKS[i].final;
    onUpdate(state, w + (s === stages.length ? 1 : 0), work.length, i);
    tick++;
  }, step);
  onUpdate(state, 0, work.length, -1);
}

function esc(s) { return String(s).replace(/[&<>"]/g, c => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;" }[c])); }

// A cover drawn as a coarse pixel grid; size 4, 8 or 16 gives the same picture at rising detail.
const cache = {};
function pixelSleeve(name, size) {
  const key = name + size; if (cache[key]) return cache[key];
  const c = document.createElement("canvas"); c.width = c.height = size;
  const g = c.getContext("2d"), h = hash(name), a = h % 360, b = (a + 60 + (h >> 7) % 120) % 360, step = 16 / size;
  for (let y = 0; y < size; y++) for (let x = 0; x < size; x++) {
    const X = x * step + step / 2, Y = y * step + step / 2;
    const fx = .25 + (h & 15) / 30, fy = .25 + (h >> 4 & 15) / 30, cx = 8 + (h >> 8 & 7) - 4, cy = 8 + (h >> 11 & 7) - 4;
    const pat = h >> 14 & 3, noise = (hash(name + Math.floor(X) + "," + Math.floor(Y)) & 3) / 6;
    const v = pat === 0 ? Math.sin(X * fx) + Math.cos(Y * fy) + noise
      : pat === 1 ? Math.sin(Math.hypot(X - cx, Y - cy) * fx * 1.6) * 1.4 + noise
      : pat === 2 ? Math.sin((X + Y) * fx) * 1.3 + Math.cos(Y * fy * 2) * .5 + noise
      : (Math.abs(X - cx) + Math.abs(Y - cy) < 5 + (h & 3) ? 1.4 : -.4) + Math.sin(Y * fy) * .5 + noise;
    g.fillStyle = v > .9 ? `hsl(${b} 60% 62%)` : v > 0 ? `hsl(${a} 45% 34%)` : `hsl(${a} 40% 16%)`;
    g.fillRect(x, y, 1, 1);
  }
  return cache[key] = c.toDataURL();
}
