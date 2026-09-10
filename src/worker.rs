use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, Sender};

use anyhow::{Context, Result, bail};

use crate::app::{Asker, Cmd, Msg, Status, Track};
use crate::config::{self, Config};
use crate::lookup;
use crate::tag::{self, CoverState};
use crate::ytdlp;

pub fn run(mut cfg: Config, tx: Sender<Msg>, cancel: Arc<AtomicBool>, cmds: Receiver<Cmd>) {
    let mut tracks = Vec::new();
    let result =
        ask_url(&mut cfg, &tx).and_then(|()| pipeline(&cfg, &tx, &cancel, &mut tracks));
    let _ = tx.send(Msg::Done(result.map_err(|e| e.to_string())));
    serve(&cfg, &tx, &mut tracks, cmds, &cancel);
}

/// The URL can arrive from a prompt rather than the command line, so running
/// earworm bare still gets somewhere. Always asks, even under --no-fix: there
/// is nothing to do without one.
fn ask_url(cfg: &mut Config, tx: &Sender<Msg>) -> Result<()> {
    if !cfg.url.is_empty() {
        return Ok(());
    }
    let asker = Asker {
        tx: tx.clone(),
        enabled: true,
    };
    let _ = tx.send(Msg::Stage("waiting for a playlist URL".into()));
    let url = asker
        .input("YouTube playlist URL", "")
        .context("cancelled")?;
    if url.is_empty() {
        bail!("no playlist URL given");
    }
    cfg.url = url;
    Ok(())
}

fn pipeline(
    cfg: &Config,
    tx: &Sender<Msg>,
    cancel: &AtomicBool,
    tracks: &mut Vec<Track>,
) -> Result<String> {
    let _ = tx.send(Msg::Stage("reading playlist".into()));
    *tracks = ytdlp::scan(cfg)?;

    if let Some(name) = folder_of(tracks).and_then(|f| {
        f.file_name()
            .map(|n| n.to_string_lossy().to_string())
    }) {
        let _ = tx.send(Msg::Playlist(name));
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

    for track in tracks.iter_mut() {
        if cancel.load(Ordering::SeqCst) {
            break;
        }
        let Some(path) = track.path.clone() else {
            continue;
        };
        if !path.exists() {
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
        match tag::identify(&path, cfg, &asker, &mut cover, track.index) {
            Ok(out) => {
                track.status = out.status;
                track.artist = out.artist;
                track.title = out.title;
                track.mbid = out.mbid;
                track.duration = tag::read(&path).map(|i| i.duration).unwrap_or(0);
                track.listed = true;
                track.name = label(&track.title, &track.artist);
                let _ = tx.send(Msg::Update {
                    index: track.index,
                    status: out.status,
                    source: Some(out.source),
                    note: Some(out.note),
                    name: Some(track.name.clone()),
                });
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

    let listed = tracks.iter().filter(|t| t.listed).count();
    let mut summary = format!("{listed} tracks");
    if let Some(playlist) = write_playlist(cfg, tracks)? {
        summary = format!("{summary}, playlist written to {}", playlist.display());
    }
    if !warning.is_empty() {
        summary = format!("{summary}   [{warning}]");
    }
    Ok(summary)
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
        };
        if let Err(err) = done {
            let _ = tx.send(Msg::Stage(format!("failed: {err}")));
            let _ = tx.send(Msg::Log(err.to_string()));
        }
    }
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
    let path = tracks[pos].path.clone().context("track has no file")?;
    if !path.exists() {
        bail!("track {index} was never downloaded");
    }

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

    tag::set_fields(&path, &artist, &title)?;
    let track = &mut tracks[pos];
    track.artist = artist;
    track.title = title;
    track.name = label(&track.title, &track.artist);
    track.status = Status::Manual;
    track.source = "typed".into();
    track.note.clear();
    track.listed = true;
    let _ = tx.send(Msg::Update {
        index,
        status: Status::Manual,
        source: Some("typed".into()),
        note: Some(String::new()),
        name: Some(track.name.clone()),
    });

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

fn folder_of(tracks: &[Track]) -> Option<PathBuf> {
    tracks
        .iter()
        .filter_map(|t| t.path.as_deref())
        .next()
        .and_then(Path::parent)
        .map(Path::to_path_buf)
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
    use super::{Pick, cover_options};
    use crate::lookup::Artwork;

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
