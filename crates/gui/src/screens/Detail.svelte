<script lang="ts">
  import { convertFileSrc } from "@tauri-apps/api/core";
  import type { Shelf, Status, Track } from "../lib/protocol";
  import { app } from "../lib/store.svelte";
  import { label, names, short, tone } from "../lib/ui.svelte";
  import { complete, customRow, foundAt, relatedRows, releaseSelected, screens, topRows } from "../lib/screens.svelte";

  interface Props {
    /** Runs the key a button stands for, so a click and the key are one action. */
    press: (key: string) => void;
  }
  let { press }: Props = $props();

  const m = $derived(app.model);
  const track = $derived(m.view === "Tracks" ? (m.tracks.find((t) => t.index === m.cursor) ?? null) : null);
  const shelf = $derived(m.view === "Library" ? (m.library.find((s) => s.path === m.shelf) ?? null) : null);

  // What the pane says under the status: plain words, and what to do next where there is something to do.
  const SAY: Partial<Record<Status, (t: Track) => string>> = {
    Ok: (t) => `Confirmed by ${t.source || "a lookup"}.`,
    Manual: () => "You chose this match.",
    Have: () => "Already in the folder. Tags read from the file.",
    Kept: () => "Read off the video title. Nothing confirmed it.",
    Weak: (t) => `${t.source || "The lookup"} found a match that didn't fit.`,
    NoMatch: () => "The lookup found nothing to match it to.",
    Failed: (t) => t.note || "It didn't work. The log has the full error.",
    Gone: () => "This video has left the playlist. The file stays until you delete it.",
    Skipped: () => "Left out at the pick.",
    Local: () => "A file of yours that the playlist doesn't have. Nothing touches it, it stays out of the .m3u8, and D never deletes it.",
    Pending: () => "Waiting its turn.",
    Downloading: () => "Downloading.",
    Downloaded: () => "Downloaded. Looking it up.",
    Tagging: () => "Writing tags and art.",
    Converting: () => "Converting.",
  };

  // The keys that fix a track in this state, first the one most likely wanted.
  function acts(t: Track): [string, string][] {
    if (["Kept", "Weak", "NoMatch"].includes(t.status)) return [["T", "search again"], ["e", "edit"]];
    if (t.status === "Failed") return [["r", "retry failed"], ["l", "log"]];
    if (t.status === "Gone") return t.departure_proven ? [["D", "delete departed"]] : [];
    // `u` only while the worker holds something to put back: a key that can only fail is worse than none.
    if (["Ok", "Manual", "Have"].includes(t.status)) return [["e", "edit"], ["c", "cover"], ...(m.undoable ? [["u", "undo"] as [string, string]] : [])];
    return [];
  }

  function length(secs: number): string {
    const [h, mm, ss] = [Math.floor(secs / 3600), Math.floor((secs % 3600) / 60), secs % 60];
    const two = (n: number) => String(n).padStart(2, "0");
    return h ? `${h}:${two(mm)}:${two(ss)}` : `${mm}:${two(ss)}`;
  }

  function ago(secs: number | null): string {
    if (!secs) return "never";
    const days = Math.floor((Date.now() / 1000 - secs) / 86400);
    return days < 1 ? "today" : days === 1 ? "yesterday" : `${days} days ago`;
  }

  const kindWord = (s: Shelf) => (s.kind === "Album" ? "● album" : s.kind === "Custom" ? "≡ custom playlist" : s.url ? "▤ playlist" : "◆ local folder");
  const missing = $derived(shelf ? shelf.files.filter(([, here]) => !here).map(([name]) => name) : []);
  const found = $derived(m.view === "Found" ? foundAt() : null);
  const entry = $derived(m.view === "Playlist" ? customRow() : undefined);
  const release = $derived(m.view === "Artist" && screens.section === "Releases" ? releaseSelected() : undefined);
  const top = $derived(m.view === "Artist" && screens.section === "Top" ? topRows()[screens.topAt] : undefined);
  const related = $derived(m.view === "Artist" && screens.section === "Related" ? relatedRows()[screens.relatedAt] : undefined);
  const order = $derived(m.view === "Pick" ? screens.pick.marks.map(([, file]) => file.replace(/\.[^.]+$/, "").replace(/^\d+\s*-\s*/, "")) : []);
  // Core's answer to what Enter takes off, never worked out here.
  const leaving = $derived(m.view === "Pick" ? screens.pick.removing : []);
</script>

<aside class="detail">
  {#if track && m.picking !== null}
    {@const [title, artist] = names(track)}
    {@const on = m.marked.includes(track.index)}
    <div class="none">no sleeve until it's downloaded</div>
    <h2>{title}{artist ? ` - ${artist}` : ""}</h2>
    <p class="say">
      {#if track.status === "Have"}Already in the folder, so there is nothing to fetch.
      {:else if track.twin}{track.twin}. Leave it selected and earworm copies that file across instead of downloading it again.
      {:else if track.duration > 1800}Over half an hour long. Probably not one track.
      {:else}{on ? "Selected." : "Not selected."}{/if}
    </p>
    <dl class="kv">
      <dt>video</dt><dd>{track.was}</dd>
      {#if track.duration}<dt>length</dt><dd>{length(track.duration)}</dd>{/if}
    </dl>
  {:else if track}
    {@const [title, artist] = names(track)}
    {@const thumb = track.path ? m.thumbs[track.path] : null}
    {#if thumb}
      <img class="cover" src={convertFileSrc(thumb)} alt="" />
    {:else}
      <div class="none">{track.status === "Failed" ? "no sleeve, the download failed" : "no sleeve yet"}</div>
    {/if}
    <h2>{title}{artist ? ` - ${artist}` : ""}</h2>
    <p class="say">{SAY[track.status]?.(track) ?? ""}</p>
    <dl class="kv">
      <dt>status</dt><dd style:color={tone(track.status)}>{label(track.status)}</dd>
      <!-- Only what the tags hold so far: before the tag pass there is nothing to show. -->
      {#each [["artist", track.artist], ["title", track.title], ["album", track.album], ["year", track.year ?? ""], ["source", track.source], ["video", track.was]] as [key, value] (key)}
        {#if value}<dt>{key}</dt><dd>{value}</dd>{/if}
      {/each}
      {#if track.twin}<dt>twin</dt><dd class="twin">{track.twin}</dd>{/if}
      {#if track.path}<dt>file</dt><dd>{short(track.path)}</dd>{/if}
    </dl>
    {#if m.done !== null}
      <div class="acts">
        {#each acts(track) as [key, what] (key)}<button onclick={() => press(key)}><span class="kc">{key}</span>{what}</button>{/each}
      </div>
    {/if}
  {:else if shelf}
    <h2>{shelf.name}</h2>
    <p class="say">{shelf.kind === "Album" ? "An album. Its tracks share one sleeve." : shelf.url ? "A playlist. Each track keeps its own sleeve." : "A folder earworm did not download. S asks for its link."}</p>
    <dl class="kv">
      <dt>kind</dt><dd>{kindWord(shelf)}</dd>
      <dt>tracks</dt><dd>{shelf.tracks}{#if shelf.missing}, <span class="warn">{shelf.missing} missing</span>{/if}</dd>
      <dt>synced</dt><dd>{ago(shelf.synced)}</dd>
      {#if shelf.url}<dt>url</dt><dd>{shelf.url}</dd>{/if}
      <dt>folder</dt><dd>{short(shelf.path)}</dd>
      {#if shelf.playlist}<dt>m3u8</dt><dd>{shelf.playlist.split("/").pop()}</dd>{/if}
    </dl>
    {#if missing.length}
      <ul class="files">{#each missing as name (name)}<li class="miss">! {name}</li>{/each}</ul>
    {/if}
    <div class="acts">
      {#each [["enter", "open"], ["S", "sync"], ["O", "show folder"], ["e", "rename"], ["D", "remove"]] as [key, what] (key)}
        <button onclick={() => press(key === "enter" ? "Enter" : key)}><span class="kc">{key}</span>{what}</button>
      {/each}
    </div>
  {:else if found}
    {@const [s, [file, here]] = found}
    <h2>{file.replace(/\.[^.]+$/, "")}</h2>
    <p class="say">{here ? `In ${s.name}. Enter opens it on this track; tab marks it for P.` : "Listed in the folder's manifest, but the file is gone."}</p>
    <dl class="kv"><dt>folder</dt><dd>{s.name}</dd><dt>file</dt><dd>{short(`${s.path}/${file}`)}</dd></dl>
  {:else if entry}
    <h2>{entry.label}</h2>
    <p class="say">{entry.path ? `From ${entry.folder}. A list holds the track, not the file, so its tags are edited there: Enter goes to it.` : `${entry.file} is not on disk. x takes it off the list.`}</p>
    <dl class="kv"><dt>folder</dt><dd>{entry.folder}</dd><dt>file</dt><dd>{entry.file}</dd></dl>
  {:else if m.view === "Pick"}
    <h2>{screens.pick.name}, after Enter</h2>
    <p class="say">Marks join in the order you made them. Unmarking a track the list holds takes it off the list, never off the disk.</p>
    <ol class="order">{#each order as t, i (i)}<li>{t}</li>{/each}{#each leaving as [folder, n] (`${folder}/${n}`)}<li class="out">{n.replace(/\.[^.]+$/, "")}</li>{/each}</ol>
  {:else if release}
    <h2>{release.title}</h2>
    <p class="say">{release.kind}{release.year ? `, ${release.year}` : ""}. {release.have === 0 ? "None of it is here." : complete(release) ? "All of it is here." : release.of === null ? `${release.have} of its tracks are here; Deezer has not said how many it has.` : `${release.have} of its ${release.of} tracks are here.`}{release.new ? " Out since you last looked." : ""}</p>
    <div class="acts"><button onclick={() => press("o")}><span class="kc">o</span>their folders</button></div>
  {:else if top}
    <h2>{top.title}</h2>
    <p class="say">{top.have ? "Already in the library." : "Not in the library."}{top.album ? ` From ${top.album}.` : ""}</p>
  {:else if related}
    <h2>{related.name}</h2>
    <p class="say">{related.fans.toLocaleString()} fans on Deezer.{related.tracks ? ` ${related.tracks} of their tracks are here.` : ""} Enter opens their page.</p>
  {/if}
</aside>
