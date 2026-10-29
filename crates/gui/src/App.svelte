<script lang="ts">
  import { invoke } from "@tauri-apps/api/core";
  import { getCurrentWindow } from "@tauri-apps/api/window";
  import { onMount } from "svelte";
  import { clearFilter, consoleHooks, handleKey, press } from "./lib/keys";
  import type { Shelf } from "./lib/protocol";
  import { firstScreen, settledCount } from "./lib/reducer";
  import { app, boot, hooks, tryCode } from "./lib/store.svelte";
  import { quit, trackRows, ui } from "./lib/ui.svelte";
  import Detail from "./screens/Detail.svelte";
  import Dialog from "./screens/Dialog.svelte";
  import FirstLaunch from "./screens/FirstLaunch.svelte";
  import Footer from "./screens/Footer.svelte";
  import Header from "./screens/Header.svelte";
  import Pick from "./screens/Pick.svelte";
  import Quit from "./screens/Quit.svelte";
  import Rail from "./screens/Rail.svelte";
  import Run from "./screens/Run.svelte";
  import ShelfView from "./screens/ShelfView.svelte";

  const m = $derived(app.model);
  const tracks = $derived(m.tracks);
  const library = $derived(m.library);
  const first = $derived(firstScreen(m) ? m.prompt!.prompt : null);
  const dialog = $derived(m.prompt && !first ? m.prompt.prompt : null);
  const detail = $derived(!first && (m.view === "Tracks" ? m.tracks.length > 0 : m.library.length > 0));

  // Core's answers to what each list shows, asked again only when what they depend on changes. A reply is
  // kept only while it still answers the latest question.
  // Each input derived on its own, so a log line or a progress tick does not ask again.
  const review = $derived(m.review);
  let asked = 0;
  $effect(() => {
    const [t, filter, id] = [tracks, ui.filter, ++asked];
    void trackRows(t, filter, review).then((rows) => {
      if (id === asked) ui.rows = rows;
    });
  });
  let askedShelves = 0;
  $effect(() => {
    const [lib, filter, only, sort, id] = [library, ui.filter, ui.only, ui.sort, ++askedShelves];
    void invoke<number[]>("shelf_rows", { library: lib, filter, only, folders: null, sort }).then((rows) => {
      if (id === askedShelves) ui.shelves = rows;
    });
  });

  // A cursor on a row nobody can see is a selection every key still acts on, so it goes to one that shows.
  $effect(() => {
    const shown = ui.rows.map((at) => tracks[at]?.index).filter((i): i is number => i !== undefined);
    const at = app.model.cursor;
    if (!shown.length || (at !== null && shown.includes(at))) return;
    const nearest = at === null ? shown[0] : shown.reduce((a, b) => (Math.abs(b - at) < Math.abs(a - at) ? b : a));
    app.model = { ...app.model, cursor: nearest };
  });
  $effect(() => {
    const paths = ui.shelves.map((at) => library[at]?.path);
    if (paths.length && !paths.includes(app.model.shelf ?? "")) app.model = { ...app.model, shelf: paths[0] ?? null };
  });

  // The window title follows the run, so a window you aren't looking at says how far it got.
  // Derived as a string, so the window is told only when the words change.
  const title = $derived.by(() => {
    const where = m.view === "Library" ? (library.find((s: Shelf) => s.path === m.shelf)?.name ?? "") : m.playlist;
    const how = m.view === "Library" || !tracks.length ? "" : m.done !== null ? m.stage : `${settledCount(m)}/${tracks.length}`;
    return ["earworm", where, how].filter(Boolean).join(" · ");
  });
  $effect(() => {
    void getCurrentWindow().setTitle(title);
  });

  // Cancelling the first URL prompt ends the session, as it does in the terminal.
  $effect(() => {
    if (m.quit) void quit();
  });

  let code = $state<string | null>(null);
  let field = $state<HTMLInputElement>();
  consoleHooks.open = () => {
    code = "";
    queueMicrotask(() => field?.focus());
  };
  consoleHooks.isOpen = () => code !== null;

  function consoleKey(e: KeyboardEvent) {
    if (e.key === "Escape") code = null;
    else if (e.key === "Enter" && code) {
      void tryCode(code);
      code = null;
    }
  }

  hooks.restart = clearFilter;
  onMount(() => void boot());

  // Dev builds report what is on screen, for checks that cannot take a screenshot.
  $effect(() => {
    if (!import.meta.env.DEV) return;
    const report = {
      view: m.view,
      first: !!first,
      dialog: dialog ? Object.keys(dialog)[0] : null,
      note: m.prompt ? (Object.values(m.prompt.prompt)[0] as { note: string }).note : null,
      picking: m.picking !== null,
      rail: ui.shelves.map((at) => library[at]?.name),
      selected: library.find((s) => s.path === m.shelf)?.name ?? null,
      tracks: tracks.map((t) => `${t.index}:${t.status}`).join(" "),
      shown: ui.rows.length,
      mode: ui.mode,
      quitting: ui.quitting,
      filter: ui.filter,
      marked: m.marked,
      theme: app.theme?.name ?? null,
      accent: app.theme ? getComputedStyle(document.documentElement).getPropertyValue("--accent").trim() : null,
      header: m.flash ?? m.stage,
      error: app.error,
    };
    void invoke("probe", { report });
  });
</script>

<!-- A click never leaves a button holding the focus, or the next space or Enter would press it again. -->
<svelte:window onkeydown={handleKey} onmousedown={(e) => (e.target as HTMLElement).closest("button") && e.preventDefault()} />

<Header />

<main class="app" class:no-detail={!detail}>
  <Rail />
  {#if app.error}
    <section class="view"><div class="fail"><h1>earworm could not start</h1><pre>{app.error}</pre></div></section>
  {:else if first}
    <FirstLaunch prompt={first as Extract<typeof first, { Input: unknown }>} />
  {:else if m.picking !== null}
    <Pick {press} />
  {:else if m.view === "Tracks" && m.tracks.length}
    <Run />
  {:else}
    <ShelfView />
  {/if}
  {#if detail}<Detail {press} />{/if}
</main>

<Footer />

{#if dialog}<Dialog prompt={dialog} />{/if}
{#if ui.quitting}<Quit />{/if}
{#if code !== null}
  <div class="console"><span class="k">&gt;</span><input bind:this={field} bind:value={code} onkeydown={consoleKey} spellcheck="false" /></div>
{/if}
