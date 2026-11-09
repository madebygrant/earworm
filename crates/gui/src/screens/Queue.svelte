<script lang="ts">
  import { tick } from "svelte";
  import { addField, dropField, queueCursor, queueFields, screens, unqueue } from "../lib/screens.svelte";
  import { app } from "../lib/store.svelte";
  import Icon from "./Icon.svelte";
  import Wall from "./Wall.svelte";

  // The links waiting, in the order the worker takes them. The one downloading has already left the list.
  const links = $derived(app.model.queue);
  const at = $derived(queueCursor());
  const typed = $derived(screens.fields.filter((f) => f.text.trim()).length);
  const boxes: HTMLInputElement[] = $state([]);

  $effect(() => {
    const want = screens.focusField;
    if (want === null) return;
    void tick().then(() => {
      boxes[want]?.focus();
      screens.focusField = null;
    });
  });

  // Enter is another field and ⌘ or ^ Enter queues them all; Backspace on an empty field drops it.
  function fieldKey(e: KeyboardEvent, i: number) {
    const field = screens.fields[i];
    if (e.key === "Enter" && (e.metaKey || e.ctrlKey)) {
      e.preventDefault();
      void queueFields();
    } else if (e.key === "Enter") {
      e.preventDefault();
      if (field.text.trim()) addField(i);
    } else if (e.key === "Escape") {
      (e.currentTarget as HTMLInputElement).blur();
    } else if (e.key === "Backspace" && !field.text && screens.fields.length > 1) {
      e.preventDefault();
      dropField(i);
    }
  }

  // Several lines pasted into one field become a field each.
  function fieldPaste(e: ClipboardEvent, i: number) {
    const lines = (e.clipboardData?.getData("text") ?? "").split(/\r?\n/).map((l) => l.trim()).filter(Boolean);
    if (lines.length < 2) return;
    e.preventDefault();
    const field = screens.fields[i];
    const box = e.currentTarget as HTMLInputElement;
    const from = box.selectionStart ?? field.text.length;
    field.text = (field.text.slice(0, from) + lines[0] + field.text.slice(box.selectionEnd ?? from)).trim();
    field.why = null;
    lines.slice(1).forEach((line, n) => addField(i + n, line));
  }
</script>

<section class="view walled">
  <div class="look">
    <form class="adder" onsubmit={(e) => (e.preventDefault(), void queueFields())}>
      {#each screens.fields as field, i}
        <div class="field" class:bad={field.why}>
          <Icon name="link" />
          <input
            bind:this={boxes[i]}
            bind:value={field.text}
            oninput={() => (field.why = null)}
            onkeydown={(e) => fieldKey(e, i)}
            onpaste={(e) => fieldPaste(e, i)}
            placeholder="https://youtube.com/playlist?list=…"
            aria-label="link {i + 1}"
            spellcheck="false"
            autocomplete="off"
          />
          {#if screens.fields.length > 1 || field.text}
            <button type="button" class="drop" title="remove this field" aria-label="remove this field" onclick={() => dropField(i)}><Icon name="remove" /></button>
          {/if}
        </div>
        {#if field.why}<p class="why">{field.why}</p>{/if}
      {/each}
      <div class="acts">
        <button type="button" class="more" onclick={() => addField()}><Icon name="plus" />Another link</button>
        <button type="submit" class="yes" disabled={!typed}>{typed > 1 ? `Queue ${typed} links` : "Queue"}</button>
      </div>
    </form>
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
      <p class="none">Type links above, or paste several on any screen. Esc goes back.</p>
    {/if}
  </div>
</section>
