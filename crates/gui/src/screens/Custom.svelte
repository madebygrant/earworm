<script lang="ts">
  import { app } from "../lib/store.svelte";
  import Wall from "./Wall.svelte";

  // A list built here, in its own order. A missing file keeps its row with a !, so nothing quietly leaves
  // the list; the folder is muted because it is where Enter goes.
  const c = $derived(app.model.custom);
  const focus = $derived(c.focus ?? c.rows[0]?.id ?? null);
  const folders = $derived(new Set(c.rows.map((r) => r.folder)).size);
  const missing = $derived(c.rows.filter((r) => !r.path).length);
</script>

<section class="view walled">
  <div class="look">
    <div class="toolbar">
      <span class="sum">{c.rows.length} {c.rows.length === 1 ? "track" : "tracks"} from {folders} {folders === 1 ? "folder" : "folders"}{#if missing}, <span class="warn">{missing} missing</span>{/if}</span>
    </div>
    {#if c.rows.length}
      <Wall at={focus ?? ""} list>
        {#snippet head()}<tr><th>#</th><th></th><th>track</th><th>folder</th></tr>{/snippet}
        <!-- The label whole, as the terminal draws it: it is display text, never taken apart. -->
        {#each c.rows as r, i (r.id)}
          <tr class:cur={r.id === focus} data-k={r.id} onclick={() => (app.model = { ...app.model, custom: { ...c, focus: r.id } })}>
            <td class="a">{i + 1}</td><td>{#if !r.path}<span class="warn" title="not on disk">!</span>{/if}</td>
            <td class="t">{r.label}</td><td class="a">{r.folder}</td>
          </tr>
        {/each}
      </Wall>
    {:else}
      <p class="none">Nothing here yet. P picks tracks, and P on a search result adds one. Esc goes back.</p>
    {/if}
  </div>
</section>
