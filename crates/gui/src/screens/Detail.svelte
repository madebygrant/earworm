<script lang="ts">
  import { convertFileSrc } from "@tauri-apps/api/core";
  import type { Shelf, Status, Track } from "../lib/protocol";
  import { app } from "../lib/store.svelte";
  import { label, names, short, tone } from "../lib/ui.svelte";

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
  {/if}
</aside>
