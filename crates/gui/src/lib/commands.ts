// The commands the page sends, as serde reads them. Only the ones a screen sends so far; the rest of
// `Cmd` arrives with the screens that use it.
export type Cmd =
  | { Edit: number }
  | { Cover: number }
  | { Search: number }
  | { Open: [string, string | null] }
  | "SyncOne"
  | { Sync: string }
  | "ResyncAll"
  | "Url"
  | "Retry"
  | { Swap: number[] }
  | { Artist: number[] }
  | { Play: string }
  | "Toggle"
  | "Undo"
  | { Rename: string }
  | "Purge"
  | { Reveal: string }
  | { Remove: string }
  | { Thumbs: string[] };
