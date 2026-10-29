<script lang="ts">
  import { convertFileSrc } from "@tauri-apps/api/core";
  import Icon from "./Icon.svelte";

  // The terminal's row: number, status word, title and artist, and the source on the right.
  interface Props {
    num: number;
    word: string;
    color: string;
    title: string;
    artist: string;
    source: string;
    thumb: string | null;
    cur: boolean;
    marked: boolean;
    onpick: () => void;
  }
  let { num, word, color, title, artist, source, thumb, cur, marked, onpick }: Props = $props();
</script>

<tr class:cur data-k={num} onclick={onpick}>
  <td class="a">{#if marked}<span title="marked"><Icon name="mark" /></span>{:else}{num}{/if}</td>
  <td><span class="sleeve" class:empty={!thumb}>{#if thumb}<img src={convertFileSrc(thumb)} alt="" />{/if}</span></td>
  <td><div class="t">{title}</div>{#if artist}<div class="a">{artist}</div>{/if}</td>
  <td class="status" style:color>{word}</td>
  <td class="src">{source}</td>
</tr>
