<script lang="ts">
  import { app } from "../lib/store.svelte";
  import { ui } from "../lib/ui.svelte";

  // The track list's narrowing, where the tracks are: the same box the rail's is, as in the terminal.
  let box = $state<HTMLInputElement>();
  $effect(() => {
    if (ui.typing) box?.focus();
  });
  const shown = $derived(ui.rows.length);
</script>

{#if ui.typing || ui.filter || app.model.review}
  <div class="band">
    <b>{app.model.review && !ui.filter ? "review" : "filter"}</b>
    {#if ui.typing}
      <input bind:this={box} bind:value={ui.filter} spellcheck="false" />
    {:else}
      <span>{ui.filter}</span>
    {/if}
    <span class="end">{shown} of {app.model.tracks.length} shown · esc clears</span>
  </div>
{/if}
