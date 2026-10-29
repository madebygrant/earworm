<script lang="ts">
  import { endPrizes, screens } from "../lib/screens.svelte";

  // An achievement, top right, gone after a few seconds: longer with more to read, as the terminal's.
  const toast = $derived(screens.toast);
  $effect(() => {
    if (!toast) return;
    const lasts = Math.min(2500 + 400 * (toast.prizes.length - 1), 3500);
    const timer = setTimeout(endPrizes, lasts);
    return () => clearTimeout(timer);
  });
</script>

{#if toast}
  <div class="look"><div class="toast" role="status">
    <span class="big">✦</span>
    {#each toast.prizes as p (p.id)}<div><b>{p.label}</b><span>{p.hint}</span></div>{/each}
  </div></div>
{/if}
