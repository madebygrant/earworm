<script lang="ts">
  import { invoke } from "@tauri-apps/api/core";
  import { onMount } from "svelte";
  import type { Shelf } from "./lib/protocol";
  import { app, boot, chooseTheme, tryCode, walkTheme } from "./lib/store.svelte";

  // The library's order is core's `view::shelf_rows`, asked only when the library itself is replaced: the
  // derived keeps its reference across every other message. A reply is kept with the library it answered,
  // so a late one for an older library is never laid over a newer one.
  const library = $derived(app.model.library);
  let answered = $state<{ library: Shelf[]; order: number[] } | null>(null);
  $effect(() => {
    const asked = library;
    invoke<number[]>("shelf_rows", { library: asked, filter: "", only: null, folders: null, sort: "Name" }).then((order) => {
      if (asked === app.model.library) answered = { library: asked, order };
    });
  });

  // Until the new order arrives, the old one over the folders that are still there.
  const rows = $derived.by(() => {
    if (!answered) return [];
    if (answered.library === library) return answered.order.map((at) => library[at]);
    const was = answered.order.map((at) => answered!.library[at].path);
    return was.flatMap((path) => library.filter((s) => s.path === path));
  });
  const open = $derived(app.model.library.find((s) => s.path === app.model.shelf) ?? null);
  const header = $derived(app.model.flash ?? app.model.stage);

  function icon(s: Shelf): string {
    if (s.kind === "Album") return "●";
    if (s.kind === "Custom") return "≡";
    // A folder earworm did not download has no URL to sync from.
    return s.url === null ? "◆" : "▤";
  }

  function select(s: Shelf) {
    app.model = { ...app.model, shelf: s.path };
  }

  function step(by: number) {
    const at = rows.findIndex((s) => s.path === app.model.shelf);
    const next = rows[Math.max(0, Math.min(rows.length - 1, at + by))];
    if (next) app.model = { ...app.model, shelf: next.path };
  }

  // `^z` then `^x` opens the console, as in the terminal; nothing on screen says it exists.
  let armed = 0;
  let console = $state<string | null>(null);
  let field = $state<HTMLInputElement>();

  function key(e: KeyboardEvent) {
    if (console !== null) return;
    if (e.ctrlKey && e.key === "t") {
      e.preventDefault();
      void walkTheme();
    } else if (e.ctrlKey && e.key === "z") {
      armed = Date.now();
    } else if (e.ctrlKey && e.key === "x" && Date.now() - armed < 1500) {
      e.preventDefault();
      console = "";
      queueMicrotask(() => field?.focus());
    } else if (e.key === "j" || e.key === "ArrowDown") {
      step(1);
    } else if (e.key === "k" || e.key === "ArrowUp") {
      step(-1);
    }
  }

  function consoleKey(e: KeyboardEvent) {
    if (e.key === "Escape") console = null;
    else if (e.key === "Enter" && console) {
      void tryCode(console);
      console = null;
    }
  }

  onMount(() => void boot());

  // Dev builds report what is on screen, for checks that cannot take a screenshot.
  $effect(() => {
    if (!import.meta.env.DEV) return;
    const report = {
      rail: rows.map((s) => s.name),
      selected: open?.name ?? null,
      theme: app.theme?.name ?? null,
      accent: app.theme ? getComputedStyle(document.documentElement).getPropertyValue("--accent").trim() : null,
      railAccent: document.querySelector(".rail li[aria-current='true']") ? getComputedStyle(document.querySelector(".rail li[aria-current='true']")!).backgroundColor : null,
      header: header,
      error: app.error,
    };
    void invoke("probe", { report });
  });
</script>

<svelte:window onkeydown={key} />

<header class="hd" data-tauri-drag-region>
  <span class="brand" data-tauri-drag-region>earworm</span>
  {#if open}<span class="sep">·</span><span data-tauri-drag-region>{open.name}</span>{/if}
  <span class="sep">·</span><span class="stage" data-tauri-drag-region>{header}</span>
  <span class="right" data-tauri-drag-region>{app.model.settings.text}</span>
</header>

<main class="app no-detail">
  <nav class="rail" aria-label="Library">
    <div class="tabs"><span aria-current="true">folders</span></div>
    {#if rows.length}
      <ul role="listbox" aria-label="Folders">
        {#each rows as s (s.path)}
          <li
            role="option"
            tabindex="-1"
            aria-selected={s.path === app.model.shelf}
            aria-current={s.path === app.model.shelf}
            onclick={() => select(s)}
            onkeydown={(e) => e.key === "Enter" && select(s)}
          >
            <span class="ic" class:album={s.kind === "Album"}>{icon(s)}</span>
            <span class="nm">{s.name}</span>
            <span class="ct" class:miss={s.missing > 0}>{s.missing > 0 ? `${s.missing}!` : s.tracks}</span>
          </li>
        {/each}
      </ul>
    {:else if !app.error}
      <p class="empty">No folders yet. A synced playlist lands here.</p>
    {/if}
    {#if app.theme}
      <label class="themes">
        <span><span class="k">^t</span> theme</span>
        <select value={app.theme.name} onchange={(e) => chooseTheme(e.currentTarget.value)}>
          {#each app.theme.names as name (name)}<option value={name}>{name}</option>{/each}
        </select>
      </label>
    {/if}
  </nav>

  <section class="view">
    {#if app.error}
      <div class="fail"><h1>earworm could not start</h1><pre>{app.error}</pre></div>
    {:else if open}
      <div class="bar"><span class="title" style="color:var(--text)">{open.name}</span><span>{open.tracks} tracks</span>
        {#if open.missing}<span class="warn">{open.missing} missing</span>{/if}</div>
      <ul class="files">
        {#each open.files as [name, here] (name)}
          <li><span class={here ? "good" : "warn"}>{here ? "on disk" : "missing"}</span><span>{name}</span></li>
        {/each}
      </ul>
    {/if}
  </section>
</main>

<footer class="ft">
  <span class="tally"><span class="muted">{app.model.library.length} folders</span></span>
  <span class="hints"><span><span class="k">j k</span> move</span><span><span class="k">^t</span> theme</span></span>
</footer>

{#if console !== null}
  <div class="console"><span class="k">&gt;</span><input bind:this={field} bind:value={console} onkeydown={consoleKey} spellcheck="false" /></div>
{/if}
