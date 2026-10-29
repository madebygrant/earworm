<script lang="ts">
  import { press } from "../lib/keys";
  import { screens } from "../lib/screens.svelte";
  import { app } from "../lib/store.svelte";
  import FilterBox from "./FilterBox.svelte";
  import Wall from "./Wall.svelte";

  // In core's order, starred first. Nothing here has a portrait, so a tile is the artist's initials.
  const m = $derived(app.model);
  // With each row's position, which is what core's new releases are kept by.
  const rows = $derived(screens.artistRows.map((at) => [at, m.artists.rows[at]] as const).filter(([, a]) => a));
  const initials = (name: string) => name.split(/\s+/).filter((w) => /\p{L}|\p{N}/u.test(w)).slice(0, 2).map((w) => [...w.replace(/^[^\p{L}\p{N}]+/u, "")][0] ?? "").join("").toUpperCase() || "?";
</script>

<section class="view walled">
  <div class="look">
    <div class="toolbar">
      <FilterBox placeholder="filter artists" bind:value={screens.artistQuery} typing={screens.artistTyping} count={screens.artistQuery ? `${rows.length} of ${m.artists.rows.length}` : ""} />
    </div>
    {#if rows.length}
      <Wall at={screens.artist ?? ""} list={false}>
        {#each rows as [at, a] (a.name)}
          {@const fresh = screens.artistNew[at]?.length ?? 0}
          <div class="s empty person" class:cur={a.name === screens.artist} data-k={a.name} role="button" tabindex="-1"
            onclick={() => (screens.artist = a.name)} ondblclick={() => press("Enter")} onkeydown={() => {}}>
            {initials(a.name)}
            {#if fresh}<span class="st accent">+{fresh} new</span>{/if}
            {#if a.starred}<span class="kind star" title="starred">★</span>{/if}
            <div class="cap">{a.name}<small>{a.tracks} {a.tracks === 1 ? "track" : "tracks"}{a.albums ? ` · ${a.albums} ${a.albums === 1 ? "album" : "albums"}` : ""}</small></div>
          </div>
        {/each}
      </Wall>
    {:else}
      <p class="none">{m.artists.loaded ? "No artist matches. Esc clears the filter." : "Reading the artists off the tags."}</p>
    {/if}
  </div>
</section>
