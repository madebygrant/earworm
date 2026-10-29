<script lang="ts">
  import { pickQuery, screens } from "../lib/screens.svelte";
  import { app } from "../lib/store.svelte";
  import Icon from "./Icon.svelte";
  import Lit from "./Lit.svelte";
  import Wall from "./Wall.svelte";

  // Adding to a list: • stays, ✓ joins, − leaves, so no state rests on colour. The box always types.
  const p = $derived(screens.pick);
  let box = $state<HTMLInputElement>();
  $effect(() => {
    if (!screens.help && !screens.discard) box?.focus();
  });
  const glyph = (marked: boolean, held: boolean) => (held && marked ? "•" : held ? "−" : marked ? "✓" : "");
</script>

<section class="view walled">
  <div class="look">
    <div class="toolbar">
      <span class="sum">Add to <b>{p.name}</b>{p.onlyMarked ? ", marked only" : ""}</span>
      <div class="find on">
        <Icon name="filter" />
        <input bind:this={box} value={p.query} oninput={(e) => pickQuery(e.currentTarget.value)} placeholder="find tracks to add" spellcheck="false" />
        <span class="n">{p.existing ? `${p.held.length} in list · +${p.add} −${p.remove}` : `${p.marks.length} marked`} · {p.rows.length} of {p.total}</span>
      </div>
    </div>
    {#if p.rows.length}
      <Wall at={String(p.cursor)} list>
        {#snippet head()}<tr><th></th><th>track</th><th>folder</th></tr>{/snippet}
        {#each p.rows as r, i (`${r.folder}/${r.name}`)}
          <tr class:cur={i === p.cursor} class:on={r.marked} data-k={i} onclick={() => (screens.pick = { ...screens.pick, cursor: i })}>
            <td class="status">{glyph(r.marked, r.held)}</td>
            <td class="t"><Lit text={r.label} at={r.at} /></td>
            <td class="a"><Lit text={app.model.library[r.shelf]?.name ?? ""} at={r.folder_at} /></td>
          </tr>
        {/each}
      </Wall>
    {:else}
      <p class="none">
        {#if p.onlyMarked && !p.marks.length}Nothing is marked. F2 shows every track.
        {:else if p.onlyMarked}No marked track matches {p.query}. F2 shows every track.
        {:else if !p.total}No tracks in the library yet. {p.existing ? "Esc cancels." : "Esc keeps the list empty."}
        {:else}No track matches {p.query}. {p.existing ? "Esc cancels." : "Esc keeps the list empty."}{/if}
      </p>
    {/if}
  </div>
</section>
