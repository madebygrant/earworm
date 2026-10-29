// The screens past the run and the library. What each list shows is core's, asked over IPC; only the
// cursors and which screen is up are kept here, as the terminal's `App` keeps them.
import { invoke } from "@tauri-apps/api/core";
import type { Cmd, Pick } from "./commands";
import type { AlbumRow, Hit, PickRow, Prize, RelatedRow, Shelf, TopRow } from "./protocol";
import { firstScreen, type View } from "./reducer";
import { app, flash } from "./store.svelte";
import { say, send, ui } from "./ui.svelte";

export type Section = "Releases" | "Top" | "Related";

export const screens = $state({
  help: false,
  hits: [] as Hit[],
  foundTotal: 0,
  // The result under the cursor as the folder and file it names, never a row number.
  found: null as Pick | null,
  // Enter opened a result's folder, so Esc from there comes back to the results.
  fromFound: false,
  // `t` is waiting on its first answer, which decides whether the screen opens at all.
  finding: false,
  pick: {
    name: "",
    // Opened from the list's own screen, so its tracks start marked and Enter sends the difference.
    existing: false,
    query: "",
    rows: [] as PickRow[],
    total: 0,
    cursor: 0,
    marks: [] as Pick[],
    held: [] as Pick[],
    onlyMarked: false,
    // What Enter would send, so the bar can say it, and the tracks it would take off.
    add: 0,
    remove: 0,
    removing: [] as Pick[],
  },
  // Esc on the picker would lose marks, so it asks first.
  discard: false,
  artistQuery: "",
  artistTyping: false,
  artistRows: [] as number[],
  // Each artist row's new releases by title, matched as core matches an artist.
  artistNew: [] as string[][],
  artist: null as string | null,
  section: "Releases" as Section,
  releaseRows: [] as number[],
  release: null as number | null,
  topAt: 0,
  relatedAt: 0,
  allKinds: false,
  // Pages left for a related artist's, so Esc walks back through them.
  back: [] as string[],
  topAsked: false,
  relatedAsked: false,
  settingsAt: 0,
  settingsBack: "Library" as View,
  // The achievements on show and when they went up, then those waiting for a calm screen.
  toast: null as { prizes: Prize[]; at: number } | null,
  waiting: [] as Prize[],
});

const m = () => app.model;
const go = (view: View) => (app.model = { ...app.model, view });

// Clamped at both ends, never wrapping, like every other list.
function step<T>(rows: T[], at: number, by: number | "first" | "last"): T | undefined {
  if (!rows.length) return undefined;
  const i = by === "first" ? 0 : by === "last" ? rows.length - 1 : Math.min(rows.length - 1, Math.max(0, at + by));
  return rows[i];
}

// ---- search (`t`)

export function foundAt(): [Shelf, [string, boolean]] | null {
  if (!screens.found) return null;
  const [folder, file] = screens.found;
  const shelf = m().library.find((s) => s.path === folder);
  const entry = shelf?.files.find(([name]) => name === file);
  return shelf && entry ? [shelf, entry] : null;
}

const pairOf = (h: Hit): Pick | null => {
  const shelf = m().library[h.shelf];
  const file = shelf?.files[h.file];
  return shelf && file ? [shelf.path, file[0]] : null;
};

// Asks core again, and keeps the cursor on its result or, when that one went, the first: a new query is a new question.
export async function searchRefresh() {
  const { hits, total } = await invoke<{ hits: Hit[]; total: number }>("search", { library: m().library, query: ui.filter, only: ui.only });
  screens.hits = hits;
  screens.foundTotal = total;
  const pairs = hits.map(pairOf);
  const at = screens.found;
  if (!at || !pairs.some((p) => p && p[0] === at[0] && p[1] === at[1])) screens.found = pairs[0] ?? null;
  if (screens.finding) {
    screens.finding = false;
    if (!screens.found) {
      go("Library");
      say(`no track matches ${ui.filter}`);
    }
  }
}

// An empty query opens the box rather than listing everything, and a query that finds nothing stays put.
export async function findTracks() {
  app.model = { ...app.model, foundMarks: [] };
  if (!ui.filter.trim()) {
    ui.typing = true;
    screens.found = null;
    screens.hits = [];
    return go("Found");
  }
  // The screen's own refresh answers, once: a query that finds nothing goes back to the library.
  screens.found = null;
  screens.finding = true;
  go("Found");
}

export function foundMove(by: number | "first" | "last") {
  const pairs = screens.hits.map(pairOf).filter((p): p is Pick => p !== null);
  const at = screens.found ? pairs.findIndex((p) => p[0] === screens.found![0] && p[1] === screens.found![1]) : 0;
  screens.found = step(pairs, Math.max(0, at), by) ?? null;
}

// A file that is gone is not markable: the add would only fail on it.
export async function foundMark() {
  const at = foundAt();
  if (!at) return;
  const [shelf, [name, here]] = at;
  if (!here) return say("that file is not on disk");
  const marks = await invoke<Pick[]>("pick_toggle", { marks: m().foundMarks, items: [[shelf.path, name]] });
  app.model = { ...app.model, foundMarks: marks };
}

export function foundPicks(): Pick[] {
  if (m().foundMarks.length) return m().foundMarks;
  const at = foundAt();
  return at ? [[at[0].path, at[1][0]]] : [];
}

export function openFound() {
  const at = foundAt();
  if (!at) return;
  screens.fromFound = true;
  void send({ Open: [at[0].path, at[1][0]] });
}

// ---- the track picker (`N`, and `P` on a list)

// The picker for a list just named: nothing marked, Enter adds.
export function openPicker(name: string) {
  screens.pick = { ...screens.pick, name, existing: false, query: "", cursor: 0, marks: [], held: [], onlyMarked: false };
  ui.typing = false;
  go("Pick");
}

// The picker over an open list: its tracks start marked, so unmarking one takes it off.
export function reopenPicker() {
  const held: Pick[] = m().custom.rows
    .filter((r) => r.path)
    .map((r) => [r.path!.slice(0, r.path!.lastIndexOf("/")), r.path!.slice(r.path!.lastIndexOf("/") + 1)]);
  screens.pick = { ...screens.pick, name: m().custom.name, existing: true, query: "", cursor: 0, marks: [...held], held, onlyMarked: false };
  go("Pick");
}

// Recomputed on a query change, a mark under F2, or a library refresh, never per frame.
export async function pickRefresh() {
  const p = screens.pick;
  const { rows, total } = await invoke<{ rows: PickRow[]; total: number }>("picker_rows", {
    library: m().library,
    query: p.query,
    marks: p.marks,
    held: p.held,
    onlyMarked: p.onlyMarked,
  });
  screens.pick = { ...screens.pick, rows, total, cursor: Math.min(screens.pick.cursor, Math.max(0, rows.length - 1)) };
  const [add, remove] = await pickChanges();
  screens.pick = { ...screens.pick, add: add.length, remove: remove.length, removing: remove };
}

export function pickQuery(query: string) {
  screens.pick = { ...screens.pick, query, cursor: 0 };
  void pickRefresh();
}

export function pickMove(by: number) {
  const last = Math.max(0, screens.pick.rows.length - 1);
  screens.pick = { ...screens.pick, cursor: Math.min(last, Math.max(0, screens.pick.cursor + by)) };
}

async function pickToggle(items: Pick[]) {
  const marks = await invoke<Pick[]>("pick_toggle", { marks: screens.pick.marks, items });
  screens.pick = { ...screens.pick, marks };
  await pickRefresh();
}

export function pickMark() {
  const row = screens.pick.rows[screens.pick.cursor];
  if (row) void pickToggle([[row.folder, row.name]]);
}

export function pickMarkAll() {
  void pickToggle(screens.pick.rows.map((r) => [r.folder, r.name]));
}

export function pickMarkedOnly() {
  screens.pick = { ...screens.pick, onlyMarked: !screens.pick.onlyMarked, cursor: 0 };
  void pickRefresh();
}

function pickChanges(): Promise<[Pick[], Pick[]]> {
  const p = screens.pick;
  const row = p.rows[p.cursor];
  return invoke<[Pick[], Pick[]]>("pick_changes", {
    under: row ? [row.folder, row.name] : null,
    marks: p.marks,
    held: p.existing ? p.held : null,
  });
}

export async function pickEnter() {
  const [add, remove] = await pickChanges();
  const p = screens.pick;
  if (!add.length && !remove.length) {
    return say(p.existing ? "no changes  ·  tab marks or unmarks a track" : "nothing to add  ·  type to find tracks, esc keeps the list empty");
  }
  // A list that was already open goes back to its own screen, where the change lands.
  go(p.existing ? "Playlist" : "Library");
  void send(p.existing ? { EditPlaylist: { name: p.name, add, remove } } : { AddToPlaylist: [add, p.name] });
}

export function pickHasChanges(): boolean {
  return screens.pick.existing ? screens.pick.add + screens.pick.remove > 0 : screens.pick.marks.length > 0;
}

export function closePicker() {
  screens.discard = false;
  if (screens.pick.existing) {
    go("Playlist");
    say("no changes");
  } else {
    go("Library");
    say(`kept ${screens.pick.name} empty  ·  t finds tracks and P adds them`);
  }
}

// ---- a custom list's screen

export function customMove(by: number | "first" | "last") {
  const rows = m().custom.rows;
  const at = Math.max(0, rows.findIndex((r) => r.id === m().custom.focus));
  const row = step(rows, at, by);
  if (row) app.model = { ...app.model, custom: { ...m().custom, focus: row.id } };
}

export function customRow() {
  const c = m().custom;
  return c.rows.find((r) => r.id === c.focus) ?? c.rows[0];
}

// ---- artists (`b`) and an artist's page

export async function artistsRefresh() {
  screens.artistRows = await invoke<number[]>("artist_rows", { rows: m().artists.rows, query: screens.artistQuery });
  screens.artistNew = await invoke<string[][]>("new_titles", { rows: m().artists.rows, fresh: m().newReleases });
  const names = screens.artistRows.map((at) => m().artists.rows[at]?.name);
  if (!names.includes(screens.artist ?? "")) screens.artist = names[0] ?? null;
}

export function openArtists() {
  go("Artists");
  void send("Artists");
}

export function artistMove(by: number | "first" | "last") {
  const names = screens.artistRows.map((at) => m().artists.rows[at]?.name).filter((n): n is string => !!n);
  screens.artist = step(names, Math.max(0, names.indexOf(screens.artist ?? "")), by) ?? null;
}

export function artistSelected() {
  return m().artists.rows.find((r) => r.name === screens.artist);
}

function showPage(name: string) {
  screens.section = "Releases";
  screens.release = null;
  screens.topAt = 0;
  screens.relatedAt = 0;
  screens.topAsked = false;
  screens.relatedAsked = false;
  app.model = { ...app.model, view: "Artist", page: { name, found: null, rows: [], error: null, loaded: false, top: null, related: null } };
  void send({ Discography: name });
}

export function openPage() {
  const row = artistSelected();
  if (!row) return;
  screens.back = [];
  showPage(row.name);
}

export function leavePage() {
  const before = screens.back.pop();
  if (before) showPage(before);
  else go("Artists");
}

// Tab: top tracks and related artists are asked for the first time they are shown, and again after a failure.
export function pageSection() {
  const next: Record<Section, Section> = { Releases: "Top", Top: "Related", Related: "Releases" };
  screens.section = next[screens.section];
  const page = m().page;
  if (screens.section === "Top" && !(page.top && "Ok" in page.top) && !screens.topAsked) {
    screens.topAsked = true;
    void invoke("send", { cmd: { Top: page.name } });
  } else if (screens.section === "Related" && !(page.related && "Ok" in page.related) && !screens.relatedAsked) {
    screens.relatedAsked = true;
    void invoke("send", { cmd: { Related: page.name } });
  }
}

export async function releasesRefresh() {
  screens.releaseRows = await invoke<number[]>("release_rows", { rows: m().page.rows, allKinds: screens.allKinds });
  const ids = screens.releaseRows.map((at) => m().page.rows[at]?.id);
  if (!ids.includes(screens.release ?? -1)) screens.release = ids[0] ?? null;
}

export function releaseSelected(): AlbumRow | undefined {
  return m().page.rows.find((r) => r.id === screens.release && screens.releaseRows.includes(m().page.rows.indexOf(r)));
}

export const topRows = (): TopRow[] => (m().page.top && "Ok" in m().page.top! ? (m().page.top as { Ok: TopRow[] }).Ok : []);
export const relatedRows = (): RelatedRow[] => (m().page.related && "Ok" in m().page.related! ? (m().page.related as { Ok: RelatedRow[] }).Ok : []);

export function pageMove(by: number | "first" | "last") {
  if (screens.section === "Releases") {
    const ids = screens.releaseRows.map((at) => m().page.rows[at]?.id).filter((i): i is number => i !== undefined);
    screens.release = step(ids, Math.max(0, ids.indexOf(screens.release ?? -1)), by) ?? null;
  } else if (screens.section === "Top") {
    const all = topRows().map((_, i) => i);
    screens.topAt = step(all, screens.topAt, by) ?? 0;
  } else {
    const all = relatedRows().map((_, i) => i);
    screens.relatedAt = step(all, screens.relatedAt, by) ?? 0;
  }
}

// A release possibly incomplete: a held one whose length never came may be two tracks of twelve.
export const complete = (r: AlbumRow) => r.have > 0 && r.of !== null && r.have >= r.of;

// Whether Enter would fetch now. Locked, it is never offered and never said.
export function canGet(): boolean {
  const r = releaseSelected();
  return app.music && m().view === "Artist" && m().prompt === null && !m().busy && screens.section === "Releases" && !!r && !complete(r);
}

// `send` clears the stop for a search as it goes, so an Esc can never land between the clear and the search.
function search(cmd: Cmd) {
  app.model = { ...app.model, searching: true };
  void send(cmd);
}

// Enter on the page: the list's own action. Locked, a release or a top track does nothing and says nothing.
export function pageEnter() {
  const page = m().page;
  if (screens.section === "Related") {
    const row = relatedRows()[screens.relatedAt];
    if (!row) return;
    screens.back.push(page.name);
    return showPage(row.name);
  }
  if (!app.music) return;
  if (screens.section === "Releases") {
    const r = releaseSelected();
    if (!r) return;
    if (complete(r)) return say(`${r.title} is already here  ·  o shows its folder`);
    return search({ GetAlbum: { artist: page.name, title: r.title } });
  }
  const titles = topRows().filter((t) => !t.have).map((t) => t.title);
  if (!titles.length) return say("every one of these is already here");
  search({ GetTop: { artist: page.name, titles } });
}

export async function stopSearch() {
  await invoke("stop");
  say("stopping the search");
}

// The library narrowed to the folders their tags are in: not a match on filenames, which say AC-DC for AC/DC.
export async function artistFolders() {
  const name = m().view === "Artist" ? m().page.name : screens.artist;
  if (!name) return;
  const at = await invoke<number | null>("artist_at", { rows: m().artists.rows, name });
  const row = at === null ? undefined : m().artists.rows[at];
  if (!row?.folders.length) return say(`no folder holds a track by ${name} now`);
  ui.byArtist = [name, row.folders];
  ui.filter = "";
  ui.typing = false;
  ui.only = null;
  go("Library");
}

// ---- settings (`,`)

export function openSettings() {
  if (m().view === "Settings") return;
  screens.settingsBack = m().view;
  screens.settingsAt = 0;
  go("Settings");
}

export function leaveSettings() {
  go(screens.settingsBack);
}

// Enter on a setting. Only through the worker and only while it is idle; a flag or the file holds some.
export async function changeSetting(walkTheme: () => Promise<void>) {
  const row = m().settings.rows[screens.settingsAt];
  if (!row) return;
  const value = row.value as "Theme" | { Switch: boolean } | { Format: string } | { Icons: string } | { Fixed: string };
  if (value === "Theme") return walkTheme();
  if (typeof value === "object" && "Fixed" in value) {
    const from = row.source === "Flag" ? (row.key === "extra" ? "the arguments after -- on the command line" : `--${row.key.replace(/_/g, "-")} on the command line`)
      : row.source === "Env" ? "ACOUSTID_API_KEY in the environment" : "the config file";
    return say(`${row.key} is set in ${from}`);
  }
  if (row.source === "Flag") return say(`--${await invoke<string>("switch_flag", { key: row.key })} was given  ·  start without it to change ${row.key}`);
  const running = m().done === null && m().tracks.length > 0;
  if (m().busy || running || m().prompt) return say("settings change once the run is over");
  if ("Switch" in value) void send({ Switch: [row.key, !value.Switch] });
  else if ("Format" in value) void send("Format");
  else if ("Icons" in value) void send({ Icons: await invoke<string>("next_icons", { icons: value.Icons }) });
}

// ---- achievements

// Nothing being typed into or answered, and nothing else on show: a toast would swallow a keystroke.
export function calm(): boolean {
  const s = m();
  return s.prompt === null && s.picking === null && !ui.typing && !screens.artistTyping && s.view !== "Pick"
    && !screens.help && !ui.quitting && !screens.discard && !firstScreen(s);
}

export async function checkPrizes() {
  const s = m();
  const fresh = await invoke<Prize[]>("earned", { library: s.library, tracks: s.tracks, done: s.done, stage: s.resting, playback: s.playback });
  const shown = new Set([...(screens.toast?.prizes ?? []), ...screens.waiting].map((p) => p.id));
  screens.waiting = [...screens.waiting, ...fresh.filter((p) => !shown.has(p.id))];
}

// Everything waiting becomes one toast.
export function showPrizes() {
  if (screens.toast || !screens.waiting.length || !calm()) return;
  screens.toast = { prizes: screens.waiting, at: Date.now() };
  screens.waiting = [];
}

// Run its course or dismissed by a key: whoever pressed the key saw it, so either way it is recorded.
export function endPrizes() {
  const toast = screens.toast;
  if (!toast) return;
  screens.toast = null;
  void invoke("shown", { ids: toast.prizes.map((p) => p.id) });
}

export { flash };
