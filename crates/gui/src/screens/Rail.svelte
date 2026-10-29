<script lang="ts">
  import type { Shelf } from "../lib/protocol";
  import { app, chooseTheme } from "../lib/store.svelte";
  import { say, ui, working } from "../lib/ui.svelte";
  import { openPage, screens } from "../lib/screens.svelte";

  const artistView = $derived(app.model.view === "Artists" || app.model.view === "Artist");
  const artists = $derived(screens.artistRows.map((at) => app.model.artists.rows[at]).filter(Boolean));
  // By row, from core, which matches an artist under any spelling.
  const fresh = (name: string) => screens.artistNew[app.model.artists.rows.findIndex((r) => r.name === name)]?.length ?? 0;
  let query = $state<HTMLInputElement>();
  $effect(() => {
    if (screens.artistTyping) query?.focus();
  });

  const rows = $derived(ui.shelves.map((at) => app.model.library[at]).filter(Boolean));
  const SORT = { Name: "name", Synced: "last synced", Missing: "most missing" };
  const KIND = { Album: "albums", Playlist: "playlists", Custom: "custom" };
  let box = $state<HTMLInputElement>();
  $effect(() => {
    if (ui.typing && app.model.view === "Library") box?.focus();
  });

  function icon(s: Shelf): string {
    if (s.kind === "Album") return "●";
    if (s.kind === "Custom") return "≡";
    // A folder earworm did not download has no URL to sync from.
    return s.url === null ? "◆" : "▤";
  }

  function count(s: Shelf): string {
    if (s.missing > 0 && s.kind !== "Custom") return `${s.missing}!`;
    return String(s.tracks);
  }

  // Leaving a run for the library waits for the run, as Esc does.
  function select(s: Shelf) {
    if (app.model.view === "Tracks" && working()) return say("still working  ·  q quits");
    app.model = { ...app.model, shelf: s.path, view: "Library" };
  }
</script>

<nav class="rail" aria-label="Library">
  <div class="tabs"><span aria-current={!artistView}>folders</span><span aria-current={artistView}><span class="k">b</span> artists</span></div>
  {#if artistView}
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
  {:else if app.model.library.length}
    <div class="q" class:on={app.model.view === "Library" && (ui.typing || ui.filter || ui.byArtist)}>
      {#if ui.typing && app.model.view === "Library"}
        <input bind:this={box} bind:value={ui.filter} placeholder="filter folders" spellcheck="false" />
      {:else}
        <span class="k">/</span> {(app.model.view === "Library" && ui.filter) || (ui.byArtist ? `${ui.byArtist[0]}'s folders · esc` : "filter folders")}
      {/if}
    </div>
    <div class="opts">
      <span><span class="k">s</span> <b>{SORT[ui.sort]}</b></span>
      <span><span class="k">a</span> <b>{ui.only ? KIND[ui.only] : "all"}</b></span>
    </div>
  {/if}
  {#if !artistView && rows.length}
    <ul role="listbox" aria-label="Folders">
      {#each rows as s (s.path)}
        <li
          role="option"
          tabindex="-1"
          aria-selected={s.path === app.model.shelf}
          aria-current={s.path === app.model.shelf}
          class:focus={s.path === app.model.shelf && app.model.view === "Library"}
          onclick={() => select(s)}
          onkeydown={(e) => e.key === "Enter" && select(s)}
        >
          <span class="ic" class:album={s.kind === "Album"}>{icon(s)}</span>
          <span class="nm">{s.name}</span>
          <span class="ct" class:miss={s.missing > 0 && s.kind !== "Custom"}>{count(s)}</span>
        </li>
      {/each}
    </ul>
  {:else if !artistView && app.model.library.length}
    <p class="empty">Nothing matches. Esc clears the filter.</p>
  {:else if !artistView && !app.error}
    <p class="empty">No folders yet. A synced playlist lands here.</p>
  {/if}
  {#if app.theme}
    <label class="themes">
      <span><span class="k">^t</span> theme</span>
      <select value={app.theme.name} onchange={(e) => chooseTheme(e.currentTarget.value)}>
        {#each app.theme.names as name (name)}<option value={name}>{name}</option>{/each}
      </select>
    </label>
  {/if}
</nav>
