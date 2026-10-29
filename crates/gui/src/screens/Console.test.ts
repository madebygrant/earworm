// @vitest-environment jsdom
// The key that closes the console must not reach the screen under it, which would take it as its own Enter.
import { flushSync, mount, unmount } from "svelte";
import { expect, it, vi } from "vitest";
import { consoleHooks } from "../lib/keys";
import { screens } from "../lib/screens.svelte";
import Console from "./Console.svelte";

const asked: string[] = [];
vi.mock("@tauri-apps/api/core", () => ({
  invoke: async (name: string) => {
    asked.push(name);
    if (name === "cheat") return { said: "", theme: null, music: true, found: { art: "music", prize: "YouTube Music links", note: "for this session" } };
    if (name === "earned") return [];
    return null;
  },
}));

it("keeps the Enter that sends a code from the window, and a fresh find shows the card and asks what it earned", async () => {
  const reached = vi.fn();
  window.addEventListener("keydown", reached);
  const view = mount(Console, { target: document.body });
  consoleHooks.open();
  flushSync();
  const input = document.querySelector<HTMLInputElement>(".console input")!;
  input.value = "anything";
  input.dispatchEvent(new Event("input", { bubbles: true }));
  input.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", bubbles: true }));
  flushSync();
  expect(reached).not.toHaveBeenCalled();
  expect(consoleHooks.isOpen()).toBe(false);
  await vi.waitFor(() => expect(screens.secret?.art).toBe("music"));
  expect(asked).toEqual(["cheat", "earned"]);
  window.removeEventListener("keydown", reached);
  unmount(view);
});
