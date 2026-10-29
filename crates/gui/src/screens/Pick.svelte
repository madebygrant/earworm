<script lang="ts">
  import { answerPick, app } from "../lib/store.svelte";
  import { names, ui } from "../lib/ui.svelte";
  import FilterBox from "./FilterBox.svelte";
  import Icon from "./Icon.svelte";
  import Wall from "./Wall.svelte";

  // The pick gate's list; Download and Take none are on the sheet beside it, and in the toolbar once the
  // window is too narrow for the sheet. Always rows: a tile has nothing to show before the download.
  const m = $derived(app.model);
  const shown = $derived(ui.rows.map((at) => m.tracks[at]).filter(Boolean));
  const cursor = $derived(m.cursor);
  const marked = $derived(m.marked);

  function note(status: string, twin: string | null): string {
    if (status === "Have") return "on disk";
    if (status === "Gone") return "left the playlist";
    return twin ? `≈ ${twin}` : "";
  }
  interface Props {
    press: (key: string) => void;
  }
  let { press }: Props = $props();
</script>

<section class="view walled">
  <div class="look">
    <div class="toolbar">
      <span class="sum"><b>{marked.length}</b> of {m.tracks.length} selected</span>
      <button class="go" onclick={() => press("Enter")}><Icon name="open" />Download {marked.length}</button>
      <button class="go plain" onclick={() => answerPick([])}>Take none</button>
      <FilterBox placeholder="filter tracks" bind:value={ui.filter} typing={ui.typing} count={ui.filter ? `${shown.length} of ${m.tracks.length}` : ""} />
    </div>
    <Wall at={String(cursor ?? "")} list>
      {#snippet head()}<tr><th></th><th>#</th><th>track</th><th></th></tr>{/snippet}
      {#each shown as t (t.index)}
        {@const [title, artist] = names(t)}
        {@const on = marked.includes(t.index)}
        <tr class:cur={t.index === cursor} class:off={!on} data-k={t.index}
          onclick={() => { app.model = { ...app.model, cursor: t.index }; press(" "); }}>
          <td><span class="tick" class:on>{#if on}<Icon name="mark" />{/if}</span></td>
          <td class="a">{t.index}</td>
          <td><div class="t">{title}</div>{#if artist}<div class="a">{artist}</div>{/if}</td>
          <td class="a" class:twin={!!t.twin}>{note(t.status, t.twin)}</td>
        </tr>
      {/each}
    </Wall>
  </div>
</section>
