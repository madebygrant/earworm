// What `h` lists on each screen, in groups as the mockup's key sheet draws them. The words are docs/keys.md's.
import type { View } from "./reducer";

export type Group = [string, [string, string][]];

const MOVE: [string, string][] = [["j k ↑ ↓", "move"], ["^d ^u", "by a screenful"], ["g G", "first, last"]];

export const SHEETS: Record<View, [string, Group[]]> = {
  Tracks: ["track list", [
    ["move", [...MOVE, ["n N", "next unconfirmed"], ["f", "follow the run"], ["w", "tiles or list"]]],
    ["narrow", [["/", "filter"], ["v", "only unconfirmed"], ["space", "mark"], ["m M", "mark status, artist"]]],
    ["fix", [["e", "edit tags"], ["T", "search again"], ["s A", "swap, set artist"], ["c", "cover art"], ["u", "undo"], ["*", "star the artist"]]],
    ["run", [["r", "retry failed"], ["S", "sync this"], ["D", "delete departed"], ["p", "play in cliamp"], ["P", "add to a playlist"], ["l L", "log, taller"]]],
    ["leave", [["esc", "filter, marks, then the library"], ["q ^c", "quit"]]],
  ]],
  Library: ["library", [
    ["move", [...MOVE, ["w", "tiles or list"]]],
    ["narrow", [["/", "filter folders"], ["s", "sort"], ["a", "kind"], ["t", "find a track"], ["b", "artists"]]],
    ["folder", [["enter", "open"], ["e", "rename"], ["D", "remove"], ["O", "show folder"], ["E", "export"], ["p", "play"]]],
    ["sync", [["S", "sync this"], ["R", "sync all"], ["n", "new link"], ["N", "new custom playlist"], ["space", "play/pause"]]],
    ["leave", [[",", "settings"], ["q ^c", "quit"]]],
  ]],
  Found: ["search", [
    ["move", MOVE],
    ["result", [["enter", "open its folder"], ["tab space", "mark"], ["P", "add to a playlist"], ["/", "edit the query"]]],
    ["leave", [["esc", "the library"], ["q ^c", "quit"]]],
  ]],
  Playlist: ["custom playlist", [
    ["move", MOVE],
    ["edit", [["J K", "move this track"], ["x", "take it off"], ["P", "add tracks"], ["enter", "go to its folder"]]],
    ["play", [["p", "play in cliamp"]]],
    ["leave", [["esc", "the library"], ["q ^c", "quit"]]],
  ]],
  Pick: ["pick tracks", [
    ["move", [["↑ ↓ ^n ^p", "move"], ["PgDn PgUp", "by a screenful"]]],
    ["mark", [["tab", "mark"], ["^a", "all shown"], ["F2", "only the marked"]]],
    ["leave", [["enter", "add or apply"], ["esc", "cancel"], ["^c", "quit"]]],
  ]],
  Artists: ["artists", [
    ["move", MOVE],
    ["artist", [["enter", "their page"], ["o", "their folders"], ["*", "star"], ["/", "filter"]]],
    ["leave", [["esc", "filter, then the library"], ["q ^c", "quit"]]],
  ]],
  Artist: ["artist page", [
    ["move", [...MOVE, ["tab", "releases, top tracks, related"]]],
    ["page", [["enter", "open a related artist"], ["a", "albums and EPs, or all"], ["o", "their folders"]]],
    ["leave", [["esc", "the page before"], ["q ^c", "quit"]]],
  ]],
  Settings: ["settings", [
    ["move", MOVE],
    ["change", [["enter space", "switch it, or the next choice"]]],
    ["leave", [["esc ,", "the screen before"], ["q ^c", "quit"]]],
  ]],
};

// The status words grouped by the colour core paints them in, which is what says how much to trust them.
export const SLOT_GROUPS: [string, string][] = [["cursor", "checked"], ["warn", "worth a look"], ["unsure", "worth a look"], ["accent", "running"], ["muted", "untouched"], ["error", "failed"]];
