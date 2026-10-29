<script lang="ts">
  import { settledCount } from "../lib/reducer";
  import { app } from "../lib/store.svelte";

  const settled = $derived(settledCount(app.model));
</script>

<div class="scrim">
  <div class="dialog" style="border-color:var(--warn)">
    <span class="t" style="color:var(--warn)">quit</span>
    <p>
      {#if app.model.tracks.length && app.model.done === null}A download is going: {settled} of {app.model.tracks.length} done.{:else}A command is running.{/if}
      Quit and stop it? The tag write under way finishes first.
    </p>
    <p class="foot"><span><span class="k">y q enter</span> quit</span><span><span class="k">any other key</span> keep going</span><span>so <span class="k">qq</span> quits without reading this</span></p>
  </div>
</div>
