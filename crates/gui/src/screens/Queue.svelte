<script lang="ts">
  import { queueCursor, screens, unqueue } from "../lib/screens.svelte";
  import { app } from "../lib/store.svelte";
  import Icon from "./Icon.svelte";
  import Wall from "./Wall.svelte";

  // The links waiting, in the order the worker takes them. The one downloading has already left the list.
  const links = $derived(app.model.queue);
  const at = $derived(queueCursor());
</script>

<section class="view walled">
  <div class="look">
    <div class="toolbar">
      <span class="sum">{links.length ? `${links.length} waiting, downloaded one after another with no pick gate` : "Nothing queued"}</span>
      {#if links.length}<button class="clear" onclick={() => (screens.clearing = true)}><Icon name="remove" />Clear</button>{/if}
    </div>
    {#if links.length}
      <Wall at={String(at)} list>
        {#snippet head()}<tr><th>#</th><th>link</th><th></th></tr>{/snippet}
        {#each links as link, i (link)}
          <tr class:cur={i === at} data-k={i} onclick={() => (screens.queueAt = i)}>
            <td class="a">{i + 1}</td><td class="t">{link}</td>
            <td><button class="unqueue" title="take it off the queue" aria-label="take it off the queue" onclick={(e) => (e.stopPropagation(), void unqueue(link))}><Icon name="remove" /></button></td>
          </tr>
        {/each}
      </Wall>
    {:else}
      <p class="none">Paste links on any screen to queue them; the link field takes several at once too. Esc goes back.</p>
    {/if}
  </div>
</section>
