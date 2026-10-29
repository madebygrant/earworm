<script lang="ts">
  import { invoke } from "@tauri-apps/api/core";
  import type { Prompt } from "../lib/protocol";
  import { app, reply } from "../lib/store.svelte";
  import { ui, type Tool } from "../lib/ui.svelte";
  import Icon from "./Icon.svelte";

  // The URL prompt the worker asks when there is nothing in the library yet, as the whole screen: the
  // field is the answer, and the tools table is `--check`'s, so a missing one is seen before a run fails.
  interface Props {
    prompt: Extract<Prompt, { Input: unknown }>;
  }
  let { prompt }: Props = $props();
  let value = $state("");
  let field = $state<HTMLInputElement>();
  // A refused link comes back as the next question's value, for editing rather than retyping.
  $effect.pre(() => {
    value = prompt.Input.value;
  });

  $effect(() => {
    field?.focus();
  });
  $effect(() => {
    if (ui.tools === null) invoke<Tool[]>("tools").then((tools) => (ui.tools = tools));
  });

  function key(e: KeyboardEvent) {
    if (e.key === "Enter") {
      ui.linkSent = true;
      void reply({ Text: value });
    }
    else if (e.key === "Escape") void reply("Cancel");
  }
  // The key is read once at start, so saving it restarts the app; until then this says why nothing moved.
  let acoustid = $state("");
  let keySaid = $state("");
  let keyBad = $state(false);
  async function saveKey(e: KeyboardEvent) {
    if (e.key !== "Enter" || !acoustid.trim()) return;
    keySaid = "saving, earworm restarts with it";
    keyBad = false;
    try {
      await invoke("acoustid_key", { key: acoustid });
    } catch (err) {
      keySaid = String(err);
      keyBad = true;
    }
  }
  const setup = $derived(app.setup);
  // Any link question after one was sent is a refusal of it, an empty link included.
  const refused = $derived(ui.linkSent);
</script>

<section class="view walled">
  <div class="look">
    <div class="hero">
      <h1>Playlist in, library out.</h1>
            <div class="url" class:bad={refused}>
        <Icon name="link" />
        <input bind:this={field} bind:value onkeydown={key} placeholder="Paste a playlist link" spellcheck="false" />
      </div>
      {#if refused}<p class="say error">{prompt.Input.header}</p>{/if}
      {#if prompt.Input.note}<p class="say">{prompt.Input.note}</p>{/if}
      <p class="say">earworm makes a folder for it in {setup?.dir ?? "your music folder"}, then names and tags every track.</p>
      <div class="card check">
        <h2>Tools</h2>
        {#if ui.tools === null}
          <div><span class="muted">·</span><span class="muted">checking</span></div>
        {:else}
          {#each ui.tools as t (t.bin)}
            {@const ok = !!(t.version || t.at)}
            <div>
              <span class={ok ? "good" : t.required ? "error" : "warn"}><Icon name={ok ? "mark" : "missing"} /></span>
              <b>{t.bin}</b>
              {#if ok}<span>{t.version ?? "found"}</span><span class="fix">{t.what}</span>
              {:else}<span class={t.required ? "error" : "warn"}>not found</span><span class="fix"><span class="copy">{t.fix}</span> for {t.what}</span>{/if}
            </div>
          {/each}
        {/if}
        {#if setup}
          <h2>Settings</h2>
          <div class:nokey={!setup.key}>
            <span class={setup.key ? "good" : "warn"}><Icon name={setup.key ? "mark" : "missing"} /></span><b>acoustid</b>
            <span class={setup.key ? "" : "warn"}>{setup.key ? "set" : "no key"}</span>
            <span class="fix">{#if setup.key}fingerprint lookups{:else}lookups fall back to Deezer and Apple; a free key from <span class="copy">acoustid.org/new-application</span> confirms far more tracks{/if}</span>
          </div>
          {#if !setup.key}
            <div class="key">
              <span></span><span></span>
              <input bind:value={acoustid} onkeydown={saveKey} placeholder="paste the key, Enter saves it" spellcheck="false" aria-label="AcoustID key" />
            </div>
            {#if keySaid}<div class="key"><span></span><span></span><span class="said" class:error={keyBad}>{keySaid}</span></div>{/if}
          {/if}
          <div><span class="good"><Icon name="mark" /></span><b>format</b><span>{setup.format}</span><span class="fix">changed on the settings screen</span></div>
          <div><span class="good"><Icon name="mark" /></span><b>folder</b><span class="path">{setup.dir}</span><span class="fix">where playlists land</span></div>
        {/if}
      </div>
    </div>
  </div>
</section>
