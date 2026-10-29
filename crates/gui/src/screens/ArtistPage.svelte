<script lang="ts">
  import type { IconName } from "../lib/icons";
  import { press } from "../lib/keys";
  import { pageSection, relatedRows, screens, topRows, type Section } from "../lib/screens.svelte";
  import { app } from "../lib/store.svelte";
  import { ui } from "../lib/ui.svelte";
  import Icon from "./Icon.svelte";
  import Wall from "./Wall.svelte";

  // One artist's page: releases, top tracks and related artists under Tab. The mark is a glyph and a word,
  // so it reads without colour. Locked, nothing here offers a download or says one could be had.
  const m = $derived(app.model);
  const page = $derived(m.page);
  const releases = $derived(screens.releaseRows.map((at) => page.rows[at]).filter(Boolean));
  const hidden = $derived(page.rows.length - screens.releaseRows.length);
  const TABS: [Section, IconName, string][] = [["Releases", "album", "releases"], ["Top", "tracks", "top tracks"], ["Related", "artists", "related artists"]];

  function mark(have: number, of: number | null): [string, string] {
    if (have === 0) return ["○ missing", "var(--muted)"];
    if (of === null) return [`● ${have} here`, "var(--accent)"];
    return have < of ? [`◐ ${have} of ${of}`, "var(--unsure)"] : ["● owned", "var(--accent)"];
  }
  const at = $derived(screens.section === "Releases" ? `r${screens.release}` : screens.section === "Top" ? `t${screens.topAt}` : `x${screens.relatedAt}`);
  const waiting = (list: typeof page.top | typeof page.related, asking: string, empty: string) =>
    list === null ? asking : "Err" in list ? list.Err : list.Ok.length ? null : empty;
  const said = $derived(
    screens.section === "Releases"
      ? !page.loaded ? `Asking Deezer about ${page.name}.`
        : page.error ? page.error
        : !page.rows.length ? `Deezer lists no releases for ${page.name}. Esc goes back.`
        : !releases.length ? `No albums or EPs. a shows the other ${hidden}.` : null
      : screens.section === "Top" ? waiting(page.top, `Asking Deezer for ${page.name}'s top tracks.`, "Deezer lists no top tracks.")
      : waiting(page.related, `Asking Deezer who is like ${page.name}.`, "Deezer lists nobody related."),
  );
  const tiles = $derived(screens.section === "Releases" && ui.mode === "tiles");
</script>

<section class="view walled">
  <div class="look">
    <div class="toolbar">
      <div class="seg3" role="group" aria-label="list">
        {#each TABS as [section, icon, title] (section)}
          <button class:on={screens.section === section} {title} aria-pressed={screens.section === section} onclick={() => pageSection(section)}><Icon name={icon} /></button>
        {/each}
      </div>
      {#if screens.section === "Releases"}
        {#if hidden || screens.allKinds}
          <div class="seg3" role="group" aria-label="kinds">
            <button class:on={!screens.allKinds} title="albums and EPs" aria-pressed={!screens.allKinds} onclick={() => screens.allKinds && press("a")}><Icon name="album" /></button>
            <button class:on={screens.allKinds} title="every release" aria-pressed={screens.allKinds} onclick={() => !screens.allKinds && press("a")}><Icon name="all" /></button>
          </div>
        {/if}
        <div class="seg3" role="group" aria-label="view">
          <button class:on={ui.mode === "tiles"} title="tiles" aria-pressed={ui.mode === "tiles"} onclick={() => (ui.mode = "tiles")}><Icon name="library" /></button>
          <button class:on={ui.mode === "list"} title="list" aria-pressed={ui.mode === "list"} onclick={() => (ui.mode = "list")}><Icon name="list" /></button>
        </div>
      {/if}
      {#if m.searching}<span class="live">searching YouTube Music</span>{/if}
    </div>
    {#if said}<p class="none">{said}</p>{/if}
    <Wall {at} list={!tiles}>
      {#snippet head()}
        {#if screens.section === "Releases"}<tr><th></th><th>release</th><th>year</th><th>kind</th></tr>
        {:else if screens.section === "Top"}<tr><th>#</th><th></th><th>track</th><th>album</th></tr>
        {:else}<tr><th>artist</th><th>here</th><th>fans</th></tr>{/if}
      {/snippet}
      {#if screens.section === "Releases"}
        {#each releases as r (r.id)}
          {@const [word, color] = mark(r.have, r.of)}
          {#if tiles}
            <div class="s empty" class:unheld={r.have === 0} class:cur={r.id === screens.release} data-k="r{r.id}" role="button" tabindex="-1"
              onclick={() => (screens.release = r.id)} onkeydown={() => {}}>
              <span style:color>{word.slice(0, 1)}</span>
              <span class="st" style:color>{word.slice(2)}</span>
              {#if r.new}<span class="kind fresh">new</span>{/if}
              <div class="cap">{r.title}<small>{r.year ?? ""} {r.kind}</small></div>
            </div>
          {:else}
            <tr class:cur={r.id === screens.release} data-k="r{r.id}" onclick={() => (screens.release = r.id)}>
              <td class="status" style:color>{word}</td>
              <td><span class="t">{r.title}</span>{#if r.new} <span class="live">new</span>{/if}</td>
              <td class="a">{r.year ?? ""}</td><td class="src">{r.kind}</td>
            </tr>
          {/if}
        {/each}
      {:else if screens.section === "Top"}
        {#each topRows() as t, i (i)}
          <tr class:cur={i === screens.topAt} data-k="t{i}" onclick={() => (screens.topAt = i)}>
            <td class="a">{i + 1}</td><td class="status" style:color={t.have ? "var(--accent)" : "var(--muted)"}>{t.have ? "● here" : "○ missing"}</td>
            <td class="t">{t.title}</td><td class="a">{t.album}</td>
          </tr>
        {/each}
      {:else}
        <!-- By position: Deezer can list two acts under one name. -->
        {#each relatedRows() as r, i (i)}
          <tr class:cur={i === screens.relatedAt} data-k="x{i}" onclick={() => (screens.relatedAt = i)}>
            <td class="t">{r.name}</td><td class="a">{r.tracks ? `${r.tracks} tracks` : ""}</td><td class="src">{r.fans.toLocaleString()}</td>
          </tr>
        {/each}
      {/if}
    </Wall>
  </div>
</section>
