<script lang="ts">
  import { screens } from "../lib/screens.svelte";
  import { app } from "../lib/store.svelte";

  // Every setting with its value and where it came from. Enter switches one through the worker.
  const rows = $derived(app.model.settings.rows);
  const SOURCE = { Default: "default", File: "config", Flag: "flag", Env: "env" };
  function shown(value: unknown): [string, string] {
    if (value === "Theme") return [app.theme?.name ?? "", ""];
    const v = value as Record<string, unknown>;
    if ("Switch" in v) return v.Switch ? ["on", "on"] : ["off", "off"];
    if ("Icons" in v) return [String(v.Icons).toLowerCase(), ""];
    const text = String(Object.values(v)[0] ?? "");
    return [text || "none", ""];
  }
  let body = $state<HTMLElement>();
  const at = $derived(screens.settingsAt);
  $effect(() => {
    body?.querySelector(`[data-i="${at}"]`)?.scrollIntoView({ block: "nearest" });
  });
</script>

<section class="view" bind:this={body}>
  <p class="sub">Every setting with its value and where it came from. Enter or space switches one and writes it to the config.</p>
  {#each rows as r, i (r.key)}
    {@const [value, tone] = shown(r.value)}
    <div class="set" class:cur={i === screens.settingsAt} class:locked={r.source === "Flag"} data-i={i} role="button" tabindex="-1"
      onclick={() => (screens.settingsAt = i)} onkeydown={() => {}}>
      <span class="key">{r.key}</span><span class="v {tone}">{value}</span>
      <span class="src">{SOURCE[r.source]}</span><span class="what">{r.what}</span>
    </div>
  {/each}
</section>
