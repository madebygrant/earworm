<script lang="ts">
  import { settledCount } from "../lib/reducer";
  import { app } from "../lib/store.svelte";

  const settled = $derived(settledCount(app.model));
</script>

<div class="look"><div class="scrim">
  <div class="dialog alert" role="alertdialog" aria-label="quit">
    <h3>Quit?</h3>
    <p>
      {#if app.model.tracks.length && app.model.done === null}A download is going: {settled} of {app.model.tracks.length} done.{:else}A command is running.{/if}
      Quit and stop it? The tag write under way finishes first.
    </p>
    <p class="foot"><span><kbd>y q enter</kbd> quit</span><span><kbd>any other key</kbd> keep going</span><span>so <kbd>qq</kbd> quits without reading this</span></p>
  </div>
</div></div>
