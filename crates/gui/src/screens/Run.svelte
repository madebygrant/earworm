<script lang="ts">
  import type { Track } from "../lib/protocol";
  import { settledCount } from "../lib/reducer";
  import { app } from "../lib/store.svelte";
  import { askThumbs, label, live, names, tone, ui, word, working } from "../lib/ui.svelte";
  import FilterBand from "./FilterBand.svelte";
  import Tile from "./Tile.svelte";
  import TrackRow from "./TrackRow.svelte";

  const m = $derived(app.model);
  const shown = $derived(ui.rows.map((at) => m.tracks[at]).filter(Boolean));
  const settled = $derived(settledCount(m));
  let body = $state<HTMLElement>();

  function frame(t: Track): "" | "queued" | "live" | "failed" | "missing" | "gone" {
    if (t.status === "Pending") return "queued";
    if (t.status === "Failed") return "failed";
    if (t.status === "Gone") return "gone";
    return live(t.status) ? "live" : "";
  }

  function pick(t: Track) {
    app.model = { ...app.model, cursor: t.index, follow: false };
  }

  // Covers once nothing else is running: the worker serves these between commands, never during a run.
  $effect(() => {
    if (working() || m.done === null) return;
    const want = m.tracks.filter((t) => t.path && word(t.status)?.settled && !(t.path in m.thumbs)).map((t) => t.path!);
    askThumbs(want);
  });

  // The cursor stays on screen, whether a key moved it or following did. Only when it moves, or every
  // message would snap the list back while someone scrolls it.
  const cursor = $derived(m.cursor);
  $effect(() => {
    const at = cursor;
    body?.querySelector(`[data-i="${at}"]`)?.scrollIntoView({ block: "nearest" });
  });

  function bar(width: number): [string, string] {
    const fill = m.tracks.length ? Math.round((settled / m.tracks.length) * width) : 0;
    return ["█".repeat(fill), "░".repeat(width - fill)];
  }
  const errorLine = (line: string) => (line.includes("ERROR") ? "error" : line.includes("WARNING") ? "warn" : "");
</script>

<section class="view run">
  <div class="scroll">
    <div class="bar">
      {#if working()}
        {@const [done, left] = bar(28)}
        <span class="prog">[<b>{done}</b>{left}]</span><span>{settled} of {m.tracks.length}</span>
      {:else}
        <span class="title" style="color:var(--text)">{m.playlist}</span><span>{m.stage}</span>
      {/if}
      <span class="mode" role="group" aria-label="View, w">
        <button aria-pressed={ui.mode === "tiles"} onclick={() => (ui.mode = "tiles")}>tiles</button>
        <button aria-pressed={ui.mode === "list"} onclick={() => (ui.mode = "list")}>list</button>
      </span>
    </div>
    <FilterBand />
    <div class={ui.mode === "tiles" ? "sheet" : ""} bind:this={body}>
      {#each shown as t (t.index)}
        {@const [title, artist] = names(t)}
        <div data-i={t.index} style="display:contents">
          {#if ui.mode === "tiles"}
            <Tile num={t.index} word={label(t.status)} color={tone(t.status)} {title} {artist} frame={frame(t)}
              thumb={t.path ? m.thumbs[t.path] : null} cur={t.index === m.cursor} marked={m.marked.includes(t.index)} onpick={() => pick(t)} />
          {:else}
            <TrackRow num={t.index} word={label(t.status)} color={tone(t.status)} {title} {artist} source={t.source}
              cur={t.index === m.cursor} marked={m.marked.includes(t.index)} onpick={() => pick(t)} />
          {/if}
        </div>
      {/each}
    </div>
    {#if !shown.length && m.tracks.length}<p class="empty-main">Nothing here matches. Esc clears it.</p>{/if}
  </div>
  {#if ui.logs}
    <div class="log" class:tall={ui.logsTall}>
      {#each m.logs as line, i (i)}<div class={errorLine(line)}>{line}</div>{/each}
    </div>
  {/if}
</section>
