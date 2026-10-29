// Run against messages recorded from the Rust side's own samples (msgs.json, rewritten by
// `EARWORM_RECORD=1 cargo test -p earworm-gui`), so what is tested here is what the worker sends.
import { describe, expect, it } from "vitest";
import recorded from "./msgs.json";
import type { Msg, Shelf, Track, Word } from "./protocol";
import { variant } from "./protocol";
import { VARIANTS, apply, digitChoice, expire, firstScreen, initial, settledCount, type State } from "./reducer";

const msgs = recorded.msgs as Msg[];
const statuses = recorded.statuses as Word[];
const sample = (name: string) => msgs.find((m) => variant(m) === name)!;

function run(state: State, ...messages: Msg[]): State {
  return messages.reduce((s, m) => apply(s, m).state, state);
}

const shelf = (name: string): Shelf => ({
  path: `/music/${name}`, name, url: null, tracks: 1, missing: 0, synced: null, files: [], playlist: null, kind: null,
});
const tracks = (...statuses: Track["status"][]): Track[] =>
  statuses.map((status, i) => ({ ...(sample("Tracks") as { Tracks: Track[] }).Tracks[0], index: i + 1, status }));

describe("the reducer", () => {
  it("handles every message the worker sends and no message it does not", () => {
    expect([...new Set(msgs.map(variant))].sort()).toEqual(VARIANTS);
  });

  it("applies every recorded message in turn", () => {
    let s = initial(statuses);
    for (const m of msgs) s = apply(s, m).state;
    expect(s.quit).toBe(true);
  });

  it("takes a track's artist and title from Meta, never from the display name", () => {
    const s = run(
      { ...initial(statuses), tracks: tracks("Tagging") },
      { Update: { index: 1, status: "Have", source: null, note: null, name: "Hallogallo - Neu!" } },
      { Meta: { index: 1, artist: "Neu!", title: "Hallogallo", album: "Neu!", year: 1972 } },
    );
    expect([s.tracks[0].artist, s.tracks[0].title, s.tracks[0].album]).toEqual(["Neu!", "Hallogallo", "Neu!"]);
  });

  it("makes the link question the whole screen only while there is nothing in the library", () => {
    const link = { Ask: { id: 1, prompt: { Input: { header: "YouTube playlist URL", note: "", value: "", escape: "Quit" as const } } } };
    expect(firstScreen(run(initial(statuses), link))).toBe(true);
    // The format question asked straight after the link is a dialog, not the first screen.
    const format = { Ask: { id: 2, prompt: { Choice: { header: "Format", note: "", options: ["opus"], escape: "Keep" as const } } } };
    expect(firstScreen(run(initial(statuses), format))).toBe(false);
    expect(firstScreen(run(initial(statuses), { Library: { shelves: [shelf("Sleep")], show: false } }, link))).toBe(false);
  });

  it("answers a choice by digit only when that option is there", () => {
    expect(digitChoice("1", 4)).toBe(0);
    expect(digitChoice("4", 4)).toBe(3);
    expect(digitChoice("5", 4)).toBeNull();
    expect(digitChoice("0", 4)).toBeNull();
  });

  it("asks again for a thumbnail a cut-short batch never answered, once the worker is idle", () => {
    const s = run(
      { ...initial(statuses), thumbsAsked: ["/a.opus", "/b.opus"] },
      { Thumb: { file: "/a.opus", thumb: null } },
      "Idle",
    );
    expect(s.thumbsAsked).toEqual(["/a.opus"]);
  });

  it("counts settled tracks by core's word for each status", () => {
    const s = { ...initial(statuses), tracks: tracks("Ok", "Pending", "Failed", "Downloading") };
    expect(settledCount(s)).toBe(2);
  });

  it("drops the thumbnails when a cover changes, so they are asked for again", () => {
    const s = run(initial(statuses), { Thumb: { file: "/music/a.opus", thumb: "/cache/1.jpg" } }, "Artwork");
    expect(s.thumbs).toEqual({});
  });

  it("lists the library's folders and opens on it when asked", () => {
    const s = run(initial(statuses), { Library: { shelves: [shelf("Krautrock"), shelf("Sleep")], show: true } });
    expect(s.view).toBe("Library");
    expect(s.library.map((x) => x.name)).toEqual(["Krautrock", "Sleep"]);
    expect(s.shelf).toBe("/music/Krautrock");
  });

  // A list made or deleted shifts every row after it, so the cursor is a path and not a position.
  it("keeps the library cursor on its folder when a refresh adds one ahead of it", () => {
    let s = run(initial(statuses), { Library: { shelves: [shelf("B"), shelf("C")], show: true } }, { Select: "/music/C" });
    s = run(s, { Library: { shelves: [shelf("A"), shelf("B"), shelf("C")], show: false } });
    expect(s.shelf).toBe("/music/C");
  });

  it("moves a following cursor with the download and leaves it once following is off", () => {
    let s = run(initial(statuses), { Tracks: tracks("Pending", "Pending") }, { Progress: { index: 2, percent: 10 } });
    expect(s.tracks[1].status).toBe("Downloading");
    expect(s.cursor).toBe(2);
    s = run({ ...s, follow: false }, { Progress: { index: 1, percent: 5 } });
    expect(s.cursor).toBe(2);
  });

  // The review's statuses are core's, sent at start, never a list kept here.
  it("reviews the first track nothing confirmed, by core's rules", () => {
    const s = run(initial(statuses), { Tracks: tracks("Ok", "Kept", "Failed") }, "Review");
    expect(s.review).toBe(true);
    expect(s.cursor).toBe(2);
    expect(run(initial(statuses), { Tracks: tracks("Ok", "Have") }, "Review").review).toBe(false);
  });

  it("asks core for the pick gate's marks rather than choosing them", () => {
    const s = run(initial(statuses), { Tracks: tracks("Pending", "Pending") }, { PickHeld: [2] });
    const step = apply(s, { Pick: { id: 4 } });
    expect(step.state.picking).toBe(4);
    expect(step.state.pickHeld).toEqual([]);
    expect(step.effects).toEqual([{ GateMarks: { tracks: s.tracks, held: [2] } }]);
  });

  // A bulk action's outcomes arrive together, and one overwriting the next leaves only the last readable.
  it("queues flashes three deep, and the run talking clears the backlog", () => {
    let s = run(initial(statuses), { Flash: "a" }, { Flash: "b" }, { Flash: "c" }, { Flash: "d" }, { Flash: "e" });
    expect([s.flash, ...s.waiting]).toEqual(["a", "b", "c", "d"]);
    s = expire(s);
    expect(s.flash).toBe("b");
    s = run(s, { Stage: "fetching" });
    expect([s.flash, s.waiting, s.stage]).toEqual([null, [], "fetching"]);
  });

  it("names a failed run failed, and a restart clears the run", () => {
    let s = run(initial(statuses), { Tracks: tracks("Ok") }, { Done: { result: { Err: "offline" }, stage: "finished" } });
    expect(s.stage).toBe("failed");
    s = run(s, "Restart");
    expect([s.done, s.tracks, s.view]).toEqual([null, [], "Tracks"]);
  });
});
