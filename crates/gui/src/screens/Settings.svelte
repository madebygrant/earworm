<script lang="ts">
  import { screens } from "../lib/screens.svelte";
  import { app } from "../lib/store.svelte";
  import Wall from "./Wall.svelte";

  // Every setting with its value and where it came from. Enter switches one through the worker.
  const rows = $derived(app.model.settings.rows);
  const SOURCE = { Default: "default", File: "config", Flag: "flag", Env: "env" };
  function shown(value: unknown): [string, string] {
    if (value === "Theme") return [app.theme?.name ?? "", ""];
    const v = value as Record<string, unknown>;
    if ("Switch" in v) return v.Switch ? ["on", "good"] : ["off", "muted"];
    if ("Icons" in v) return [String(v.Icons).toLowerCase(), ""];
    const text = String(Object.values(v)[0] ?? "");
    return [text || "none", ""];
  }
</script>

<section class="view walled">
  <div class="look">
    <div class="toolbar"><span class="sum">Enter or space switches a setting and writes it to the config. One set by a flag stays as the flag said.</span></div>
    <Wall at={String(screens.settingsAt)} list>
      {#snippet head()}<tr><th>setting</th><th>value</th><th>from</th><th></th></tr>{/snippet}
      {#each rows as r, i (r.key)}
        {@const [value, tone] = shown(r.value)}
        <tr class:cur={i === screens.settingsAt} data-k={i} onclick={() => (screens.settingsAt = i)}>
          <td class="t">{r.key}</td><td class="status {tone}" class:warn={r.source === "Flag"}>{value}</td>
          <td class="src">{SOURCE[r.source]}</td><td class="a what">{r.what}</td>
        </tr>
      {/each}
    </Wall>
  </div>
</section>
