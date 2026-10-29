<script lang="ts">
  import { screens } from "../lib/screens.svelte";
  import { app } from "../lib/store.svelte";
  import { ui } from "../lib/ui.svelte";
  import Lit from "./Lit.svelte";

  // Every track in the library matching the box, flat, with the folder each is in: core's search, ranked.
  const m = $derived(app.model);
  let box = $state<HTMLInputElement>();
  $effect(() => {
    if (ui.typing) box?.focus();
  });
  let body = $state<HTMLElement>();
  const at = $derived(screens.found ? `${screens.found[0]}\u0000${screens.found[1]}` : "");
  $effect(() => {
    body?.querySelector(`[data-k="${CSS.escape(at)}"]`)?.scrollIntoView({ block: "nearest" });
  });
  const marked = (folder: string, file: string) => m.foundMarks.some(([f, n]) => f === folder && n === file);
</script>

<section class="view">
  <div class="band">
    <b>search</b>
    {#if ui.typing}<input bind:this={box} bind:value={ui.filter} spellcheck="false" />{:else}<span>{ui.filter}</span>{/if}
    <span class="end">{screens.hits.length} of {screens.foundTotal}{#if m.foundMarks.length} · {m.foundMarks.length} marked{/if}</span>
  </div>
  <div bind:this={body}>
    {#each screens.hits as h (`${h.shelf}:${h.file}`)}
      {@const shelf = m.library[h.shelf]}
      {@const file = shelf?.files[h.file]}
      {#if shelf && file}
        {@const key = `${shelf.path}\u0000${file[0]}`}
        <div class="r found" class:cur={key === at} data-k={key} role="button" tabindex="-1"
          onclick={() => (screens.found = [shelf.path, file[0]])} onkeydown={() => {}}>
          <span class="n">{marked(shelf.path, file[0]) ? "✓" : ""}</span><span class="miss">{file[1] ? "" : "!"}</span>
          <span class="t"><Lit text={h.label} at={h.at} /></span><span class="src"><Lit text={shelf.name} at={h.folder_at} /></span>
        </div>
      {/if}
    {/each}
  </div>
  {#if !screens.hits.length}
    <p class="empty-main">{ui.filter.trim() ? `No track matches ${ui.filter}. Esc goes back to the library.` : "Type to find a track in any folder."}</p>
  {/if}
</section>
