<script lang="ts">
  // The terminal's row: number, status word, title and artist, and the source on the right.
  interface Props {
    num: number;
    word: string;
    color: string;
    title: string;
    artist: string;
    source: string;
    cur: boolean;
    marked: boolean;
    onpick: () => void;
  }
  let { num, word, color, title, artist, source, cur, marked, onpick }: Props = $props();
</script>

<div class="r" class:cur role="button" tabindex="-1" onclick={onpick} onkeydown={(e) => e.key === "Enter" && onpick()}>
  <span class="n">{marked ? "✓" : num}</span><span class="w" style:color>{word}</span>
  <span class="t">{title} {#if artist}<span class="a">- {artist}</span>{/if}</span><span class="src">{source}</span>
</div>
