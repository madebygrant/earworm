<script lang="ts">
  import { convertFileSrc } from "@tauri-apps/api/core";
  import { untrack } from "svelte";
  import { press } from "../lib/keys";
  import type { Track } from "../lib/protocol";
  import { app } from "../lib/store.svelte";
  import { askThumbs, label, names, tone, ui, word, working } from "../lib/ui.svelte";
  import FilterBox from "./FilterBox.svelte";
  import Icon from "./Icon.svelte";
  import TrackRow from "./TrackRow.svelte";
  import Wall from "./Wall.svelte";

  const m = $derived(app.model);
  const shown = $derived(ui.rows.map((at) => m.tracks[at]).filter(Boolean));
  const slot = (t: Track) => word(t.status)?.slot ?? "text";
  const thumb = (t: Track) => (t.path ? (m.thumbs[t.path] ?? null) : null);

  function pick(t: Track) {
    app.model = { ...app.model, cursor: t.index, follow: false };
  }

  // Covers once nothing else is running: the worker serves these between commands, never during a run.
  const tracks = $derived(m.tracks);
  const done = $derived(m.done !== null && !working());
  const artwork = $derived(m.artwork);
  $effect(() => {
    void artwork;
    if (!done) return;
    const want = tracks.filter((t) => t.path && word(t.status)?.settled).map((t) => t.path!);
    untrack(() => askThumbs(want));
  });

  const narrowed = $derived(!!ui.filter || m.review);
  const errorLine = (line: string) => (line.includes("ERROR") ? "error" : line.includes("WARNING") ? "warn" : "");
  const pad = (n: number) => String(n).padStart(2, "0");
</script>

<section class="view walled">
  <div class="look">
    <div class="toolbar">
      <div class="seg3" role="group" aria-label="review">
        <button class:on={!m.review} title="all" aria-pressed={!m.review} disabled={ui.typing} onclick={() => m.review && press("v")}><Icon name="all" /></button>
        <button class:on={m.review} title="worth a look" aria-pressed={m.review} disabled={ui.typing} onclick={() => !m.review && press("v")}><Icon name="missing" /></button>
      </div>
      <div class="seg3" role="group" aria-label="view">
        <button class:on={ui.mode === "tiles"} title="tiles" aria-pressed={ui.mode === "tiles"} onclick={() => (ui.mode = "tiles")}><Icon name="library" /></button>
        <button class:on={ui.mode === "list"} title="list" aria-pressed={ui.mode === "list"} onclick={() => (ui.mode = "list")}><Icon name="list" /></button>
      </div>
      <div class="seg3" role="group" aria-label="next">
        <button title="next unconfirmed" disabled={ui.typing} onclick={() => press("n")}><Icon name="open" /></button>
      </div>
      <FilterBox placeholder="filter tracks" count={narrowed ? `${shown.length} of ${m.tracks.length}` : ""} />
    </div>
    {#if shown.length}
      <Wall at={String(m.cursor ?? "")} list={ui.mode === "list"}>
        {#snippet head()}<tr><th>#</th><th></th><th>track</th><th>confidence</th><th>status</th><th>source</th><th>length</th></tr>{/snippet}
        {#each shown as t (t.index)}
          {@const [title, artist] = names(t)}
          {@const marked = m.marked.includes(t.index)}
          {#if ui.mode === "list"}
            <TrackRow num={t.index} word={label(t.status)} color={tone(t.status)} {title} {artist} source={t.source}
              score={t.score} duration={t.duration} thumb={thumb(t)} cur={t.index === m.cursor} {marked} onpick={() => pick(t)} />
          {:else}
            {@const art = thumb(t)}
            <div class="s" class:empty={!art} class:error={slot(t) === "error"} class:gone={t.status === "Gone"} class:cur={t.index === m.cursor}
              data-k={t.index} role="button" tabindex="-1" onclick={() => pick(t)} onkeydown={() => {}}>
              {#if art}<img src={convertFileSrc(art)} alt="" />{:else}{slot(t) === "error" ? "!" : pad(t.index)}{/if}
              <span class="st {slot(t)}">{label(t.status)}</span>
              {#if marked}<span class="kind" title="marked"><Icon name="mark" /></span>{/if}
              <div class="cap">{title}<small>{artist}</small></div>
            </div>
          {/if}
        {/each}
      </Wall>
    {:else}
      <p class="none">Nothing here matches. Esc clears it.</p>
    {/if}
  </div>
  {#if ui.logs}
    <div class="log" class:tall={ui.logsTall}>
      {#each m.logs as line, i (i)}<div class={errorLine(line)}>{line}</div>{/each}
    </div>
  {/if}
</section>
