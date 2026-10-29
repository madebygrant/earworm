<script lang="ts">
  import type { IconName } from "../lib/icons";
  import { press } from "../lib/keys";
  import { settledCount } from "../lib/reducer";
  import { screens } from "../lib/screens.svelte";
  import { app, walkTheme } from "../lib/store.svelte";
  import { ui, working } from "../lib/ui.svelte";
  import Icon from "./Icon.svelte";

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
  const open = $derived(
    m.view === "Library" ? (m.library.find((s) => s.path === m.shelf)?.name ?? "")
    : m.view === "Playlist" ? m.custom.name
    : m.view === "Pick" ? screens.pick.name
    : m.view === "Artist" ? m.page.name
    : ["Found", "Artists", "Settings"].includes(m.view) ? ""
    : m.playlist,
  );
  // A screen's own word, which a message from a key always shows over: a refusal hidden by it reads as a dead key.
  // The library's stage is only ever "library", which the pill already says.
  const WORD: Partial<Record<string, string>> = { Library: "", Found: "search", Playlist: "custom playlist", Pick: "pick tracks", Artists: "", Artist: "", Settings: "" };
  const word = $derived(m.flash ?? (m.busy ? undefined : WORD[m.view]) ?? m.stage);
  const count = $derived(m.view !== "Tracks" || !m.tracks.length ? "" : `${settledCount(m)} of ${m.tracks.length}`);

  type Pill = { name: string; icon: IconName; on: boolean; go: (() => void) | null };
  // Each pill presses the key that screen answers, so a click is never something the keyboard cannot do.
  // Run has no key: a run is reached by opening its folder, so the pill only says where you are.
  // A question, a focused box or the pick gate has the keys, and Esc there means something else entirely.
  const free = $derived(!m.prompt && !ui.typing && !screens.artistTyping && m.picking === null);
  const library = $derived(free && m.view === "Library");
  const narrowed = $derived(!!(ui.only || ui.filter || ui.byArtist));
  // Only where Esc lands on the library: on a run it first clears a filter or marks, and is refused while working.
  const toLibrary = $derived(
    free && (["Found", "Artists", "Playlist"].includes(m.view) || (m.view === "Library" && narrowed)
      || (m.view === "Tracks" && !ui.filter && !m.review && !m.marked.length && !working() && m.library.length > 0)),
  );
  const pills = $derived<Pill[]>([
    { name: "Library", icon: "library", on: m.view === "Found" || (m.view === "Library" && ui.only !== "Custom"), go: toLibrary ? () => press("Escape") : null },
    { name: "Run", icon: "run", on: m.view === "Tracks", go: null },
    { name: "Artists", icon: "artists", on: m.view === "Artists" || m.view === "Artist", go: library ? () => press("b") : null },
    { name: "Playlists", icon: "playlists", on: m.view === "Playlist" || m.view === "Pick" || (m.view === "Library" && ui.only === "Custom"), go: library ? showCustom : null },
    { name: "Settings", icon: "settings", on: m.view === "Settings", go: free && m.view !== "Pick" ? () => press(",") : null },
  ]);
  const here = $derived(pills.find((p) => p.on)?.name ?? "Library");
  // The pill already names the screen, so its own word is not said again.
  const said = $derived(word.toLowerCase() === here.toLowerCase() ? "" : word);

  // Under the crumb: a run's own settings, or where an empty library will fill. Dropped whole rather than
  // cut, since half a description of the run is a wrong one.
  const brief = $derived(
    !m.library.length && !m.tracks.length ? `${app.setup?.dir ?? ""} · nothing synced yet`
    : m.view === "Tracks" ? m.settings.brief.map(([name, value]) => `${name} ${value}`).join(" · ")
    : "",
  );
  let briefEl = $state<HTMLElement>();
  let fits = $state(true);
  function measure() {
    fits = true;
    requestAnimationFrame(() => (fits = !!briefEl && briefEl.scrollWidth <= briefEl.clientWidth));
  }
  $effect(() => {
    void brief;
    measure();
  });

  // `a` walks the kinds to the same place; the pill goes straight there.
  function showCustom() {
    ui.only = "Custom";
  }
</script>

<svelte:window onresize={measure} />

<div class="look">
  <header data-tauri-drag-region>
    <div class="logo" data-tauri-drag-region><i></i>earworm</div>
    <div class="crumb" data-tauri-drag-region>
      <div class="line" data-tauri-drag-region>
        <span class="where">{here}{#if open}{" / "}<b>{open}</b>{/if}{#if said}{" · "}{said}{/if}</span>
        {#if busy && !m.flash}<span class="spin">{SPIN[tick % SPIN.length]}</span>{/if}
        {#if count}<span class="count">{count}</span>{/if}
      </div>
      {#if brief}<div class="brief" bind:this={briefEl} style:visibility={fits ? "visible" : "hidden"}>{brief}</div>{/if}
    </div>
    <nav class="pills" aria-label="Screens">
      {#each pills as p (p.name)}
        <button class:on={p.on} aria-current={p.on} title={p.name} disabled={!p.go && !p.on} onclick={() => p.go?.()}><Icon name={p.icon} /><span>{p.name}</span></button>
      {/each}
    </nav>
    <div class="tools">
      <button class="search" title="Find a track" disabled={!library} onclick={() => press("t")}><Icon name="search" /><span>Find a track</span></button>
      <button class="icon" title="theme: {app.theme?.name ?? ''}" aria-label="next theme" onclick={() => void walkTheme()}><Icon name="theme" /></button>
    </div>
  </header>
</div>
