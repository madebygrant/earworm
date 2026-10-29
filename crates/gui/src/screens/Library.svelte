<script lang="ts">
  import { invoke } from "@tauri-apps/api/core";
  import { onDestroy, onMount, untrack } from "svelte";
  import { SvelteSet } from "svelte/reactivity";
  import type { IconName } from "../lib/icons";
  import { press } from "../lib/keys";
  import type { Kind, Shelf, Sort } from "../lib/protocol";
  import { app } from "../lib/store.svelte";
  import { askThumbs, ui, working } from "../lib/ui.svelte";
  import { ago, kindIcon, kindWord, shelfArt } from "../lib/wall";
  import Art from "./Art.svelte";
  import FilterBox from "./FilterBox.svelte";
  import Icon from "./Icon.svelte";
  import Wall from "./Wall.svelte";

  const m = $derived(app.model);
  const rows = $derived(ui.shelves.map((at) => m.library[at]).filter(Boolean));

  // Core's kinds and sorts, as `a` and `s` walk them, so a click lands on the same ones the keys do.
  const ICON: Record<string, IconName> = { all: "all", Album: "album", Playlist: "playlist", Custom: "custom", Name: "alpha", Synced: "clock", Missing: "missing" };
  let kinds = $state<[Kind | null, string][]>([]);
  let sorts = $state<[Sort, string][]>([]);
  async function walk<T>(from: T, next: (at: T) => Promise<[T, string]>): Promise<[T, string][]> {
    const out: [T, string][] = [];
    let at = from;
    do {
      const got = await next(at);
      out.push(got);
      at = got[0];
    } while (at !== from && out.length < 16);
    // The walk ends where it started, which goes first.
    return [out[out.length - 1], ...out.slice(0, -1)];
  }
  onMount(async () => {
    kinds = await walk<Kind | null>(null, (only) => invoke("next_kind", { only }));
    sorts = await walk<Sort>("Name", (sort) => invoke("next_sort", { sort }));
  });

  // Covers for the folders scrolled into view, so a big library is not one long batch the next command waits
  // behind. `wanted` is read tracked, so a folder scrolling in asks; `askThumbs` runs untracked, since it reads
  // `app.model` and every message would ask again.
  // One observer for every tile, each tile's folder looked up by its element.
  const seen = new SvelteSet<string>();
  const paths = new Map<Element, string>();
  const io = new IntersectionObserver((entries) => {
    for (const e of entries) if (e.isIntersecting && paths.has(e.target)) seen.add(paths.get(e.target)!);
  });
  onDestroy(() => io.disconnect());
  function watch(node: HTMLElement, path: string) {
    paths.set(node, path);
    io.observe(node);
    return {
      update: (to: string) => paths.set(node, to),
      destroy: () => (io.unobserve(node), paths.delete(node)),
    };
  }
  const library = $derived(m.library);
  const artwork = $derived(m.artwork);
  const busy = $derived(working());
  const wanted = $derived(library.filter((s) => seen.has(s.path)).flatMap((s) => { const a = shelfArt(s); return typeof a === "object" ? a.files : []; }));
  $effect(() => {
    void artwork;
    const files = wanted;
    if (!busy) untrack(() => askThumbs(files));
  });

  function select(s: Shelf) {
    app.model = { ...app.model, shelf: s.path };
  }

  function sub(s: Shelf): string {
    const n = `${s.tracks} ${s.tracks === 1 ? "track" : "tracks"}`;
    return s.kind === "Custom" || s.synced === null ? n : `${n} · ${ago(s.synced)}`;
  }
  const missing = (s: Shelf) => (s.kind !== "Custom" && s.missing > 0 ? s.missing : 0);
  // Dashed where there is no cover to wait for.
  const bare = (s: Shelf) => typeof shelfArt(s) !== "object";
</script>

<section class="view walled">
  <div class="look">
    {#if m.library.length}
      <div class="toolbar">
        <div class="seg3" role="group" aria-label="kind">
          {#each kinds as [kind, title] (title)}
            <button class:on={ui.only === kind} {title} aria-pressed={ui.only === kind} onclick={() => (ui.only = kind)}><Icon name={ICON[kind ?? "all"] ?? "all"} /></button>
          {/each}
        </div>
        <div class="seg3" role="group" aria-label="sort">
          {#each sorts as [sort, title] (sort)}
            <button class:on={ui.sort === sort} title="sorted by {title}" aria-pressed={ui.sort === sort} onclick={() => (ui.sort = sort)}><Icon name={ICON[sort] ?? "alpha"} /></button>
          {/each}
        </div>
        <div class="seg3" role="group" aria-label="view">
          <button class:on={ui.mode === "tiles"} title="tiles" aria-pressed={ui.mode === "tiles"} onclick={() => (ui.mode = "tiles")}><Icon name="library" /></button>
          <button class:on={ui.mode === "list"} title="list" aria-pressed={ui.mode === "list"} onclick={() => (ui.mode = "list")}><Icon name="list" /></button>
        </div>
        <FilterBox bind:value={ui.filter} typing={ui.typing} placeholder="filter folders" said={ui.byArtist ? `${ui.byArtist[0]}'s folders` : ""} />
      </div>
    {/if}
    {#if rows.length}
      <Wall at={m.shelf ?? ""} list={ui.mode === "list"}>
        {#snippet head()}<tr><th></th><th>folder</th><th>kind</th><th>tracks</th><th>synced</th></tr>{/snippet}
        {#each rows as s (s.path)}
          {#if ui.mode === "list"}
            <tr class:cur={s.path === m.shelf} data-k={s.path} use:watch={s.path} onclick={() => select(s)} ondblclick={() => press("Enter")}>
              <td><span class="sleeve" class:empty={bare(s)}><Art shelf={s} /></span></td>
              <td><span class="t">{s.name}</span></td>
              <td class="a" title={kindWord(s)}><Icon name={kindIcon(s)} /></td>
              <td>{s.tracks}{#if missing(s)} <span class="warn">{missing(s)} missing</span>{/if}</td>
              <td class="a">{s.kind === "Custom" ? "" : ago(s.synced)}</td>
            </tr>
          {:else}
            <div class="s" class:empty={bare(s)} class:cur={s.path === m.shelf} data-k={s.path} use:watch={s.path} role="button" tabindex="-1"
              onclick={() => select(s)} ondblclick={() => press("Enter")} onkeydown={() => {}}>
              <Art shelf={s} />
              {#if missing(s)}<span class="st warn">{missing(s)} missing</span>{/if}
              <span class="kind" title={kindWord(s)}><Icon name={kindIcon(s)} /></span>
              <div class="cap">{s.name}<small>{sub(s)}</small></div>
            </div>
          {/if}
        {/each}
      </Wall>
    {:else if m.library.length}
      <p class="none">Nothing matches. Esc clears the filter.</p>
    {:else if !app.error}
      <p class="none">No folders yet. n syncs a new link.</p>
    {/if}
  </div>
</section>
