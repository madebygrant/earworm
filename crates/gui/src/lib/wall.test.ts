import { describe, expect, it } from "vitest";
import type { Shelf } from "./protocol";
import { ago, shelfArt } from "./wall";

const shelf = (over: Partial<Shelf>): Shelf => ({
  path: "/m/Krautrock", name: "Krautrock", url: "https://y/p", tracks: 0, missing: 0, synced: null,
  files: [], playlist: null, kind: "Playlist", ...over,
});
const files = (n: number, gone: number[] = []): [string, boolean][] => Array.from({ length: n }, (_, i) => [`${i + 1} - t.opus`, !gone.includes(i)]);

describe("shelfArt", () => {
  it("draws a playlist from its first four files that are on disk", () => {
    expect(shelfArt(shelf({ files: files(6, [1]) }))).toEqual({
      files: ["/m/Krautrock/1 - t.opus", "/m/Krautrock/3 - t.opus", "/m/Krautrock/4 - t.opus", "/m/Krautrock/5 - t.opus"],
      mosaic: true,
    });
  });
  it("draws an album from one sleeve", () => {
    expect(shelfArt(shelf({ kind: "Album", files: files(3, [0]) }))).toEqual({ files: ["/m/Krautrock/2 - t.opus"], mosaic: false });
  });
  it("asks nothing for a custom playlist or a folder earworm did not download", () => {
    expect(shelfArt(shelf({ kind: "Custom", files: files(3) }))).toBe("custom");
    expect(shelfArt(shelf({ url: null, files: files(3) }))).toBe("adopted");
    expect(shelfArt(shelf({ files: files(2, [0, 1]) }))).toBe("none");
  });
});

describe("ago", () => {
  it("counts whole days", () => {
    const now = 100 * 86400;
    expect([ago(null, now), ago(now - 60, now), ago(now - 86400, now), ago(now - 3 * 86400, now)]).toEqual(["never", "today", "yesterday", "3 days ago"]);
  });
});
