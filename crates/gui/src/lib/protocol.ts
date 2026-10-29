// The worker's messages as serde writes them: a unit variant is a bare string, any other is an object
// with one key. Kept by hand beside crates/core/src/protocol.rs; msgs.json, recorded from the Rust
// samples, is what the reducer test checks it against.

export type Status =
  | "Pending" | "Have" | "Downloading" | "Downloaded" | "Tagging" | "Converting" | "Ok" | "Manual"
  | "Kept" | "Weak" | "NoMatch" | "Failed" | "Gone" | "Skipped" | "Local";

// One status's word and rules, sent by core at start so the page keeps no copy of them.
export interface Word {
  status: Status;
  label: string;
  look: boolean;
  settled: boolean;
  fetched: boolean;
  in_hand: boolean;
  tally: number | null;
  // The palette slot it is drawn in, which is also its CSS variable.
  slot: string;
}

export type Kind = "Album" | "Playlist" | "Custom";
export type Sort = "Name" | "Synced" | "Missing";
export type Result<T, E> = { Ok: T } | { Err: E };

export interface Track {
  index: number;
  id: string;
  name: string;
  was: string;
  path: string | null;
  status: Status;
  source: string;
  note: string;
  percent: number;
  artist: string;
  title: string;
  album: string;
  year: number | null;
  duration: number;
  mbid: string | null;
  listed: boolean;
  departure_proven: boolean;
  isrc: string | null;
  twin: string | null;
  score: number | null;
}

export interface Shelf {
  path: string;
  name: string;
  url: string | null;
  tracks: number;
  missing: number;
  synced: number | null;
  files: [string, boolean][];
  playlist: string | null;
  kind: Kind | null;
}

export interface Playback {
  player: "Missing" | "Stopped" | "Running";
  folder: string | null;
  track: string | null;
  playing: boolean;
}

export interface PlaylistRow { id: string; label: string; folder: string; file: string; path: string | null }
export interface ArtistRow { name: string; tracks: number; albums: number; starred: boolean; folders: string[] }
export interface AlbumRow { id: number; title: string; year: number | null; kind: string; have: number; of: number | null; new: boolean }
export interface TopRow { title: string; album: string; have: boolean }
export interface RelatedRow { name: string; fans: number; tracks: number }

export type Escape = "Skip" | "Quit" | "Keep";
// What a typed path names, for the system's file dialog.
export type Browse = "Folder" | "File";
export type Prompt =
  | { Choice: { header: string; note: string; options: string[]; escape: Escape; pictures: (string | null)[] } }
  | { Form: { header: string; note: string; fields: [string, string][]; escape: Escape } }
  | { Input: { header: string; note: string; value: string; escape: Escape; browse: Browse | null } };

// One match from core's picker matcher: where it is in the library and which characters matched.
export interface Hit { shelf: number; file: number; score: number; at: number[]; folder_at: number[]; label: string }
// A picker row: the match, the track it names, and its marks.
export interface PickRow extends Hit { folder: string; name: string; marked: boolean; held: boolean }
// An achievement as the toast shows it.
export interface Prize { id: string; label: string; hint: string }

export interface SettingRow {
  key: string;
  value: unknown;
  source: "Default" | "File" | "Flag" | "Env";
  what: string;
  when: "NextRun" | "Now" | "NextStart";
}

export type Msg =
  | { Stage: string }
  | { Flash: string }
  | { Playlist: string }
  | { Folder: string }
  | { Player: Playback }
  | { OnAir: [string, string[]] | null }
  | { Tracks: Track[] }
  | { Progress: { index: number; percent: number } }
  | { Twin: { index: number; twin: string | null } }
  | { Score: { index: number; score: number | null } }
  | { Update: { index: number; status: Status; source: string | null; note: string | null; name: string | null } }
  | { Meta: { index: number; artist: string; title: string; album: string; year: number | null } }
  | { Path: { index: number; path: string } }
  | { Log: string }
  | { Library: { shelves: Shelf[]; show: boolean } }
  | { Custom: { name: string; rows: PlaylistRow[]; show: boolean; focus: string | null } }
  | { Order: { name: string; ids: string[]; focus: string | null } }
  | { Select: string }
  | "FoundUnmark"
  | { Artists: { rows: ArtistRow[]; focus: string | null } }
  | { Discography: { name: string; found: [string, number] | null; rows: Result<AlbumRow[], string> } }
  | { PickHeld: number[] }
  | { Top: { name: string; rows: Result<TopRow[], string> } }
  | { Related: { name: string; rows: Result<RelatedRow[], string> } }
  | { NewReleases: [string, number, string][] }
  | { Counted: { name: string; id: number; of: number } }
  | { PickTracks: string }
  | { Renamed: { old: string; new: string } }
  | { Ask: { id: number; prompt: Prompt } }
  | { Pick: { id: number } }
  | { Done: { result: Result<string, string>; stage: string } }
  | "Unmark"
  | "Idle"
  | "Restart"
  | { Settings: { text: string; format: string; rows: SettingRow[]; brief: [string, string][] } }
  | { Undoable: string | null }
  | { Dropped: number[] }
  | "Review"
  | { Focus: number }
  | "Artwork"
  | "Quit"
  | { Thumb: { file: string; thumb: string | null } };

export type Reply = { Choice: number } | { Fields: string[] } | { Text: string } | { Picked: number[] } | "Cancel";

// The variant a message is, as serde tags it.
export function variant(msg: Msg): string {
  return typeof msg === "string" ? msg : Object.keys(msg)[0];
}
