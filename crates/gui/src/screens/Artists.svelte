<script lang="ts">
  import { artistSelected, screens } from "../lib/screens.svelte";
  import { app } from "../lib/store.svelte";

  // The artist under the rail's cursor, before their page is opened: what the library holds of them.
  const row = $derived(artistSelected());
  const m = $derived(app.model);
  const fresh = $derived(row ? (screens.artistNew[m.artists.rows.indexOf(row)] ?? []) : []);
</script>

<section class="view">
  {#if row}
    <div class="bar"><span class="title" style="color:var(--text)">{row.name}</span><span>{row.starred ? "starred" : ""}</span></div>
    <dl class="kv">
      <dt>tracks</dt><dd>{row.tracks}</dd>
      <dt>albums</dt><dd>{row.albums}</dd>
      <dt>folders</dt><dd>{row.folders.length}</dd>
      {#if fresh.length}<dt>new</dt><dd class="live">{fresh.join(", ")}</dd>{/if}
    </dl>
    <p class="sub" style="margin-top:16px">Enter opens their releases, top tracks and related artists. o narrows the library to their folders.</p>
  {:else if !m.artists.loaded}
    <p class="empty-main">Reading the artists off the tags.</p>
  {:else}
    <p class="empty-main">No artist matches. Esc clears the filter.</p>
  {/if}
</section>
