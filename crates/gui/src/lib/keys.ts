// The keys, routed in the order the terminal's `handle_key` routes them: the theme, then the console, then
// a question, then the quit question, then the screen. What a key does is the terminal's (docs/keys.md);
// `w` is the one this app adds. Which rows a marking key takes is core's, asked over IPC.
import { invoke } from "@tauri-apps/api/core";
import type { Kind, Sort, Status } from "./protocol";
import { answerPick, app, walkTheme } from "./store.svelte";
import { canBrowse, canCommand, quit, say, send, ui, working } from "./ui.svelte";

let armed = 0;

/** Set by the app shell: whether the cheat console is open, and how to open it. */
export const consoleHooks = { open: () => {}, isOpen: () => false };

// The Ctrl chords a screen answers: paging, and ^c to quit. Any other Ctrl+letter is not the bare letter.
const CTRL = ["c", "d", "u"];

export async function handleKey(e: KeyboardEvent) {
  // A modifier on its own is not a keypress anyone means: it would dismiss the quit question before the Y
  // of Shift+Y, and disarm `^z` when Ctrl is pressed again for the `^x`.
  if (["Shift", "Control", "Alt", "Meta"].includes(e.key)) return;
  if (e.ctrlKey && e.key === "t") {
    e.preventDefault();
    return void walkTheme();
  }
  // ⌘c copies and ⌘r is not retry: the platform's shortcuts are left to the platform.
  if (e.metaKey || e.altKey) return;
  if (consoleHooks.isOpen()) return;
  // `^z` then `^x`: two keys, because each alone is a quit reflex, and any key in between disarms it.
  const wasArmed = armed;
  armed = 0;
  if (e.ctrlKey && e.key === "z") return void (armed = Date.now());
  if (e.ctrlKey && e.key === "x") {
    if (Date.now() - wasArmed < 1500) {
      e.preventDefault();
      consoleHooks.open();
    }
    return;
  }
  // The dialog and the first screen take their own keys while the worker is asking.
  if (app.model.prompt) return;
  if (ui.quitting) return quitKey(e);
  if (ui.typing) return filterKey(e);
  if (e.ctrlKey && !CTRL.includes(e.key)) return;
  // A focused control keeps its own keys: arrows on the theme picker are not cursor moves, and space or
  // Enter on a button tabbed to presses that button.
  const target = e.target as HTMLElement | null;
  if (target instanceof HTMLSelectElement || target instanceof HTMLInputElement) return;
  if (target instanceof HTMLButtonElement && (e.key === " " || e.key === "Enter")) return;
  if (app.model.picking !== null) return pickKey(e);
  if (app.model.view === "Library") return libraryKey(e);
  if (app.model.view === "Tracks") return trackKey(e);
}

// Only a named key confirms, so an unrecognised one can never be an accidental yes; `qq` quits unread.
function quitKey(e: KeyboardEvent) {
  e.preventDefault();
  ui.quitting = false;
  if (["y", "Y", "q", "Enter"].includes(e.key) || (e.ctrlKey && e.key === "c")) void quit();
}

export function askQuit() {
  if (working()) ui.quitting = true;
  else void quit();
}

function filterKey(e: KeyboardEvent) {
  if (e.key === "Escape") {
    e.preventDefault();
    clearFilter();
  } else if (e.key === "Enter") {
    e.preventDefault();
    ui.typing = false;
  }
}

/** Every narrowing goes together: they are one band on screen saying the list is not all of it. */
export function clearFilter() {
  ui.filter = "";
  ui.typing = false;
  ui.only = null;
  app.model = { ...app.model, review: false };
}

// Track indices of the rows on screen, in order.
function order(): number[] {
  return ui.rows.map((at) => app.model.tracks[at]?.index).filter((i) => i !== undefined);
}

// How many tiles a row of the sheet holds, read off the grid the browser laid out.
function columns(): number {
  const sheet = document.querySelector(".view .sheet");
  if (!sheet || ui.mode !== "tiles") return 1;
  return getComputedStyle(sheet).gridTemplateColumns.split(" ").length;
}

// A key that moves the cursor ends following, or the next progress message drags it straight back. Said
// while a run is going, since a cursor that stops being dragged otherwise looks like a broken key.
function stopFollowing() {
  if (!app.model.follow) return;
  app.model = { ...app.model, follow: false };
  if (working()) say("following off  ·  f follows again");
}

function move(by: number | "first" | "last") {
  const rows = order();
  if (!rows.length) return;
  stopFollowing();
  const at = Math.max(0, rows.indexOf(app.model.cursor ?? rows[0]));
  const next = by === "first" ? 0 : by === "last" ? rows.length - 1 : Math.min(rows.length - 1, Math.max(0, at + by));
  app.model = { ...app.model, cursor: rows[next] };
}

function page(): number {
  // One selector list would match `.view` first, which is the ancestor and never the part that scrolls.
  const view = document.querySelector(".view .scroll") ?? document.querySelector(".view");
  const tile = document.querySelector(".view .tile, .view .r");
  if (!view || !tile) return 10;
  const rows = Math.max(1, Math.floor(view.clientHeight / tile.getBoundingClientRect().height));
  return rows * columns();
}

function moveKey(e: KeyboardEvent): boolean {
  const k = e.key;
  const by =
    k === "j" ? 1 : k === "k" ? -1 : k === "ArrowRight" ? 1 : k === "ArrowLeft" ? -1
    : k === "ArrowDown" ? columns() : k === "ArrowUp" ? -columns()
    : k === "PageDown" || (e.ctrlKey && k === "d") ? page() : k === "PageUp" || (e.ctrlKey && k === "u") ? -page()
    : k === "g" ? "first" : k === "G" ? "last" : null;
  if (by === null) return false;
  e.preventDefault();
  move(by);
  return true;
}

async function take(take: "Showing" | { Status: Status } | { ArtistOf: number }): Promise<number[] | null> {
  const m = app.model;
  return invoke<number[] | null>("take", { tracks: m.tracks, filter: ui.filter, review: m.review, take });
}

// Marks the group, or takes it off when it is all marked already.
async function flip(group: number[]) {
  app.model = { ...app.model, marked: await invoke<number[]>("flip", { marked: app.model.marked, group }) };
}

function cursorTrack() {
  const m = app.model;
  // Filtered out means not selected, or `e` would edit a track that is not on screen.
  return order().includes(m.cursor ?? -1) ? m.tracks.find((t) => t.index === m.cursor) : undefined;
}

async function markKey(e: KeyboardEvent): Promise<boolean> {
  const picking = app.model.picking !== null;
  if (!(e.key === " " || e.key === "m" || (picking && (e.key === "a" || e.key === "i")))) return false;
  // Before the IPC round trip below, or the browser has already scrolled the page on the space.
  e.preventDefault();
  const t = cursorTrack();
  if (e.key === " ") {
    if (t) await flip([t.index]);
  } else if (e.key === "m") {
    if (t) await flip((await take({ Status: t.status })) ?? []);
  } else if (e.key === "a" && app.model.picking !== null) {
    await flip((await take("Showing")) ?? []);
  } else if (e.key === "i" && app.model.picking !== null) {
    const shown = (await take("Showing")) ?? [];
    app.model = { ...app.model, marked: await invoke<number[]>("invert", { marked: app.model.marked, tracks: shown }) };
  }
  return true;
}

async function pickKey(e: KeyboardEvent) {
  if (moveKey(e) || (await markKey(e))) return;
  if (e.key === "Enter") {
    // An empty selection downloads nothing, which is what Esc is for and not what Enter looks like.
    if (!app.model.marked.length) return say("nothing selected  ·  a marks all  ·  esc downloads none");
    void answerPick([...app.model.marked].sort((a, b) => a - b));
  } else if (e.key === "Escape") {
    if (ui.filter) clearFilter();
    else void answerPick([]);
  } else if (e.key === "/") {
    e.preventDefault();
    ui.typing = true;
  } else if (e.key === "l") ui.logs = !ui.logs;
  else if (e.key === "q") askQuit();
}

function targets(): number[] {
  const m = app.model;
  if (m.marked.length) return [...m.marked].sort((a, b) => a - b);
  const t = cursorTrack();
  return t ? [t.index] : [];
}

/** The keys the detail pane's buttons stand for, so a click and a key are one action. */
export function press(key: string) {
  void handleKey(new KeyboardEvent("keydown", { key }));
}

async function trackKey(e: KeyboardEvent) {
  if (moveKey(e) || (await markKey(e))) return;
  const m = app.model;
  const t = cursorTrack();
  const k = e.key;
  if (k === "q" || (e.ctrlKey && k === "c")) return askQuit();
  if (k === "Escape") return escapeTracks();
  if (k === "/") {
    e.preventDefault();
    ui.typing = true;
  } else if (k === "f") app.model = { ...m, follow: !m.follow };
  else if (k === "v") void review();
  else if (k === "n" || k === "N") attention(k === "n");
  else if (k === "l") ui.logs = !ui.logs;
  else if (k === "L" && ui.logs) ui.logsTall = !ui.logsTall;
  else if (k === "w") ui.mode = ui.mode === "tiles" ? "list" : "tiles";
  else if (k === "M") {
    const got = t ? await take({ ArtistOf: t.index }) : null;
    if (got === null) say("no artist on this track yet");
    else await flip(got);
  } else if (!canCommand()) return;
  else if (k === "u" && !e.ctrlKey && m.undoable) void send("Undo");
  else if (k === "e" && t) void send({ Edit: t.index });
  else if (k === "c" && t) void send({ Cover: t.index });
  else if (k === "T" && t) void send({ Search: t.index });
  else if (k === "r") void send("Retry");
  else if (k === "S") void send("SyncOne");
  else if (k === "D" && m.tracks.some((x) => x.status === "Gone" && x.departure_proven)) void send("Purge");
  else if (k === "p" && m.playback.player === "Running" && m.folder) void send({ Play: m.folder });
  else if (k === "s") void send({ Swap: targets() });
  else if (k === "A") void send({ Artist: targets() });
}

// The filter first, then the marks, then the library, and never out of the app.
function escapeTracks() {
  const m = app.model;
  if (ui.filter || m.review) return clearFilter();
  if (m.marked.length) {
    const n = m.marked.length;
    app.model = { ...m, marked: [] };
    return say(`unmarked ${n} ${n === 1 ? "track" : "tracks"}`);
  }
  if (working()) return say("still working  ·  q quits");
  if (!m.library.length) return say("nothing behind this  ·  q quits");
  app.model = { ...m, view: "Library" };
}

async function review() {
  const m = app.model;
  const on = !m.review;
  if (on && !m.tracks.some((t) => m.statuses.find((w) => w.status === t.status)?.look)) {
    return say("nothing here needs a look");
  }
  app.model = { ...m, review: on };
}

// Next or previous track nothing confirmed, through the rows on screen, stopping at the ends.
function attention(forward: boolean) {
  const m = app.model;
  const rows = order();
  const looks = (i: number) => {
    const t = m.tracks.find((x) => x.index === i);
    return !!t && !!m.statuses.find((w) => w.status === t.status)?.look;
  };
  if (!rows.some(looks)) return say("nothing here needs a look");
  const at = rows.indexOf(m.cursor ?? -1);
  const next = forward ? rows.slice(at + 1).find(looks) : rows.slice(0, Math.max(0, at)).reverse().find(looks);
  if (next === undefined) return say(`that is the ${forward ? "last" : "first"} one to look at`);
  stopFollowing();
  app.model = { ...app.model, cursor: next };
}

function shelfOrder(): string[] {
  return ui.shelves.map((at) => app.model.library[at]?.path).filter((p) => p !== undefined);
}

async function libraryKey(e: KeyboardEvent) {
  const m = app.model;
  const shelf = m.library.find((s) => s.path === m.shelf);
  const k = e.key;
  const rows = shelfOrder();
  const at = Math.max(0, rows.indexOf(m.shelf ?? ""));
  const go = (i: number) => rows.length && (app.model = { ...app.model, shelf: rows[Math.min(rows.length - 1, Math.max(0, i))] });
  if (k === "j" || k === "ArrowDown") go(at + 1);
  else if (k === "k" || k === "ArrowUp") go(at - 1);
  else if (k === "g") go(0);
  else if (k === "G") go(rows.length - 1);
  else if (k === "PageDown" || (e.ctrlKey && k === "d")) go(at + 10);
  else if (k === "PageUp" || (e.ctrlKey && k === "u")) go(at - 10);
  else if (k === "q" || (e.ctrlKey && k === "c")) askQuit();
  else if (k === "Escape") {
    if (ui.filter || ui.only) clearFilter();
    else say("this is the top  ·  q quits");
  } else if (k === "/") {
    e.preventDefault();
    ui.typing = true;
  } else if (k === "s") {
    const [sort, label] = await invoke<[Sort, string]>("next_sort", { sort: ui.sort });
    ui.sort = sort;
    say(`sorted by ${label}`);
  } else if (k === "a") {
    const [only, label] = await invoke<[Kind | null, string]>("next_kind", { only: ui.only });
    ui.only = only;
    say(`showing ${label}`);
  } else if (k === "w") ui.mode = ui.mode === "tiles" ? "list" : "tiles";
  else if (k === "l") ui.logs = !ui.logs;
  else if (!canBrowse()) return;
  else if (k === "R") void send("ResyncAll");
  else if (k === "n") void send("Url");
  else if (k === " " && m.playback.player === "Running") void send("Toggle");
  else if (!shelf) return;
  else if (shelf.kind === "Custom") say("a custom playlist has no screen in the desktop app yet");
  else if (k === "Enter") void send({ Open: [shelf.path, null] });
  else if (k === "S") void send({ Sync: shelf.path });
  else if (k === "O") void send({ Reveal: shelf.path });
  else if (k === "e") void send({ Rename: shelf.path });
  else if (k === "D") void send({ Remove: shelf.path });
  else if (k === "p" && m.playback.player === "Running") void send({ Play: shelf.path });
}
