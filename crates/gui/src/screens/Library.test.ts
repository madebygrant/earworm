// @vitest-environment jsdom
// The library asks for covers only for folders scrolled into view, through an effect a unit test of the
// reducer cannot see; a dependency read untracked there once left a built app with no covers at all.
import { flushSync, mount, unmount } from "svelte";
import { afterEach, expect, it, vi } from "vitest";
import recorded from "../lib/msgs.json";
import type { Shelf, Word } from "../lib/protocol";
import { initial } from "../lib/reducer";
import { app } from "../lib/store.svelte";
import { ui } from "../lib/ui.svelte";
import Library from "./Library.svelte";

const sent: unknown[] = [];
vi.mock("@tauri-apps/api/core", () => ({
  invoke: async (cmd: string, args: { cmd?: unknown }) => {
    if (cmd === "send") return sent.push(args.cmd), true;
    if (cmd === "next_kind") return [null, "all"];
    if (cmd === "next_sort") return ["Name", "name"];
    return null;
  },
  convertFileSrc: (path: string) => path,
}));

// jsdom lays nothing out, so the observers are driven by hand.
const watched: [IntersectionObserverCallback, Element[]][] = [];
globalThis.IntersectionObserver = class {
  els: Element[] = [];
  constructor(fire: IntersectionObserverCallback) { watched.push([fire, this.els]); }
  observe(el: Element) { this.els.push(el); }
  unobserve() {}
  disconnect() {}
} as unknown as typeof IntersectionObserver;
globalThis.ResizeObserver = class { observe() {} disconnect() {} } as unknown as typeof ResizeObserver;
Element.prototype.scrollIntoView = () => {};
globalThis.CSS ??= { escape: (s: string) => s } as typeof CSS;

const album: Shelf = {
  path: "/music/Neu!", name: "Neu!", url: "https://example.com/list", tracks: 1, missing: 0, synced: null,
  files: [["01 - Hallogallo.opus", true]], playlist: null, kind: "Album",
};

afterEach(() => void (sent.length = 0));

it("asks for a folder's cover when it scrolls into view after the library has drawn", () => {
  app.model = { ...initial(recorded.statuses as Word[]), view: "Library", library: [album], shelf: album.path };
  // App.svelte asks core for the order; alone, the screen is handed it.
  ui.shelves = [0];
  const view = mount(Library, { target: document.body });
  flushSync();
  expect(sent).toEqual([]);
  for (const [fire, els] of watched) fire(els.map((target) => ({ target, isIntersecting: true }) as IntersectionObserverEntry), {} as IntersectionObserver);
  flushSync();
  expect(sent).toEqual([{ Thumbs: ["/music/Neu!/01 - Hallogallo.opus"] }]);
  unmount(view);
});
