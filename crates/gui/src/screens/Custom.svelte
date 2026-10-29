<script lang="ts">
  import { app } from "../lib/store.svelte";

  // A list built here, in its own order. A missing file keeps its row with a !, so nothing quietly leaves
  // the list; the folder is muted because it is where Enter goes.
  const c = $derived(app.model.custom);
  const focus = $derived(c.focus ?? c.rows[0]?.id ?? null);
  const folders = $derived(new Set(c.rows.map((r) => r.folder)).size);
  const missing = $derived(c.rows.filter((r) => !r.path).length);
  let body = $state<HTMLElement>();
  $effect(() => {
    body?.querySelector(`[data-id="${CSS.escape(focus ?? "")}"]`)?.scrollIntoView({ block: "nearest" });
  });
</script>

<section class="view">
  <div class="bar">
    <span class="title" style="color:var(--text)">{c.name}</span>
    <span>{c.rows.length} {c.rows.length === 1 ? "track" : "tracks"} from {folders} {folders === 1 ? "folder" : "folders"}{#if missing}, {missing} missing{/if}</span>
  </div>
  <div bind:this={body}>
    <!-- The label whole, as the terminal draws it: it is display text, never taken apart. -->
    {#each c.rows as r, i (r.id)}
      <div class="r list" class:cur={r.id === focus} data-id={r.id} role="button" tabindex="-1"
        onclick={() => (app.model = { ...app.model, custom: { ...c, focus: r.id } })} onkeydown={() => {}}>
        <span class="n">{i + 1}</span><span class="miss">{r.path ? "" : "!"}</span>
        <span class="t">{r.label}</span><span class="src">{r.folder}</span>
      </div>
    {/each}
  </div>
  {#if !c.rows.length}<p class="empty-main">Nothing here yet. P picks tracks, and P on a search result adds one. Esc goes back.</p>{/if}
</section>
