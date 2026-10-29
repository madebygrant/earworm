<script lang="ts">
  import { invoke } from "@tauri-apps/api/core";
  import type { Status } from "../lib/protocol";
  import { firstScreen } from "../lib/reducer";
  import { canGet, relatedRows, screens, topRows } from "../lib/screens.svelte";
  import { app } from "../lib/store.svelte";
  import { label, tone, ui, working } from "../lib/ui.svelte";

  const m = $derived(app.model);
  // Core's `view::counts`, asked again when the tracks change; a reply is kept only while it is the latest.
  let tally = $state<{ status: Status; n: number }[]>([]);
  const tracks = $derived(m.tracks);
  let asked = 0;
  $effect(() => {
    const id = ++asked;
    void invoke<[Status, number][] | null>("counts", { tracks }).then((counts) => {
      if (id === asked) tally = (counts ?? []).map(([status, n]) => ({ status, n }));
    });
  });
  const missing = $derived(m.library.reduce((n, s) => n + (s.kind === "Custom" ? 0 : s.missing), 0));
  const errors = $derived(m.logs.some((l) => l.includes("ERROR")));

  const first = $derived(firstScreen(m));
  const hints = $derived.by((): [string, string][] => {
    if (first) return [["enter", "sync"], ["⌘v", "paste"], ["esc", "quit"]];
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
    const keys: [string, string][] = [["w", "tiles/list"]];
    if (m.done !== null) keys.push(["e", "edit"], ["r", "retry"]);
    if (m.undoable) keys.push(["u", `undo ${m.undoable}`]);
    return keys;
  });
</script>

<footer class="ft">
  <span class="tally">
    {#if m.view === "Library" && m.picking === null}
      <span class="muted">{m.library.length} {m.library.length === 1 ? "folder" : "folders"}</span>{#if missing}<span class="warn">{missing} missing</span>{/if}
    {:else if m.view === "Artist" && screens.section === "Releases" && m.page.loaded}
      {@const shown = screens.releaseRows.map((at) => m.page.rows[at]).filter(Boolean)}
      {@const owned = shown.filter((r) => r.have > 0 && r.of !== null && r.have >= r.of).length}
      {@const none = shown.filter((r) => r.have === 0).length}
      <span class="good">{owned} owned</span><span class="warn">{shown.length - owned - none} partial</span><span class="muted">{none} missing</span>
    {:else if m.view === "Tracks" || m.picking !== null}
      {#if m.picking !== null}<span class="live">{m.marked.length} selected</span>{/if}
      {#each tally as t (t.status)}<span style:color={tone(t.status)}>{t.n} {label(t.status)}</span>{/each}
      {#if m.marked.length && m.picking === null}<span class="live">{m.marked.length} marked</span>{/if}
    {/if}
  </span>
  <span class="hints">
    {#if m.follow && working() && m.view === "Tracks"}<span>⟳ following</span>{/if}
    {#if !first && (m.view !== "Library" || m.logs.length)}<span><span class="k">l</span> <span class:hot={errors}>{errors ? "! log" : "log"}</span></span>{/if}
    {#each hints as [key, what] (key)}<span><span class="k">{key}</span> {what}</span>{/each}
    {#if !first && m.view !== "Pick"}<span><span class="k">q</span> quit</span>{/if}
  </span>
</footer>
