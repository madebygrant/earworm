<script lang="ts">
  // Text with the characters a query matched underlined, as the terminal's picker draws them.
  interface Props {
    text: string;
    at: number[];
  }
  let { text, at }: Props = $props();
  const chars = $derived([...text]);
</script>

{#each chars as c, i (i)}{#if at.includes(i)}<u>{c}</u>{:else}{c}{/if}{/each}
