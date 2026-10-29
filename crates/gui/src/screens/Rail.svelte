<script lang="ts">
  import { app } from "../lib/store.svelte";
  import { openPage, screens } from "../lib/screens.svelte";

  const artists = $derived(screens.artistRows.map((at) => app.model.artists.rows[at]).filter(Boolean));
  // By row, from core, which matches an artist under any spelling.
  const fresh = (name: string) => screens.artistNew[app.model.artists.rows.findIndex((r) => r.name === name)]?.length ?? 0;
  let query = $state<HTMLInputElement>();
  $effect(() => {
    if (screens.artistTyping) query?.focus();
  });
</script>

<nav class="rail" aria-label="Artists">
  <div class="q" class:on={screens.artistTyping || screens.artistQuery}>
    {#if screens.artistTyping}
      <input bind:this={query} bind:value={screens.artistQuery} placeholder="filter artists" spellcheck="false" />
    {:else}
      <span class="k">/</span> {screens.artistQuery || "filter artists"}
    {/if}
  </div>
  <ul role="listbox" aria-label="Artists">
    {#each artists as a (a.name)}
      <li role="option" tabindex="-1" aria-selected={a.name === screens.artist} aria-current={a.name === screens.artist}
        class:focus={a.name === screens.artist && app.model.view === "Artists"}
        onclick={() => (screens.artist = a.name)} ondblclick={openPage} onkeydown={() => {}}>
        <span class="star">{a.starred ? "★" : ""}</span><span class="nm">{a.name}</span>
        <span class="ct" class:live={fresh(a.name) > 0}>{fresh(a.name) ? `+${fresh(a.name)} new` : a.tracks}</span>
      </li>
    {/each}
  </ul>
  {#if !artists.length}<p class="empty">{app.model.artists.loaded ? "No artist matches. Esc clears the filter." : "Reading the artists off the tags."}</p>{/if}
</nav>
