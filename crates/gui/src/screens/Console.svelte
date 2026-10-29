<script lang="ts">
  import { consoleHooks } from "../lib/keys";
  import { celebrate } from "../lib/screens.svelte";
  import { tryCode } from "../lib/store.svelte";

  // The `^z ^x` line. It names nothing it unlocks; a fresh find is the card's to celebrate.
  let code = $state<string | null>(null);
  let field = $state<HTMLInputElement>();
  consoleHooks.open = () => {
    code = "";
    queueMicrotask(() => field?.focus());
  };
  consoleHooks.isOpen = () => code !== null;

  function key(e: KeyboardEvent) {
    // The key that closes the console is the console's: by the time it reaches the window the console is shut,
    // and the screen under it would take it as its own Enter or Esc.
    if (e.key === "Escape" || e.key === "Enter") e.stopPropagation();
    if (e.key === "Escape") code = null;
    else if (e.key === "Enter" && code) {
      void tryCode(code).then(celebrate);
      code = null;
    }
  }
</script>

{#if code !== null}
  <div class="console"><span class="k">&gt;</span><input bind:this={field} bind:value={code} onkeydown={key} spellcheck="false" /></div>
{/if}
