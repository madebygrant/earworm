// The bar's hints per screen, a key and a word each. Every key is one the screen's `h` sheet lists.
import { firstScreen, type State } from "./reducer";
import { canGet, relatedRows, screens, topRows } from "./screens.svelte";
import { app } from "./store.svelte";
import { running, working } from "./ui.svelte";

const PASTE = typeof navigator !== "undefined" && navigator.platform.startsWith("Mac") ? "⌘v" : "^v";

export function hints(m: State): [string, string][] {
  if (firstScreen(m)) return [["enter", "sync"], [PASTE, "paste"], ["esc", "quit"]];
  const keys = screenKeys(m);
  // The picker types `l` into its query.
  if (m.view !== "Pick" && (m.view !== "Library" || m.logs.length)) keys.unshift(["l", m.logs.some((l) => l.includes("ERROR")) ? "! log" : "log"]);
  if (m.view !== "Pick") keys.push(["q", "quit"]);
  return keys;
}

function screenKeys(m: State): [string, string][] {
  if (m.picking !== null) return [["space", "mark"], ["m", "same status"], ["a", "all"], ["i", "invert"], ["enter", "download"], ["esc", "none"]];
  if (m.view === "Library") return [["enter", "open"], ["n", "new link"], ["N", "new custom playlist"], ["t", "find a track"], ["S", "sync"], ["b", "artists"], [",", "settings"], ["h", "keys"]];
  if (m.view === "Found") return [["enter", "open its folder"], ["tab", "mark"], ["P", "add to playlist"], ["/", "edit"], ["esc", "library"]];
  if (m.view === "Playlist") return [["enter", "go to source"], ["P", "add tracks"], ["J K", "move"], ["x", "remove"], ["esc", "library"]];
  if (m.view === "Pick") {
    const p = screens.pick;
    const enter = p.existing ? (p.add + p.remove ? `+${p.add} −${p.remove}` : "no changes") : p.marks.length ? `add ${p.marks.length}` : "add";
    return [["enter", enter], ["tab", "mark"], ["^a", "all"], ["esc", p.existing ? "cancel" : "keep empty"], ["F2", "marked"], ["F1", "keys"]];
  }
  if (m.view === "Artists") return [["enter", "their page"], ["o", "their folders"], ["*", "star"], ["/", "filter"], ["esc", "library"]];
  if (m.view === "Artist") {
    if (m.searching) return [["esc", "stop the search"]];
    const keys: [string, string][] = [];
    // Offered only when Enter would fetch now: locked, busy or complete, it is not advertised.
    if (screens.section === "Releases" && canGet()) keys.push(["enter", "get"]);
    const missing = topRows().filter((t) => !t.have).length;
    if (screens.section === "Top" && missing && app.music && !m.busy) keys.push(["enter", `get the ${missing} missing`]);
    if (screens.section === "Related" && relatedRows().length && !m.busy) keys.push(["enter", "open"]);
    keys.push(["tab", screens.section === "Releases" ? "top tracks" : screens.section === "Top" ? "related" : "releases"]);
    // Only when it would change something: nothing hidden is a dead key.
    const hidden = m.page.rows.length - screens.releaseRows.length;
    if (screens.section === "Releases" && screens.allKinds) keys.push(["a", "albums and EPs"]);
    else if (screens.section === "Releases" && hidden > 0) keys.push(["a", `show ${hidden} more`]);
    keys.push(["o", "their folders"]);
    keys.push(["esc", screens.back.length ? "back" : "artists"]);
    return keys;
  }
  if (m.view === "Settings") return [["enter", "change"], ["esc", "back"]];
  // While it runs the screen is for watching: the cursor can follow it, and fixing waits for the end.
  const f: [string, string] = ["f", m.follow ? "stop following" : "follow"];
  if (running()) return [f, ["/", "filter"], ["v", m.review ? "all" : "worth a look"]];
  const keys: [string, string][] = [["w", "tiles/list"], ["e", "edit"], ["r", "retry"]];
  if (working()) keys.unshift(f);
  if (m.undoable) keys.push(["u", `undo ${m.undoable}`]);
  return keys;
}
