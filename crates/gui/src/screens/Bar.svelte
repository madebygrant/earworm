<script lang="ts">
  import { invoke } from "@tauri-apps/api/core";
  import { hints } from "../lib/bar";
  import type { Status } from "../lib/protocol";
  import { settledCount } from "../lib/reducer";
  import { screens } from "../lib/screens.svelte";
  import { app } from "../lib/store.svelte";
  import { label, tone, working } from "../lib/ui.svelte";

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
  const keys = $derived(hints(m));
  // The run's progress line, while there is a run to measure.
  const progress = $derived(m.view === "Tracks" && tracks.length && m.done === null && m.picking === null ? settledCount(m) / tracks.length : null);
</script>

<div class="look">
  <footer class="bar">
    <span class="hints">
      {#if m.follow && working() && m.view === "Tracks"}<span>⟳ following</span>{/if}
      {#each keys as [key, what] (key)}<span class:hot={what === "! log"}><kbd>{key}</kbd>{what}</span>{/each}
    </span>
    {#if progress !== null}<span class="progress" role="progressbar" aria-valuenow={Math.round(progress * 100)}><i style:width="{progress * 100}%"></i></span>{/if}
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
  </footer>
</div>
