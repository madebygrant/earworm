<script lang="ts">
  import { press } from "../lib/keys";
  import Icon from "./Icon.svelte";

  // A query typed into here on `/`, and a click is the `/`: `ui.filter`, or the artists screen's own.
  interface Props {
    placeholder: string;
    said?: string;
    count?: string;
    value: string;
    typing: boolean;
  }
  let { placeholder, said = "", count = "", value = $bindable(), typing }: Props = $props();
  let box = $state<HTMLInputElement>();
  $effect(() => {
    if (typing) box?.focus();
  });
</script>

<div class="find" class:on={typing || value || said}>
  <Icon name="filter" />
  {#if typing}
    <input bind:this={box} bind:value {placeholder} spellcheck="false" />
  {:else}
    <button onclick={() => press("/")}>{value || said || placeholder}</button>
  {/if}
  {#if count}<span class="n">{count}</span>{/if}
</div>
