// The run screen's numbers, read off the tracks and core's status table.
import type { Track, Word } from "./protocol";

export interface RunStats {
  total: number;
  fetched: number;
  // In the slot core paints checked tracks in.
  confirmed: number;
  look: Track[];
  failed: Track[];
  queued: number;
  skipped: number;
}

// Every progress tick asks again, so the table is looked up once and the tracks walked once.
export function runStats(tracks: Track[], words: Word[]): RunStats & { line: number[] } {
  const word = new Map(words.map((w) => [w.status, w]));
  const s = { total: tracks.length, fetched: 0, confirmed: 0, look: [] as Track[], failed: [] as Track[], queued: 0, skipped: 0, line: [] as number[] };
  for (const t of tracks) {
    const w = word.get(t.status);
    // Core's `fetched`, not "has a path": the listing gives a track its path before it is fetched.
    if (w?.fetched) s.fetched++;
    if (w?.slot === "cursor") s.confirmed++;
    // Core flags a failure as worth a look too; it has its own tile.
    if (w?.slot === "error") s.failed.push(t);
    else if (w?.look) s.look.push(t);
    if (t.status === "Pending") s.queued++;
    if (t.status === "Skipped") s.skipped++;
    s.line.push(s.confirmed / tracks.length);
  }
  return s;
}

// One bar per track, the latest `n`, as far as its download got.
export function bars(tracks: Track[], words: Word[], n = 16): number[] {
  const word = new Map(words.map((w) => [w.status, w]));
  return tracks.slice(-n).map((t) => (word.get(t.status)?.fetched ? 100 : word.get(t.status)?.in_hand ? t.percent : 0));
}

// The terminal's: minutes and seconds, however many minutes.
export function clock(secs: number): string {
  return secs ? `${Math.floor(secs / 60)}:${String(secs % 60).padStart(2, "0")}` : "";
}
