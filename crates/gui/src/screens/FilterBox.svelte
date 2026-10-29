<script lang="ts">
  import { press } from "../lib/keys";
  import { ui } from "../lib/ui.svelte";
  import Icon from "./Icon.svelte";

  // The one `ui.filter`, typed into here on `/`; a click is the `/`.
  let { placeholder, said = "", count = "" }: { placeholder: string; said?: string; count?: string } = $props();
  let box = $state<HTMLInputElement>();
  $effect(() => {
    if (ui.typing) box?.focus();
  });
</script>

<div class="find" class:on={ui.typing || ui.filter || said}>
  <Icon name="filter" />
  {#if ui.typing}
    <input bind:this={box} bind:value={ui.filter} {placeholder} spellcheck="false" />
  {:else}
    <button onclick={() => press("/")}>{ui.filter || said || placeholder}</button>
  {/if}
  {#if count}<span class="n">{count}</span>{/if}
</div>
