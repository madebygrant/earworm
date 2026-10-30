// What only the page knows: which screen mode, which pane is open, the filter box, and the actions keys
// and buttons share. Everything the worker knows is in the reducer's state.
import { invoke } from "@tauri-apps/api/core";
import type { Cmd } from "./commands";
import type { Kind, Sort, Status, Track } from "./protocol";
import { app, flash } from "./store.svelte";

export interface Tool {
  bin: string;
  what: string;
  required: boolean;
  version: string | null;
  at: string | null;
  fix: string;
}

export const ui = $state({
  /** Tiles or the list, on `w`. */
  mode: "tiles" as "tiles" | "list",
  logs: false,
  logsTall: false,
  /** The quit question, raised by `q` while something is running. */
  quitting: false,
  filter: "",
  typing: false,
  sort: "Name" as Sort,
  only: null as Kind | null,
  // The library narrowed to one artist's folders by `o`, named in the band and cleared with the others.
  byArtist: null as [string, string[]] | null,
  tools: null as Tool[] | null,
  /** The first screen has sent a link, so the next link question is a refusal of it, empty link included. */
  linkSent: false,
  /** Positions in `tracks` the list shows, and in `library` the wall shows, in order; core's answers. */
  rows: [] as number[],
  shelves: [] as number[],
});

/** Something is running: a command in flight, or a run that has tracks and has not finished. */
export function working(): boolean {
  const m = app.model;
  return m.busy || (m.done === null && m.tracks.length > 0);
}

// A run's own screen is up: tracks listed and not yet finished, past the pick gate. The header, the bar
// and the screen choice all ask this, so they agree on when the run screen shows.
export function running(): boolean {
  const m = app.model;
  return m.view === "Tracks" && m.tracks.length > 0 && m.done === null && m.picking === null;
}

/** The run is over and nothing else is going, so a command will be read straight away. */
export function canCommand(): boolean {
  const m = app.model;
  return m.view === "Tracks" && m.done !== null && m.prompt === null && !m.busy && m.tracks.length > 0;
}

// Not while a run is going either: a library key reaching here mid-run would send a sync or remove that
// then would queue behind a pipeline that is still writing.
export function canBrowse(): boolean {
  const m = app.model;
  return m.view === "Library" && m.prompt === null && !working();
}

// Marked busy when sent, not when the worker reports it: the gap is long enough for a second press to queue
// a duplicate. Before the send rather than after, or a quick command's `Idle` lands first and busy sticks.
export async function send(cmd: Cmd) {
  app.model = { ...app.model, busy: true };
  if (!(await invoke<boolean>("send", { cmd }))) app.model = { ...app.model, busy: false };
}

/** Covers for the files given, which arrive as `Msg::Thumb`. Not a command anyone waits on, so not busy. */
export function askThumbs(files: string[]) {
  const m = app.model;
  const fresh = files.filter((f) => !(f in m.thumbs) && !m.thumbsAsked.includes(f));
  if (!fresh.length) return;
  app.model = { ...m, thumbsAsked: [...m.thumbsAsked, ...fresh] };
  void invoke("send", { cmd: { Thumbs: fresh } });
}

export function word(status: Status) {
  return app.model.statuses.find((w) => w.status === status);
}

export function label(status: Status): string {
  return word(status)?.label ?? status;
}

/** The CSS colour a status is drawn in, from core's slot for it. */
export function tone(status: Status): string {
  return `var(--${word(status)?.slot ?? "text"})`;
}

export function live(status: Status): boolean {
  return word(status)?.slot === "accent";
}

/** What a tile or row shows as the title and artist: the tags once written, the video title until then. */
export function names(t: Track): [string, string] {
  if (t.title) return [t.title, t.artist];
  const at = t.name.indexOf(" - ");
  return at > 0 ? [t.name.slice(at + 3), t.name.slice(0, at)] : [t.name, ""];
}

/** The tracks the list is showing, in order: core's `view::rows` when anything narrows it. */
export async function trackRows(tracks: Track[], filter: string, review: boolean): Promise<number[]> {
  if (!filter && !review) return tracks.map((_, i) => i);
  return invoke<number[]>("rows", { tracks, filter, review });
}

export async function quit() {
  await invoke("quit");
}

export function say(text: string) {
  flash(text);
}

/** A path as the sheet shows it: from the music folder's own name down, e.g. `music/Krautrock/01 - a.opus`. */
export function short(path: string): string {
  const dir = app.setup?.dir;
  if (!dir) return path;
  const name = dir.replace(/\/+$/, "").split("/").pop()!;
  const at = path.lastIndexOf(`/${name}/`);
  return at >= 0 ? path.slice(at + 1) : path;
}
