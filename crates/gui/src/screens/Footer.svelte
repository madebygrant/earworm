<script lang="ts">
  import { invoke } from "@tauri-apps/api/core";
  import type { Status } from "../lib/protocol";
  import { firstScreen } from "../lib/reducer";
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
    if (m.view === "Library") return [["enter", "open"], ["S", "sync"], ["R", "sync all"], ["n", "new link"], ["w", "tiles/list"]];
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
    {:else}
      {#if m.picking !== null}<span class="live">{m.marked.length} selected</span>{/if}
      {#each tally as t (t.status)}<span style:color={tone(t.status)}>{t.n} {label(t.status)}</span>{/each}
      {#if m.marked.length && m.picking === null}<span class="live">{m.marked.length} marked</span>{/if}
    {/if}
  </span>
  <span class="hints">
    {#if m.follow && working() && m.view === "Tracks"}<span>⟳ following</span>{/if}
    {#if !first && (m.view !== "Library" || m.logs.length)}<span><span class="k">l</span> <span class:hot={errors}>{errors ? "! log" : "log"}</span></span>{/if}
    {#each hints as [key, what] (key)}<span><span class="k">{key}</span> {what}</span>{/each}
    {#if !first}<span><span class="k">q</span> quit</span>{/if}
  </span>
</footer>
