// @vitest-environment jsdom
// The link fields' own keys and paste live in the component, where the key handler's tests cannot reach.
import { flushSync, mount, tick, unmount } from "svelte";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import recorded from "../lib/msgs.json";
import type { Word } from "../lib/protocol";
import { initial } from "../lib/reducer";
import { screens } from "../lib/screens.svelte";
import { app } from "../lib/store.svelte";
import Queue from "./Queue.svelte";

const calls: [string, unknown][] = [];
vi.mock("@tauri-apps/api/core", () => ({
  invoke: async (cmd: string, args: unknown) => {
    calls.push([cmd, args]);
    if (cmd === "enqueue_fields") return { said: "queued", problems: (args as { fields: string[] }).fields.map(() => null) };
    return null;
  },
}));
globalThis.ResizeObserver = class { observe() {} disconnect() {} } as unknown as typeof ResizeObserver;
Element.prototype.scrollIntoView = () => {};

let view: ReturnType<typeof mount>;
const boxes = () => [...document.querySelectorAll<HTMLInputElement>(".adder input")];
const type = (box: HTMLInputElement, text: string) => {
  box.value = text;
  box.dispatchEvent(new Event("input", { bubbles: true }));
};
const press = (box: HTMLInputElement, key: string, extra: KeyboardEventInit = {}) =>
  box.dispatchEvent(new KeyboardEvent("keydown", { key, bubbles: true, cancelable: true, ...extra }));
// The focus request is served after a tick, then cleared.
const settle = async () => (flushSync(), await tick(), await tick(), flushSync());

beforeEach(() => {
  calls.length = 0;
  screens.fields = [{ text: "", why: null }];
  screens.focusField = null;
  app.model = { ...initial(recorded.statuses as Word[]), view: "Queue", queue: [], queueRev: 0 };
  view = mount(Queue, { target: document.body });
  flushSync();
});
afterEach(() => unmount(view));

it("Enter on a filled field opens a focused field below it, and on an empty one does nothing", async () => {
  press(boxes()[0], "Enter");
  await settle();
  expect(boxes()).toHaveLength(1);
  type(boxes()[0], "https://youtu.be/aaaaaaaaaaa");
  press(boxes()[0], "Enter");
  await settle();
  expect(boxes()).toHaveLength(2);
  expect(document.activeElement).toBe(boxes()[1]);
});

it("Backspace on an empty field drops it and focuses the one before, and Esc leaves the field", async () => {
  type(boxes()[0], "https://youtu.be/aaaaaaaaaaa");
  press(boxes()[0], "Enter");
  await settle();
  press(boxes()[1], "Backspace");
  await settle();
  expect(boxes()).toHaveLength(1);
  expect(document.activeElement).toBe(boxes()[0]);
  press(boxes()[0], "Escape");
  expect(document.activeElement).toBe(document.body);
});

it("⌘ or ^ Enter queues every field", async () => {
  type(boxes()[0], "https://youtu.be/aaaaaaaaaaa");
  press(boxes()[0], "Enter", { metaKey: true });
  await settle();
  expect(calls).toEqual([["enqueue_fields", { fields: ["https://youtu.be/aaaaaaaaaaa"] }]]);
});

it("several lines pasted into a field become a field each, replacing what was selected", async () => {
  const box = boxes()[0];
  type(box, "keep REPLACE");
  box.setSelectionRange(5, 12);
  const paste = new Event("paste", { bubbles: true, cancelable: true });
  Object.assign(paste, { clipboardData: { getData: () => "https://youtu.be/a\n\nhttps://youtu.be/b\n" } });
  box.dispatchEvent(paste);
  await settle();
  expect(boxes().map((b) => b.value)).toEqual(["keep https://youtu.be/a", "https://youtu.be/b"]);
});
