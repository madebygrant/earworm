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
    let result = match ask_url(&mut cfg, &tx) {
        Ok(true) => pipeline(&cfg, &tx, &cancel, &mut tracks),
        /* Cancelling the first question cancels the tool: no run started, so
           there is nothing to stay open for and no failure worth reporting. */
        Ok(false) => {
            let _ = tx.send(Msg::Quit);
            return;
        }
        Err(err) => Err(err),
    };
    let _ = tx.send(Msg::Done(result.map_err(|e| e.to_string())));
    serve(&cfg, &tx, &mut tracks, cmds, &cancel);
}

/// The URL can arrive from a prompt rather than the command line, so running
/// earworm bare still gets somewhere. Always asks, even under --no-fix: there
/// is nothing to do without one. `Ok(false)` means the user cancelled.
fn ask_url(cfg: &mut Config, tx: &Sender<Msg>) -> Result<bool> {
    if !cfg.url.is_empty() {
        return Ok(true);
    }
    let asker = Asker {
        tx: tx.clone(),
        enabled: true,
    };
    let _ = tx.send(Msg::Stage("waiting for a playlist URL".into()));

    /* Asks again rather than failing: a mistyped link is the likely case, and
       handing back the text means fixing it instead of retyping it. */
    let mut header = "YouTube playlist URL".to_string();
    let mut typed = String::new();
    loop {
        let Some(answer) = asker.input_or_quit(&header, &typed) else {
            return Ok(false);
        };
        if let Some(url) = youtube_url(&answer) {
            cfg.url = url;
            return Ok(true);
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

fn pipeline(
    cfg: &Config,
    tx: &Sender<Msg>,
    cancel: &AtomicBool,
    tracks: &mut Vec<Track>,
) -> Result<String> {
    let _ = tx.send(Msg::Stage("reading playlist".into()));
    *tracks = ytdlp::scan(cfg)?;

    let playlist = folder_of(tracks)
        .and_then(|f| f.file_name().map(|n| n.to_string_lossy().to_string()))
        .unwrap_or_default();
    if !playlist.is_empty() {
        let _ = tx.send(Msg::Playlist(playlist.clone()));
    }
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

    save_manifest(tracks);
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
        if wanted.is_some_and(|only| !only.contains(&track.index)) {
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
            let _ = tx.send(Msg::Stage(format!("failed: {err}")));
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
        let _ = tx.send(Msg::Stage("nothing to retry".into()));
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
        written: folder.is_some_and(|f| COVER_NAMES.iter().any(|n| f.join(n).exists())),
    };
    tag_tracks(cfg, tx, cancel, tracks, &playlist, asker, &mut cover, Some(&failed));

    save_manifest(tracks);
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

    let track = &mut tracks[pos];
    track.artist = artist;
    track.title = title;
    track.name = label(&track.title, &track.artist);
    track.status = Status::Manual;
    track.source = source.into();
    track.note.clear();
    track.listed = true;
    let _ = tx.send(Msg::Update {
        index: track.index,
        status: Status::Manual,
        source: Some(source.into()),
        note: Some(String::new()),
        name: Some(track.name.clone()),
    });
    rename(cfg, tx, &mut tracks[pos]);
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
        save_manifest(tracks);
        write_playlist(cfg, tracks)?;
        let _ = tx.send(Msg::Unmark);
    }
    let _ = tx.send(Msg::Stage(match failed {
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
    save_manifest(tracks);
    write_playlist(cfg, tracks)?;
    let _ = tx.send(Msg::Stage(format!("saved track {index}")));
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
    let _ = tx.send(Msg::Stage("cancelled".into()));
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
    let _ = tx.send(Msg::Stage(format!("cover set from {label}")));
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

fn save_manifest(tracks: &[Track]) {
    let Some(folder) = folder_of(tracks) else {
        return;
    };
    let entries = tracks
        .iter()
        .filter(|t| t.listed)
        .filter_map(|t| t.path.clone().map(|p| (t.id.clone(), p)));
    let _ = manifest::write(&folder, entries);
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
    use crate::lookup::Artwork;

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
                "{} with rename={rename_on} was renamed",
                status.label()
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
