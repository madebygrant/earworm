// A quick command's `Idle` can arrive before the send that started it returns.
import { describe, expect, it, vi } from "vitest";

vi.mock("@tauri-apps/api/core", () => ({
  invoke: async (name: string) => {
    if (name !== "send") return null;
    const { dispatch } = await import("./store.svelte");
    dispatch("Idle");
    return true;
  },
}));

describe("send", () => {
  it("is not left busy when the worker finishes before the send returns", async () => {
    const { send } = await import("./ui.svelte");
    const { app } = await import("./store.svelte");
    await send("Artists");
    expect(app.model.busy).toBe(false);
  });
});
