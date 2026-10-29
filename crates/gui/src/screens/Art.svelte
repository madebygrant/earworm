<script lang="ts">
  import { convertFileSrc } from "@tauri-apps/api/core";
  import type { Shelf } from "../lib/protocol";
  import { app } from "../lib/store.svelte";
  import { kindIcon, shelfArt } from "../lib/wall";
  import Icon from "./Icon.svelte";

  // Fills its parent. Until a sleeve's cover arrives, or when the file has none, the kind's icon stands in.
  let { shelf }: { shelf: Shelf } = $props();
  const art = $derived(shelfArt(shelf));
  const thumbs = $derived(typeof art === "object" ? art.files.map((f) => app.model.thumbs[f] ?? null) : []);
  const mosaic = $derived(typeof art === "object" && art.mosaic);
</script>

<div class="art" class:mosaic>
  {#if mosaic}
    {#each [0, 1, 2, 3] as i (i)}<i>{#if thumbs[i]}<img src={convertFileSrc(thumbs[i]!)} alt="" />{/if}</i>{/each}
  {:else if thumbs[0]}
    <img src={convertFileSrc(thumbs[0])} alt="" />
  {:else}
    <Icon name={kindIcon(shelf)} />
  {/if}
</div>
