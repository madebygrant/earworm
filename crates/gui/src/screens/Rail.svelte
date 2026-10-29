<script lang="ts">
  import type { Shelf } from "../lib/protocol";
  import { app, chooseTheme } from "../lib/store.svelte";
  import { say, ui, working } from "../lib/ui.svelte";

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
  <div class="tabs"><span aria-current="true">folders</span></div>
  {#if app.model.library.length}
    <div class="q" class:on={app.model.view === "Library" && (ui.typing || ui.filter)}>
      {#if ui.typing && app.model.view === "Library"}
        <input bind:this={box} bind:value={ui.filter} placeholder="filter folders" spellcheck="false" />
      {:else}
        <span class="k">/</span> {(app.model.view === "Library" && ui.filter) || "filter folders"}
      {/if}
    </div>
    <div class="opts">
      <span><span class="k">s</span> <b>{SORT[ui.sort]}</b></span>
      <span><span class="k">a</span> <b>{ui.only ? KIND[ui.only] : "all"}</b></span>
    </div>
  {/if}
  {#if rows.length}
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
  {:else if app.model.library.length}
    <p class="empty">Nothing matches. Esc clears the filter.</p>
  {:else if !app.error}
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
