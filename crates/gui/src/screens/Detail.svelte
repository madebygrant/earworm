<script lang="ts">
  import { convertFileSrc } from "@tauri-apps/api/core";
  import type { Shelf, Status, Track } from "../lib/protocol";
  import { answerPick, app } from "../lib/store.svelte";
  import { label, names, short, tone, ui, word } from "../lib/ui.svelte";
  import { clock } from "../lib/run";
  import { ago, shelfArt } from "../lib/wall";
  import { artistSelected, complete, customRow, foundAt, relatedRows, releaseSelected, screens, topRows } from "../lib/screens.svelte";
  import Art from "./Art.svelte";
  import Sheet, { type Act, type Field } from "./Sheet.svelte";

  interface Props {
    /** Runs the key a button stands for, so a click and the key are one action. */
    press: (key: string) => void;
  }
  let { press: key }: Props = $props();
  // A button is its key, but a focused filter box takes every key; the click ends the typing first, as
  // Enter in the box would, and keeps what was typed.
  function press(k: string) {
    ui.typing = false;
    screens.artistTyping = false;
    key(k);
  }

  const m = $derived(app.model);
  // The pick gate's "take none" is no key: Esc clears a filter first, so that button answers the gate itself.
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
  function acts(t: Track): Act[] {
    if (["Kept", "Weak", "NoMatch"].includes(t.status)) {
      return [{ key: "T", icon: "search", word: "Search again" }, { key: "e", icon: "rename", word: "edit" }, { key: "c", icon: "cover", word: "cover" }, { key: "s", icon: "swap", word: "swap" }];
    }
    if (t.status === "Failed") return [{ key: "r", icon: "retry", word: "Retry failed" }, { key: "l", icon: "log", word: "Log" }];
    if (t.status === "Gone") return t.departure_proven ? [{ key: "D", icon: "remove", word: "Delete departed" }] : [];
    // `u` only while the worker holds something to put back: a key that can only fail is worse than none.
    if (["Ok", "Manual", "Have"].includes(t.status)) {
      return [{ key: "e", icon: "rename", word: "Edit tags" }, { key: "c", icon: "cover", word: "Cover" }, ...(m.undoable ? [{ key: "u", icon: "undo", word: "Undo" } as Act] : [])];
    }
    return [];
  }

  function trackFields(t: Track): Field[] {
    const all: Field[] = [
      { icon: "artists", name: "artist", value: t.artist },
      { icon: "album", name: "album", value: t.album },
      { icon: "clock", name: "year", value: t.year ? String(t.year) : "" },
      { icon: "video", name: "video", value: t.was },
      { icon: "swap", name: "twin", value: t.twin ?? "" },
      { icon: "file", name: "file", value: t.path ? short(t.path) : "" },
    ];
    // Only what the tags hold so far: before the tag pass there is nothing to show.
    return all.filter((f) => f.value);
  }

  // A custom playlist has nothing upstream to sync and no folder of its own to show.
  function shelfActs(s: Shelf): Act[] {
    const play: Act[] = m.playback.player === "Running" ? [{ key: "p", icon: "play", word: "play" }] : [];
    const folder: Act[] = s.kind === "Custom" ? [] : [{ key: "S", icon: "run", word: "sync" }];
    const show: Act[] = s.kind === "Custom" ? [] : [{ key: "O", icon: "folder", word: "show" }];
    return [
      { key: "Enter", icon: "open", word: "Open" }, ...folder, ...play, ...show, { key: "e", icon: "rename", word: "rename" },
      { key: "E", icon: "m3u8", word: "export" }, { key: "D", icon: "remove", word: "remove" },
    ];
  }

  function shelfWhy(s: Shelf): string {
    if (s.kind === "Album") return "An album. Its tracks share one sleeve.";
    if (s.kind === "Custom") return "A custom playlist, built here from tracks in other folders.";
    return s.url ? "A playlist. Each track keeps its own sleeve." : "A folder earworm did not download. S asks for its link.";
  }

  function sleeveless(t: Track): string {
    if (t.status === "Failed") return "no sleeve, the download failed";
    return word(t.status)?.look ? "no sleeve until it's confirmed" : "no sleeve yet";
  }

  const missing = $derived(shelf ? shelf.files.filter(([, here]) => !here).map(([name]) => name) : []);
  const found = $derived(m.view === "Found" ? foundAt() : null);
  const entry = $derived(m.view === "Playlist" ? customRow() : undefined);
  const release = $derived(m.view === "Artist" && screens.section === "Releases" ? releaseSelected() : undefined);
  const top = $derived(m.view === "Artist" && screens.section === "Top" ? topRows()[screens.topAt] : undefined);
  const related = $derived(m.view === "Artist" && screens.section === "Related" ? relatedRows()[screens.relatedAt] : undefined);
  const artist = $derived(m.view === "Artists" ? artistSelected() : undefined);
  const news = $derived(artist ? (screens.artistNew[m.artists.rows.indexOf(artist)] ?? []) : []);
  const order = $derived(m.view === "Pick" ? screens.pick.marks.map(([, file]) => file.replace(/\.[^.]+$/, "").replace(/^\d+\s*-\s*/, "")) : []);
  // Core's answer to what Enter takes off, never worked out here.
  const leaving = $derived(m.view === "Pick" ? screens.pick.removing : []);
</script>

{#if track && m.picking !== null}
  {@const [title, artist] = names(track)}
  {@const on = m.marked.includes(track.index)}
  {@const n = m.marked.length}
  <Sheet
    press={(k) => (k === "none" ? void answerPick([]) : press(k))}
    title={`${title}${artist ? ` - ${artist}` : ""}`}
    why={track.status === "Have" ? "Already in the folder, so there is nothing to fetch."
      : track.twin ? `${track.twin}. Leave it selected and earworm copies that file across instead of downloading it again.`
      : track.duration > 1800 ? "Over half an hour long. Probably not one track."
      : on ? "Selected: it downloads." : "Not selected: it is left out."}
    fields={[{ icon: "video", name: "video", value: track.was }, ...(track.duration ? [{ icon: "clock", name: "length", value: clock(track.duration) } as Field] : [])]}
    acts={[{ key: "Enter", icon: "open", word: `Download ${n} ${n === 1 ? "track" : "tracks"}` }, { key: " ", icon: "mark", word: on ? "unmark" : "mark" }, { key: "none", icon: "stop", word: "take none" }]}
  >
    {#snippet extra()}<p class="why">Esc takes none and still tags what's already in the folder.</p>{/snippet}
  </Sheet>
{:else if track}
  {@const [title, artist] = names(track)}
  {@const thumb = track.path ? m.thumbs[track.path] : null}
  {@const why = SAY[track.status]?.(track)}
  <Sheet {press} title={`${title}${artist ? ` - ${artist}` : ""}`} why={why ? `${label(track.status)} · ${why}` : label(track.status)} tone={tone(track.status)}
    art={!!thumb} fields={trackFields(track)} acts={m.done !== null ? acts(track) : []}>
    {#snippet picture()}
      {#if thumb}<img src={convertFileSrc(thumb)} alt="" />{:else}{sleeveless(track)}{/if}
    {/snippet}
  </Sheet>
{:else if shelf}
  <Sheet
    {press}
    title={shelf.name}
    why={shelfWhy(shelf)}
    art={typeof shelfArt(shelf) === "object"}
    fields={[
      { icon: "tracks", name: "tracks", value: `${shelf.tracks}${shelf.missing ? `, ${shelf.missing} missing` : ""}`, tone: shelf.missing ? "var(--warn)" : undefined },
      ...(shelf.kind === "Custom" ? [] : [{ icon: "clock", name: "synced", value: ago(shelf.synced) } as Field]),
      shelf.kind === "Custom" ? { icon: "file", name: "file", value: short(shelf.path) } : { icon: "folder", name: "folder", value: short(shelf.path) },
    ]}
    acts={shelfActs(shelf)}
  >
    {#snippet picture()}
      <Art {shelf} />
    {/snippet}
    {#snippet extra()}
      {#if missing.length}<ul class="files">{#each missing as name (name)}<li class="warn">! {name}</li>{/each}</ul>{/if}
    {/snippet}
  </Sheet>
{:else if found}
  {@const [s, [file, here]] = found}
  <Sheet
    {press}
    title={file.replace(/\.[^.]+$/, "")}
    why={here ? `In ${s.name}. Enter opens it on this track; tab marks it for P.` : "Listed in the folder's manifest, but the file is gone."}
    fields={[{ icon: "folder", name: "folder", value: s.name }, { icon: "file", name: "file", value: short(`${s.path}/${file}`) }]}
  />
{:else if entry}
  <Sheet
    {press}
    title={entry.label}
    why={entry.path ? `From ${entry.folder}. A list holds the track, not the file, so its tags are edited there: Enter goes to it.` : `${entry.file} is not on disk. x takes it off the list.`}
    fields={[{ icon: "folder", name: "folder", value: entry.folder }, { icon: "file", name: "file", value: entry.file }]}
  />
{:else if m.view === "Pick"}
  <Sheet {press} title={`${screens.pick.name}, after Enter`} why="Marks join in the order you made them. Unmarking a track the list holds takes it off the list, never off the disk.">
    {#snippet extra()}
      <ol class="order">{#each order as t, i (i)}<li>{t}</li>{/each}{#each leaving as [folder, n] (`${folder}/${n}`)}<li class="out">{n.replace(/\.[^.]+$/, "")}</li>{/each}</ol>
    {/snippet}
  </Sheet>
{:else if artist}
  <Sheet
    {press}
    title={artist.name}
    why={artist.starred ? "Starred. Their new releases are looked for at every start." : "Enter opens their releases, top tracks and related artists."}
    fields={[
      { icon: "tracks", name: "tracks", value: String(artist.tracks) },
      { icon: "album", name: "albums", value: String(artist.albums) },
      { icon: "folder", name: "folders", value: artist.folders.map((f) => f.split("/").pop()).join(", ") || "none" },
    ]}
    acts={[{ key: "Enter", icon: "open", word: "Their page" }, { key: "o", icon: "folder", word: "folders" }, { key: "*", icon: "mark", word: artist.starred ? "unstar" : "star" }]}
  >
    {#snippet extra()}
      {#if news.length}<ul class="files">{#each news as t (t)}<li class="live">new: {t}</li>{/each}</ul>{/if}
    {/snippet}
  </Sheet>
{:else if release}
  <Sheet
    {press}
    title={release.title}
    why={`${release.kind}${release.year ? `, ${release.year}` : ""}. ${release.have === 0 ? "None of it is here." : complete(release) ? "All of it is here." : release.of === null ? `${release.have} of its tracks are here; Deezer has not said how many it has.` : `${release.have} of its ${release.of} tracks are here.`}${release.new ? " Out since you last looked." : ""}`}
    acts={[{ key: "o", icon: "folder", word: "Their folders" }]}
  />
{:else if top}
  <Sheet {press} title={top.title} why={`${top.have ? "Already in the library." : "Not in the library."}${top.album ? ` From ${top.album}.` : ""}`} />
{:else if related}
  <Sheet {press} title={related.name} why={`${related.fans.toLocaleString()} fans on Deezer.${related.tracks ? ` ${related.tracks} of their tracks are here.` : ""} Enter opens their page.`} />
{/if}
