import { describe, expect, it } from "vitest";
import recorded from "./msgs.json";
import type { Msg, Track, Word } from "./protocol";
import { variant } from "./protocol";
import { bars, clock, runStats } from "./run";

const msgs = recorded.msgs as Msg[];
const statuses = recorded.statuses as Word[];
const base = (msgs.find((m) => variant(m) === "Tracks") as { Tracks: Track[] }).Tracks[0];
const track = (status: Track["status"], over: Partial<Track> = {}): Track => ({ ...base, status, path: null, percent: 0, ...over });

describe("runStats", () => {
  it("counts by core's slots and flags, and fetched by what got past its download", () => {
    const tracks = [
      track("Ok", { path: "/a" }), track("Manual", { path: "/b" }), track("Weak", { path: "/c" }),
      track("Failed", { path: "/e" }), track("Pending", { path: "/f" }), track("Skipped"), track("Tagging", { path: "/d" }), track("Gone", { path: "/g" }),
    ];
    const s = runStats(tracks, statuses);
    expect([s.total, s.fetched, s.confirmed, s.look.length, s.failed.length, s.queued, s.skipped]).toEqual([8, 4, 2, 1, 1, 1, 1]);
  });
});

describe("the tiles' charts", () => {
  it("draws a download's own progress until it is fetched", () => {
    expect(bars([track("Downloading", { percent: 40 }), track("Tagging", { percent: 0 }), track("Pending", { path: "/a" }), track("Gone", { path: "/b" })], statuses)).toEqual([40, 100, 0, 0]);
  });
  it("climbs with each confirmed track in list order", () => {
    expect(runStats([track("Ok"), track("Failed"), track("Ok"), track("Pending")], statuses).line).toEqual([0.25, 0.25, 0.5, 0.5]);
  });
  it("says a length as the terminal's clock does, and nothing for none", () => {
    expect([clock(0), clock(65), clock(3725)]).toEqual(["", "1:05", "62:05"]);
  });
});
