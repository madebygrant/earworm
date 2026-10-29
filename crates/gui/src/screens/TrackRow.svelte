<script lang="ts">
  import { convertFileSrc } from "@tauri-apps/api/core";
  import { clock } from "../lib/run";
  import Icon from "./Icon.svelte";

  // The terminal's row, plus how sure the match is and how long the track runs.
  interface Props {
    num: number;
    word: string;
    color: string;
    title: string;
    artist: string;
    source: string;
    score: number | null;
    duration: number;
    thumb: string | null;
    cur: boolean;
    marked: boolean;
    onpick: () => void;
  }
  let { num, word, color, title, artist, source, score, duration, thumb, cur, marked, onpick }: Props = $props();
  // The status cell is `{#key word}`, so a new word fades in and the column is seen to settle.
  // Ten segments; nothing at all where the lookup gave no score, since an empty bar would read as zero.
  const lit = $derived(score === null ? null : Math.round(score * 10));
</script>

<tr class:cur data-k={num} onclick={onpick}>
  <td class="a">{#if marked}<span title="marked"><Icon name="mark" /></span>{:else}{num}{/if}</td>
  <td><span class="sleeve" class:empty={!thumb}>{#if thumb}<img src={convertFileSrc(thumb)} alt="" />{/if}</span></td>
  <td><div class="t">{title}</div>{#if artist}<div class="a">{artist}</div>{/if}</td>
  <td class="conf">
    {#if lit !== null}<span class="seg" aria-hidden="true">{#each Array(10) as _, i (i)}<i class:f={i < lit}></i>{/each}</span>{Math.round(score! * 100)}%{/if}
  </td>
  <td>{#key word}<span class="status fade" style:color>{word}</span>{/key}</td>
  <td class="src">{source}</td>
  <td class="a">{clock(duration)}</td>
</tr>
