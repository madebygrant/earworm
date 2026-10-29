<script lang="ts">
  import type { IconName } from "../lib/icons";
  import { press } from "../lib/keys";
  import { settledCount } from "../lib/reducer";
  import { leaveSettings, screens } from "../lib/screens.svelte";
  import { app, walkTheme } from "../lib/store.svelte";
  import { running, send, ui, working } from "../lib/ui.svelte";
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
  // The run's chip opens its folder, as Enter on that folder in the library does.
  // A question, a focused box or the pick gate has the keys, and Esc there means something else entirely.
  const free = $derived(!m.prompt && !ui.typing && !screens.artistTyping && m.picking === null);
  // Settings sits over another screen, so a pill there is judged from that screen and leaves settings first.
  const at = $derived(m.view === "Settings" ? screens.settingsBack : m.view);
  const library = $derived(free && at === "Library");
  const narrowed = $derived(!!(ui.only || ui.filter || ui.byArtist));
  // Only where Esc lands on the library: on a run it first clears a filter or marks, and is refused while working.
  const toLibrary = $derived(
    free && (["Found", "Artists", "Playlist"].includes(at) || (at === "Library" && (narrowed || m.view === "Settings"))
      || (at === "Tracks" && !ui.filter && !m.review && !m.marked.length && !working() && m.library.length > 0)),
  );
  const out = (go: () => void) => () => {
    if (m.view === "Settings") leaveSettings();
    // Back on the library, the Library pill has arrived; anywhere else it still has to press Esc.
    if (go === toLib && app.model.view === "Library" && !narrowed) return;
    go();
  };
  const toLib = () => press("Escape");

  // The last download, as a chip rather than a pill: a run is started by syncing a link, never gone to,
  // so the chip appears once there is one and leads back to its folder from anywhere else.
  const run = $derived(m.run && (!m.run.done || m.library.some((s) => s.path === m.run!.folder) || m.folder === m.run.folder) ? m.run : null);
  const runHere = $derived(!!run && at === "Tracks" && m.folder === run.folder);
  // Lit, the chip names the folder and counts it, so the crumb says only what the run is doing.
  const runOn = $derived((runHere && m.view !== "Settings") || running());
  const runGo = $derived(
    !run || !free || runOn ? null
    : runHere ? leaveSettings
    : working() || !run.folder ? null
    : () => {
        if (m.view === "Settings") leaveSettings();
        void send({ Open: [run.folder!, null] });
      },
  );
  const live = $derived(!!run && !run.done && m.folder === run.folder && m.tracks.length > 0);
  const runSaid = $derived(
    !run ? "" : live ? `${settledCount(m)} of ${m.tracks.length}` : !run.done ? "starting"
    : !run.ok ? "failed" : run.failed ? `${run.failed} failed` : run.look ? `${run.look} to look at` : "done",
  );
  const runTone = $derived(run?.done && (!run.ok || run.failed) ? "error" : run?.done && run.look ? "warn" : "");

  const pills = $derived<Pill[]>([
    // A folder opened from the library is the library's; only the run's own folder is the chip's.
    { name: "Library", icon: "library", on: m.view === "Found" || (m.view === "Library" && ui.only !== "Custom") || (m.view === "Tracks" && m.tracks.length > 0 && !runOn), go: toLibrary ? out(toLib) : null },
    { name: "Artists", icon: "artists", on: m.view === "Artists" || m.view === "Artist", go: library ? out(() => press("b")) : null },
    { name: "Playlists", icon: "playlists", on: m.view === "Playlist" || m.view === "Pick" || (m.view === "Library" && ui.only === "Custom"), go: library ? out(showCustom) : null },
    { name: "Settings", icon: "settings", on: m.view === "Settings", go: free && m.view !== "Pick" ? () => press(",") : null },
  ]);
  // Before a link has any tracks, no pill is lit and the screen is the run's.
  const here = $derived(runOn ? "Run" : (pills.find((p) => p.on)?.name ?? (m.view === "Tracks" ? "Run" : "Library")));
  // The pill already names the screen, so its own word is not said again.
  const said = $derived(word.toLowerCase() === here.toLowerCase() ? "" : word);

  // Under the crumb: a finished run's settings (a running one has them on its card), or where an empty
  // library will fill. Dropped whole rather than cut, since half a description of the run is a wrong one.
  const brief = $derived(
    !m.library.length && !m.tracks.length ? `${app.setup?.dir ?? ""} · nothing synced yet`
    : m.view === "Tracks" && !running() ? m.settings.brief.map(([name, value]) => `${name} ${value}`).join(" · ")
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
        {#if runOn}
          <span class="where">{word || "Run"}</span>
        {:else}
          <span class="where">{here}{#if open}{" / "}<b>{open}</b>{/if}{#if said}{" · "}{said}{/if}</span>
        {/if}
        {#if busy && !m.flash}<span class="spin">{SPIN[tick % SPIN.length]}</span>{/if}
        {#if count && !runOn}<span class="count">{count}</span>{/if}
      </div>
      {#if brief}<div class="brief" bind:this={briefEl} style:visibility={fits ? "visible" : "hidden"} style:height={fits ? null : "0"}>{brief}</div>{/if}
    </div>
    <nav class="pills" aria-label="Screens">
      {#each pills as p (p.name)}
        <button class:on={p.on} aria-current={p.on} title={p.name} disabled={!p.go && !p.on} onclick={() => p.go?.()}><Icon name={p.icon} /><span>{p.name}</span></button>
      {/each}
    </nav>
    <div class="tools">
      {#if run}
        <button class="run" class:on={runOn} aria-current={runOn} title={runOn ? "this run" : `back to the run: ${run.name}`} disabled={!runGo && !runOn} onclick={() => runGo?.()}>
          <Icon name="run" /><span class="name">{run.name}</span><span class="said {runTone}">{runSaid}</span>
          {#if live}<span class="progress"><i style:width="{(100 * settledCount(m)) / m.tracks.length}%"></i></span>{/if}
        </button>
      {/if}
      <button class="search" title="Find a track" disabled={!library} onclick={out(() => press("t"))}><Icon name="search" /><span>Find a track</span></button>
      <button class="icon" title="theme: {app.theme?.name ?? ''}" aria-label="next theme" onclick={() => void walkTheme()}><Icon name="theme" /></button>
    </div>
  </header>
</div>
