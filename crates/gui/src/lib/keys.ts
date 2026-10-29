// The keys, routed in the order the terminal's `handle_key` routes them: the theme, then the console, then
// a question, then the quit question, then the screen. What a key does is the terminal's (docs/keys.md);
// `w` is the one this app adds. Which rows a marking key takes is core's, asked over IPC.
import { invoke } from "@tauri-apps/api/core";
import type { Kind, Sort, Status } from "./protocol";
import * as sc from "./screens.svelte";
import { screens } from "./screens.svelte";
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
  // Dismissed by any key, which goes no further; ^c is the exception, since it arrived unasked.
  if (screens.toast && !(e.ctrlKey && e.key === "c")) {
    e.preventDefault();
    return sc.endPrizes();
  }
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
  if (screens.discard) return discardKey(e);
  // The sheet swallows the key that closes it, or closing it would be a guess about what the key also did.
  if (screens.help && e.key !== "q") {
    e.preventDefault();
    screens.help = false;
    return;
  }
  const view = app.model.view;
  if (view === "Settings") return settingsKey(e);
  // Never where it would be a typed comma: a query box or the picker.
  // Not over the pick gate, which the worker is blocked on and which would stay drawn over the settings.
  if (e.key === "," && !ui.typing && !screens.artistTyping && view !== "Pick" && app.model.picking === null) {
    e.preventDefault();
    return sc.openSettings();
  }
  // Letters type in the picker, so its keys come before every guard on a focused input.
  if (view === "Pick") return pickerKey(e);
  if (view === "Artists" && screens.artistTyping) return artistQueryKey(e);
  if (ui.typing) return filterKey(e);
  if (e.ctrlKey && !CTRL.includes(e.key)) return;
  // A focused control keeps its own keys: arrows on the theme picker are not cursor moves, and space or
  // Enter on a button tabbed to presses that button.
  const target = e.target as HTMLElement | null;
  if (target instanceof HTMLSelectElement || target instanceof HTMLInputElement) return;
  if (target instanceof HTMLButtonElement && (e.key === " " || e.key === "Enter")) return;
  if (view === "Found") return foundKey(e);
  if (view === "Library" && app.model.picking === null) return libraryKey(e);
  if (view === "Artist") return pageKey(e);
  if (view === "Artists") return artistsKey(e);
  if (view === "Playlist") return playlistKey(e);
  if (app.model.picking !== null) return pickKey(e);
  if (view === "Tracks") return trackKey(e);
}

// The picker's own question: y or Enter drops the marks, Esc keeps picking, as on the quit question.
function discardKey(e: KeyboardEvent) {
  e.preventDefault();
  screens.discard = false;
  if (["y", "Y", "Enter"].includes(e.key)) sc.closePicker();
  else if (e.ctrlKey && e.key === "c") void quit();
}

const help = (e: KeyboardEvent) => (e.key === "h" || e.key === "?") && (screens.help = true);

// Letters type into the box, so help is F1, the marked view F2 and quitting ^c.
function pickerKey(e: KeyboardEvent) {
  const k = e.key;
  const done = () => e.preventDefault();
  if (e.ctrlKey && k === "c") return void quit();
  if (k === "F1") return done(), void (screens.help = true);
  if (k === "F2") return done(), sc.pickMarkedOnly();
  if (k === "Escape") return done(), sc.pickHasChanges() ? void (screens.discard = true) : sc.closePicker();
  if (k === "Enter") return done(), void sc.pickEnter();
  if (k === "Tab") return done(), sc.pickMark();
  if (k === "ArrowDown" || (e.ctrlKey && k === "n")) return done(), sc.pickMove(1);
  if (k === "ArrowUp" || (e.ctrlKey && k === "p")) return done(), sc.pickMove(-1);
  if (k === "PageDown") return done(), sc.pickMove(page());
  if (k === "PageUp") return done(), sc.pickMove(-page());
  if (e.ctrlKey && k === "a") return done(), sc.pickMarkAll();
}

function artistQueryKey(e: KeyboardEvent) {
  if (e.key === "Escape") {
    e.preventDefault();
    screens.artistTyping = false;
    screens.artistQuery = "";
  } else if (e.key === "Enter") {
    e.preventDefault();
    screens.artistTyping = false;
  } else if (e.key === "ArrowDown") sc.artistMove(1);
  else if (e.key === "ArrowUp") sc.artistMove(-1);
}

// The common movement keys for a list that is not the tracks, as one step function.
function listMove(e: KeyboardEvent, move: (by: number | "first" | "last") => void): boolean {
  const k = e.key;
  const by = k === "j" || k === "ArrowDown" ? 1 : k === "k" || k === "ArrowUp" ? -1 : k === "g" ? "first" : k === "G" ? "last"
    : k === "PageDown" || (e.ctrlKey && k === "d") ? page() : k === "PageUp" || (e.ctrlKey && k === "u") ? -page() : null;
  if (by === null) return false;
  e.preventDefault();
  move(by);
  return true;
}

// Enter opens the folder a result is in, on the track; Esc goes back to the library and keeps the query.
function foundKey(e: KeyboardEvent) {
  const k = e.key;
  if (listMove(e, sc.foundMove) || help(e)) return;
  if (k === "q" || (e.ctrlKey && k === "c")) return askQuit();
  if (k === "Escape") return void (app.model = { ...app.model, view: "Library" });
  if (k === "/") return e.preventDefault(), void (ui.typing = true);
  if (k === "l") return void (ui.logs = !ui.logs);
  if (k === "Tab" || k === " ") return e.preventDefault(), void sc.foundMark();
  if (app.model.busy || app.model.prompt) return;
  if (k === "Enter") sc.openFound();
  else if (k === "P") {
    const picks = sc.foundPicks();
    if (picks.length) void send({ AddToPlaylist: [picks, null] });
  }
}

function artistsKey(e: KeyboardEvent) {
  const k = e.key;
  if (listMove(e, sc.artistMove) || help(e)) return;
  if (k === "q" || (e.ctrlKey && k === "c")) return askQuit();
  if (k === "Escape") {
    if (screens.artistQuery) screens.artistQuery = "";
    else app.model = { ...app.model, view: "Library" };
    return;
  }
  if (k === "/") return e.preventDefault(), void (screens.artistTyping = true);
  if (k === "l") return void (ui.logs = !ui.logs);
  if (app.model.busy) return;
  const row = sc.artistSelected();
  if (k === "*" && row) void send({ Star: row.name });
  else if (k === "Enter") sc.openPage();
  else if (k === "o") void sc.artistFolders();
}

// Not gated on busy: switching lists needs no worker. Locked, Enter on a release or a top track is silent.
function pageKey(e: KeyboardEvent) {
  const k = e.key;
  if (listMove(e, sc.pageMove) || help(e)) return;
  if (k === "q" || (e.ctrlKey && k === "c")) return askQuit();
  if (k === "Escape") return app.model.searching ? void sc.stopSearch() : sc.leavePage();
  if (k === "l") return void (ui.logs = !ui.logs);
  if (k === "Tab") return e.preventDefault(), sc.pageSection();
  if (k === "w") return void (ui.mode = ui.mode === "tiles" ? "list" : "tiles");
  if (k === "a" && screens.section === "Releases") {
    screens.allKinds = !screens.allKinds;
    return;
  }
  if (app.model.busy || app.model.prompt) return;
  if (k === "o") void sc.artistFolders();
  else if (k === "Enter") sc.pageEnter();
}

// Its rows are ids resolved to files in other folders, so none of the track list's keys apply: Enter goes
// to the source, which is where a track's tags are edited.
function playlistKey(e: KeyboardEvent) {
  const k = e.key;
  const name = app.model.custom.name;
  if (listMove(e, sc.customMove) || help(e)) return;
  if (k === "q" || (e.ctrlKey && k === "c")) return askQuit();
  if (k === "Escape") return void (app.model = { ...app.model, view: "Library" });
  if (k === "l") return void (ui.logs = !ui.logs);
  if (app.model.busy || app.model.prompt) return;
  const row = sc.customRow();
  if ((k === "J" || k === "K") && row) void send({ MoveEntry: { name, id: row.id, up: k === "K" } });
  else if ((k === "x" || k === "Delete") && row) void send({ RemoveEntry: { name, id: row.id } });
  else if (k === "P") sc.reopenPicker();
  else if (k === "p" && app.model.playback.player === "Running") void send({ PlayPlaylist: name });
  else if (k === "Enter" && row) {
    if (!row.path) return say(`${row.file} is not on disk  ·  x takes it off the list`);
    sc.screens.fromFound = null;
    void send({ Open: [row.path.slice(0, row.path.lastIndexOf("/")), row.file] });
  }
}

function settingsKey(e: KeyboardEvent) {
  const k = e.key;
  const rows = app.model.settings.rows.length;
  if (help(e)) return;
  if (k === "q" || (e.ctrlKey && k === "c")) return askQuit();
  if (k === "Escape" || k === ",") return e.preventDefault(), sc.leaveSettings();
  if (k === "l") return void (ui.logs = !ui.logs);
  if (listMove(e, (by) => {
    const at = screens.settingsAt;
    screens.settingsAt = by === "first" ? 0 : by === "last" ? rows - 1 : Math.min(rows - 1, Math.max(0, at + by));
  })) return;
  if (k === "Enter" || k === " ") {
    e.preventDefault();
    void sc.changeSetting(walkTheme);
  }
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
  ui.byArtist = null;
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

// The marked tracks, or the one under the cursor, as folder and filename: the worker works out the id.
function picks(): [string, string][] {
  return targets()
    .map((i) => app.model.tracks.find((t) => t.index === i)?.path)
    .filter((p): p is string => !!p)
    .map((p) => [p.slice(0, p.lastIndexOf("/")), p.slice(p.lastIndexOf("/") + 1)]);
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
  if (help(e)) return;
  if (k === "Escape") return escapeTracks();
  if (k === "/") {
    e.preventDefault();
    ui.typing = true;
  } else if (k === "f") app.model = { ...m, follow: !m.follow };
  else if (k === "v") void review();
  else if (k === "n" || k === "N") attention(k === "n");
  else if (k === "l") ui.logs = !ui.logs;
  else if (k === "L" && ui.logs) ui.logsTall = !ui.logsTall;
  else if ((k === "J" || k === "K") && ui.logs) document.querySelector(".log")?.scrollBy({ top: k === "J" ? 48 : -48 });
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
  else if (k === "P") void send({ AddToPlaylist: [picks(), null] });
  else if (k === "*" && t) {
    const lead = await invoke<string>("lead_artist", { artist: t.artist });
    if (!lead) say("this track has no artist to star  ·  e sets one");
    else void send({ Star: lead });
  }
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
  // A folder opened from a search result goes back to the results.
  const query = sc.screens.fromFound;
  sc.screens.fromFound = null;
  if (query?.trim()) ui.filter = query;
  const back = query?.trim() ? "Found" : "Library";
  app.model = { ...m, view: back };
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

function customKey(k: string, name: string, playing: boolean) {
  if (k === "Enter") void send({ OpenPlaylist: name });
  else if (k === "e") void send({ RenamePlaylist: name });
  else if (k === "D") void send({ DeletePlaylist: name });
  else if (k === "E") void send({ Export: { List: name } });
  else if (k === "p" && playing) void send({ PlayPlaylist: name });
  else if (k === "S") say("a custom playlist is built here  ·  there is nothing upstream to sync");
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
  else if (help(e)) return;
  else if (k === "Escape") {
    if (ui.filter || ui.only || ui.byArtist) clearFilter();
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
  else if (k === "N") void send("NewPlaylist");
  else if (k === "t") void sc.findTracks();
  else if (k === "b") sc.openArtists();
  else if (k === " " && m.playback.player === "Running") void send("Toggle");
  else if (!shelf) return;
  // A list's row carries its file as the path, so it goes by name and never to a folder command.
  else if (shelf.kind === "Custom") customKey(k, shelf.name, m.playback.player === "Running");
  else if (k === "E") void send({ Export: { Folder: shelf.path } });
  else if (k === "Enter") {
    sc.screens.fromFound = null;
    void send({ Open: [shelf.path, null] });
  }
  else if (k === "S") void send({ Sync: shelf.path });
  else if (k === "O") void send({ Reveal: shelf.path });
  else if (k === "e") void send({ Rename: shelf.path });
  else if (k === "D") void send({ Remove: shelf.path });
  else if (k === "p" && m.playback.player === "Running") void send({ Play: shelf.path });
}
