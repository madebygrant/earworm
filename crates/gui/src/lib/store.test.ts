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
  invoke: async (name: string) => {
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
});
