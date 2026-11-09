<script lang="ts">
  import { clearQueue, screens } from "../lib/screens.svelte";
  import { app } from "../lib/store.svelte";

  // Drawn by App rather than the queue screen, which a queued link starting can take away from under it:
  // the question would go and its keys stay, and the next Enter on the track list would empty the queue.
  const n = $derived(app.model.queue.length);
</script>

<div class="look"><div class="scrim">
  <div class="dialog alert" role="alertdialog" aria-label="empty the queue">
    <h3>Empty the queue?</h3>
    <p>{n} {n === 1 ? "link" : "links"} waiting.</p>
    <p class="foot"><span><kbd>y enter</kbd> empty it</span><span><kbd>any other key</kbd> keep them</span></p>
    <p class="acts"><button class="yes" onclick={() => void clearQueue()}>Empty it</button><button onclick={() => (screens.clearing = false)}>Keep them</button></p>
  </div>
</div></div>
