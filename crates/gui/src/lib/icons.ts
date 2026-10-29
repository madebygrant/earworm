// The symbols in icons.svg, so a misspelt name fails svelte-check rather than drawing nothing.
export const ICONS = [
  "library", "run", "artists", "playlists", "settings", "search", "theme", "filter", "album", "playlist",
  "custom", "adopted", "alpha", "clock", "missing", "link", "folder", "file", "m3u8", "tracks", "open",
  "play", "rename", "remove", "plus", "keys", "move", "mark", "swap", "undo", "cover", "retry", "stop",
  "log", "follow", "video", "hide", "list", "all",
] as const;

export type IconName = (typeof ICONS)[number];
