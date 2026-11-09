// @vitest-environment jsdom
// The queue screen's keys go to the shell, which owns the queue: the page only asks and draws what comes back.
import { beforeEach, expect, it, vi } from "vitest";

const calls: [string, unknown][] = [];
vi.mock("@tauri-apps/api/core", () => ({
  invoke: async (cmd: string, args: unknown) => {
    calls.push([cmd, args]);
    if (cmd === "unqueue" || cmd === "clear_queue") return true;
    if (cmd === "enqueue") return "queued 2";
    if (cmd === "enqueue_fields") return { said: "queued 1", problems: (args as { fields: string[] }).fields.map((f) => (f.includes("youtube") ? null : "not a link")) };
    if (cmd === "paste_links") return (args as { text: string }).text.includes("\n") ? { lead: "a", said: "queued 1" } : { lead: null, said: null };
    return null;
  },
}));

const key = (k: string, extra: Partial<KeyboardEvent> = {}) =>
  ({ key: k, ctrlKey: false, metaKey: false, altKey: false, target: null, preventDefault: vi.fn(), ...extra }) as unknown as KeyboardEvent;

// The quit question and the clearing question are page state the tests share, and either takes every key.
beforeEach(async () => {
  calls.length = 0;
  (await import("./ui.svelte")).ui.quitting = false;
  (await import("./screens.svelte")).screens.clearing = false;
});

it("Q opens the queue from the library, x removes the link under the cursor, Esc goes back", async () => {
  const { app } = await import("./store.svelte");
  const { handleKey } = await import("./keys");
  app.model = { ...app.model, view: "Library", prompt: null, queue: ["a", "b"], queueRev: 1 };
  await handleKey(key("Q"));
  expect(app.model.view).toBe("Queue");
  await handleKey(key("j"));
  await handleKey(key("x"));
  expect(calls).toContainEqual(["unqueue", { link: "b" }]);
  await handleKey(key("Escape"));
  expect(app.model.view).toBe("Library");
});

it("X asks first, and only y or Enter empties the queue", async () => {
  const { app } = await import("./store.svelte");
  const { screens } = await import("./screens.svelte");
  const { handleKey } = await import("./keys");
  app.model = { ...app.model, view: "Queue", prompt: null, queue: ["a"], queueRev: 1 };
  await handleKey(key("X"));
  expect(screens.clearing).toBe(true);
  await handleKey(key("q"));
  expect(screens.clearing).toBe(false);
  expect(calls.map(([c]) => c)).not.toContain("clear_queue");
  await handleKey(key("X"));
  await handleKey(key("y"));
  await Promise.resolve();
  expect(calls.map(([c]) => c)).toContain("clear_queue");
});

it("q asks before quitting while links wait", async () => {
  const { app } = await import("./store.svelte");
  const { ui } = await import("./ui.svelte");
  const { handleKey } = await import("./keys");
  app.model = { ...app.model, view: "Library", prompt: null, busy: false, tracks: [], queue: ["a"], queueRev: 1 };
  await handleKey(key("q"));
  expect(ui.quitting).toBe(true);
  expect(calls.map(([c]) => c)).not.toContain("quit");
});

// The worker takes links while the screen is up, so x acts on the row drawn, not one past the end.
it("x takes the last row after the queue shrank under the cursor", async () => {
  const { app } = await import("./store.svelte");
  const { screens } = await import("./screens.svelte");
  const { handleKey } = await import("./keys");
  app.model = { ...app.model, view: "Queue", prompt: null, picking: null, queue: ["a", "b", "c"], queueRev: 1 };
  screens.queueAt = 2;
  app.model = { ...app.model, queue: ["a", "b"], queueRev: 2 };
  await handleKey(key("x"));
  expect(calls).toContainEqual(["unqueue", { link: "b" }]);
});

// The pick gate opened under the queue screen: its keys must reach it, not the queue's.
it("the queue's keys stand aside for a pick gate", async () => {
  const { app } = await import("./store.svelte");
  const { handleKey } = await import("./keys");
  app.model = { ...app.model, view: "Queue", prompt: null, picking: 7, queue: ["a"], queueRev: 1 };
  await handleKey(key("x"));
  expect(calls.map(([c]) => c)).not.toContain("unqueue");
  app.model = { ...app.model, picking: null };
});

// Drawn by App, so a link starting under it cannot leave the question's keys live with nothing showing.
it("the empty-queue question keeps its keys on whatever screen is up", async () => {
  const { app } = await import("./store.svelte");
  const { screens } = await import("./screens.svelte");
  const { handleKey } = await import("./keys");
  app.model = { ...app.model, view: "Tracks", prompt: null, queue: ["a"], queueRev: 1 };
  screens.clearing = true;
  await handleKey(key("Escape"));
  expect(screens.clearing).toBe(false);
  expect(calls.map(([c]) => c)).not.toContain("clear_queue");
});

const pasted = (text: string, target: EventTarget | null = null) => {
  const e = { clipboardData: { getData: () => text }, preventDefault: vi.fn(), target, currentTarget: target } as unknown as ClipboardEvent;
  return e;
};
const urlPrompt = { Input: { header: "YouTube playlist URL", note: "", value: "", escape: "Keep" as const, browse: null, links: true } };

it("several links pasted at the URL field go to the shell with the question's id, which answers it", async () => {
  const { app } = await import("./store.svelte");
  const { pasteLinks } = await import("./screens.svelte");
  app.model = { ...app.model, prompt: { id: 41, prompt: urlPrompt } };
  const box = Object.assign(document.createElement("input"), { value: "" });
  const set = vi.fn();
  let answered = false;
  await pasteLinks(pasted("https://youtu.be/a\nhttps://youtu.be/b", box), set, () => (answered = true));
  expect(calls).toContainEqual(["paste_links", { text: "https://youtu.be/a\nhttps://youtu.be/b", id: 41 }]);
  expect(app.model.prompt).toBe(null);
  expect(answered).toBe(true);
  expect(set).not.toHaveBeenCalled();
});

it("one link pasted at the URL field goes in at the caret, on one line", async () => {
  const { app } = await import("./store.svelte");
  const { pasteLinks } = await import("./screens.svelte");
  app.model = { ...app.model, prompt: { id: 42, prompt: urlPrompt } };
  const box = document.createElement("input");
  box.value = "ab";
  box.setSelectionRange(1, 1);
  const set = vi.fn();
  await pasteLinks(pasted(" https://youtu.be/a ", box), set);
  expect(set).toHaveBeenCalledWith("ahttps://youtu.be/ab");
  expect(app.model.prompt?.id).toBe(42);
});

it("a paste outside a box is queued, and a box or the console keeps its own", async () => {
  const { windowPaste, consoleHooks } = await import("./keys");
  windowPaste(pasted("https://youtu.be/a", document.createElement("input")));
  consoleHooks.isOpen = () => true;
  windowPaste(pasted("https://youtu.be/a", document.body));
  expect(calls.map(([c]) => c)).not.toContain("enqueue");
  consoleHooks.isOpen = () => false;
  windowPaste(pasted("https://youtu.be/a", document.body));
  expect(calls).toContainEqual(["enqueue", { text: "https://youtu.be/a", lead: null }]);
});

// Opening the queue sends no command, so a busy library still opens it, as the terminal's does.
it("Q opens the queue from a busy library", async () => {
  const { app } = await import("./store.svelte");
  const { handleKey } = await import("./keys");
  app.model = { ...app.model, view: "Library", prompt: null, picking: null, busy: true, queue: ["a"], queueRev: 1 };
  await handleKey(key("Q"));
  expect(app.model.view).toBe("Queue");
  app.model = { ...app.model, busy: false };
});

it("a link field keeps its letters, and n on the queue focuses the last field", async () => {
  const { app } = await import("./store.svelte");
  const { screens } = await import("./screens.svelte");
  const { ui } = await import("./ui.svelte");
  const { handleKey } = await import("./keys");
  app.model = { ...app.model, view: "Queue", prompt: null, queue: ["a"], queueRev: 1 };
  const target = document.createElement("input");
  for (const k of ["x", "X", "q", ",", "Q", "Escape"]) await handleKey(key(k, { target }));
  expect(calls).toEqual([]);
  expect(ui.quitting).toBe(false);
  expect(screens.clearing).toBe(false);
  expect(app.model.view).toBe("Queue");
  screens.fields = [{ text: "", why: null }, { text: "", why: null }];
  await handleKey(key("n"));
  expect(screens.focusField).toBe(1);
});

it("the link fields go to the shell together, and only those it refused stay, with why", async () => {
  const { screens, queueFields } = await import("./screens.svelte");
  screens.fields = [
    { text: " https://youtube.com/playlist?list=a ", why: null },
    { text: "", why: null },
    { text: "hello", why: null },
  ];
  await queueFields();
  expect(calls).toEqual([["enqueue_fields", { fields: ["https://youtube.com/playlist?list=a", "hello"] }]]);
  expect(screens.fields).toEqual([{ text: "hello", why: "not a link" }]);
  screens.fields = [{ text: "https://youtube.com/watch?v=b", why: null }];
  await queueFields();
  expect(screens.fields).toEqual([{ text: "", why: null }]);
});

it("a field typed in while the shell answered keeps what was typed, and focus waits only on the queue screen", async () => {
  const { app } = await import("./store.svelte");
  const { screens, queueFields } = await import("./screens.svelte");
  app.model = { ...app.model, view: "Tracks" };
  screens.focusField = null;
  screens.fields = [{ text: "https://youtube.com/watch?v=a", why: null }, { text: "", why: null }];
  const going = queueFields();
  screens.fields[0].text = "https://youtube.com/watch?v=a2";
  screens.fields[1].text = "https://youtube.com/watch?v=b";
  await going;
  expect(screens.fields.map((f) => f.text)).toEqual(["https://youtube.com/watch?v=a2", "https://youtube.com/watch?v=b"]);
  expect(screens.focusField).toBe(null);
});
