<script lang="ts">
  import { invoke } from "@tauri-apps/api/core";
  import { getCurrentWindow } from "@tauri-apps/api/window";
  import { onMount, untrack } from "svelte";
  import { clearFilter, consoleHooks, handleKey, press } from "./lib/keys";
  import type { Shelf } from "./lib/protocol";
  import { firstScreen, settledCount } from "./lib/reducer";
  import { app, boot, hooks, tryCode } from "./lib/store.svelte";
  import { quit, running as isRunning, trackRows, ui } from "./lib/ui.svelte";
  import * as sc from "./lib/screens.svelte";
  import { screens } from "./lib/screens.svelte";
  import ArtistPage from "./screens/ArtistPage.svelte";
  import Artists from "./screens/Artists.svelte";
  import Custom from "./screens/Custom.svelte";
  import Detail from "./screens/Detail.svelte";
  import Discard from "./screens/Discard.svelte";
  import Help from "./screens/Help.svelte";
  import Picker from "./screens/Picker.svelte";
  import Search from "./screens/Search.svelte";
  import Settings from "./screens/Settings.svelte";
  import Toast from "./screens/Toast.svelte";
  import Dialog from "./screens/Dialog.svelte";
  import FirstLaunch from "./screens/FirstLaunch.svelte";
  import Bar from "./screens/Bar.svelte";
  import Header from "./screens/Header.svelte";
  import Pick from "./screens/Pick.svelte";
  import Quit from "./screens/Quit.svelte";
  import Run from "./screens/Run.svelte";
  import Library from "./screens/Library.svelte";
  import Room from "./screens/Room.svelte";
  import sprite from "./icons.svg?raw";

  const m = $derived(app.model);
  const tracks = $derived(m.tracks);
  const library = $derived(m.library);
  const first = $derived(firstScreen(m) ? m.prompt!.prompt : null);
  const dialog = $derived(m.prompt && !first ? m.prompt.prompt : null);
  // A run in progress has its own right-hand column, the run's card, in place of the sheet.
  const running = $derived(!first && isRunning());
  const detail = $derived(
    !first && !running &&
      (m.view === "Tracks" ? m.tracks.length > 0
      : m.view === "Library" ? m.library.length > 0
      : m.view === "Found" ? screens.hits.length > 0
      : m.view === "Playlist" ? m.custom.rows.length > 0
      : m.view === "Artists" ? screens.artistRows.length > 0
      : m.view === "Pick" || m.view === "Artist"),
  );

  // `N` named a list: the worker carries on and the picker is the page's, so it opens here, once.
  $effect(() => {
    const name = m.pickTracks;
    if (name === null) return;
    untrack(() => {
      app.model = { ...app.model, pickTracks: null };
      sc.openPicker(name);
    });
  });
  // Core's answers for each screen's list, asked again only when what they read changes. Untracked, or the
  // state a refresh reads and then writes would make the effect its own trigger.
  const view = $derived(m.view);
  $effect(() => {
    void [library, ui.filter, ui.only];
    if (view === "Found") untrack(() => void sc.searchRefresh());
  });
  $effect(() => {
    void library;
    if (view === "Pick") untrack(() => void sc.pickRefresh());
  });
  const artistRows = $derived(m.artists.rows);
  const newReleases = $derived(m.newReleases);
  $effect(() => {
    void [artistRows, screens.artistQuery, newReleases];
    untrack(() => void sc.artistsRefresh());
  });
  const releases = $derived(m.page.rows);
  $effect(() => {
    void [releases, screens.allKinds];
    untrack(() => void sc.releasesRefresh());
  });
  // What could earn an achievement changed; a toast waits for a calm screen.
  // A string of what the rules read, so a progress tick, which changes no status, asks nothing.
  const facts = $derived(
    JSON.stringify([library.map((s) => [s.path, s.tracks, s.kind, s.url !== null]), tracks.map((t) => t.status), m.done, m.resting, m.playback]),
  );
  $effect(() => {
    void facts;
    untrack(() => void sc.checkPrizes());
  });
  $effect(() => {
    void [screens.waiting.length, sc.calm()];
    untrack(() => sc.showPrizes());
  });

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
    const [lib, filter, only, sort, folders, id] = [library, ui.filter, ui.only, ui.sort, ui.byArtist?.[1] ?? null, ++askedShelves];
    void invoke<number[]>("shelf_rows", { library: lib, filter, only, folders, sort }).then((rows) => {
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
      wall: ui.shelves.map((at) => library[at]?.name),
      selected: library.find((s) => s.path === m.shelf)?.name ?? null,
      tracks: tracks.map((t) => `${t.index}:${t.status}`).join(" "),
      shown: ui.rows.length,
      mode: ui.mode,
      quitting: ui.quitting,
      help: screens.help,
      toast: screens.toast?.prizes.map((p) => p.id) ?? null,
      hits: screens.hits.length,
      found: screens.found?.[1] ?? null,
      pick: m.view === "Pick" ? { name: screens.pick.name, rows: screens.pick.rows.length, marks: screens.pick.marks.length, add: screens.pick.add, remove: screens.pick.remove } : null,
      custom: m.view === "Playlist" ? m.custom.rows.map((r) => r.label) : null,
      artists: m.view === "Artists" || m.view === "Artist" ? screens.artistRows.length : null,
      artist: screens.artist,
      page: m.view === "Artist" ? { name: m.page.name, section: screens.section, releases: screens.releaseRows.length, error: m.page.error } : null,
      settingsAt: m.view === "Settings" ? m.settings.rows[screens.settingsAt]?.key : null,
      hints: [...document.querySelectorAll(".bar .hints > span")].map((e) => e.textContent?.trim()).join(" | "),
      byArtist: ui.byArtist?.[0] ?? null,
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

<!-- Once, for every Icon's <use>; positioned out of the body's grid. -->
{@html sprite}

<div class="shell stage">
<Header />

<main class="app" class:no-detail={!detail}>
  {#if app.error}
    <section class="view"><div class="fail"><h1>earworm could not start</h1><pre>{app.error}</pre></div></section>
  {:else if first}
    <FirstLaunch prompt={first as Extract<typeof first, { Input: unknown }>} />
  {:else if m.picking !== null}
    <Pick {press} />
  {:else if m.view === "Found"}
    <Search />
  {:else if m.view === "Playlist"}
    <Custom />
  {:else if m.view === "Pick"}
    <Picker />
  {:else if m.view === "Artists"}
    <Artists />
  {:else if m.view === "Artist"}
    <ArtistPage />
  {:else if m.view === "Settings"}
    <Settings />
  {:else if running}
    <Room />
  {:else if m.view === "Tracks" && m.tracks.length}
    <Run />
  {:else}
    <Library />
  {/if}
  {#if detail}<Detail {press} />{/if}
</main>

<Bar />
</div>

{#if dialog}<Dialog prompt={dialog} />{/if}
{#if ui.quitting}<Quit />{/if}
{#if screens.help}<Help />{/if}
{#if screens.discard}<Discard />{/if}
<Toast />
{#if code !== null}
  <div class="console"><span class="k">&gt;</span><input bind:this={field} bind:value={code} onkeydown={consoleKey} spellcheck="false" /></div>
{/if}
