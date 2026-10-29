<script lang="ts">
  import { screens } from "../lib/screens.svelte";

  // Esc on the picker with marks to lose. Esc here means keep picking, as it does on the quit question.
  const p = $derived(screens.pick);
</script>

<div class="look"><div class="scrim">
  <div class="dialog alert" role="alertdialog" aria-label="leave the picker">
    <h3>Leave the picker?</h3>
    <p>{p.existing ? `Drop the changes to ${p.name}: +${p.add} −${p.remove}?` : `Drop the ${p.marks.length} marked ${p.marks.length === 1 ? "track" : "tracks"}? ${p.name} stays empty.`}</p>
    <p class="foot"><span><kbd>y enter</kbd> drop them</span><span><kbd>any other key</kbd> keep picking</span></p>
  </div>
</div></div>
