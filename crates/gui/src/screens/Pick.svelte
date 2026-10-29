<script lang="ts">
  import { answerPick, app } from "../lib/store.svelte";
  import { names, ui } from "../lib/ui.svelte";
  import FilterBand from "./FilterBand.svelte";

  interface Props {
    press: (key: string) => void;
  }
  let { press }: Props = $props();

  // Always rows: the gate is a list of what to fetch, and a tile has nothing to show before the download.
  const m = $derived(app.model);
  const shown = $derived(ui.rows.map((at) => m.tracks[at]).filter(Boolean));
  const n = $derived(m.marked.length);
  let body = $state<HTMLElement>();
  const cursor = $derived(m.cursor);
  $effect(() => {
    body?.querySelector(`[data-i="${cursor}"]`)?.scrollIntoView({ block: "nearest" });
  });

  function note(status: string, twin: string | null): string {
    if (status === "Have") return "on disk";
    if (status === "Gone") return "left the playlist";
    return twin ? `≈ ${twin}` : "";
  }
</script>

<section class="view">
  <div class="band">
    <b>pick</b><span>{n} of {m.tracks.length} selected</span>
    <span class="end">space mark · a all · i invert · / filter · enter downloads · esc none</span>
  </div>
  <FilterBand />
  <div bind:this={body}>
    {#each shown as t (t.index)}
      {@const [title, artist] = names(t)}
      {@const on = m.marked.includes(t.index)}
      <div class="r pick" class:cur={t.index === m.cursor} class:off={!on} data-i={t.index} role="button" tabindex="-1"
        onclick={() => { app.model = { ...app.model, cursor: t.index }; press(" "); }} onkeydown={() => {}}>
        <span class="n">{t.index}</span><span class="box" class:on>[{on ? "x" : " "}]</span>
        <span class="t">{title} {#if artist}<span class="a">- {artist}</span>{/if}</span>
        <span class="src" class:twin={!!t.twin}>{note(t.status, t.twin)}</span>
      </div>
    {/each}
  </div>
  <div class="go">
    <button class="btn primary" onclick={() => press("Enter")}>Download {n} {n === 1 ? "track" : "tracks"}</button>
    <!-- Not Esc, which clears a filter first: the button says what it does. -->
    <button class="btn" onclick={() => answerPick([])}>Take none</button>
    <span class="muted">Esc takes none and still tags what's already in the folder.</span>
  </div>
</section>
