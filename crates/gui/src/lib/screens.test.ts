// The artist page's one secret: locked, it lists releases and top tracks and never offers or starts a
// download, and says nothing about why. Unlocked, Enter on a release not all here asks the worker.
import { beforeEach, describe, expect, it, vi } from "vitest";

const sent: unknown[] = [];
let cheat: unknown = null;
vi.mock("@tauri-apps/api/core", () => ({
  invoke: async (name: string, args: { cmd?: unknown }) => {
    if (name === "send") sent.push(args.cmd);
    if (name === "cheat") return cheat;
    return name === "send" ? true : null;
  },
}));

const release = (id: number, have: number, of: number | null) => ({ id, title: `R${id}`, year: 2000, kind: "album", have, of, new: false });

describe("the artist page", () => {
  beforeEach(() => {
    vi.resetModules();
    sent.length = 0;
  });

  async function page(music: boolean) {
    const { app } = await import("./store.svelte");
    const sc = await import("./screens.svelte");
    const rows = [release(1, 0, null), release(2, 12, 12)];
    app.music = music;
    app.model = {
      ...app.model,
      view: "Artist",
      page: { name: "Can", found: null, rows, error: null, loaded: true, top: { Ok: [{ title: "Vitamin C", album: "Ege Bamyasi", have: false }] }, related: null },
    };
    sc.screens.releaseRows = [0, 1];
    sc.screens.release = 1;
    return sc;
  }

  it("locked, offers no download and Enter on a release or the top tracks sends nothing", async () => {
    const sc = await page(false);
    expect(sc.canGet()).toBe(false);
    sc.pageEnter();
    sc.screens.section = "Top";
    sc.pageEnter();
    await new Promise((r) => setTimeout(r, 0));
    expect(sent).toEqual([]);
  });

  it("unlocked, Enter on a missing release asks for it, and a whole one is never fetched again", async () => {
    const sc = await page(true);
    expect(sc.canGet()).toBe(true);
    sc.pageEnter();
    await new Promise((r) => setTimeout(r, 0));
    expect(sent).toEqual([{ GetAlbum: { artist: "Can", title: "R1" } }]);
    sc.screens.release = 2;
    expect(sc.canGet()).toBe(false);
  });
});

// A found code gets a card that answers the Enter just typed; the achievement it earns waits for the card.
describe("a found code", () => {
  beforeEach(() => vi.resetModules());

  it("shows the card and holds the achievement toast until it closes", async () => {
    const sc = await import("./screens.svelte");
    sc.celebrate({ art: "music", prize: "YouTube Music links", note: "for this session" });
    sc.screens.waiting = [{ id: "secret", label: "Found a secret", hint: "" }];
    expect(sc.calm()).toBe(false);
    sc.showPrizes();
    expect(sc.screens.toast).toBeNull();
    sc.endSecret();
    sc.showPrizes();
    expect(sc.screens.toast?.prizes.map((p) => p.id)).toEqual(["secret"]);
  });

  it("is said, not celebrated, when it was already found", async () => {
    const { app, tryCode } = await import("./store.svelte");
    const sc = await import("./screens.svelte");
    cheat = { said: "already found  ·  YouTube Music links", theme: null, music: true, found: null };
    sc.celebrate(await tryCode("again"));
    expect(sc.screens.secret).toBeNull();
    expect(app.model.flash).toBe("already found  ·  YouTube Music links");
  });
});
