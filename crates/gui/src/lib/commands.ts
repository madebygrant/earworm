// The commands the page sends, as serde reads them.
export type Pick = [string, string];
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
  | "Format"
  | { Switch: [string, boolean] }
  | { Icons: string }
  | "Undo"
  | { Rename: string }
  | "Purge"
  | { Reveal: string }
  | { Remove: string }
  | "NewPlaylist"
  | { EditPlaylist: { name: string; add: Pick[]; remove: Pick[] } }
  | { AddToPlaylist: [Pick[], string | null] }
  | { OpenPlaylist: string }
  | { MoveEntry: { name: string; id: string; up: boolean } }
  | { RemoveEntry: { name: string; id: string } }
  | { RenamePlaylist: string }
  | { DeletePlaylist: string }
  | { PlayPlaylist: string }
  | { Export: { List: string } | { Folder: string } }
  | "Artists"
  | { Star: string }
  | { Discography: string }
  | { GetAlbum: { artist: string; title: string } }
  | { Top: string }
  | { Related: string }
  | { GetTop: { artist: string; titles: string[] } }
  | { Thumbs: string[] };
