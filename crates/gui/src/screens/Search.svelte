<script lang="ts">
  import { screens } from "../lib/screens.svelte";
  import { app } from "../lib/store.svelte";
  import { ui } from "../lib/ui.svelte";
  import FilterBox from "./FilterBox.svelte";
  import Icon from "./Icon.svelte";
  import Lit from "./Lit.svelte";
  import Wall from "./Wall.svelte";

  // Every track in the library matching the box, flat, with the folder each is in: core's search, ranked.
  const m = $derived(app.model);
  const at = $derived(screens.found ? `${screens.found[0]}\u0000${screens.found[1]}` : "");
  const marked = (folder: string, file: string) => m.foundMarks.some(([f, n]) => f === folder && n === file);
</script>

<section class="view walled">
  <div class="look">
    <div class="toolbar">
      <FilterBox placeholder="find a track" bind:value={ui.filter} typing={ui.typing}
        count={`${screens.hits.length} of ${screens.foundTotal}${m.foundMarks.length ? ` · ${m.foundMarks.length} marked` : ""}`} />
    </div>
    {#if screens.hits.length}
      <Wall {at} list>
        {#snippet head()}<tr><th></th><th>track</th><th>folder</th></tr>{/snippet}
        {#each screens.hits as h (`${h.shelf}:${h.file}`)}
          {@const shelf = m.library[h.shelf]}
          {@const file = shelf?.files[h.file]}
          {#if shelf && file}
            {@const key = `${shelf.path}\u0000${file[0]}`}
            <tr class:cur={key === at} data-k={key} onclick={() => (screens.found = [shelf.path, file[0]])}>
              <td class="a">{#if marked(shelf.path, file[0])}<span title="marked"><Icon name="mark" /></span>{/if}{#if !file[1]}<span class="warn" title="not on disk">!</span>{/if}</td>
              <td class="t"><Lit text={h.label} at={h.at} /></td>
              <td class="a"><Lit text={shelf.name} at={h.folder_at} /></td>
            </tr>
          {/if}
        {/each}
      </Wall>
    {:else}
      <p class="none">{ui.filter.trim() ? `No track matches ${ui.filter}. Esc goes back to the library.` : "Type to find a track in any folder."}</p>
    {/if}
  </div>
</section>
