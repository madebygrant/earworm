<script lang="ts">
  import { relatedRows, screens, topRows, type Section } from "../lib/screens.svelte";
  import { app } from "../lib/store.svelte";
  import { ui } from "../lib/ui.svelte";

  // One artist's page: releases, top tracks and related artists under Tab. The mark is a glyph and a word,
  // so it reads without colour. Locked, nothing here offers a download or says one could be had.
  const m = $derived(app.model);
  const page = $derived(m.page);
  const releases = $derived(screens.releaseRows.map((at) => page.rows[at]).filter(Boolean));
  const hidden = $derived(page.rows.length - screens.releaseRows.length);
  const row = $derived(m.artists.rows.find((r) => r.name === page.name));
  const TABS: [Section, string][] = [["Releases", "releases"], ["Top", "top tracks"], ["Related", "related artists"]];

  function mark(have: number, of: number | null): [string, string] {
    if (have === 0) return ["○ missing", "var(--muted)"];
    if (of === null) return [`● ${have} here`, "var(--accent)"];
    return have < of ? [`◐ ${have} of ${of}`, "var(--unsure)"] : ["● owned", "var(--accent)"];
  }
  let body = $state<HTMLElement>();
  const at = $derived(screens.section === "Releases" ? `r${screens.release}` : screens.section === "Top" ? `t${screens.topAt}` : `x${screens.relatedAt}`);
  $effect(() => {
    body?.querySelector(`[data-k="${at}"]`)?.scrollIntoView({ block: "nearest" });
  });
  const waiting = (list: typeof page.top | typeof page.related, asking: string, empty: string) =>
    list === null ? asking : "Err" in list ? list.Err : list.Ok.length ? null : empty;
</script>

<section class="view">
  <div class="bar">
    <span class="title" style="color:var(--text)">{page.name}</span>
    {#if row}<span>{row.tracks} {row.tracks === 1 ? "track" : "tracks"} in {row.folders.length} {row.folders.length === 1 ? "folder" : "folders"}{row.starred ? " · starred" : ""}</span>{/if}
    {#if m.searching}<span class="live">searching YouTube Music</span>{/if}
  </div>
  <div class="tabsrow">
    {#each TABS as [key, label] (key)}<span aria-current={screens.section === key}>{label}</span>{/each}
    <span class="muted" style="margin-left:auto"><span class="k">tab</span> next list {#if screens.section === "Releases" && (hidden || screens.allKinds)} · <span class="k">a</span> {screens.allKinds ? "albums and EPs" : `show ${hidden} more`}{/if}</span>
  </div>
  <div bind:this={body}>
    {#if screens.section === "Releases"}
      {#if !page.loaded}<p class="empty-main">Asking Deezer about {page.name}.</p>
      {:else if page.error}<p class="empty-main">{page.error}</p>
      {:else if !page.rows.length}<p class="empty-main">Deezer lists no releases for {page.name}. Esc goes back.</p>
      {:else if !releases.length}<p class="empty-main">No albums or EPs. a shows the other {hidden}.</p>{/if}
      <!-- Tiles, as the run's are, with the mark where a sleeve would be: Deezer's list carries no art. -->
      {#if ui.mode === "tiles"}
        <div class="sheet">
          {#each releases as r (r.id)}
            {@const [word, color] = mark(r.have, r.of)}
            <div class="tile" class:cur={r.id === screens.release} data-k="r{r.id}" role="button" tabindex="-1" onclick={() => (screens.release = r.id)} onkeydown={() => {}}>
              <div class="frame" class:unheld={r.have === 0}><span class="num" style:color>{word.slice(0, 1)}</span></div>
              <div class="cap"><span class="w" style:color>{word}</span>{#if r.new}<span class="live">✦ new</span>{/if}</div>
              <div class="tt">{r.title}</div><div class="ar">{r.year ?? ""} {r.kind}</div>
            </div>
          {/each}
        </div>
      {/if}
      {#each ui.mode === "tiles" ? [] : releases as r (r.id)}
        {@const [word, color] = mark(r.have, r.of)}
        <div class="r rel" class:cur={r.id === screens.release} data-k="r{r.id}" role="button" tabindex="-1"
          onclick={() => (screens.release = r.id)} onkeydown={() => {}}>
          <span class="w" style:color>{word}</span><span class="n">{r.year ?? ""}</span>
          <span class="t">{r.title}{#if r.new} <span class="live">✦ new</span>{/if}</span><span class="src">{r.kind}</span>
        </div>
      {/each}
    {:else if screens.section === "Top"}
      {@const said = waiting(page.top, `Asking Deezer for ${page.name}'s top tracks.`, "Deezer lists no top tracks.")}
      {#if said}<p class="empty-main">{said}</p>{/if}
      {#each topRows() as t, i (i)}
        <div class="r rel" class:cur={i === screens.topAt} data-k="t{i}" role="button" tabindex="-1"
          onclick={() => (screens.topAt = i)} onkeydown={() => {}}>
          <span class="w" style:color={t.have ? "var(--accent)" : "var(--muted)"}>{t.have ? "● here" : "○ missing"}</span><span class="n">{i + 1}</span>
          <span class="t">{t.title}</span><span class="src">{t.album}</span>
        </div>
      {/each}
    {:else}
      {@const said = waiting(page.related, `Asking Deezer who is like ${page.name}.`, "Deezer lists nobody related.")}
      {#if said}<p class="empty-main">{said}</p>{/if}
      <!-- By position: Deezer can list two acts under one name. -->
      {#each relatedRows() as r, i (i)}
        <div class="r rel" class:cur={i === screens.relatedAt} data-k="x{i}" role="button" tabindex="-1"
          onclick={() => (screens.relatedAt = i)} onkeydown={() => {}}>
          <span class="w muted">{r.tracks ? `${r.tracks} here` : ""}</span><span class="n"></span>
          <span class="t">{r.name}</span><span class="src">{r.fans.toLocaleString()} fans</span>
        </div>
      {/each}
    {/if}
  </div>
</section>
