// @vitest-environment jsdom
// `t` opens a box that takes the focus while the key is still being handled, so the key's default would type
// a t into it.
import { expect, it, vi } from "vitest";

vi.mock("@tauri-apps/api/core", () => ({ invoke: async () => null }));

it("t opens the search without letting the letter through", async () => {
  const { app } = await import("./store.svelte");
  const { handleKey } = await import("./keys");
  app.model = { ...app.model, view: "Library", prompt: null };
  const preventDefault = vi.fn();
  await handleKey({ key: "t", ctrlKey: false, metaKey: false, altKey: false, target: null, preventDefault } as unknown as KeyboardEvent);
  expect(app.model.view).toBe("Found");
  expect(preventDefault).toHaveBeenCalled();
});
