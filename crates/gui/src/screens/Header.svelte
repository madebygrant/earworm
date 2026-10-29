<script lang="ts">
  import { settledCount } from "../lib/reducer";
  import { screens } from "../lib/screens.svelte";
  import { app } from "../lib/store.svelte";
  import { working } from "../lib/ui.svelte";

  const SPIN = "⠋⠙⠹⠸⠼⠴⠦⠧⠇⠏";
  let tick = $state(0);
  // On its own derived, or every message would clear the interval before it fired and freeze the spinner.
  const busy = $derived(working());
  $effect(() => {
    if (!busy) return;
    const timer = setInterval(() => tick++, 90);
    return () => clearInterval(timer);
  });

  const m = $derived(app.model);
  // Dropped whole rather than cut: half a description of the run is a wrong one.
  let rightEl = $state<HTMLElement>();
  let fits = $state(true);
  function measure() {
    fits = true;
    requestAnimationFrame(() => (fits = !!rightEl && rightEl.scrollWidth <= rightEl.clientWidth));
  }
  $effect(() => {
    void right;
    measure();
  });
  const open = $derived(
    m.view === "Library" ? (m.library.find((s) => s.path === m.shelf)?.name ?? "")
    : m.view === "Playlist" ? m.custom.name
    : m.view === "Pick" ? screens.pick.name
    : ["Found", "Artists", "Artist", "Settings"].includes(m.view) ? ""
    : m.playlist,
  );
  // A screen's own word, which a message from a key always shows over: a refusal hidden by it reads as a dead key.
  const WORD: Partial<Record<string, string>> = { Found: "search", Playlist: "custom playlist", Pick: "pick tracks", Artists: "artists", Settings: "settings" };
  const word = $derived(m.flash ?? (m.view === "Artist" ? m.page.name : m.busy ? undefined : WORD[m.view]) ?? m.stage);
  const count = $derived(m.view !== "Tracks" || !m.tracks.length ? "" : `${settledCount(m)}/${m.tracks.length}`);
  // Nothing synced yet, which is a different question from whether the first screen is showing.
  const empty = $derived(!m.library.length && !m.tracks.length);
  const right = $derived(empty ? `${app.setup?.dir ?? ""} · nothing synced yet` : m.view === "Library" ? libraryLine() : m.settings.text);

  function libraryLine(): string {
    const folders = m.library.filter((s) => s.kind !== "Custom");
    const tracks = folders.reduce((n, s) => n + s.tracks, 0);
    return `${folders.length} ${folders.length === 1 ? "folder" : "folders"} · ${tracks.toLocaleString()} ${tracks === 1 ? "track" : "tracks"}`;
  }
</script>

<svelte:window onresize={measure} />

<header class="hd" data-tauri-drag-region>
  <span class="brand" data-tauri-drag-region>earworm</span>
  {#if open}<span class="sep">·</span><span data-tauri-drag-region>{open}</span>{/if}
  <span class="sep">·</span>
  {#if busy && !m.flash}<span class="spin">{SPIN[tick % SPIN.length]}</span>{/if}
  <span class="stage" data-tauri-drag-region>{word}</span>
  {#if count}<span class="sep">·</span><span data-tauri-drag-region>{count}</span>{/if}
  <span class="right" data-tauri-drag-region bind:this={rightEl} style:visibility={fits ? "visible" : "hidden"}>{right}</span>
</header>
