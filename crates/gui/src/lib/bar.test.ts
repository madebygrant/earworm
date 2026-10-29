import { describe, expect, it, vi } from "vitest";

vi.mock("@tauri-apps/api/core", () => ({ invoke: async () => null }));

describe("the bar", () => {
  it("hints only keys the screen's key sheet lists", async () => {
    const { hints } = await import("./bar");
    const { initial } = await import("./reducer");
    const { SHEETS } = await import("./keysheet");
    const { app } = await import("./store.svelte");
    app.music = true;
    // The pick gate is left out: it has no sheet, since `h` there is not a key.
    for (const view of Object.keys(SHEETS) as (keyof typeof SHEETS)[]) {
      const m = { ...initial(), view, done: { Ok: "" } as never, undoable: "edit", logs: ["x"], library: [{} as never] };
      const listed = new Set(SHEETS[view][1].flatMap(([, keys]) => keys.flatMap(([k]) => k.split(" "))));
      // `h` and F1 open the sheet itself.
      const keys = hints(m).flatMap(([k]) => k.split(" ")).filter((k) => k !== "h" && k !== "F1");
      expect.soft(keys.filter((k) => !listed.has(k)), view).toEqual([]);
    }
  });
});
