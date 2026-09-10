use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, Sender};

use anyhow::{Context, Result, bail};

use crate::app::{Asker, Cmd, Msg, Status, Track};
use crate::config::{self, Config};
use crate::lookup;
use crate::manifest;
use crate::tag::{self, CoverState};
use crate::ytdlp;

pub fn run(mut cfg: Config, tx: Sender<Msg>, cancel: Arc<AtomicBool>, cmds: Receiver<Cmd>) {
    let mut tracks = Vec::new();
    let result = match ask_start(&mut cfg, &tx) {
        Start::Playlist => pipeline(&cfg, &tx, &cancel, &mut tracks),
        Start::Resync(saved) => resync(&mut cfg, &tx, &cancel, &mut tracks, saved),
        /* Cancelling the first question cancels the tool: no run started, so
           there is nothing to stay open for. */
        Start::Cancelled => {
            let _ = tx.send(Msg::Quit);
            return;
        }
    };
    if finish(&mut cfg, &tx, &cancel, &mut tracks, result) {
        serve(&cfg, &tx, &mut tracks, cmds, &cancel);
    }
}

enum Start {
    Playlist,
    /// Carries the folders it found, so the answer is not looked up twice.
    Resync(Vec<(PathBuf, String)>),
    Cancelled,
}

/* Nothing to do without a URL, so ask for one. The menu only appears when
   there is a library to resync: offering it on a first run would be an option
   that can only fail. */
fn ask_start(cfg: &mut Config, tx: &Sender<Msg>) -> Start {
    if !cfg.url.is_empty() {
        return Start::Playlist;
    }
    let saved = saved_playlists(&cfg.dir);
    if cfg.resync {
        return Start::Resync(saved);
    }

    if !saved.is_empty() {
        let _ = tx.send(Msg::Stage("waiting for an answer".into()));
        let count = saved.len();
        let plural = if count == 1 { "playlist" } else { "playlists" };
        let options = vec![
            "sync a playlist by URL".to_string(),
            format!("resync the {count} {plural} already downloaded"),
            "quit".into(),
        ];
        let asker = Asker {
            tx: tx.clone(),
            enabled: true,
        };
        match asker.choose_or_quit("earworm", options) {
            Some(0) => {}
            Some(1) => return Start::Resync(saved),
            // Esc means quit here, the same as it does on the URL question.
            _ => return Start::Cancelled,
        }
    }

    let _ = tx.send(Msg::Stage("waiting for a playlist URL".into()));
    match prompt_url(tx) {
        Some(url) => {
            cfg.url = url;
            Start::Playlist
        }
        None => Start::Cancelled,
    }
}

/* Asks again rather than failing: a mistyped link is the likely case, and
   handing back the text means fixing it instead of retyping it. `None` is a
   cancel, which is why the first question can end the tool and a later one
   can go back to the menu. */
fn prompt_url(tx: &Sender<Msg>) -> Option<String> {
    let asker = Asker {
        tx: tx.clone(),
        enabled: true,
    };
    let mut header = "YouTube playlist URL".to_string();
    let mut typed = String::new();
    loop {
        let answer = asker.input_or_quit(&header, &typed)?;
        if let Some(url) = youtube_url(&answer) {
            return Some(url);
        }
        header = if answer.trim().is_empty() {
            "Paste a YouTube link".into()
        } else {
            "Not a YouTube link".into()
        };
        typed = answer;
    }
}

const HOSTS: [&str; 5] = [
    "youtube.com",
    "www.youtube.com",
    "m.youtube.com",
    "music.youtube.com",
    "youtu.be",
];

/// Normalises a pasted link, or `None` if it is not one earworm can download.
/// The host is matched after the scheme and any credentials, so a lookalike
/// path like `evil.com/youtube.com/watch?v=x` cannot pass as YouTube.
fn youtube_url(input: &str) -> Option<String> {
    let trimmed = input.trim();
    if trimmed.is_empty() {
        return None;
    }
    // A pasted link often arrives without one, and yt-dlp wants it.
    let full = if trimmed.contains("://") {
        trimmed.to_string()
    } else {
        format!("https://{trimmed}")
    };

    let rest = full.split_once("://")?.1;
    let (authority, path) = rest.split_once('/').unwrap_or((rest, ""));
    let host = authority
        .rsplit('@')
        .next()?
        .split(':')
        .next()?
        .to_ascii_lowercase();
    if !HOSTS.contains(&host.as_str()) {
        return None;
    }

    let (route, query) = path.split_once('?').unwrap_or((path, ""));
    let has = |key: &str| {
        query
            .split('&')
            .any(|p| p.starts_with(key) && p.len() > key.len())
    };
    /* Bare youtube.com is a valid URL and a useless one, so the link has to
       name something: a video, a playlist, or a channel to walk. */
    let ok = match route.trim_end_matches('/') {
        "" => false,
        "watch" => has("v="),
        "playlist" => has("list="),
        other => host == "youtu.be" || !other.is_empty(),
    };
    ok.then_some(full)
}

/* The run is over and the UI would otherwise just sit there, so say so and
   offer the two things worth doing next. Returns false when the answer was to
   quit, which is the one case that must not fall through to the command loop.
   Keeping it open is first, so Enter never starts work or ends the session. */
fn finish(
    cfg: &mut Config,
    tx: &Sender<Msg>,
    cancel: &AtomicBool,
    tracks: &mut Vec<Track>,
    result: Result<String>,
) -> bool {
    let asker = Asker {
        tx: tx.clone(),
        enabled: true,
    };
    /* Held as the finished `Result` for the whole loop and only ever cloned.
       Rebuilding it from the displayed text turned a failed run into a
       successful one, and `main` reports the exit status from this. */
    let mut result: Result<String, String> = result.map_err(|e| e.to_string());
    loop {
        let _ = tx.send(Msg::Done(result.clone()));
        if cancel.load(Ordering::SeqCst) {
            return false;
        }

        let (title, note) = match &result {
            Ok(summary) => ("finished", summary.clone()),
            Err(err) => ("failed", err.clone()),
        };
        let options = vec![
            "keep this open".to_string(),
            "sync another playlist".into(),
            "quit".into(),
        ];
        match asker.choose_noted(title, &note, options) {
            Some(1) => {
                // Cancelling the URL question goes back here, not out, and
                // leaves the last run's outcome exactly as it was.
                let Some(url) = prompt_url(tx) else {
                    continue;
                };
                let _ = tx.send(Msg::Restart);
                tracks.clear();
                cfg.url = url;
                result = pipeline(cfg, tx, cancel, tracks).map_err(|e| e.to_string());
            }
            Some(2) => {
                let _ = tx.send(Msg::Quit);
                return false;
            }
            // Esc keeps it open too: the safe answer to "what now?" is nothing.
            _ => return true,
        }
    }
}

/// Every playlist folder already under `--dir`, in name order, each with the
/// URL it recorded when it was written.
fn saved_playlists(dir: &Path) -> Vec<(PathBuf, String)> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut found: Vec<(PathBuf, String)> = entries
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.is_dir())
        .filter_map(|p| manifest::read_url(&p).map(|url| (p, url)))
        .collect();
    found.sort();
    found
}

/* One pipeline per folder rather than one list of everything: `Track::index`
   is a playlist position, and two playlists both have a track 1, so merging
   them would make every row, mark and command ambiguous. */
fn resync(
    cfg: &mut Config,
    tx: &Sender<Msg>,
    cancel: &AtomicBool,
    tracks: &mut Vec<Track>,
    found: Vec<(PathBuf, String)>,
) -> Result<String> {
    if found.is_empty() {
        bail!(
            "no playlist under {} has a saved URL yet; sync one by URL first",
            cfg.dir.display()
        );
    }

    let mut synced = 0;
    let mut listed = 0;
    for (n, (folder, url)) in found.iter().enumerate() {
        if cancel.load(Ordering::SeqCst) {
            break;
        }
        let name = folder.file_name().unwrap_or_default().to_string_lossy();
        let _ = tx.send(Msg::Stage(format!("{} of {}: {name}", n + 1, found.len())));
        // Each folder starts from an empty list, so its rows are its own.
        tracks.clear();
        let _ = tx.send(Msg::Restart);
        cfg.url = url.clone();

        match pipeline(cfg, tx, cancel, tracks) {
            Ok(summary) => {
                synced += 1;
                listed += tracks.iter().filter(|t| t.listed).count();
                let _ = tx.send(Msg::Log(format!("{name}: {summary}")));
            }
            // One dead playlist should not cost the rest of the library.
            Err(err) => {
                let _ = tx.send(Msg::Log(format!("{name}: {err}")));
            }
        }
    }

    Ok(format!(
        "{synced} of {} playlists, {listed} tracks",
        found.len()
    ))
}

fn pipeline(
    cfg: &Config,
    tx: &Sender<Msg>,
    cancel: &AtomicBool,
    tracks: &mut Vec<Track>,
) -> Result<String> {
    let _ = tx.send(Msg::Stage("reading playlist".into()));
    let listing = ytdlp::scan(cfg)?;
    *tracks = listing.tracks;

    let playlist = folder_of(tracks)
        .and_then(|f| f.file_name().map(|n| n.to_string_lossy().to_string()))
        .unwrap_or_default();
    if !playlist.is_empty() {
        let _ = tx.send(Msg::Playlist(playlist.clone()));
    }
    note_departures(cfg, tx, listing.complete, tracks);
    let _ = tx.send(Msg::Tracks(tracks.clone()));

    let have = tracks.iter().filter(|t| t.status == Status::Have).count();
    let _ = tx.send(Msg::Stage(format!(
        "downloading  ({have} of {} already on disk)",
        tracks.len()
    )));

    /* yt-dlp exits non-zero if any single entry failed, and aborting here
       would throw away the tags, cover and playlist for every track that did
       download. Carry the failure into the summary instead. */
    let mut warning = String::new();
    if let Err(err) = ytdlp::run(cfg, tracks, tx) {
        warning = err.to_string();
        let _ = tx.send(Msg::Log(format!("download: {warning}")));
    }

    let _ = tx.send(Msg::Stage("identifying".into()));
    let asker = Asker {
        tx: tx.clone(),
        enabled: cfg.fix,
    };
    let mut cover = CoverState { written: false };
    tag_tracks(cfg, tx, cancel, tracks, &playlist, &asker, &mut cover, None);

    save_manifest(cfg, tracks);
    let playlist_file = write_playlist(cfg, tracks)?;
    let mut summary = summarise(tracks, playlist_file.as_deref());
    if !warning.is_empty() {
        summary = format!("{summary}   [{warning}]");
    }
    Ok(summary)
}

/// The identify-and-write pass. `wanted` limits it to a set of track indices,
/// which is how a retry re-tags its own failures without disturbing tracks
/// that already came out right.
#[allow(clippy::too_many_arguments)]
fn tag_tracks(
    cfg: &Config,
    tx: &Sender<Msg>,
    cancel: &AtomicBool,
    tracks: &mut [Track],
    playlist: &str,
    asker: &Asker,
    cover: &mut CoverState,
    wanted: Option<&HashSet<usize>>,
) {
    for track in tracks.iter_mut() {
        if cancel.load(Ordering::SeqCst) {
            break;
        }
        if wanted.is_some_and(|only| !only.contains(&track.index))
            || track.status == Status::Gone
        {
            continue;
        }
        let Some(path) = track.path.clone() else {
            continue;
        };
        if !path.is_file() {
            track.status = Status::Failed;
            let _ = tx.send(Msg::Update {
                index: track.index,
                status: Status::Failed,
                source: None,
                note: Some("no file".into()),
                name: None,
            });
            continue;
        }

        /* Already tagged by an earlier run, so re-identifying costs a lookup
           and changes nothing. Still listed, so the .m3u8 stays complete. */
        if track.status == Status::Have {
            if let Ok(info) = tag::read(&path) {
                track.artist = info.artist;
                track.title = info.title;
                track.duration = info.duration;
                track.listed = true;
                track.name = label(&track.title, &track.artist);
                let _ = tx.send(Msg::Update {
                    index: track.index,
                    status: Status::Have,
                    source: Some("on disk".into()),
                    note: None,
                    name: Some(track.name.clone()),
                });
            }
            continue;
        }

        let _ = tx.send(Msg::Update {
            index: track.index,
            status: Status::Tagging,
            source: None,
            note: None,
            name: None,
        });

        // One bad track must not cost the playlist for every good one.
        match tag::identify(&path, cfg, asker, cover, track.index) {
            Ok(out) => {
                track.status = out.status;
                track.artist = out.artist;
                track.title = out.title;
                track.mbid = out.mbid;
                let info = tag::read(&path).ok();
                track.duration = info.as_ref().map(|i| i.duration).unwrap_or(0);
                track.listed = true;
                track.name = label(&track.title, &track.artist);

                /* Only when the lookup found no real album: a playlist name is
                   a worse answer than the release the track came from. */
                if cfg.album
                    && !playlist.is_empty()
                    && info.is_none_or(|i| i.album.is_empty())
                    && let Err(err) = tag::set_album(&path, playlist)
                {
                    let _ = tx.send(Msg::Log(format!("album: {err}")));
                }

                let _ = tx.send(Msg::Update {
                    index: track.index,
                    status: out.status,
                    source: Some(out.source),
                    note: Some(out.note),
                    name: Some(track.name.clone()),
                });
                rename(cfg, tx, track);
            }
            Err(err) => {
                track.status = Status::Failed;
                let _ = tx.send(Msg::Update {
                    index: track.index,
                    status: Status::Failed,
                    source: None,
                    note: Some(err.to_string()),
                    name: None,
                });
            }
        }
    }

}

/// Serves edits and cover changes after the run, so the tracks stay editable
/// without a second pass over the network.
fn serve(
    cfg: &Config,
    tx: &Sender<Msg>,
    tracks: &mut [Track],
    cmds: Receiver<Cmd>,
    cancel: &AtomicBool,
) {
    let asker = Asker {
        tx: tx.clone(),
        enabled: true,
    };
    while let Ok(cmd) = cmds.recv() {
        let done = match cmd {
            Cmd::Edit(index) => edit(cfg, tx, tracks, &asker, index),
            Cmd::Cover(index) => cover(tx, tracks, &asker, cancel, index),
            Cmd::Retry => retry(cfg, tx, tracks, cancel, &asker),
            Cmd::Swap(targets) => swap(cfg, tx, tracks, &targets),
            Cmd::Artist(targets) => artist(cfg, tx, tracks, &asker, &targets),
        };
        if let Err(err) = done {
            let _ = tx.send(Msg::Flash(format!("failed: {err}")));
            let _ = tx.send(Msg::Log(err.to_string()));
        }
        // Last, so the keys stay dead until everything above has landed.
        let _ = tx.send(Msg::Idle);
    }
}

/// Downloads and re-tags whatever came out `failed`. The archive names every
/// track that has a file, so yt-dlp fetches only the gaps and leaves the
/// finished tracks untouched rather than remuxing them.
fn retry(
    cfg: &Config,
    tx: &Sender<Msg>,
    tracks: &mut [Track],
    cancel: &AtomicBool,
    asker: &Asker,
) -> Result<()> {
    let failed: HashSet<usize> = tracks
        .iter()
        .filter(|t| t.status == Status::Failed)
        .map(|t| t.index)
        .collect();
    if failed.is_empty() {
        let _ = tx.send(Msg::Flash("nothing to retry".into()));
        return Ok(());
    }

    let _ = tx.send(Msg::Stage(format!("retrying {} tracks", failed.len())));
    for track in tracks.iter_mut().filter(|t| failed.contains(&t.index)) {
        track.status = Status::Pending;
        track.note.clear();
        track.source.clear();
        let _ = tx.send(Msg::Update {
            index: track.index,
            status: Status::Pending,
            source: Some(String::new()),
            note: Some(String::new()),
            name: None,
        });
    }

    let mut warning = String::new();
    if let Err(err) = ytdlp::run(cfg, tracks, tx) {
        warning = err.to_string();
        let _ = tx.send(Msg::Log(format!("retry: {warning}")));
    }

    let folder = folder_of(tracks);
    let playlist = folder
        .as_ref()
        .and_then(|f| f.file_name().map(|n| n.to_string_lossy().to_string()))
        .unwrap_or_default();
    /* The folder image is already there from the first pass, and rewriting it
       from a retried track would change the album art for every other one. */
    let mut cover = CoverState {
        written: folder.as_deref().is_some_and(has_cover),
    };
    tag_tracks(cfg, tx, cancel, tracks, &playlist, asker, &mut cover, Some(&failed));

    save_manifest(cfg, tracks);
    let playlist_file = write_playlist(cfg, tracks)?;

    /* Rebuilt rather than appended to: this replaces the run's own summary,
       which is what gets printed on exit, so it has to still say where the
       tracks went. */
    let still = tracks
        .iter()
        .filter(|t| t.status == Status::Failed)
        .count();
    let outcome = match still {
        0 => format!("retried {}, all resolved", failed.len()),
        n => format!("retried {}, {n} still failing", failed.len()),
    };
    let mut summary = format!(
        "{}   [{outcome}]",
        summarise(tracks, playlist_file.as_deref())
    );
    if !warning.is_empty() {
        summary = format!("{summary}   [{warning}]");
    }
    let _ = tx.send(Msg::Done(Ok(summary)));
    Ok(())
}

/// Writes one track's tags and brings everything that follows from them back
/// into line: the row, the filename, and the name the playlist will use.
fn write_track(
    cfg: &Config,
    tx: &Sender<Msg>,
    tracks: &mut [Track],
    pos: usize,
    artist: String,
    title: String,
    source: &str,
) -> Result<()> {
    let path = tracks[pos].path.clone().context("track has no file")?;
    if !path.is_file() {
        bail!("track {} was never downloaded", tracks[pos].index);
    }
    tag::set_fields(&path, &artist, &title)?;

    let departed = tracks[pos].status == Status::Gone;
    let track = &mut tracks[pos];
    track.artist = artist;
    track.title = title;
    track.name = label(&track.title, &track.artist);
    track.status = Status::Manual;
    track.source = source.into();
    track.note.clear();
    if !departed {
        track.listed = true;
    }
    let _ = tx.send(Msg::Update {
        index: track.index,
        status: Status::Manual,
        source: Some(source.into()),
        note: Some(String::new()),
        name: Some(track.name.clone()),
    });
    // Its index is synthetic, so renaming would stamp a playlist position
    // this track no longer has onto the file, next to the real one.
    if !departed {
        rename(cfg, tx, &mut tracks[pos]);
    }
    Ok(())
}

/// Runs one tag change over a set of tracks, reporting per-track failures
/// without abandoning the rest, and rewriting the playlist once at the end.
fn bulk(
    cfg: &Config,
    tx: &Sender<Msg>,
    tracks: &mut [Track],
    targets: &[usize],
    what: &str,
    mut fields: impl FnMut(&Track) -> (String, String),
) -> Result<()> {
    let mut changed = 0;
    let mut failed = 0;
    for target in targets {
        let Some(pos) = tracks.iter().position(|t| t.index == *target) else {
            continue;
        };
        let (artist, title) = fields(&tracks[pos]);
        if let Some(missing) = match (artist.is_empty(), title.is_empty()) {
            (true, true) => Some("artist and title"),
            (true, false) => Some("artist"),
            (false, true) => Some("title"),
            _ => None,
        } {
            failed += 1;
            let _ = tx.send(Msg::Log(format!("{what}: track {target} has no {missing}")));
            continue;
        }
        match write_track(cfg, tx, tracks, pos, artist, title, what) {
            Ok(()) => changed += 1,
            Err(err) => {
                failed += 1;
                let _ = tx.send(Msg::Log(format!("{what}: track {target}: {err}")));
            }
        }
    }

    /* Both only on a change: an action that wrote nothing has not earned the
       selection, and rebuilding a twenty-track one by hand is the annoyance
       the deferred unmark exists to avoid. */
    if changed > 0 {
        save_manifest(cfg, tracks);
        write_playlist(cfg, tracks)?;
        let _ = tx.send(Msg::Unmark);
    }
    let _ = tx.send(Msg::Flash(match failed {
        0 => format!("{what} {changed} tracks"),
        n => format!("{what} {changed} tracks, {n} could not be written"),
    }));
    Ok(())
}

fn swap(cfg: &Config, tx: &Sender<Msg>, tracks: &mut [Track], targets: &[usize]) -> Result<()> {
    bulk(cfg, tx, tracks, targets, "swapped", |track| {
        (track.title.clone(), track.artist.clone())
    })
}

fn artist(
    cfg: &Config,
    tx: &Sender<Msg>,
    tracks: &mut [Track],
    asker: &Asker,
    targets: &[usize],
) -> Result<()> {
    // Prefilled from the first target, since the usual case is correcting a
    // name the lookup nearly got right.
    let current = targets
        .first()
        .and_then(|i| tracks.iter().find(|t| t.index == *i))
        .map(|t| t.artist.clone())
        .unwrap_or_default();
    let header = format!("Artist for {} tracks", targets.len());
    let Some(new) = asker.input(&header, &current) else {
        return cancelled(tx);
    };
    if new.is_empty() {
        bail!("artist cannot be empty");
    }
    bulk(cfg, tx, tracks, targets, "set artist on", |track| {
        (new.clone(), track.title.clone())
    })
}

fn edit(
    cfg: &Config,
    tx: &Sender<Msg>,
    tracks: &mut [Track],
    asker: &Asker,
    index: usize,
) -> Result<()> {
    let Some(pos) = tracks.iter().position(|t| t.index == index) else {
        return Ok(());
    };
    let Some(artist) = asker.input(&format!("Artist for track {index}"), &tracks[pos].artist)
    else {
        return cancelled(tx);
    };
    let Some(title) = asker.input(&format!("Title for track {index}"), &tracks[pos].title) else {
        return cancelled(tx);
    };
    if artist.is_empty() || title.is_empty() {
        bail!("artist and title cannot be empty");
    }

    write_track(cfg, tx, tracks, pos, artist, title, "typed")?;
    save_manifest(cfg, tracks);
    write_playlist(cfg, tracks)?;
    let _ = tx.send(Msg::Flash(format!("saved track {index}")));
    Ok(())
}

/// Where a chosen row's image comes from. Keeping this beside the label means
/// the pick is resolved by identity rather than by re-reading the label text.
#[derive(Debug, PartialEq)]
enum Pick {
    Url(String),
    Keep,
    OwnFile,
}

fn cover_options(found: &[lookup::Artwork], current: Option<String>) -> Vec<(String, Pick)> {
    let mut rows: Vec<(String, Pick)> = found
        .iter()
        .map(|a| (a.label.clone(), Pick::Url(a.url.clone())))
        .collect();
    if let Some(desc) = current {
        rows.push((format!("keep the current cover  {desc}"), Pick::Keep));
    }
    rows.push(("use my own file...".into(), Pick::OwnFile));
    rows
}

/// Either extension can be the folder cover, so both are candidates for the
/// "keep" row and both are cleared before a new one is written.
const COVER_NAMES: [&str; 2] = ["cover.jpg", "cover.png"];

/* Files the manifest remembers whose video is no longer in the playlist. They
   are reported and otherwise left alone: nothing here deletes anything. */
fn note_departures(cfg: &Config, tx: &Sender<Msg>, complete: bool, tracks: &mut Vec<Track>) {
    let Some(folder) = folder_of(tracks) else {
        return;
    };
    let current: HashSet<&str> = tracks.iter().map(|t| t.id.as_str()).collect();
    let mut missing: Vec<(String, PathBuf)> = manifest::read(&folder)
        .into_iter()
        .filter(|(id, _)| !current.contains(id.as_str()))
        .collect();
    if missing.is_empty() {
        return;
    }

    /* Absent from the listing is not the same as gone from the playlist, and
       only a scan that asked for all of it and got all of it can tell the two
       apart. yt-dlp exits 0 for a filtered listing, so `--playlist-items 4`
       would otherwise report every other track as departed. Which arguments
       filter is not knowable, so any of them is enough to stay quiet. */
    let doubt = if !complete {
        Some("yt-dlp did not finish listing the playlist")
    } else if !cfg.extra.is_empty() {
        Some("the extra yt-dlp arguments may have filtered it")
    } else {
        None
    };
    if let Some(why) = doubt {
        let count = missing.len();
        let files = if count == 1 { "file" } else { "files" };
        let _ = tx.send(Msg::Log(format!(
            "{count} {files} on disk are not in this listing, and are not being \
             called gone because {why}"
        )));
        return;
    }

    missing.sort_by(|a, b| a.1.cmp(&b.1));
    // Numbered past the playlist so the rows sort last and no index collides.
    let next = tracks.iter().map(|t| t.index).max().unwrap_or(0);
    for (offset, (id, path)) in missing.into_iter().enumerate() {
        let name = path
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_default();
        let mut track = Track::new(next + offset + 1, id, name, path);
        track.status = Status::Gone;
        track.source = "not in playlist".into();
        tracks.push(track);
    }
}

/// Whether the folder already has its one image. Checking only `cover.jpg`
/// would miss a PNG, and `tag::apply` writes a JPEG unconditionally, so the
/// folder would end up with both and players would pick either.
fn has_cover(folder: &Path) -> bool {
    COVER_NAMES.iter().any(|name| folder.join(name).is_file())
}

fn current_cover(folder: &Path) -> Option<String> {
    for name in COVER_NAMES {
        let Ok(bytes) = std::fs::read(folder.join(name)) else {
            continue;
        };
        let (w, h) = lookup::jpeg_size(&bytes);
        return Some(if w > 0 {
            format!("({name}  {w}x{h})")
        } else {
            format!("({name})")
        });
    }
    None
}

fn cancelled(tx: &Sender<Msg>) -> Result<()> {
    let _ = tx.send(Msg::Flash("cancelled".into()));
    Ok(())
}

fn cover(
    tx: &Sender<Msg>,
    tracks: &mut [Track],
    asker: &Asker,
    cancel: &AtomicBool,
    index: usize,
) -> Result<()> {
    let Some(pos) = tracks.iter().position(|t| t.index == index) else {
        return Ok(());
    };
    let folder = folder_of(tracks).context("no folder to write a cover into")?;

    let _ = tx.send(Msg::Stage("looking for artwork".into()));
    let found = lookup::artwork(
        &tracks[pos].artist,
        &tracks[pos].title,
        tracks[pos].mbid.as_deref(),
    );
    let options = cover_options(&found, current_cover(&folder));
    let labels: Vec<String> = options.iter().map(|(label, _)| label.clone()).collect();

    // The candidates come from the selected track, so the header has to say so.
    let name = folder.file_name().unwrap_or_default().to_string_lossy();
    let header = format!("Cover for {name}   matches for {}", tracks[pos].name);
    let Some(pick) = asker.choose(&header, labels) else {
        return cancelled(tx);
    };
    let Some((label, source)) = options.get(pick) else {
        return Ok(());
    };

    let image = match source {
        Pick::Url(url) => {
            let _ = tx.send(Msg::Stage("downloading artwork".into()));
            lookup::fetch(url).context("could not download that cover")?
        }
        Pick::Keep => return cancelled(tx),
        Pick::OwnFile => {
            let Some(typed) = asker.input("Image file", "") else {
                return cancelled(tx);
            };
            if typed.is_empty() {
                bail!("no file given");
            }
            let path = config::expand(&typed);
            std::fs::read(&path).with_context(|| format!("cannot read {}", path.display()))?
        }
    };
    let ext = tag::image_extension(&image).context("not a JPEG or PNG")?;

    /* The folder gets exactly one cover: leaving the old extension behind
       means players keep showing the art that was just replaced. */
    for stale in COVER_NAMES {
        let _ = std::fs::remove_file(folder.join(stale));
    }
    std::fs::write(folder.join(format!("cover.{ext}")), &image)?;

    let targets: Vec<PathBuf> = tracks
        .iter()
        .filter(|t| t.listed)
        .filter_map(|t| t.path.clone())
        .collect();
    for (done, path) in targets.iter().enumerate() {
        if cancel.load(Ordering::SeqCst) {
            break;
        }
        let _ = tx.send(Msg::Stage(format!(
            "embedding cover  {}/{}",
            done + 1,
            targets.len()
        )));
        // One unwritable track should not stop the rest getting the artwork.
        if let Err(err) = tag::set_cover(path, &image) {
            let _ = tx.send(Msg::Log(format!("{}: {err}", path.display())));
        }
    }
    let _ = tx.send(Msg::Flash(format!("cover set from {label}")));
    Ok(())
}

/// Only what earworm is confident about: a `kept` or `weak` track still
/// carries whatever the video title happened to say, and baking that into a
/// filename makes a bad guess look like a decision.
fn rename(cfg: &Config, tx: &Sender<Msg>, track: &mut Track) {
    if !cfg.rename || !matches!(track.status, Status::Ok | Status::Manual) {
        return;
    }
    let Some(old) = track.path.clone() else {
        return;
    };
    let ext = old
        .extension()
        .map(|e| e.to_string_lossy().to_string())
        .unwrap_or_else(|| "opus".into());
    let new = old.with_file_name(filename(track.index, &track.artist, &track.title, &ext));
    if new == old {
        return;
    }
    // Never write over a track that is already there, even a stale one.
    if new.exists() {
        let _ = tx.send(Msg::Log(format!(
            "rename: {} already exists",
            new.file_name().unwrap_or_default().to_string_lossy()
        )));
        return;
    }
    match std::fs::rename(&old, &new) {
        Ok(()) => {
            track.path = Some(new.clone());
            let _ = tx.send(Msg::Path {
                index: track.index,
                path: new,
            });
        }
        Err(err) => {
            let _ = tx.send(Msg::Log(format!("rename: {err}")));
        }
    }
}

fn filename(index: usize, artist: &str, title: &str, ext: &str) -> String {
    let stem = format!("{index:02} - {} - {}", clean(artist), clean(title));
    /* The 255 limit a path component gets is bytes, not characters, and one
       accented character is two of them. Counted the same way, cut on a char
       boundary, and left room for the extension. */
    let budget = 250usize.saturating_sub(ext.len() + 1);
    let mut used = 0;
    let stem: String = stem
        .chars()
        .take_while(|c| {
            used += c.len_utf8();
            used <= budget
        })
        .collect();
    format!("{}.{ext}", stem.trim_end())
}

/// Separators and control characters would make a name the filesystem either
/// rejects or reads as a second directory.
fn clean(text: &str) -> String {
    let swapped: String = text
        .chars()
        .map(|c| match c {
            '/' | '\\' | ':' => '-',
            c if c.is_control() => ' ',
            c => c,
        })
        .collect();
    let collapsed = swapped.split_whitespace().collect::<Vec<_>>().join(" ");
    // A leading dot hides the file; a trailing one confuses some filesystems.
    let trimmed = collapsed.trim_matches('.').trim().to_string();
    if trimmed.is_empty() {
        "unknown".into()
    } else {
        trimmed
    }
}

fn save_manifest(cfg: &Config, tracks: &[Track]) {
    let Some(folder) = folder_of(tracks) else {
        return;
    };
    /* Keyed on the file, not on `listed`: dropping a departed track here
       forgets it, and a video that comes back to the playlist would download
       again beside the copy already on disk. */
    let entries = tracks
        .iter()
        .filter(|t| t.path.as_deref().is_some_and(Path::is_file))
        .filter_map(|t| t.path.clone().map(|p| (t.id.clone(), p)));
    let _ = manifest::write(&folder, &cfg.url, entries);
}

fn folder_of(tracks: &[Track]) -> Option<PathBuf> {
    tracks
        .iter()
        .filter_map(|t| t.path.as_deref())
        .next()
        .and_then(Path::parent)
        .map(Path::to_path_buf)
}

/// The line printed on exit, so it has to name the folder: it is the only
/// place the destination appears once the UI is gone.
fn summarise(tracks: &[Track], playlist: Option<&Path>) -> String {
    let listed = tracks.iter().filter(|t| t.listed).count();
    let mut summary = match folder_of(tracks) {
        Some(folder) => format!("{listed} tracks in {}", folder.display()),
        None => format!("{listed} tracks"),
    };
    if let Some(name) = playlist.and_then(Path::file_name) {
        summary = format!("{summary}, playlist {}", name.to_string_lossy());
    }
    // Said out loud, since the rows scroll off and nothing else mentions them.
    let departed = tracks.iter().filter(|t| t.status == Status::Gone).count();
    if departed > 0 {
        summary = format!("{summary}, {departed} no longer in the playlist");
    }
    summary
}

fn write_playlist(cfg: &Config, tracks: &[Track]) -> Result<Option<PathBuf>> {
    if !cfg.m3u8 {
        return Ok(None);
    }
    let listed: Vec<&Track> = tracks.iter().filter(|t| t.listed).collect();
    let Some(folder) = listed.first().and_then(|t| t.path.as_deref()).and_then(Path::parent) else {
        return Ok(None);
    };
    let name = folder.file_name().unwrap_or_default().to_string_lossy();
    let playlist = folder.join(format!("{name}.m3u8"));

    let mut body = String::from("#EXTM3U\n");
    for track in listed {
        body.push_str(&format!(
            "#EXTINF:{},{}\n{}\n",
            track.duration,
            label(&track.title, &track.artist),
            track.path.as_deref().unwrap_or(Path::new("")).display()
        ));
    }
    std::fs::write(&playlist, body)?;
    Ok(Some(playlist))
}

fn blank(value: &str) -> &str {
    if value.is_empty() { "?" } else { value }
}

/// One ordering for the track rows and the .m3u8 entries, so a song reads the
/// same in the UI as in the playlist.
fn label(title: &str, artist: &str) -> String {
    format!("{} - {}", blank(title), blank(artist))
}

#[cfg(test)]
mod tests {

    #[test]
    fn accepts_the_shapes_a_youtube_link_actually_comes_in() {
        for link in [
            "https://www.youtube.com/playlist?list=PLabc",
            "https://youtube.com/watch?v=dQw4w9WgXcQ",
            "https://music.youtube.com/playlist?list=OLAK5uy_x",
            "https://m.youtube.com/watch?v=abc&list=PLx",
            "https://youtu.be/dQw4w9WgXcQ",
            "https://www.youtube.com/shorts/abc123",
            "https://www.youtube.com/@someartist",
            "  https://youtu.be/abc  ",
        ] {
            assert!(youtube_url(link).is_some(), "rejected {link}");
        }
    }

    #[test]
    fn a_missing_scheme_is_added_rather_than_rejected() {
        assert_eq!(
            youtube_url("youtube.com/watch?v=abc").as_deref(),
            Some("https://youtube.com/watch?v=abc")
        );
        assert_eq!(
            youtube_url("http://youtu.be/abc").as_deref(),
            Some("http://youtu.be/abc"),
            "an explicit scheme is left alone"
        );
    }

    #[test]
    fn rejects_empty_and_non_youtube_input() {
        for link in [
            "",
            "   ",
            "hello",
            "https://vimeo.com/12345",
            "https://youtube.com",
            "https://youtube.com/",
            "https://www.youtube.com/watch",
            "https://www.youtube.com/watch?v=",
            "https://www.youtube.com/playlist?list=",
            "https://www.youtube.com/playlist?v=abc",
        ] {
            assert!(youtube_url(link).is_none(), "accepted {link:?}");
        }
    }

    /* A host that merely mentions youtube.com must not pass: the whole point
       of parsing the authority is that a path cannot impersonate one. */
    #[test]
    fn rejects_hosts_that_only_look_like_youtube() {
        for link in [
            "https://evil.com/youtube.com/watch?v=abc",
            "https://youtube.com.evil.com/watch?v=abc",
            "https://notyoutube.com/watch?v=abc",
            "https://youtube.com@evil.com/watch?v=abc",
            "https://evil.com/?next=https://youtube.com/watch?v=abc",
        ] {
            assert!(youtube_url(link).is_none(), "accepted {link}");
        }
    }
    use super::*;
    use std::sync::mpsc;
    use crate::app::Reply;
    use crate::lookup::Artwork;

    /* The exit status comes from the last `Msg::Done`, and rebuilding the
       result from the text on screen turned a failed run into a successful
       one, so `earworm; echo $?` reported 0 after a playlist that could not
       be read. Reachable by backing out of "sync another playlist". */
    #[test]
    fn a_failed_run_stays_failed_through_the_finish_menu() {
        let (tx, rx) = mpsc::channel();
        let cancel = AtomicBool::new(false);
        let mut cfg = config(true);
        let mut tracks = Vec::new();

        let outcomes = std::thread::scope(|scope| {
            let worker = scope.spawn(|| {
                finish(
                    &mut cfg,
                    &tx,
                    &cancel,
                    &mut tracks,
                    Err(anyhow::anyhow!("could not read the playlist")),
                )
            });

            // Answer the menu, then back out of the URL question it opens.
            let replies = [Reply::Choice(1), Reply::Cancel, Reply::Cancel];
            let mut sent = 0;
            let mut dones = Vec::new();
            while sent < replies.len() {
                match rx.recv().unwrap() {
                    Msg::Done(result) => dones.push(result),
                    Msg::Ask(_, reply) => {
                        reply.send(replies[sent].clone()).unwrap();
                        sent += 1;
                    }
                    _ => {}
                }
            }
            assert!(worker.join().unwrap(), "backing out should keep it open");
            dones
        });

        assert_eq!(outcomes.len(), 2, "the menu was not shown twice");
        for outcome in outcomes {
            assert!(
                outcome.is_err(),
                "a failed run was reported as {outcome:?}, so the exit status is 0"
            );
        }
    }

    /* Only folders that recorded where they came from, in a stable order, and
       no crash on a directory holding anything else. */
    #[test]
    fn saved_playlists_finds_the_folders_that_know_their_url() {
        let root = scratch("library");
        for (name, url) in [("Beta", Some("u-beta")), ("Alpha", Some("u-alpha")), ("NoUrl", None)] {
            let folder = root.join(name);
            std::fs::create_dir_all(&folder).unwrap();
            match url {
                Some(u) => manifest::write(&folder, u, std::iter::empty()).unwrap(),
                // Written before the header existed, so it cannot be resynced.
                None => std::fs::write(manifest::path(&folder), "vid\tfile.opus\n").unwrap(),
            }
        }
        std::fs::write(root.join("loose.opus"), "not a folder").unwrap();

        let found = saved_playlists(&root);
        let names: Vec<String> = found
            .iter()
            .map(|(p, _)| p.file_name().unwrap().to_string_lossy().to_string())
            .collect();
        assert_eq!(names, ["Alpha", "Beta"], "wrong folders or wrong order");
        assert_eq!(found[0].1, "u-alpha");
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn saved_playlists_is_empty_for_a_directory_that_is_not_there() {
        assert!(saved_playlists(Path::new("/nonexistent-earworm-library")).is_empty());
    }

    #[test]
    fn the_summary_counts_departures_separately_from_the_playlist() {
        let dir = scratch("summary");
        let mut tracks = playlist_of(&dir, &[("a", "01 - A.opus")]);
        tracks[0].listed = true;
        let mut gone = Track::new(9, "b".into(), "09 - B.opus".into(), dir.join("09 - B.opus"));
        gone.status = Status::Gone;
        tracks.push(gone);

        let line = summarise(&tracks, Some(&dir.join("Mix.m3u8")));
        assert!(line.contains("1 tracks in"), "{line}");
        assert!(line.contains("playlist Mix.m3u8"), "{line}");
        assert!(line.contains("1 no longer in the playlist"), "{line}");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    fn channel() -> (Sender<Msg>, mpsc::Receiver<Msg>) {
        mpsc::channel()
    }

    fn playlist_of(dir: &Path, ids: &[(&str, &str)]) -> Vec<Track> {
        ids.iter()
            .enumerate()
            .map(|(i, (id, name))| {
                std::fs::write(dir.join(name), "audio").unwrap();
                Track::new(i + 1, (*id).into(), (*name).into(), dir.join(name))
            })
            .collect()
    }

    /* The whole point of the reporting step: a file the manifest remembers
       whose video has left the playlist gets a row instead of vanishing
       silently, and nothing touches it. */
    #[test]
    fn a_departed_video_becomes_a_row_of_its_own() {
        let dir = scratch("departed");
        let mut tracks = playlist_of(&dir, &[("keep1", "01 - A.opus"), ("keep2", "02 - B.opus")]);
        std::fs::write(dir.join("09 - Old.opus"), "audio").unwrap();
        manifest::write(
            &dir,
            "u",
            [
                ("keep1".to_string(), dir.join("01 - A.opus")),
                ("dropped".to_string(), dir.join("09 - Old.opus")),
            ]
            .into_iter(),
        )
        .unwrap();

        note_departures(&config(true), &channel().0, true, &mut tracks);

        assert_eq!(tracks.len(), 3, "the departed file got no row");
        let gone = tracks.last().unwrap();
        assert_eq!(gone.status, Status::Gone);
        assert_eq!(gone.id, "dropped");
        assert_eq!(gone.name, "09 - Old.opus");
        assert!(!gone.listed, "a departed track must stay out of the playlist");
        assert!(gone.index > 2, "its index collides with a real track");
        assert!(dir.join("09 - Old.opus").is_file(), "the file was touched");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /* The premise, not the mechanics. `--playlist-items 4` makes every other
       track absent from the listing while it sits in the playlist untouched,
       and yt-dlp exits 0, so nothing but the arguments themselves says the
       listing was narrowed. Reported as gone, a later prune deletes music. */
    #[test]
    fn a_filtered_listing_is_never_read_as_a_departure() {
        for (complete, extra) in [
            (true, vec!["--playlist-items".to_string(), "4".to_string()]),
            (false, Vec::new()),
            (false, vec!["--playlist-end".to_string(), "2".to_string()]),
        ] {
            let dir = scratch("filtered");
            let mut tracks = playlist_of(&dir, &[("keep1", "01 - A.opus")]);
            std::fs::write(dir.join("09 - Old.opus"), "audio").unwrap();
            manifest::write(
                &dir,
                "u",
                [("dropped".to_string(), dir.join("09 - Old.opus"))].into_iter(),
            )
            .unwrap();

            let mut cfg = config(true);
            cfg.extra = extra.clone();
            let (tx, rx) = channel();
            note_departures(&cfg, &tx, complete, &mut tracks);

            assert_eq!(
                tracks.len(),
                1,
                "reported a departure with complete={complete} extra={extra:?}"
            );
            let logged: Vec<String> = rx
                .try_iter()
                .filter_map(|m| match m {
                    Msg::Log(line) => Some(line),
                    _ => None,
                })
                .collect();
            assert_eq!(logged.len(), 1, "said nothing about the files it skipped");
            assert!(logged[0].contains("not in this listing"), "{:?}", logged[0]);
            std::fs::remove_dir_all(&dir).unwrap();
        }
    }

    /// A real opus file, since `tag::set_fields` has to succeed for the code
    /// under test to be reached at all. `None` if ffmpeg is not installed.
    fn tiny_opus(at: &Path) -> Option<()> {
        let made = std::process::Command::new("ffmpeg")
            .args(["-v", "error", "-f", "lavfi", "-i", "anullsrc=r=48000:cl=mono"])
            .args(["-t", "0.2", "-c:a", "libopus", "-y"])
            .arg(at)
            .status()
            .ok()?;
        made.success().then_some(())
    }

    /* A departed track has no playlist position, so renumbering it stamps one
       it does not have onto the file, possibly beside the real holder. The
       non-departed half of this test is what proves the guard is load-bearing
       rather than the rename simply never happening. */
    #[test]
    fn only_a_track_still_in_the_playlist_gets_renumbered() {
        let dir = scratch("editgone");
        let listed = dir.join("01 - A.opus");
        let orphan = dir.join("09 - Old.opus");
        if tiny_opus(&listed).is_none() || tiny_opus(&orphan).is_none() {
            eprintln!("no ffmpeg, skipping");
            let _ = std::fs::remove_dir_all(&dir);
            return;
        }

        let mut tracks = vec![
            Track::new(1, "keep1".into(), "01 - A.opus".into(), listed.clone()),
            Track::new(2, "dropped".into(), "09 - Old.opus".into(), orphan.clone()),
        ];
        tracks[1].status = Status::Gone;
        let (tx, _rx) = channel();
        let cfg = config(true);

        write_track(&cfg, &tx, &mut tracks, 1, "New".into(), "Title".into(), "typed").unwrap();
        assert!(orphan.is_file(), "the departed file was moved");
        assert!(
            !dir.join("02 - New - Title.opus").exists(),
            "a departed track was given a playlist number"
        );

        write_track(&cfg, &tx, &mut tracks, 0, "New".into(), "Title".into(), "typed").unwrap();
        assert!(
            dir.join("01 - New - Title.opus").is_file(),
            "a listed track stopped being renamed, so the guard is too wide"
        );
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_manifest_entry_whose_file_is_gone_is_not_reported() {
        let dir = scratch("vanished");
        let mut tracks = playlist_of(&dir, &[("keep1", "01 - A.opus")]);
        manifest::write(
            &dir,
            "u",
            [("dropped".to_string(), dir.join("nothing here.opus"))].into_iter(),
        )
        .unwrap();

        note_departures(&config(true), &channel().0, true, &mut tracks);
        assert_eq!(tracks.len(), 1, "a deleted file is not a departure");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /* Forgetting a departed track would re-download it beside the copy
       already on disk if the video ever returned to the playlist. */
    #[test]
    fn the_manifest_keeps_remembering_a_departed_track() {
        let dir = scratch("remember");
        let mut tracks = playlist_of(&dir, &[("keep1", "01 - A.opus")]);
        std::fs::write(dir.join("09 - Old.opus"), "audio").unwrap();
        manifest::write(
            &dir,
            "u",
            [("dropped".to_string(), dir.join("09 - Old.opus"))].into_iter(),
        )
        .unwrap();

        note_departures(&config(true), &channel().0, true, &mut tracks);
        tracks[0].listed = true;
        save_manifest(&config(true), &tracks);

        let back = manifest::read(&dir);
        assert!(back.contains_key("dropped"), "the departed id was forgotten");
        assert!(back.contains_key("keep1"));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /* A retry must not add a second folder image: the writer in `tag::apply`
       always writes cover.jpg, so a check that only looked for that name left
       a PNG cover with a JPEG beside it. Driven off the constant, so adding a
       name without teaching this check about it fails here. */
    #[test]
    fn every_cover_name_counts_as_the_folder_already_having_one() {
        let dir = scratch("cover");
        assert!(!has_cover(&dir), "an empty folder claimed a cover");

        for name in COVER_NAMES {
            std::fs::write(dir.join(name), "image").unwrap();
            assert!(has_cover(&dir), "{name} was not recognised");
            std::fs::remove_file(dir.join(name)).unwrap();
            assert!(!has_cover(&dir), "{name} counted after deletion");
        }

        // A directory of that name is not an image either.
        std::fs::create_dir(dir.join(COVER_NAMES[0])).unwrap();
        assert!(!has_cover(&dir));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    fn scratch(tag: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("earworm-rename-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn track_at(dir: &Path, name: &str, status: Status) -> Track {
        std::fs::write(dir.join(name), "audio").unwrap();
        let mut track = Track::new(4, "vid".into(), name.into(), dir.join(name));
        track.status = status;
        track.artist = "Ed Sheeran".into();
        track.title = "Sapphire".into();
        track
    }

    fn config(rename: bool) -> Config {
        Config {
            url: String::new(),
            dir: PathBuf::new(),
            parse: true,
            m3u8: true,
            cover: true,
            lookup: true,
            fix: true,
            resync: false,
            rename,
            album: true,
            extra: Vec::new(),
            acoustid_key: None,
        }
    }

    #[test]
    fn renames_a_confident_track_and_reports_the_new_path() {
        let dir = scratch("ok");
        let (tx, rx) = mpsc::channel();
        let mut track = track_at(&dir, "04 - Ed Sheeran - Sapphire (Official).opus", Status::Ok);
        rename(&config(true), &tx, &mut track);

        let want = dir.join("04 - Ed Sheeran - Sapphire.opus");
        assert!(want.is_file(), "renamed file missing");
        assert_eq!(track.path.as_deref(), Some(want.as_path()));
        assert!(
            rx.try_iter()
                .any(|m| matches!(m, Msg::Path { path, .. } if path == want)),
            "the UI was never told the path changed"
        );
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /* A kept or weak track still carries whatever the video title said, so
       renaming would dress a guess up as a decision. */
    #[test]
    fn leaves_unconfident_and_disabled_tracks_alone() {
        for (status, rename_on) in [(Status::Kept, true), (Status::Weak, true), (Status::Ok, false)]
        {
            let dir = scratch("skip");
            let (tx, _rx) = mpsc::channel();
            let name = "04 - something else.opus";
            let mut track = track_at(&dir, name, status);
            rename(&config(rename_on), &tx, &mut track);
            assert!(
                dir.join(name).is_file(),
                "{status:?} with rename={rename_on} was renamed"
            );
            std::fs::remove_dir_all(&dir).unwrap();
        }
    }

    #[test]
    fn never_writes_over_a_file_that_is_already_there() {
        let dir = scratch("clash");
        let (tx, _rx) = mpsc::channel();
        let mut track = track_at(&dir, "04 - old name.opus", Status::Ok);
        std::fs::write(dir.join("04 - Ed Sheeran - Sapphire.opus"), "someone else").unwrap();

        rename(&config(true), &tx, &mut track);
        assert!(dir.join("04 - old name.opus").is_file(), "source vanished");
        assert_eq!(
            std::fs::read_to_string(dir.join("04 - Ed Sheeran - Sapphire.opus")).unwrap(),
            "someone else",
            "the existing file was overwritten"
        );
        std::fs::remove_dir_all(&dir).unwrap();
    }


    #[test]
    fn builds_a_name_from_the_corrected_tags() {
        assert_eq!(
            filename(7, "Kurt Vile", "Pretty Pimpin", "opus"),
            "07 - Kurt Vile - Pretty Pimpin.opus"
        );
        assert_eq!(
            filename(112, "A", "B", "opus"),
            "112 - A - B.opus",
            "an index past 99 is not truncated"
        );
    }

    /* A separator in a tag would either be rejected by the filesystem or read
       as a directory, and a leading dot would hide the track. */
    #[test]
    fn strips_what_a_filename_cannot_hold() {
        assert_eq!(clean("AC/DC"), "AC-DC");
        assert_eq!(clean("Sunday\\Monday"), "Sunday-Monday");
        assert_eq!(clean("Bad: The Sequel"), "Bad- The Sequel");
        assert_eq!(clean(".hidden."), "hidden");
        assert_eq!(clean("a\nb"), "a b");
        assert_eq!(clean("  spaced   out  "), "spaced out");
        assert_eq!(clean("..."), "unknown", "never an empty component");
    }

    #[test]
    fn keeps_a_long_name_inside_the_filesystem_limit() {
        let long = "é".repeat(400);
        let name = filename(1, &long, &long, "opus");
        assert!(name.len() < 255, "{} bytes", name.len());
        assert!(name.ends_with(".opus"));
    }

    fn art(label: &str, url: &str) -> Artwork {
        Artwork {
            label: label.into(),
            url: url.into(),
        }
    }

    #[test]
    fn every_row_maps_back_to_its_own_source() {
        let found = [art("Deezer  A", "http://a"), art("Deezer  B", "http://b")];
        let rows = cover_options(&found, Some("(cover.jpg  500x500)".into()));
        assert_eq!(rows.len(), 4);
        assert_eq!(rows[0].1, Pick::Url("http://a".into()));
        assert_eq!(rows[1].1, Pick::Url("http://b".into()));
        assert_eq!(rows[2].1, Pick::Keep);
        assert_eq!(rows[3].1, Pick::OwnFile);
    }

    /// The empty case is the one that breaks silently: index 0 has to be the
    /// own-file row, not a missing artwork entry.
    #[test]
    fn own_file_is_row_zero_when_there_is_nothing_else() {
        let rows = cover_options(&[], None);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].1, Pick::OwnFile);
    }

    #[test]
    fn keep_row_is_absent_without_a_current_cover() {
        let rows = cover_options(&[art("Deezer  A", "http://a")], None);
        assert!(!rows.iter().any(|(_, pick)| *pick == Pick::Keep));
        assert_eq!(rows[1].1, Pick::OwnFile);
    }
}
