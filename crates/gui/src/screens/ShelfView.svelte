<script lang="ts">
  import { app } from "../lib/store.svelte";
  import { askThumbs, ui, working } from "../lib/ui.svelte";
  import Tile from "./Tile.svelte";
  import TrackRow from "./TrackRow.svelte";

  // The folder under the rail's cursor, read off the library with no worker round trip: its files, and
  // which of them are missing. Enter opens it as a run would show it.
  const m = $derived(app.model);
  const shelf = $derived(m.library.find((s) => s.path === m.shelf) ?? null);
  const files = $derived(shelf ? shelf.files.map(([name, here], i) => ({ name, here, num: number(name) ?? i + 1, path: `${shelf.path}/${name}` })) : []);

  // The playlist position the filename starts with, which is the track's number wherever it sits in the list.
  function number(name: string): number | null {
    const at = /^(\d+)\s*-/.exec(name);
    return at ? Number(at[1]) : null;
  }

  function parts(name: string): [string, string] {
    const stem = name.replace(/\.[^.]+$/, "").replace(/^\d+\s*-\s*/, "");
    const at = stem.indexOf(" - ");
    return at > 0 ? [stem.slice(at + 3), stem.slice(0, at)] : [stem, ""];
  }

  $effect(() => {
    if (working()) return;
    askThumbs(files.filter((f) => f.here && !(f.path in m.thumbs)).map((f) => f.path));
  });
</script>

<section class="view">
  {#if shelf}
    <div class="bar">
      <span class="title" style="color:var(--text)">{shelf.name}</span>
      <span>{shelf.url ? "enter opens it, S syncs it" : "not downloaded by earworm"}</span>
      <span class="mode" role="group" aria-label="View, w">
        <button aria-pressed={ui.mode === "tiles"} onclick={() => (ui.mode = "tiles")}>tiles</button>
        <button aria-pressed={ui.mode === "list"} onclick={() => (ui.mode = "list")}>list</button>
      </span>
    </div>
    <div class={ui.mode === "tiles" ? "sheet" : ""}>
      {#each files as f (f.name)}
        {@const [title, artist] = parts(f.name)}
        {#if ui.mode === "tiles"}
          <Tile num={f.num} word={f.here ? "on disk" : "missing"} color={f.here ? "var(--muted)" : "var(--warn)"} {title} {artist}
            frame={f.here ? "" : "missing"} thumb={m.thumbs[f.path]} cur={false} marked={false} onpick={() => {}} />
        {:else}
          <TrackRow num={f.num} word={f.here ? "on disk" : "missing"} color={f.here ? "var(--muted)" : "var(--warn)"} {title} {artist}
            source="" cur={false} marked={false} onpick={() => {}} />
        {/if}
      {/each}
    </div>
  {:else if !m.library.length}
    <p class="empty-main">No folders yet. n syncs a new link.</p>
  {/if}
</section>
