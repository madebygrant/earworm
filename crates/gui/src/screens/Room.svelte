<script lang="ts">
  import type { Track } from "../lib/protocol";
  import { press } from "../lib/keys";
  import { bars, runStats } from "../lib/run";
  import { app } from "../lib/store.svelte";
  import { label, names, tone, ui } from "../lib/ui.svelte";
  import FilterBox from "./FilterBox.svelte";
  import Icon from "./Icon.svelte";
  import TrackRow from "./TrackRow.svelte";
  import Wall from "./Wall.svelte";

  // Only while the run goes; a finished one is the folder's wall, since a settled list is for fixing.
  const m = $derived(app.model);
  const tracks = $derived(m.tracks);
  const statuses = $derived(m.statuses);
  // Each on its own, so a log line, which changes none of them, leaves the table alone.
  const cursor = $derived(m.cursor);
  const marked = $derived(m.marked);
  const logs = $derived(m.logs);
  const shown = $derived(ui.rows.map((at) => tracks[at]).filter(Boolean));
  const s = $derived(runStats(tracks, statuses));
  const heights = $derived(bars(tracks, statuses));
  const spark = $derived(s.line.map((v, i) => `${(i / Math.max(1, s.line.length - 1)) * 100},${36 - v * 34}`).join(" "));
  const pct = $derived(s.total ? Math.round((s.fetched / s.total) * 100) : 0);
  const brief = $derived(m.settings.brief.map(([k, v]) => `${k} ${v}`).join(" · "));

  // `/` and `v` narrow the run as they do a folder, so the table carries the box and the review state.
  function pick(t: Track) {
    app.model = { ...app.model, cursor: t.index, follow: false };
  }
  const first = (list: Track[]) => (list.length ? names(list[0])[0] : "");
  const errorLine = (l: string) => (l.includes("ERROR") ? "error" : l.includes("WARNING") ? "warn" : "");
</script>

<section class="view room">
  <div class="look">
    <div class="watch">
      <div class="tiles">
        <div class="tile">
          <div class="top"><div><div class="n">{s.fetched}<small> / {s.total}</small></div><div class="l">fetched</div></div><span class="ic" style:background="var(--accent)"><Icon name="tracks" /></span></div>
          <div class="bars">{#each heights as h, i (i)}<b style:height="{Math.max(6, h)}%"></b>{/each}</div>
        </div>
        <div class="tile">
          <div class="top"><div><div class="n">{s.confirmed}</div><div class="l">confirmed</div></div><span class="ic" style:background="var(--cursor)"><Icon name="mark" /></span></div>
          <svg class="spark" viewBox="0 0 100 38" preserveAspectRatio="none" aria-hidden="true"><polyline points={spark} /></svg>
        </div>
        <div class="tile">
          <div class="top"><div><div class="n">{s.look.length}</div><div class="l">worth a look</div></div><span class="ic" style:background="var(--warn)"><Icon name="missing" /></span></div>
          {#if s.look.length}<div class="why">{first(s.look)}<span style:color={tone(s.look[0].status)}>{s.look[0].note || label(s.look[0].status)}</span></div>{/if}
        </div>
        <div class="tile">
          <div class="top"><div><div class="n">{s.failed.length}</div><div class="l">failed</div></div><span class="ic" style:background="var(--error)"><Icon name="retry" /></span></div>
          {#if s.failed.length}<div class="why">{first(s.failed)}<span style:color="var(--error)">{s.failed[0].note || label(s.failed[0].status)}</span></div>{/if}
        </div>
      </div>
      <div class="card table">
        <div class="toolbar">
          <div class="seg3" role="group" aria-label="review">
            <button class:on={!m.review} title="all" aria-pressed={!m.review} disabled={ui.typing} onclick={() => m.review && press("v")}><Icon name="all" /></button>
            <button class:on={m.review} title="worth a look" aria-pressed={m.review} disabled={ui.typing} onclick={() => !m.review && press("v")}><Icon name="missing" /></button>
          </div>
          <FilterBox bind:value={ui.filter} typing={ui.typing} placeholder="filter tracks" count={ui.filter || m.review ? `${shown.length} of ${tracks.length}` : ""} />
        </div>
        <Wall at={String(cursor ?? "")} list>
          {#snippet head()}<tr><th>#</th><th></th><th>track</th><th>confidence</th><th>status</th><th>source</th><th>length</th></tr>{/snippet}
          {#each shown as t (t.index)}
            {@const [title, artist] = names(t)}
            <TrackRow num={t.index} word={label(t.status)} color={tone(t.status)} {title} {artist} source={t.source} score={t.score}
              duration={t.duration} thumb={null} cur={t.index === cursor} marked={marked.includes(t.index)} onpick={() => pick(t)} />
          {/each}
        </Wall>
      </div>
    </div>
    <aside class="side">
      <div class="card run">
        <div class="head"><h2>This run</h2></div>
        {#if brief}<p class="brief">{brief}</p>{/if}
        <div class="gauge">
          <svg viewBox="0 0 260 140" aria-hidden="true">
            <defs><linearGradient id="arc" x1="0" x2="1"><stop offset="0" style="stop-color: var(--accent2)" /><stop offset="1" style="stop-color: var(--accent)" /></linearGradient></defs>
            <path d="M 20 130 A 110 110 0 0 1 240 130" pathLength="100" class="track" />
            <path d="M 20 130 A 110 110 0 0 1 240 130" pathLength="100" class="fill" stroke-dasharray="{pct} 100" />
          </svg>
          <div class="big">{pct}%</div>
          <div class="cap">{s.fetched} of {s.total} fetched · {s.confirmed} confirmed</div>
        </div>
        <div class="kv">
          <div><b>{s.look.length}</b><span>worth a look</span></div><div><b>{s.failed.length}</b><span>failed</span></div>
          <div><b>{s.queued}</b><span>queued</span></div><div><b>{s.skipped}</b><span>skipped</span></div>
        </div>
      </div>
      {#if ui.logs}
        <div class="card">
          <div class="head"><h2>yt-dlp</h2><button class="hide" title="hide the log" onclick={() => (ui.logs = false)}><Icon name="hide" /></button></div>
          <div class="log" class:tall={ui.logsTall}>{#each logs as l, i (i)}<div class={errorLine(l)}>{l}</div>{/each}</div>
        </div>
      {/if}
    </aside>
  </div>
</section>
