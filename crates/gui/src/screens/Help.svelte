<script lang="ts">
  import { SHEETS, SLOT_GROUPS } from "../lib/keysheet";
  import { app } from "../lib/store.svelte";

  // `h`: the keys for the screen that is up, then what the status words mean. Any key closes it.
  const [title, groups] = $derived(SHEETS[app.model.view]);
  const legend = $derived(
    SLOT_GROUPS.map(([slot, said]) => [slot, said, app.model.statuses.filter((w) => w.slot === slot).map((w) => w.label)] as const).filter(([, , words]) => words.length),
  );
</script>

<div class="scrim">
  <div class="dialog keysheet">
    <span class="t">keys, {title}</span>
    <div class="keys">
      {#each groups as [name, keys] (name)}
        <div><h4>{name}</h4><dl>{#each keys as [k, what] (k)}<dt>{k}</dt><dd>{what}</dd>{/each}</dl></div>
      {/each}
      {#if app.model.view === "Tracks"}
        <div class="legend"><h4 style="width:100%">status words</h4>
          {#each legend as [slot, said, words] (slot)}<span><span style:color="var(--{slot})">{words.join(" ")}</span> {said}</span>{/each}
        </div>
      {/if}
    </div>
    <p class="foot"><span><span class="k">any key</span> close</span><span><span class="k">^t</span> theme: {app.theme?.name}</span><span><span class="k">,</span> settings</span></p>
  </div>
</div>
