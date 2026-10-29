<script lang="ts">
  import type { Found } from "../lib/protocol";
  import { endSecret } from "../lib/screens.svelte";

  // A found code, centred: it answers the Enter just typed, so unlike an achievement it may take the next key.
  // A secret theme is already painted by now, so its card is drawn in the palette it unlocked.
  let { found }: { found: Found } = $props();

  $effect(() => {
    const timer = setTimeout(endSecret, 3500);
    return () => clearTimeout(timer);
  });

  // Captured so a focused field underneath never sees the key; ^c and the platform's shortcuts pass.
  function key(e: KeyboardEvent) {
    if (["Shift", "Control", "Alt", "Meta"].includes(e.key) || e.metaKey || e.altKey) return;
    if (e.ctrlKey && e.key === "c") return;
    e.preventDefault();
    e.stopPropagation();
    endSecret();
  }

  const sparks = [[8, 12, 0, "✦"], [88, 20, 300, "✧"], [16, 70, 700, "·"], [80, 72, 500, "✦"], [52, 2, 900, "✧"]] as const;
  const notes = [[-34, 600, "♪"], [-6, 900, "♫"], [22, 1200, "♪"]] as const;
</script>

<svelte:window onkeydowncapture={key} />

<div class="look"><div class="unlock {found.art}" role="status">
  <span class="t">✦ secret found ✦</span>
  <div class="art" aria-hidden="true">
    {#if found.art === "music"}
      {#each sparks as [x, y, d, c], i (i)}<span class="spark" style:left="{x}%" style:top="{y}%" style:animation-delay="{d}ms">{c}</span>{/each}
      {#each notes as [x, d, c], i (i)}<span class="note" style:left="calc(50% + {x}px)" style:animation-delay="{d}ms">{c}</span>{/each}
      <div class="chest"><div class="glow"></div><div class="lid"></div></div>
    {:else if found.art === "vaporwave"}
      <div class="sun"></div><div class="grid"></div>
      {#each sparks.slice(0, 2) as [x, y, d, c], i (i)}<span class="spark" style:left="{x}%" style:top="{y}%" style:animation-delay="{d}ms">{c}</span>{/each}
    {:else}
      <div class="lcd"><span class="drop">earworm</span><span class="blip">▲ ▼ ◀ ▶</span></div>
    {/if}
  </div>
  <div class="words"><b>{found.prize} unlocked</b><span>{found.note}</span></div>
  <p class="foot"><kbd>any key</kbd> carry on</p>
</div></div>
