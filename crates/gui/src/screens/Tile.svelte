<script lang="ts">
  import { convertFileSrc } from "@tauri-apps/api/core";

  // One track as a tile. The frame's state is its border; the word under it says the same thing in text.
  interface Props {
    num: number;
    word: string;
    color: string;
    title: string;
    artist: string;
    frame: "" | "queued" | "live" | "failed" | "missing" | "gone";
    thumb: string | null | undefined;
    cur: boolean;
    marked: boolean;
    onpick: () => void;
  }
  let { num, word, color, title, artist, frame, thumb, cur, marked, onpick }: Props = $props();
  const pad = (n: number) => String(n).padStart(2, "0");
</script>

<div class="tile" class:cur role="button" tabindex="-1" onclick={onpick} onkeydown={(e) => e.key === "Enter" && onpick()}>
  <div class="frame {frame}">
    {#if thumb && frame !== "failed" && frame !== "missing"}
      <img alt="" src={convertFileSrc(thumb)} />
    {:else if frame === "failed" || frame === "missing"}
      !
    {:else if frame !== "live"}
      <span class="num">{pad(num)}</span>
    {/if}
    {#if marked}<span class="mark">✓</span>{/if}
  </div>
  <div class="cap"><span class="n">{pad(num)}</span><span class="w" style:color>{word}</span></div>
  <div class="tt">{title}</div>
  <div class="ar">{artist}</div>
</div>
