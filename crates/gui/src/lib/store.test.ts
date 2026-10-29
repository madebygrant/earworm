// The boot order is what keeps a reloaded page from losing or doubling messages: live ones that arrive
// while `start` is answering wait, then the history and the held ones apply once each, in order.
import { beforeEach, describe, expect, it, vi } from "vitest";

let handlers: ((e: { payload: unknown }) => void)[] = [];
const deliver = (payload: unknown) => handlers.forEach((h) => h({ payload }));
let reply: unknown;
vi.mock("@tauri-apps/api/event", () => ({
  listen: async (name: string, handler: (e: { payload: unknown }) => void) => {
    if (name === "earworm://msg") handlers.push(handler);
    return () => {};
  },
}));
vi.mock("@tauri-apps/api/core", () => ({
  invoke: async (name: string, args: Record<string, never>) => {
    // Core's two, reduced to what the test needs: unread drops the pages read, mark_new reads one page.
    if (name === "unread") return (args.found as [string][]).filter(([a]) => !(args.read as string[]).includes(a));
    if (name === "mark_new") return [args.rows, (args.fresh as [string][]).filter(([a]) => a !== args.name), [...(args.read as string[]), args.name]];
    if (name !== "start") return [];
    // A live message lands while `start` is still answering, one the history also holds.
    deliver({ seq: 1, msg: { Log: "one" } });
    deliver({ seq: 2, msg: { Log: "two" } });
    return reply;
  },
}));
vi.stubGlobal("document", { documentElement: { style: { setProperty() {}, colorScheme: "" } } });

const theme = { name: "graphite", light: false, names: ["graphite"], vars: {}, said: null };
const setup = { text: "", format: "opus", rows: [], dir: "~/Music", key: false };

describe("boot", () => {
  beforeEach(() => {
    vi.resetModules();
    handlers = [];
  });

  it("applies the history, then the held messages it did not hold, each once and in order", async () => {
    reply = { theme, statuses: [], setup, replay: [{ seq: 0, msg: { Log: "zero" } }, { seq: 1, msg: { Log: "one" } }] };
    const { app, boot } = await import("./store.svelte");
    await boot();
    expect(app.model.logs).toEqual(["zero", "one", "two"]);
    deliver({ seq: 3, msg: { Log: "three" } });
    expect(app.model.logs).toEqual(["zero", "one", "two", "three"]);
  });

  it("listens once however often it is mounted", async () => {
    reply = { theme, statuses: [], setup, replay: [] };
    const { app, boot } = await import("./store.svelte");
    await boot();
    await boot();
    deliver({ seq: 3, msg: { Log: "x" } });
    expect(app.model.logs.filter((l) => l === "x")).toHaveLength(1);
  });

  it("clears the page's own narrowing when a new run starts", async () => {
    reply = { theme, statuses: [], setup, replay: [] };
    const { boot, hooks } = await import("./store.svelte");
    let cleared = 0;
    hooks.restart = () => cleared++;
    await boot();
    deliver({ seq: 3, msg: "Restart" });
    expect(cleared).toBe(1);
  });

  // Run side by side from snapshots, the page's read wiped the startup check's answer for every other artist.
  it("keeps new releases for other artists when a page is read while the check answers", async () => {
    reply = { theme, statuses: [], setup, replay: [] };
    const { app, boot } = await import("./store.svelte");
    await boot();
    app.model = { ...app.model, page: { ...app.model.page, name: "Can" } };
    deliver({ seq: 3, msg: { NewReleases: [["Can", 1, "Live"], ["Neu!", 2, "75"]] } });
    deliver({ seq: 4, msg: { Discography: { name: "Can", found: null, rows: { Ok: [] } } } });
    await new Promise((r) => setTimeout(r, 0));
    await new Promise((r) => setTimeout(r, 0));
    expect(app.model.newReleases).toEqual([["Neu!", 2, "75"]]);
    expect(app.model.pagesRead).toEqual(["Can"]);
  });
});
