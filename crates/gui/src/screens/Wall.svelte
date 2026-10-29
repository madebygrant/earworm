<script lang="ts">
  import { untrack, type Snippet } from "svelte";
  import { shown } from "../lib/wall";

  // Follows `at` only when it or the layout changes, so scrolling by hand is never snapped back.
  let { at, list, head, children }: { at: string; list: boolean; head?: Snippet; children: Snippet } = $props();
  let el = $state<HTMLElement>();
  const item = () => el?.querySelector(`[data-k="${CSS.escape(at)}"]`);
  const follow = () => item()?.scrollIntoView({ block: "nearest" });
  $effect(() => {
    void [at, list];
    follow();
    pinned = true;
  });
  // Again when the scroller is resized, which the first layout does after the first scroll ran, but only
  // while the cursor is in view: one scrolled away by hand stays away.
  let pinned = true;
  function scrolled() {
    const [r, box] = [item()?.getBoundingClientRect(), el?.getBoundingClientRect()];
    pinned = !!r && !!box && r.bottom > box.top && r.top < box.bottom;
  }
  $effect(() => {
    if (!el) return;
    const ro = new ResizeObserver(() => pinned && untrack(follow));
    ro.observe(el);
    return () => ro.disconnect();
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

<div class="scroll" bind:this={el} onscroll={scrolled}>
  {#if list}
    <table><thead>{@render head?.()}</thead><tbody>{@render children()}</tbody></table>
  {:else}
    <div class="wall">{@render children()}</div>
  {/if}
</div>
