<script lang="ts">
  import { pickQuery, screens } from "../lib/screens.svelte";
  import { app } from "../lib/store.svelte";
  import Lit from "./Lit.svelte";

  // Adding to a list: • stays, ✓ joins, − leaves, so no state rests on colour. The box always types.
  const p = $derived(screens.pick);
  let box = $state<HTMLInputElement>();
  $effect(() => {
    if (!screens.help && !screens.discard) box?.focus();
  });
  let body = $state<HTMLElement>();
  const cursor = $derived(p.cursor);
  $effect(() => {
    body?.querySelector(`[data-i="${cursor}"]`)?.scrollIntoView({ block: "nearest" });
  });
  const glyph = (marked: boolean, held: boolean) => (held && marked ? "•" : held ? "−" : marked ? "✓" : "");
</script>

<section class="view">
  <div class="band">
    <b>add to {p.name}{p.onlyMarked ? " · marked only" : ""}</b>
    <input bind:this={box} value={p.query} oninput={(e) => pickQuery(e.currentTarget.value)} spellcheck="false" />
    <span class="end">{p.existing ? `${p.held.length} in list · +${p.add} −${p.remove}` : `${p.marks.length} marked`} · {p.rows.length} of {p.total}</span>
  </div>
  <div bind:this={body}>
    {#each p.rows as r, i (`${r.folder}/${r.name}`)}
      <div class="r pk" class:cur={i === p.cursor} data-i={i} role="button" tabindex="-1"
        onclick={() => (screens.pick = { ...screens.pick, cursor: i })} onkeydown={() => {}}>
        <span class="g">{glyph(r.marked, r.held)}</span>
        <span class="t" class:on={r.marked}><Lit text={r.label} at={r.at} /></span>
        <span class="src"><Lit text={app.model.library[r.shelf]?.name ?? ""} at={r.folder_at} /></span>
      </div>
    {/each}
  </div>
  {#if !p.rows.length}
    <p class="empty-main">
      {#if p.onlyMarked && !p.marks.length}Nothing is marked. F2 shows every track.
      {:else if p.onlyMarked}No marked track matches {p.query}. F2 shows every track.
      {:else if !p.total}No tracks in the library yet. {p.existing ? "Esc cancels." : "Esc keeps the list empty."}
      {:else}No track matches {p.query}. {p.existing ? "Esc cancels." : "Esc keeps the list empty."}{/if}
    </p>
  {/if}
</section>
