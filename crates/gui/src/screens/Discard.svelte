<script lang="ts">
  import { screens } from "../lib/screens.svelte";

  // Esc on the picker with marks to lose. Esc here means keep picking, as it does on the quit question.
  const p = $derived(screens.pick);
</script>

<div class="scrim">
  <div class="dialog" style="border-color:var(--warn)">
    <span class="t" style="color:var(--warn)">leave the picker</span>
    <p>{p.existing ? `Drop the changes to ${p.name}: +${p.add} −${p.remove}?` : `Drop the ${p.marks.length} marked ${p.marks.length === 1 ? "track" : "tracks"}? ${p.name} stays empty.`}</p>
    <p class="foot"><span><span class="k">y enter</span> drop them</span><span><span class="k">any other key</span> keep picking</span></p>
  </div>
</div>
