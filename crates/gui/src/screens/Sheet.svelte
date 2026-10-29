<script lang="ts" module>
  import type { IconName } from "../lib/icons";

  // An icon with its name as the hover title, the value, and a colour for a status.
  export type Field = { icon: IconName; name: string; value: string; tone?: string };
  // The key a button stands for, so a click and the key are one action. The first is the likely one.
  export type Act = { key: string; icon: IconName; word: string };
</script>

<script lang="ts">
  import type { Snippet } from "svelte";
  import Icon from "./Icon.svelte";

  interface Props {
    title: string;
    why?: string;
    // The reason line in the status's colour, which is how much to trust it.
    tone?: string;
    picture?: Snippet;
    // Art fills the picture box edge to edge; a placeholder sits in a dashed one.
    art?: boolean;
    fields?: Field[];
    acts?: Act[];
    extra?: Snippet;
    press: (key: string) => void;
  }
  let { title, why = "", tone, picture, art = false, fields = [], acts = [], extra, press }: Props = $props();
</script>

<div class="look">
  <aside class="sheet">
    {#if picture}<div class="big" class:art>{@render picture()}</div>{/if}
    <h3>{title}</h3>
    {#if why}<p class="why" style:color={tone}>{why}</p>{/if}
    {#if fields.length}
      <dl>
        {#each fields as f (f.name)}
          <dt title={f.name}><Icon name={f.icon} /></dt><dd style:color={f.tone}>{f.value}</dd>
        {/each}
      </dl>
    {/if}
    {@render extra?.()}
    {#if acts.length}
      <div class="acts">
        {#each acts as a, i (a.key)}
          <button class:go={i === 0} onclick={() => press(a.key)}><Icon name={a.icon} />{a.word}</button>
        {/each}
      </div>
    {/if}
  </aside>
</div>
