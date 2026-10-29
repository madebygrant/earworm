<script lang="ts">
  import type { Snippet } from "svelte";
  import { shown } from "../lib/wall";

  // Follows `at` only when it or the layout changes, so scrolling by hand is never snapped back.
  let { at, list, head, children }: { at: string; list: boolean; head?: Snippet; children: Snippet } = $props();
  let el = $state<HTMLElement>();
  $effect(() => {
    const [k] = [at, list];
    el?.querySelector(`[data-k="${CSS.escape(k)}"]`)?.scrollIntoView({ block: "nearest" });
  });
  // The movement keys measure the wall on screen through this rather than by class name.
  $effect(() => {
    const me = el;
    shown.el = me ?? null;
    return () => {
      if (shown.el === me) shown.el = null;
    };
  });
</script>

<div class="scroll" bind:this={el}>
  {#if list}
    <table><thead>{@render head?.()}</thead><tbody>{@render children()}</tbody></table>
  {:else}
    <div class="wall">{@render children()}</div>
  {/if}
</div>
