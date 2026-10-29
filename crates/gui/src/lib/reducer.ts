// The page's `App::apply`: one pure function from a message to the next state, so it runs in Vitest.
// What a list shows is core's `view`, asked over IPC, and anything else core must answer comes back
// out as an effect for the store to run.

import type {
  AlbumRow, ArtistRow, Kind, Msg, Playback, PlaylistRow, Prompt, RelatedRow, Result, SettingRow, Shelf, Status,
  TopRow, Track, Word,
} from "./protocol";
import { variant } from "./protocol";

export type View = "Tracks" | "Library" | "Found" | "Playlist" | "Pick" | "Artists" | "Artist" | "Settings";

export interface Page {
  name: string;
  found: [string, number] | null;
  rows: AlbumRow[];
  error: string | null;
  loaded: boolean;
  top: Result<TopRow[], string> | null;
  related: Result<RelatedRow[], string> | null;
}

export interface State {
  /** Each status's word and rules, from core at start. */
  statuses: Word[];
  view: View;
  stage: string;
  /** What the header goes back to once a flash has been read. */
  resting: string;
  flash: string | null;
  /** Flashes that arrived while one was still being read, three at most. */
  waiting: string[];
  playlist: string;
  folder: string | null;
  playback: Playback;
  onAir: [string, string[]] | null;
  tracks: Track[];
  logs: string[];
  library: Shelf[];
  /** The library row the cursor is on, by path, so a refresh that adds a folder keeps it on the same one. */
  shelf: string | null;
  custom: { name: string; rows: PlaylistRow[]; focus: string | null };
  artists: { rows: ArtistRow[]; loaded: boolean; focus: string | null };
  page: Page;
  newReleases: [string, number, string][];
  // Artists whose page was read this session, by core's `artists::key`, which a late check must not mark again.
  pagesRead: string[];
  /** The track the cursor is on, by index; `follow` lets progress move it. */
  cursor: number | null;
  follow: boolean;
  review: boolean;
  marked: number[];
  pickHeld: number[];
  /** The pick gate's question id while it is open. */
  picking: number | null;
  prompt: { id: number; prompt: Prompt } | null;
  pickTracks: string | null;
  foundMarks: [string, string][];
  done: Result<string, string> | null;
  busy: boolean;
  searching: boolean;
  settings: { text: string; format: string; rows: SettingRow[]; brief: [string, string][] };
  undoable: string | null;
  thumbs: Record<string, string | null>;
  // Files a `Thumbs` went out for and no reply has come back: asked once, not again on every message.
  thumbsAsked: string[];
  /** Bumped when a cover changed under a track, so anything showing one reloads it. */
  artwork: number;
  quit: boolean;
}

export type Effect =
  | { GateMarks: { tracks: Track[]; held: number[] } }
  | { Unread: { found: [string, number, string][]; read: string[] } }
  | { MarkNew: { name: string; rows: AlbumRow[]; fresh: [string, number, string][]; read: string[] } };

export interface Step {
  state: State;
  effects: Effect[];
}

const QUEUE = 3;

export function initial(statuses: Word[] = []): State {
  return {
    statuses,
    view: "Tracks",
    stage: "starting",
    resting: "starting",
    flash: null,
    waiting: [],
    playlist: "",
    folder: null,
    playback: { player: "Missing", folder: null, track: null, playing: false },
    onAir: null,
    tracks: [],
    logs: [],
    library: [],
    shelf: null,
    custom: { name: "", rows: [], focus: null },
    artists: { rows: [], loaded: false, focus: null },
    page: { name: "", found: null, rows: [], error: null, loaded: false, top: null, related: null },
    newReleases: [],
    pagesRead: [],
    cursor: null,
    follow: true,
    review: false,
    marked: [],
    pickHeld: [],
    picking: null,
    prompt: null,
    pickTracks: null,
    foundMarks: [],
    done: null,
    busy: false,
    searching: false,
    settings: { text: "", format: "", rows: [], brief: [] },
    undoable: null,
    thumbs: {},
    thumbsAsked: [],
    artwork: 0,
    quit: false,
  };
}

// The worker's first question with nothing in the library is the link, and then it is the whole screen.
export function firstScreen(s: State): boolean {
  return s.prompt !== null && "Input" in s.prompt.prompt && !s.library.length && !s.tracks.length;
}

// The option a digit answers on a choice of `options`, or none. Bounded by what is on screen: the finish
// menu's last row is quit, so a digit past the end must do nothing rather than choose the last.
export function digitChoice(key: string, options: number): number | null {
  return /^[1-9]$/.test(key) && Number(key) <= options ? Number(key) - 1 : null;
}

// Tracks that have reached a status the run ends on, by core's `settled`.
export function settledCount(s: State): number {
  return s.tracks.filter((t) => word(s, t.status)?.settled).length;
}

/** Shows a message in the header, or queues it behind the one being read. */
export function say(s: State, text: string): State {
  if (s.flash !== null) {
    return s.waiting.length < QUEUE ? { ...s, waiting: [...s.waiting, text] } : s;
  }
  return { ...s, flash: text };
}

/** Whether `say` would keep a flash rather than drop it from a full queue. */
export function hasRoom(s: State): boolean {
  return s.flash === null || s.waiting.length < QUEUE;
}

/** The flash has been read: the next queued one, or back to what the run last said. */
export function expire(s: State): State {
  if (s.flash === null) return s;
  const [next, ...rest] = s.waiting;
  return next === undefined ? { ...s, flash: null } : { ...s, flash: next, waiting: rest };
}

export function word(s: State, status: Status): Word | undefined {
  return s.statuses.find((w) => w.status === status);
}

function looks(s: State, status: Status): boolean {
  return word(s, status)?.look ?? false;
}

function track(s: State, index: number, change: (t: Track) => Track): State {
  return { ...s, tracks: s.tracks.map((t) => (t.index === index ? change(t) : t)) };
}

function followTo(s: State, index: number): State {
  return s.follow ? { ...s, cursor: index } : s;
}

type Key<M> = M extends string ? M : keyof M & string;
type Body<M> = M extends string ? undefined : M[keyof M];
// One entry per variant: leaving a message out is a type error here, and an unknown one throws below.
type Handlers = { [M in Msg as Key<M>]: (s: State, body: Body<M>) => State | Step };

const handlers: Handlers = {
  // Only while the run itself is talking: a command's progress would otherwise become what the header
  // falls back to once the command is cancelled.
  Stage: (s, text) => ({ ...s, resting: s.done === null ? text : s.resting, stage: text, flash: null, waiting: [] }),
  Flash: (s, text) => say(s, text),
  Playlist: (s, name) => ({ ...s, playlist: name }),
  Folder: (s, folder) => ({ ...s, folder }),
  Player: (s, playback) => ({ ...s, playback }),
  OnAir: (s, onAir) => ({ ...s, onAir }),
  Tracks: (s, tracks) => ({ ...s, tracks }),
  Progress: (s, { index, percent }) =>
    followTo(track(s, index, (t) => ({ ...t, percent, status: "Downloading" })), index),
  Twin: (s, { index, twin }) => track(s, index, (t) => ({ ...t, twin })),
  Update: (s, { index, status, source, note, name }) =>
    followTo(
      track(s, index, (t) => ({
        ...t,
        status,
        source: source ?? t.source,
        note: note ?? t.note,
        name: name ?? t.name,
      })),
      index,
    ),
  Meta: (s, { index, artist, title, album, year }) => track(s, index, (t) => ({ ...t, artist, title, album, year })),
  Path: (s, { index, path }) => track(s, index, (t) => ({ ...t, path })),
  Log: (s, line) => ({ ...s, logs: [...s.logs, line] }),
  Library: (s, { shelves, show }) => {
    // Kept on the same folder by its path, since a list made or deleted shifts every row after it.
    const shelf = shelves.some((x) => x.path === s.shelf) ? s.shelf : (shelves[0]?.path ?? null);
    // A deleted list is not on air, whatever cliamp still plays.
    const alive = s.onAir && shelves.some((x) => x.kind === "Custom" && x.name === s.onAir![0]);
    return { ...s, library: shelves, shelf, onAir: alive ? s.onAir : null, view: show ? "Library" : s.view };
  },
  Custom: (s, { name, rows, show, focus }) => ({
    ...s,
    custom: { name, rows, focus: focus ?? (s.custom.name === name ? s.custom.focus : null) },
    view: show ? "Playlist" : s.view,
  }),
  // Applied only when every id is one the list holds, as the terminal does.
  Order: (s, { name, ids, focus }) => {
    if (s.custom.name !== name) return s;
    const rows = ids.map((id) => s.custom.rows.find((r) => r.id === id));
    if (rows.some((r) => r === undefined)) return s;
    return { ...s, custom: { name, rows: rows as PlaylistRow[], focus: focus ?? s.custom.focus } };
  },
  Select: (s, path) => (s.library.some((x) => x.path === path) ? { ...s, shelf: path } : s),
  FoundUnmark: (s) => ({ ...s, foundMarks: [] }),
  Artists: (s, { rows, focus }) => ({ ...s, artists: { rows, loaded: true, focus: focus ?? s.artists.focus } }),
  // Which releases are new is core's to mark, with `artists::key`, as the list arrives.
  Discography: (s, { name, found, rows }) => {
    if (s.page.name !== name) return s;
    const page = { ...s.page, found, loaded: true };
    if (!("Ok" in rows)) return { ...s, page: { ...page, error: rows.Err } };
    return {
      state: { ...s, page: { ...page, rows: rows.Ok } },
      effects: [{ MarkNew: { name, rows: rows.Ok, fresh: s.newReleases, read: s.pagesRead } }],
    };
  },
  PickHeld: (s, held) => ({ ...s, pickHeld: held }),
  Top: (s, { name, rows }) => (s.page.name === name ? { ...s, page: { ...s.page, top: rows } } : s),
  Related: (s, { name, rows }) => (s.page.name === name ? { ...s, page: { ...s.page, related: rows } } : s),
  NewReleases: (s, found) => ({ state: s, effects: [{ Unread: { found, read: s.pagesRead } }] }),
  Counted: (s, { name, id, of }) =>
    s.page.name === name ? { ...s, page: { ...s.page, rows: s.page.rows.map((r) => (r.id === id ? { ...r, of } : r)) } } : s,
  PickTracks: (s, name) => ({ ...s, pickTracks: name, view: "Pick" }),
  Renamed: (s, { old, new: renamed }) =>
    s.onAir && s.onAir[0] === old ? { ...s, onAir: [renamed, s.onAir[1]] } : s,
  Ask: (s, { id, prompt }) => ({ ...s, prompt: { id, prompt } }),
  // The gate opens with everything it could fetch marked; core decides which those are.
  Pick: (s, { id }) => ({
    state: { ...s, picking: id, pickHeld: [], follow: false },
    effects: [{ GateMarks: { tracks: s.tracks, held: s.pickHeld } }],
  }),
  Done: (s, { result, stage }) => {
    const said = "Ok" in result ? stage : "failed";
    return { ...s, stage: said, resting: said, flash: null, done: result };
  },
  Unmark: (s) => ({ ...s, marked: [] }),
  // A batch the worker cut short at `stop` ends here too, so what it never answered can be asked again.
  Idle: (s) => ({ ...s, busy: false, searching: false, thumbsAsked: s.thumbsAsked.filter((f) => f in s.thumbs) }),
  Restart: (s) => ({
    ...s,
    searching: false,
    done: null,
    tracks: [],
    marked: [],
    playlist: "",
    folder: null,
    cursor: null,
    follow: true,
    review: false,
    undoable: null,
    // Whatever is starting has tracks, and they are the point.
    view: "Tracks",
  }),
  Settings: (s, { text, format, rows, brief }) => ({ ...s, settings: { text, format, rows, brief } }),
  Undoable: (s, undoable) => ({ ...s, undoable }),
  Dropped: (s, gone) => ({
    ...s,
    tracks: s.tracks.filter((t) => !gone.includes(t.index)),
    marked: s.marked.filter((i) => !gone.includes(i)),
    cursor: s.cursor !== null && gone.includes(s.cursor) ? null : s.cursor,
  }),
  // Not a jump on its own: the cursor lands on the first, and the rest stay on screen.
  Review: (s) => {
    const first = s.tracks.find((t) => looks(s, t.status));
    return { ...s, review: first !== undefined, follow: false, cursor: first ? first.index : s.cursor };
  },
  Focus: (s, index) => (s.tracks.some((t) => t.index === index) ? { ...s, cursor: index, follow: false } : s),
  // A cover changed somewhere, and a thumbnail is a copy of one, so every one is asked for again.
  Artwork: (s) => ({ ...s, artwork: s.artwork + 1, thumbs: {}, thumbsAsked: [] }),
  Quit: (s) => ({ ...s, quit: true }),
  Thumb: (s, { file, thumb }) => ({ ...s, thumbs: { ...s.thumbs, [file]: thumb } }),
};

/** Every message the reducer knows, for the test that holds it to the recorded ones. */
export const VARIANTS = Object.keys(handlers).sort();

export function apply(s: State, msg: Msg): Step {
  const name = variant(msg);
  const handle = (handlers as Record<string, (s: State, body: unknown) => State | Step>)[name];
  if (!handle) throw new Error(`no handler for ${name}`);
  const body = typeof msg === "string" ? undefined : (msg as Record<string, unknown>)[name];
  const out = handle(s, body);
  return "effects" in out ? out : { state: out, effects: [] };
}

export type { Kind };
