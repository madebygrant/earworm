<script lang="ts">
  import { press } from "../lib/keys";
  import { screens } from "../lib/screens.svelte";
  import { app } from "../lib/store.svelte";
  import FilterBox from "./FilterBox.svelte";
  import Wall from "./Wall.svelte";

  // In core's order, starred first. A table rather than a wall: nothing here has a picture to tile.
  const m = $derived(app.model);
  // With each row's position, which is what core's new releases are kept by.
  const rows = $derived(screens.artistRows.map((at) => [at, m.artists.rows[at]] as const).filter(([, a]) => a));
</script>

<section class="view walled">
  <div class="look">
    <div class="toolbar">
      <FilterBox placeholder="filter artists" bind:value={screens.artistQuery} typing={screens.artistTyping} count={screens.artistQuery ? `${rows.length} of ${m.artists.rows.length}` : ""} />
    </div>
    {#if rows.length}
      <Wall at={screens.artist ?? ""} list={true}>
        {#snippet head()}<tr><th></th><th>artist</th><th>tracks</th><th>albums</th><th></th></tr>{/snippet}
        {#each rows as [at, a] (a.name)}
          {@const fresh = screens.artistNew[at]?.length ?? 0}
          <tr class:cur={a.name === screens.artist} data-k={a.name} onclick={() => (screens.artist = a.name)} ondblclick={() => press("Enter")}>
            <td class="star" title={a.starred ? "starred" : ""}>{a.starred ? "★" : ""}</td>
            <td><span class="t">{a.name}</span></td>
            <td class="a">{a.tracks}</td>
            <td class="a">{a.albums || ""}</td>
            <td class="live">{fresh ? `${fresh} new` : ""}</td>
          </tr>
        {/each}
      </Wall>
    {:else}
      <p class="none">{m.artists.loaded ? "No artist matches. Esc clears the filter." : "Reading the artists off the tags."}</p>
    {/if}
  </div>
</section>
