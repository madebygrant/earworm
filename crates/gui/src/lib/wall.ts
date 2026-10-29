// What a folder looks like on the library wall and in its sheet.
import type { IconName } from "./icons";
import type { Shelf } from "./protocol";

// A custom playlist's files are names without their folders, so there is no path to ask a thumbnail for.
export type Art = { files: string[]; mosaic: boolean } | "custom" | "adopted" | "none";

export function shelfArt(s: Shelf): Art {
  if (s.kind === "Custom") return "custom";
  if (s.url === null) return "adopted";
  const here = s.files.filter(([, on]) => on).map(([name]) => `${s.path}/${name}`);
  if (!here.length) return "none";
  return s.kind === "Album" ? { files: here.slice(0, 1), mosaic: false } : { files: here.slice(0, 4), mosaic: true };
}

// The wall on screen, for the movement keys to measure; set by `Wall.svelte` while it is mounted.
export const shown = { el: null as HTMLElement | null };

export function kindWord(s: Shelf): string {
  return s.kind === "Album" ? "album" : s.kind === "Custom" ? "custom playlist" : s.url ? "playlist" : "not downloaded by earworm";
}

export function kindIcon(s: Shelf): IconName {
  return s.kind === "Album" ? "album" : s.kind === "Custom" ? "custom" : s.url ? "playlist" : "adopted";
}

export function ago(secs: number | null, now = Date.now() / 1000): string {
  if (!secs) return "never";
  const days = Math.floor((now - secs) / 86400);
  return days < 1 ? "today" : days === 1 ? "yesterday" : `${days} days ago`;
}
