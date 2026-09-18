use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;
use std::sync::mpsc::{self, Receiver, Sender};

use anyhow::{Context, Result, bail};

use crate::app::{Asker, Cmd, Escape, Msg, Reply, Shelf, Status, Track, Watch};
use crate::config::{self, Config, composed, decomposed};
use crate::lookup;
use crate::manifest;
use crate::player;
use crate::tag::{self, CoverState};
use crate::ytdlp;

pub fn run(mut cfg: Config, tx: Sender<Msg>, cancel: Arc<AtomicBool>, cmds: Receiver<Cmd>) {
    let mut tracks = Vec::new();
    let result = match ask_start(&mut cfg, &tx) {
        Start::Playlist => pipeline(&cfg, &tx, &cancel, &mut tracks, cfg.pick),
        Start::Resync(saved) => resync(&mut cfg, &tx, &cancel, &mut tracks, saved),
        /* Nothing has been synced and nothing has failed, so there is no
           outcome to report: the library screen is the whole answer. */
        Start::Library(shelves) => {
            let _ = tx.send(Msg::Stage("library".into()));
            let _ = tx.send(Msg::Library {
                shelves,
                show: true,
            });
            serve(&mut cfg, &tx, &mut tracks, cmds, &cancel);
            return;
        }
        /* Cancelling the first question cancels the tool: no run started, so
           there is nothing to stay open for. */
        Start::Cancelled => {
            let _ = tx.send(Msg::Quit);
            return;
        }
    };
    if finish(&mut cfg, &tx, &cancel, &mut tracks, result) {
        serve(&mut cfg, &tx, &mut tracks, cmds, &cancel);
    }
}

enum Start {
    Playlist,
    /// Carries the folders it found, so the answer is not looked up twice.
    Resync(Vec<Shelf>),
    Library(Vec<Shelf>),
    Cancelled,
}

/* Nothing to do without a URL. A library on disk is a better answer than a
   question: its rows are the playlists the URL question was going to be
   about, and one of them can be opened without downloading anything. */
fn ask_start(cfg: &mut Config, tx: &Sender<Msg>) -> Start {
    if !cfg.url.is_empty() {
        return Start::Playlist;
    }
    let saved = library(&cfg.dir);
    if cfg.resync {
        return Start::Resync(saved);
    }
    if !saved.is_empty() {
        return Start::Library(saved);
    }

    let _ = tx.send(Msg::Stage("waiting for a playlist URL".into()));
    match prompt_url(tx, Escape::Quit) {
        Some(url) => {
            cfg.url = url;
            /* The only format question before the first download: the run
               adopts whatever this leaves behind. A URL from the command line
               returns above, having arrived fully specified. `start_url` asks
               the same thing for the library's `n`, so the two stay in step. */
            let asker = Asker {
                tx: tx.clone(),
                enabled: true,
            };
            report(tx, set_format(cfg, tx, &asker));
            Start::Playlist
        }
        None => Start::Cancelled,
    }
}

/* Asks again rather than failing: a mistyped link is the likely case, and
   handing back the text means fixing it instead of retyping it. `None` is a
   cancel, and `escape` is what the caller does with one: only the very first
   question has nothing to go back to. */
fn prompt_url(tx: &Sender<Msg>, escape: Escape) -> Option<String> {
    let asker = Asker {
        tx: tx.clone(),
        enabled: true,
    };
    let mut header = "YouTube playlist URL".to_string();
    let mut typed = String::new();
    /* The first thing a new user is ever asked, on a screen with nothing else
       on it yet. Both facts they need before typing: that one video is a
       valid answer, and what the only other key does. */
    let note = match escape {
        Escape::Quit => "a playlist or a single video  ·  esc quits",
        _ => "a playlist or a single video",
    };
    loop {
        let answer = asker.input_noted(&header, note, &typed, escape)?;
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
        /* Both resolved each pass, and deliberately not cached: syncing another
           playlist inside this loop moves the folder, and cliamp is started and
           stopped from another window while this menu sits open. The row has to
           mean what `p` means, or the same action is offered on one screen and
           hidden on the next. */
        let playable = folder_of(tracks)
            .filter(|f| player::playlist_file(f).is_file())
            .filter(|_| player::running());
        let _ = tx.send(Msg::Done {
            result: result.clone(),
            stage: "finished",
        });
        if cancel.load(Ordering::SeqCst) {
            return false;
        }

        let (title, note) = match &result {
            Ok(summary) => ("finished", summary.clone()),
            Err(err) => ("failed", err.clone()),
        };
        /* Labels and answers built together: the row for cliamp is only there
           when cliamp is, and a bare index would then mean a different thing
           depending on what is installed. */
        /* First, ahead of keeping the menu open: when the run left guesses
           behind, looking at them is the thing the run is asking for, and
           Enter on it still starts no work and ends no session. */
        let mut rows: Vec<(String, Answer)> = Vec::new();
        let looking = tracks.iter().filter(|t| t.status.wants_a_look()).count();
        if looking > 0 {
            let plural = if looking == 1 { "track" } else { "tracks" };
            rows.push((
                format!("review {looking} {plural} worth a look"),
                Answer::Review,
            ));
        }
        rows.push(("keep this open".into(), Answer::Keep));
        if playable.is_some() {
            rows.push(("play it in cliamp".into(), Answer::Play));
        }
        rows.push(("sync another playlist".into(), Answer::Another));
        /* "Next time as flac" is a thought people have at the end of a run,
           on a screen where `f` means follow. Only with a file to write it
           to: without one the choice would last until the tool closed, which
           is not what "next time" means. */
        if cfg.config_file.is_some() {
            rows.push((
                format!("format for next time  ({})", cfg.format),
                Answer::Format,
            ));
        }
        rows.push(("quit".into(), Answer::Quit));

        let options: Vec<String> = rows.iter().map(|(label, _)| label.clone()).collect();
        let chosen = asker
            .choose_noted(title, &note, options)
            .and_then(|n| rows.get(n).map(|(_, answer)| *answer));
        match chosen {
            Some(Answer::Play) => {
                if let Some(folder) = playable.clone() {
                    match player::load(&folder) {
                        Ok(said) => {
                            let _ = tx.send(Msg::Flash(said));
                        }
                        Err(err) => {
                            let _ = tx.send(Msg::Log(format!("cliamp: {err}")));
                            let _ = tx.send(Msg::Flash(format!("cliamp: {err}")));
                        }
                    }
                }
            }
            Some(Answer::Another) => {
                // Cancelling the URL question goes back here, not out, and
                // leaves the last run's outcome exactly as it was.
                let Some(url) = prompt_url(tx, Escape::Keep) else {
                    continue;
                };
                let _ = tx.send(Msg::Restart);
                tracks.clear();
                cfg.url = url;
                // As in `start_url`: this playlist is not the last one.
                cfg.folder = None;
                let gate = cfg.pick;
                result = pipeline(cfg, tx, cancel, tracks, gate).map_err(|e| e.to_string());
            }
            /* Closes the menu like Keep does: the answer is on the list
               behind it, and nothing more is being asked of the worker. */
            Some(Answer::Review) => {
                let _ = tx.send(Msg::Review);
                return true;
            }
            /* Straight back to the menu, which now says the new format:
               the question it was asked from is still the open one. */
            Some(Answer::Format) => {
                if let Err(err) = set_format(cfg, tx, &asker) {
                    let _ = tx.send(Msg::Flash(format!("failed: {err}")));
                    let _ = tx.send(Msg::Log(err.to_string()));
                }
            }
            Some(Answer::Quit) => {
                let _ = tx.send(Msg::Quit);
                return false;
            }
            // Esc keeps it open too: the safe answer to "what now?" is nothing.
            _ => return true,
        }
    }
}

#[derive(Clone, Copy)]
enum Answer {
    Keep,
    Review,
    Format,
    Play,
    Another,
    Quit,
}

/// Every playlist folder already under `--dir`, in name order. Read from the
/// manifests alone: this runs before the UI is up and must not touch the
/// network. A folder with no recorded URL is not one of ours.
pub fn library(dir: &Path) -> Vec<Shelf> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut found: Vec<Shelf> = entries
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.is_dir())
        .filter_map(|path| {
            // One read for all three: this walks every folder under --dir.
            let sidecar = manifest::load(&path);
            let url = sidecar.url?;
            // One stat per entry, feeding both the count and the preview.
            let files: Vec<(String, bool)> = sidecar
                .entries
                .iter()
                .map(|(_, f)| {
                    let name = f.file_name().unwrap_or_default().to_string_lossy().into();
                    (name, f.is_file())
                })
                .collect();
            let missing = files.iter().filter(|(_, here)| !here).count();
            Some(Shelf {
                name: path.file_name().unwrap_or_default().to_string_lossy().into(),
                url,
                tracks: files.len() - missing,
                missing,
                synced: sidecar.synced,
                files,
                path,
            })
        })
        .collect();
    found.sort_by(|a, b| a.name.cmp(&b.name));
    found
}

/* The tags on disk are the whole truth here: no listing, no lookup, no
   network. Which means the folder opens instantly and offline, and every
   post-run command works on it, but nothing knows whether the playlist
   upstream has changed. `S` is what answers that. */
fn open_shelf(
    cfg: &mut Config,
    tx: &Sender<Msg>,
    tracks: &mut Vec<Track>,
    shelf: &Shelf,
) -> Result<String> {
    let entries = manifest::entries(&shelf.path);
    if entries.is_empty() {
        bail!(
            "{} has no tracks recorded in its manifest  ·  sync it to fill one in",
            shelf.name
        );
    }

    let _ = tx.send(Msg::Restart);
    let _ = tx.send(Msg::Playlist(shelf.name.clone()));
    let _ = tx.send(Msg::Folder(shelf.path.clone()));
    cfg.url = shelf.url.clone();
    /* Carried onto the config so a later `S` writes back into this folder
       rather than into whatever the playlist is called upstream. */
    cfg.folder = manifest::load(&shelf.path).name;
    tracks.clear();

    let mut missing = 0;
    for (id, file) in entries {
        if !file.is_file() {
            missing += 1;
            continue;
        }
        let name = file
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_default();
        /* Taken from the filename, not from the row's position: every file is
           written with its playlist number in front, and a folder missing a
           track would otherwise renumber everything after the gap the next
           time one of them was edited. 0 means the name had no usable number
           and one is handed out below. */
        let index = numbered(&name).unwrap_or(0);
        let mut track = Track::new(index, id, name, file.clone());
        track.status = Status::Have;
        track.source = "on disk".into();
        track.listed = true;
        if let Ok(info) = tag::read(&file) {
            track.artist = info.artist;
            track.title = info.title;
            track.album = info.album;
            track.year = info.year;
            track.duration = info.duration;
            // An untagged file would otherwise read as "? - ?", which says
            // less about which track it is than the filename does.
            if !track.title.is_empty() {
                track.name = label(&track.title, &track.artist);
            }
        }
        // Otherwise every row reports it was "was" its own filename.
        track.was = track.name.clone();
        tracks.push(track);
    }

    mark_departed(&shelf.path, tracks);

    /* Two files can carry the same number: removing a video renumbers the
       tracks after it, and the file that left keeps the name it had, so a
       folder ends up with two `05 - `. Every command finds its track by index
       and takes the first match, so sharing one makes a row write tags to
       another row's file. Tracks still in the playlist claim their own number
       first, and everything else is numbered past them, which is where a
       departed row sits after a real sync too. */
    let mut taken: HashSet<usize> = HashSet::new();
    for track in tracks.iter_mut() {
        let live = track.status != Status::Gone;
        if !live || track.index == 0 || !taken.insert(track.index) {
            track.index = 0;
        }
    }
    let mut next = taken.iter().copied().max().unwrap_or(0);
    for track in tracks.iter_mut().filter(|t| t.index == 0) {
        next += 1;
        track.index = next;
    }

    tracks.sort_by_key(|t| t.index);
    let _ = tx.send(Msg::Tracks(tracks.clone()));

    let departed = tracks.iter().filter(|t| t.status == Status::Gone).count();
    let mut summary = format!(
        "{} tracks in {}",
        tracks.len() - departed,
        shelf.path.display()
    );
    if departed > 0 {
        summary = format!("{summary}, {departed} no longer in the playlist");
    }
    if missing > 0 {
        let files = if missing == 1 { "file" } else { "files" };
        summary = format!("{summary}, {missing} {files} missing");
    }
    Ok(summary)
}

/* Which files on disk the playlist actually holds. Offline there is no listing
   to ask, but the .m3u8 from the last sync is that same answer written down:
   `write_playlist` leaves out exactly the tracks that had departed. `Gone` and
   not just `listed = false`, because that is what the guards elsewhere are
   keyed on, and without it the first edit re-lists the track and renames it to
   a playlist position it does not hold. */
fn mark_departed(folder: &Path, tracks: &mut [Track]) {
    let Some(names) = last_playlist(folder) else {
        return;
    };
    /* On the stem, not the whole filename. A conversion changes a track's
       extension and nothing else about it, and the playlist file cannot be
       kept in step with perfect atomicity: `repoint_playlist` narrows that to
       one track and one instant, and this is what makes even that harmless.
       Two files differing only by extension both match one entry, which errs
       towards calling nothing departed, and that is already the safe answer
       here for the same reason the guard below gives. */
    let stems: HashSet<String> = names
        .iter()
        .map(|name| stem_of(Path::new(name)))
        .collect();
    let holds = |track: &Track| {
        track
            .path
            .as_deref()
            .is_some_and(|path| stems.contains(&stem_of(path)))
    };
    /* Nothing matched, so the file describes some other folder and both
       answers are guesses. Calling nothing departed is the safe one: it cannot
       empty a playlist that is still correct. */
    if !tracks.iter().any(holds) {
        return;
    }
    for track in tracks.iter_mut().filter(|t| !holds(t)) {
        track.status = Status::Gone;
        track.listed = false;
        track.source = "not in playlist".into();
    }
}

/* Composed, because `write_playlist` writes decomposed entries and the files
   are on disk composed. Matched raw, every Hangul or accented name misses while
   the ASCII ones still match, which is exactly enough to clear the "believe
   none of it" guard below and call the rest `Gone`: the half-converted folder's
   bug by a different road. */
fn stem_of(path: &Path) -> String {
    composed(&path.file_stem().unwrap_or_default().to_string_lossy())
}



/// Filenames from the folder's own `.m3u8`, or `None` when it has none, which
/// is every folder synced with `--no-m3u8`. Matched by name rather than by the
/// absolute path the file records, so a folder that has moved still resolves.
fn last_playlist(folder: &Path) -> Option<HashSet<String>> {
    let name = folder.file_name()?.to_string_lossy().to_string();
    let text = std::fs::read_to_string(folder.join(format!("{name}.m3u8"))).ok()?;
    let names: HashSet<String> = text
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
        .filter_map(|line| Path::new(line).file_name())
        .map(|name| name.to_string_lossy().to_string())
        .collect();
    (!names.is_empty()).then_some(names)
}

/// The playlist position yt-dlp wrote in front of the filename, which survives
/// a rename because earworm writes it back in the same place.
fn numbered(name: &str) -> Option<usize> {
    let digits: String = name.chars().take_while(char::is_ascii_digit).collect();
    // A file that is all digits has no title, so this is not one of ours.
    if digits.is_empty() || digits.len() == name.len() {
        return None;
    }
    digits.parse().ok().filter(|n| *n > 0)
}

/* One pipeline per folder rather than one list of everything: `Track::index`
   is a playlist position, and two playlists both have a track 1, so merging
   them would make every row, mark and command ambiguous. */
fn resync(
    cfg: &mut Config,
    tx: &Sender<Msg>,
    cancel: &AtomicBool,
    tracks: &mut Vec<Track>,
    found: Vec<Shelf>,
) -> Result<String> {
    if found.is_empty() {
        bail!(
            "no playlist under {} has a saved URL yet; sync one by URL first",
            cfg.dir.display()
        );
    }

    let mut synced = 0;
    let mut listed = 0;
    for (n, shelf) in found.iter().enumerate() {
        if cancel.load(Ordering::SeqCst) {
            break;
        }
        let name = &shelf.name;
        let _ = tx.send(Msg::Stage(format!("{} of {}: {name}", n + 1, found.len())));
        // Each folder starts from an empty list, so its rows are its own.
        tracks.clear();
        let _ = tx.send(Msg::Restart);
        cfg.url = shelf.url.clone();
        cfg.folder = manifest::load(&shelf.path).name;

        match pipeline(cfg, tx, cancel, tracks, false) {
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

/// `gate` is separate from `cfg.pick` because `--resync` walks the whole
/// library unattended: a question per folder is the one thing that would stop
/// it, and the user asked for the sync, not for each playlist in it.
fn pipeline(
    cfg: &Config,
    tx: &Sender<Msg>,
    cancel: &AtomicBool,
    tracks: &mut Vec<Track>,
    gate: bool,
) -> Result<String> {
    let _ = tx.send(Msg::Stage("listing playlist".into()));
    let listing = ytdlp::scan(cfg)?;
    *tracks = listing.tracks;

    let playlist = folder_of(tracks)
        .and_then(|f| f.file_name().map(|n| n.to_string_lossy().to_string()))
        .unwrap_or_default();
    if !playlist.is_empty() {
        let _ = tx.send(Msg::Playlist(playlist.clone()));
        if let Some(folder) = folder_of(tracks) {
            let _ = tx.send(Msg::Folder(folder));
        }
    }
    note_departures(cfg, tx, listing.complete, tracks);
    let _ = tx.send(Msg::Tracks(tracks.clone()));

    // Held on to, because `to_bring` has to honour the same answer.
    let picked = gate.then(|| ask_which(tx, tracks)).flatten();

    let have = tracks.iter().filter(|t| t.status == Status::Have).count();
    let _ = tx.send(Msg::Stage(format!(
        "fetching  ({have} of {} already on disk)",
        tracks.len()
    )));

    /* Decided here and not a moment later: this is the last point at which
       every track either holds the file an earlier run left or holds nothing,
       and the download is about to overwrite `path` for anything it fetches.
       Empty unless `convert` is on. */
    let plans = to_bring(cfg, tracks, picked.as_ref());
    let refetch: HashSet<usize> = plans
        .iter()
        .filter(|(_, plan)| **plan == Bring::Refetch)
        .map(|(index, _)| *index)
        .collect();
    let was: HashMap<usize, PathBuf> = tracks
        .iter()
        .filter(|t| refetch.contains(&t.index))
        .filter_map(|t| t.path.clone().map(|p| (t.index, p)))
        .collect();

    // Before the download, which is what writes the one this pass may drop.
    let theirs = folder_covers(tracks);

    /* yt-dlp exits non-zero if any single entry failed, and aborting here
       would throw away the tags, cover and playlist for every track that did
       download. Carry the failure into the summary instead. */
    let mut warning = String::new();
    if let Err(err) = ytdlp::run(cfg, tracks, tx, &refetch) {
        warning = err.to_string();
        let _ = tx.send(Msg::Log(format!("download: {warning}")));
    }

    let (adopted, missing, broke) = adopt_refetched(cfg, tx, tracks, &was);
    let (encoded, unencoded) = convert_tracks(cfg, tx, cancel, tracks, &plans);
    let converted = adopted + encoded;
    // Both halves failing is one thing to the reader: it did not come across.
    let stuck = broke + unencoded;

    let _ = tx.send(Msg::Stage("tagging".into()));
    let asker = Asker {
        tx: tx.clone(),
        enabled: cfg.fix,
    };
    let mut cover = CoverState { written: false };
    tag_tracks(cfg, tx, cancel, tracks, &playlist, &asker, &mut cover, None);

    sync_manifest(cfg, tracks);
    let playlist_file = write_playlist(cfg, tracks)?;
    drop_folder_cover(tracks, &theirs);
    let mut summary = summarise(tracks, playlist_file.as_deref());
    /* Said out loud, because the rows scroll off and a --resync's one line
       per folder is the only record anyone reads afterwards. */
    if converted > 0 {
        summary = format!("{summary}, {converted} brought to {}", cfg.format);
    }
    /* Both said out loud. The rows scroll off, a --resync writes one line per
       folder and that is all anyone reads afterwards, and a folder where every
       transcode failed for want of an encoder used to report as a clean sync. */
    if stuck > 0 {
        summary = format!("{summary}, {stuck} could not be brought across");
    }
    if missing > 0 {
        summary = format!("{summary}, {missing} no longer downloadable");
    }
    if !warning.is_empty() {
        summary = format!("{summary}   [{warning}]");
    }
    Ok(summary)
}

/* What a sync has to do to bring one track to the current format. Decided
   before the download, which is the only point at which every track either
   has the file an earlier run left or has no file at all. */
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Bring {
    Leave,
    Convert,
    Refetch,
}

/// The format a file on disk actually holds, as a `FORMATS` name, or `None`
/// for a container earworm does not write.
fn format_on_disk(path: &Path) -> Option<&'static str> {
    let ext = path.extension()?.to_str()?.to_ascii_lowercase();
    /* `alac` and `m4a` share this extension, so the container has to be asked
       which codec is inside it. `None` from that is a genuine "cannot tell",
       and falling back to the extension would pick one of the two at random
       and re-encode a file on no evidence. */
    if ext == "m4a" {
        return tag::mp4_format(path);
    }
    config::FORMATS
        .iter()
        .find(|(_, e, _)| *e == ext)
        .map(|(name, _, _)| *name)
}

/* The per-track decision. Two facts drive it, and both were measured rather
   than assumed:

   yt-dlp's `bestaudio` on YouTube is itag 251, Opus in WebM. Checked with
   `--print %(acodec)s` against a real video: the AAC stream (itag 140) is
   there and is not what gets picked. So `--audio-format opus` copies the
   stream it already has, and every other format is ffmpeg re-encoding that
   same Opus. A download is one lossy generation for opus and two for m4a,
   mp3 and vorbis; flac and alac are the one generation stored losslessly.

   That gives each format on disk a generation count: opus, flac and alac
   hold one, and m4a, mp3 and vorbis hold two. Converting adds another
   whenever the target is lossy, and adds none when it is not.

   - `m4a`, `mp3`, `vorbis` on disk: already two, so converting makes three
     where a download makes at most two. Fetch it again. `m4a` belongs here
     despite being a container YouTube serves, because it is not the
     container `bestaudio` hands over.
   - target `opus`: the only one a download remuxes rather than re-encodes,
     so a fetch is one generation against the two any conversion would cost.
     Fetch it again unless the file already is one.
   - everything else: convert. The source holds one generation and a download
     would cost exactly what ffmpeg costs here, without the network.

   `Have` only. `Gone` holds no playlist position and its index is synthetic,
   `Skipped` was never fetched, `Pending` is about to arrive in the target
   format anyway. */
fn bring(track: &Track, format: &str) -> Bring {
    if track.status != Status::Have {
        return Bring::Leave;
    }
    let Some(path) = track.path.as_deref().filter(|p| p.is_file()) else {
        return Bring::Leave;
    };
    match format_on_disk(path) {
        // Not a container earworm writes, so nothing is claimed about it.
        None => Bring::Leave,
        Some(now) if now == format => Bring::Leave,
        Some("m4a" | "mp3" | "vorbis") => Bring::Refetch,
        Some(_) if format == "opus" => Bring::Refetch,
        Some(_) => Bring::Convert,
    }
}

/// Tracks a sync would bring across, keyed by index. Empty when `convert` is
/// off, which is what keeps a format change from rewriting anything.
///
/// `picked` is the pick gate's answer when it ran. The gate marks unpicked
/// `Pending` tracks `Skipped`, which says nothing about the ones already on
/// disk, so without honouring it here a run where somebody asked for two new
/// tracks also re-downloaded every mp3 in the folder, with nothing on that
/// screen mentioning it.
fn to_bring(
    cfg: &Config,
    tracks: &[Track],
    picked: Option<&HashSet<usize>>,
) -> HashMap<usize, Bring> {
    if !cfg.convert {
        return HashMap::new();
    }
    tracks
        .iter()
        .filter(|t| picked.is_none_or(|only| only.contains(&t.index)))
        .filter_map(|t| match bring(t, &cfg.format) {
            Bring::Leave => None,
            plan => Some((t.index, plan)),
        })
        .collect()
}

/* Puts `new` in the place of the track's current file, carrying across
   everything the old one held. The order is the design: nothing is removed
   until the replacement has been read back, because a video pulled from
   YouTube since the first download cannot be fetched again and deleting
   first loses it for good.

   Tags go on through lofty rather than ffmpeg's `-map_metadata`: `with_tag`
   knows which tag type each container takes, and ffmpeg's field mapping
   across containers is inconsistent in the way that has already cost a day.
   They are read off the old file rather than taken from the row, because a
   refetched track's row has already been overwritten by the download.

   The track lands back on `Have`, so `tag_tracks` reads the new file and
   neither identifies nor renames it. A converted track is not a fresh
   find: sending it through the lookup would replace the hand-typed
   correction this just carried across with a guess. */
fn swap_in(
    tx: &Sender<Msg>,
    track: &mut Track,
    new: &Path,
    format: &str,
    owned: &HashSet<PathBuf>,
) -> Result<Option<PathBuf>> {
    let old = track.path.clone().context("track has no file")?;

    let kept = tag::read(&old).ok();
    let cover = tag::read_cover(&old);

    /* An ffmpeg that half-wrote exits non-zero, but a full disk is the case
       that makes reading it back worth the second or two. */
    let size = std::fs::metadata(new).map(|m| m.len()).unwrap_or(0);
    if size == 0 {
        bail!("the new file is empty");
    }
    tag::read(new).context("the new file will not read back")?;
    /* And that it is the format this run asked for. On the refetch path the
       file is whatever yt-dlp wrote, so a postprocessor that fell back to the
       source container would otherwise be renamed, recorded and reported as
       converted while being nothing of the kind, and `format_on_disk` would
       then say `Leave` about it on every later sync. */
    match format_on_disk(new) {
        Some(got) if got == format => {}
        Some(got) => bail!("the new file is {got}, not {format}"),
        None => bail!("the new file is not a format earworm writes"),
    }

    if let Some(info) = kept {
        let fields = tag::Fields {
            artist: info.artist,
            title: info.title,
            album: info.album,
            year: info.year,
        };
        tag::set_fields(new, &fields).context("could not carry the tags across")?;
    }
    if let Some(image) = cover {
        // Not fatal: a track with the right audio and no art beats no track.
        if let Err(err) = tag::set_cover(new, &image) {
            let _ = tx.send(Msg::Log(format!("convert: cover: {err}")));
        }
    }

    /* The old file's name with the extension the chosen format lands on, so a
       name an edit gave the track survives. Taken from the format rather than
       from `new`, which on the refetch path is whatever yt-dlp decided to call
       it. `alac` and `m4a` share one, which is the case where this lands
       exactly where the old file already is. */
    let target = old.with_extension(config::extension(format));

    if target != *new {
        if target.exists() && target != old {
            /* Something is already sitting on the name. If a track holds it,
               it is somebody's file and nothing here may take it. If nothing
               does, it is this pass's own leftover: an interrupted swap
               renames the replacement into place and is killed before the
               sidecar moves, and refusing forever would wedge the track,
               re-fetching and discarding a download on every later sync. */
            if owned.contains(&target) {
                bail!(
                    "{} is another track's file  ·  rename one of them and sync again",
                    target.file_name().unwrap_or_default().to_string_lossy()
                );
            }
            let _ = tx.send(Msg::Log(format!(
                "convert: track {}: replacing a leftover {}",
                track.index,
                target.file_name().unwrap_or_default().to_string_lossy()
            )));
        }
        /* `alac` and `m4a` share an extension, so this is sometimes a rename
           straight over the old file. Left to the rename rather than removing
           it first: the replacement is atomic, where a remove would open a
           moment in which the track has no file at all. */
        std::fs::rename(new, &target).context("could not put the new file in place")?;
    }
    /* Between the two, so the playlist never names a file that is not there.
       The sidecar is saved per track for this reason and the `.m3u8` has to
       move with it: it is written once at the end of the pass, so quitting
       part-way through a conversion used to leave it naming the old files.
       Every converted track then failed `mark_departed`'s filename match the
       next time the folder was opened from the library, and a folder that was
       only half done still had enough unconverted tracks matching to get past
       the "believe nothing" guard, so each one was called departed. */
    if let (Some(folder), Some(from), Some(to)) =
        (old.parent(), old.file_name(), target.file_name())
        && from != to
    {
        repoint_playlist(folder, from, to);
    }
    track.path = Some(target.clone());
    track.status = Status::Have;
    let _ = tx.send(Msg::Path {
        index: track.index,
        path: target.clone(),
    });
    /* Handed back rather than removed here, so the caller can move the
       sidecar onto the new file first. Killed in between, the manifest names
       the replacement, which is there, and the next sync sees a track already
       in the target format and leaves it alone. Removed first, the manifest
       still names a file that has gone. */
    Ok((target != old && old.exists()).then_some(old))
}

/* Every file the run's other tracks hold. `swap_in` consults it to tell a
   leftover from an interrupted swap, which it may replace, from a file that
   belongs to another track, which it may not. Recomputed per track because a
   conversion earlier in the pass has moved one of them. */
fn files_held(tracks: &[Track], except: usize) -> HashSet<PathBuf> {
    tracks
        .iter()
        .filter(|t| t.index != except)
        .filter_map(|t| t.path.clone())
        .collect()
}

/* Renames one entry in the folder's own playlist file, leaving the order and
   everything else in it alone. Not `write_playlist`, which builds the whole
   file from the tracks that are `listed`: nothing is listed until the tagging
   pass sets it, so calling that here would write an empty playlist over a
   good one. Absent or unreadable is the ordinary `--no-m3u8` case and not
   worth reporting. */
fn repoint_playlist(folder: &Path, from: &std::ffi::OsStr, to: &std::ffi::OsStr) {
    let Some(name) = folder.file_name() else {
        return;
    };
    let playlist = folder.join(format!("{}.m3u8", name.to_string_lossy()));
    let Ok(text) = std::fs::read_to_string(&playlist) else {
        return;
    };
    let mut changed = false;
    let mut out = String::with_capacity(text.len());
    for line in text.lines() {
        let entry = line.trim();
        /* `#EXTINF` carries the duration and title, never the filename. Matched
           on the composed form because a playlist written before this folder
           was last synced holds composed entries, and `from` comes off disk. */
        let hit = !entry.is_empty()
            && !entry.starts_with('#')
            && Path::new(entry)
                .file_name()
                .is_some_and(|name| composed(&name.to_string_lossy()) == composed(&from.to_string_lossy()));
        if hit {
            changed = true;
            // Through the path, so a relative prefix survives the swap.
            out.push_str(&decomposed(
                &Path::new(entry).with_file_name(to).to_string_lossy(),
            ));
        } else {
            out.push_str(line);
        }
        out.push('\n');
    }
    if changed {
        let _ = config::write_atomically(&playlist, &out);
    }
}

/* The half-written output of a conversion, beside the track rather than in a
   temp directory: a rename across filesystems is a copy, and the folder is
   where the headroom has to be anyway. Hidden and prefixed so `sweep_scratch`
   can recognise its own leavings and nothing else. */
const SCRATCH: &str = ".earworm-convert-";

/* Whatever a killed ffmpeg left behind. Quitting mid-conversion kills the
   child from the shutdown path, so the loop below never reaches the cleanup
   in its own error arm, and the part-written file stays in the user's music
   folder. Overwriting the same name on the next attempt covers only a repeat
   of the same track into the same format; change the format in between and
   the orphan is there for good. Matched on earworm's own hidden prefix, so
   this can never take a file somebody else put there. */
fn sweep_scratch(folder: &Path) {
    let Ok(entries) = std::fs::read_dir(folder) else {
        return;
    };
    for path in entries.flatten().map(|e| e.path()) {
        let leftover = path
            .file_name()
            .and_then(|n| n.to_str())
            .is_some_and(|n| n.starts_with(SCRATCH));
        if leftover && path.is_file() {
            let _ = std::fs::remove_file(&path);
        }
    }
}

/* The convert half, run between the download and the tagging. */
fn convert_tracks(
    cfg: &Config,
    tx: &Sender<Msg>,
    cancel: &AtomicBool,
    tracks: &mut [Track],
    plans: &HashMap<usize, Bring>,
) -> (usize, usize) {
    let todo: Vec<usize> = (0..tracks.len())
        .filter(|i| plans.get(&tracks[*i].index) == Some(&Bring::Convert))
        .collect();
    if todo.is_empty() {
        return (0, 0);
    }
    let _ = tx.send(Msg::Stage(format!("converting {} tracks", todo.len())));
    // Before writing any of our own, so a killed run does not accumulate them.
    if let Some(folder) = folder_of(tracks) {
        sweep_scratch(&folder);
    }

    let ext = config::extension(&cfg.format);
    let (mut done, mut failed) = (0, 0);
    for i in todo {
        if cancel.load(Ordering::SeqCst) {
            break;
        }
        let index = tracks[i].index;
        let Some(old) = tracks[i].path.clone() else {
            continue;
        };
        tracks[i].status = Status::Converting;
        let _ = tx.send(Msg::Update {
            index,
            status: Status::Converting,
            source: None,
            note: None,
            name: None,
        });

        // Beside the track and named so it cannot collide with a real file.
        let scratch = old.with_file_name(format!("{SCRATCH}{index}.{ext}"));
        let _ = std::fs::remove_file(&scratch);
        // What the file actually holds, so the encoder knows whether there
        // is any depth worth preserving.
        let source = format_on_disk(&old);
        let owned = files_held(tracks, index);
        let outcome = tag::transcode(&old, &scratch, &cfg.format, source)
            .and_then(|()| swap_in(tx, &mut tracks[i], &scratch, &cfg.format, &owned));
        match outcome {
            Ok(stale) => {
                done += 1;
                tracks[i].source = "converted".into();
                let _ = tx.send(Msg::Update {
                    index,
                    status: Status::Have,
                    source: Some("converted".into()),
                    note: None,
                    name: None,
                });
                /* Per track, not at the end. The sidecar is a small text file
                   and rewriting it 300 times costs nothing next to 300
                   transcodes, and it is what leaves a cancelled run with
                   every track either fully swapped or fully untouched. Before
                   the old file goes, so a kill in between leaves the sidecar
                   naming a file that is there. */
                save_manifest(cfg, tracks);
                if let Some(old) = stale {
                    let _ = std::fs::remove_file(old);
                }
            }
            Err(err) => {
                failed += 1;
                let _ = std::fs::remove_file(&scratch);
                // Straight back to where it was: the old file never moved.
                tracks[i].status = Status::Have;
                let _ = tx.send(Msg::Update {
                    index,
                    status: Status::Have,
                    source: Some("on disk".into()),
                    note: Some("not converted".into()),
                    name: None,
                });
                let _ = tx.send(Msg::Log(format!("convert: track {index}: {err}")));
            }
        }
    }
    (done, failed)
}

/* The refetch half, run after the download. yt-dlp has already written the
   new file under the name its own template predicts and the `@D` line has
   already overwritten `track.path` with it, which is why the old path has to
   have been remembered before the run. */
fn adopt_refetched(
    cfg: &Config,
    tx: &Sender<Msg>,
    tracks: &mut [Track],
    was: &HashMap<usize, PathBuf>,
) -> (usize, usize, usize) {
    /* Three outcomes, not two. A download that never arrived and one that
       arrived and could not be used are different things to have happened,
       and one message covering both told the reader the wrong one half the
       time. */
    let (mut done, mut left, mut broke) = (0, 0, 0);
    for i in 0..tracks.len() {
        let index = tracks[i].index;
        let Some(old) = was.get(&index) else {
            continue;
        };
        let fresh = tracks[i].path.clone();
        /* Unchanged means yt-dlp never wrote it: the video is gone, private,
           or the download failed. The old file is still there and still in
           the manifest, so the track keeps the format it had. */
        let downloaded = fresh
            .as_deref()
            .filter(|p| *p != old.as_path() && p.is_file())
            .map(Path::to_path_buf);
        let Some(new) = downloaded else {
            left += 1;
            tracks[i].path = Some(old.clone());
            tracks[i].status = Status::Have;
            let _ = tx.send(Msg::Path {
                index,
                path: old.clone(),
            });
            let _ = tx.send(Msg::Update {
                index,
                status: Status::Have,
                source: Some("on disk".into()),
                note: Some("could not refetch".into()),
                name: None,
            });
            continue;
        };
        /* The old file is what carries the tags, so it has to be the one
           `swap_in` reads. It points at the download by now. */
        tracks[i].path = Some(old.clone());
        let owned = files_held(tracks, index);
        match swap_in(tx, &mut tracks[i], &new, &cfg.format, &owned) {
            Ok(stale) => {
                done += 1;
                tracks[i].source = "converted".into();
                let _ = tx.send(Msg::Update {
                    index,
                    status: Status::Have,
                    source: Some("converted".into()),
                    note: None,
                    name: None,
                });
                // Before the old file goes. See the same call in `convert_tracks`.
                save_manifest(cfg, tracks);
                if let Some(old) = stale {
                    let _ = std::fs::remove_file(old);
                }
            }
            Err(err) => {
                broke += 1;
                // The download is the one to drop: the old file is the record.
                let _ = std::fs::remove_file(&new);
                tracks[i].path = Some(old.clone());
                tracks[i].status = Status::Have;
                let _ = tx.send(Msg::Path {
                    index,
                    path: old.clone(),
                });
                let _ = tx.send(Msg::Update {
                    index,
                    status: Status::Have,
                    source: Some("on disk".into()),
                    note: Some("not converted".into()),
                    name: None,
                });
                let _ = tx.send(Msg::Log(format!("convert: track {index}: {err}")));
            }
        }
    }
    (done, left, broke)
}

/* Blocks the worker on the UI's answer while the screen keeps drawing. Marks
   the rest `Skipped` rather than dropping them: `write_playlist` builds the
   .m3u8 from the tracks it is given, so a shortened list would rewrite a whole
   playlist file from the handful this run happened to fetch. */
/// Returns what the user chose, because the conversion has to honour it too.
/// `None` is a gate that never got an answer, which is not the same as one
/// that came back empty.
fn ask_which(tx: &Sender<Msg>, tracks: &mut [Track]) -> Option<HashSet<usize>> {
    let (reply_tx, reply_rx) = mpsc::channel();
    let _ = tx.send(Msg::Stage("choose tracks".into()));
    if tx.send(Msg::Pick(reply_tx)).is_err() {
        return None;
    }
    let picked: HashSet<usize> = match reply_rx.recv() {
        Ok(Reply::Picked(picked)) => picked.into_iter().collect(),
        // The UI is gone, and skipping the whole playlist on the way out is
        // worse than the download the shutdown is about to kill anyway.
        _ => return None,
    };
    for track in tracks.iter_mut() {
        // Already on disk or already departed: nothing was going to fetch it.
        if picked.contains(&track.index) || track.status != Status::Pending {
            continue;
        }
        track.status = Status::Skipped;
        let _ = tx.send(Msg::Update {
            index: track.index,
            status: Status::Skipped,
            source: None,
            note: Some("not picked".into()),
            name: None,
        });
    }
    Some(picked)
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
        /* Neither has a file this run is entitled to touch: a departed one is
           somebody else's playlist position, a skipped one was never fetched.
           Falling through marks both `Failed` for having no file. */
        if wanted.is_some_and(|only| !only.contains(&track.index))
            || matches!(track.status, Status::Gone | Status::Skipped)
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
                track.album = info.album;
                track.year = info.year;
                track.duration = info.duration;
                track.listed = true;
                track.name = label(&track.title, &track.artist);
                let _ = tx.send(Msg::Update {
                    index: track.index,
                    status: Status::Have,
                    /* A conversion earlier in this same pass already said
                       where the file came from, and "on disk" is true but
                       drops the one word saying this run rewrote it. */
                    source: Some(match track.source.as_str() {
                        "" => "on disk".into(),
                        already => already.to_string(),
                    }),
                    note: None,
                    name: Some(track.name.clone()),
                });
                offer_meta(tx, track);
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
                track.album = info.as_ref().map(|i| i.album.clone()).unwrap_or_default();
                track.year = info.as_ref().and_then(|i| i.year);
                track.listed = true;
                track.name = label(&track.title, &track.artist);

                /* Only when the lookup found no real album: a playlist name is
                   a worse answer than the release the track came from. */
                if cfg.album && !playlist.is_empty() && track.album.is_empty() {
                    match tag::set_album(&path, playlist) {
                        // Or the row and the file disagree, and the edit form
                        // prefills an album the track no longer has.
                        Ok(()) => track.album = playlist.to_string(),
                        Err(err) => {
                            let _ = tx.send(Msg::Log(format!("album: {err}")));
                        }
                    }
                }

                let _ = tx.send(Msg::Update {
                    index: track.index,
                    status: out.status,
                    source: Some(out.source),
                    note: Some(out.note),
                    name: Some(track.name.clone()),
                });
                // After the playlist-name fallback, or the pane shows the
                // album the file had rather than the one it now holds.
                offer_meta(tx, track);
                /* Last, so it measures whatever finally ended up in the file:
                   `--embed-thumbnail` is the fallback for a track the lookup
                   could not place, and for a YouTube Art Track that is 2048
                   square and several times the size of the audio. */
                match tag::cap_cover(&path) {
                    Ok(true) => {
                        let _ = tx.send(Msg::Log(format!(
                            "cover: track {} resized to {}px",
                            track.index,
                            tag::COVER_MAX
                        )));
                    }
                    Ok(false) => {}
                    Err(err) => {
                        let _ = tx.send(Msg::Log(format!("cover: track {}: {err}", track.index)));
                    }
                }
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
/// without a second pass over the network, and serves the library screen,
/// which has no run behind it at all.
/* The choice outlives the run it is made in, so it goes back to the config
   file rather than living in this process. Nothing already downloaded is
   converted: the manifest names those files by the name they have, so they
   stay downloaded and stay in the format they arrived in. */
/* The folder name is yt-dlp's `%(playlist)s`, which means it is chosen again
   on every download: renaming it without recording the choice would last
   until the next sync and then arrive back as a second folder. The `#name`
   header is what makes it stick, and `cfg.folder` is what reads it. */
fn rename_shelf(
    cfg: &mut Config,
    tx: &Sender<Msg>,
    asker: &Asker,
    tracks: &[Track],
    folder: &Path,
) -> Result<()> {
    let current = folder
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .context("that folder has no name to change")?;
    let Some(name) = asker.input_noted(
        "Playlist name",
        "renames the folder and its .m3u8  ·  tags are left alone",
        &current,
        Escape::Keep,
    ) else {
        return cancelled(tx);
    };
    let name = name.trim().to_string();
    if name.is_empty() {
        let _ = tx.send(Msg::Flash(format!("still called {current}")));
        return Ok(());
    }
    /* Confirming the name a folder already has is how one renamed outside
       earworm gets pinned. Nothing moves; the header is the whole point,
       because without it the next sync fetches the playlist again under the
       title it has upstream and leaves this folder sitting beside it. */
    if name == current {
        if manifest::load(folder).name.is_some() {
            let _ = tx.send(Msg::Flash(format!("already keeping {name}")));
            return Ok(());
        }
        manifest::set_name(folder, &name).with_context(|| {
            format!("recording the name in {}", manifest::path(folder).display())
        })?;
        pin_open_run(cfg, tracks, folder, &name);
        let _ = tx.send(Msg::Flash(format!("keeping the name {name}")));
        return Ok(());
    }
    /* A separator would move the folder somewhere else entirely and a leading
       dot would hide it, neither of which is what renaming means. */
    if name.contains(['/', '\\']) || name.starts_with('.') {
        bail!("a playlist name cannot contain a slash or start with a dot");
    }
    let parent = folder.parent().context("that folder has nowhere to sit")?;
    let to = parent.join(&name);
    if to.exists() {
        bail!("{name} is already a folder here  ·  pick another name");
    }

    /* Before the move, since the stored name is built from the path this
       folder has now: afterwards there is nothing left that names it. */
    player::forget(folder);
    std::fs::rename(folder, &to)
        .with_context(|| format!("renaming {} to {name}", folder.display()))?;

    /* The .m3u8 is named after its folder and `last_playlist` finds it that
       way. Its entries are relative, so the move itself left them valid. */
    let old_playlist = to.join(format!("{current}.m3u8"));
    if old_playlist.is_file() {
        let _ = std::fs::rename(&old_playlist, to.join(format!("{name}.m3u8")));
    }
    manifest::set_name(&to, &name)
        .with_context(|| format!("recording the name in {}", manifest::path(&to).display()))?;

    if pin_open_run(cfg, tracks, folder, &name) {
        // The run on screen is this folder, so it has moved under it.
        let _ = tx.send(Msg::Playlist(name.clone()));
        let _ = tx.send(Msg::Folder(to.clone()));
    }
    let _ = tx.send(Msg::Library {
        shelves: library(&cfg.dir),
        show: false,
    });
    let _ = tx.send(Msg::Flash(format!("renamed to {name}")));
    Ok(())
}

/* `D` on the library screen. The library is read off the disk, so a playlist
   leaves it by losing its claim to be one: forgetting drops the `.earworm`
   sidecar and the `.m3u8` but keeps the audio, deleting takes the folder as
   well. The question is asked here rather than in the UI because the reply
   channel lives on the worker, and the safe answer goes first because Enter
   takes the highlighted row. */
fn remove_shelf(
    cfg: &mut Config,
    tx: &Sender<Msg>,
    asker: &Asker,
    tracks: &mut Vec<Track>,
    folder: &Path,
) -> Result<()> {
    /* The folder arrives from the UI rather than from the scan, and deleting
       the wrong one does not come back, so check rather than assume. */
    if !folder.starts_with(&cfg.dir) || folder == cfg.dir {
        bail!("that folder is outside {}", cfg.dir.display());
    }
    let name = folder
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .context("that folder has no name to remove")?;
    let Some(choice) = asker.choose_noted(
        &format!("Remove {name}?"),
        "forget keeps the audio and drops the playlist  ·  delete takes the folder as well",
        vec![
            "keep it".into(),
            format!("forget {name}"),
            format!("delete {name} and its files"),
        ],
    ) else {
        return cancelled(tx);
    };
    if choice == 0 {
        let _ = tx.send(Msg::Flash(format!("keeping {name}")));
        return Ok(());
    }
    /* Before either removal, since the stored name is built from the path
       this folder has now: afterwards there is nothing left that names it. */
    player::forget(folder);
    if choice == 1 {
        /* Already gone is gone: the row was listed from a manifest that
           someone may have deleted by hand since. */
        for file in [manifest::path(folder), folder.join(format!("{name}.m3u8"))] {
            match std::fs::remove_file(&file) {
                Ok(()) => {}
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                Err(e) => bail!("removing {}: {e}", file.display()),
            }
        }
    } else {
        std::fs::remove_dir_all(folder)
            .with_context(|| format!("deleting {}", folder.display()))?;
    }
    /* The track list on screen is this folder's files, so it goes with it:
       `Restart` is what clears the run state everywhere else too. */
    let open = folder_of(tracks).as_deref() == Some(folder);
    if open {
        let _ = tx.send(Msg::Restart);
        tracks.clear();
    }
    let _ = tx.send(Msg::Library {
        shelves: library(&cfg.dir),
        show: open,
    });
    let _ = tx.send(Msg::Flash(if choice == 1 {
        format!("forgot {name}  ·  audio kept")
    } else {
        format!("deleted {name}")
    }));
    Ok(())
}

/* The open run has to follow its own folder: `S` would otherwise sync the
   playlist back under the name it has upstream and leave this folder behind
   as a copy. Reopening any other folder reads the header instead. */
fn pin_open_run(cfg: &mut Config, tracks: &[Track], folder: &Path, name: &str) -> bool {
    let ours = folder_of(tracks).as_deref() == Some(folder);
    if ours {
        cfg.folder = Some(name.to_string());
    }
    ours
}

fn set_format(cfg: &mut Config, tx: &Sender<Msg>, asker: &Asker) -> Result<()> {
    let options: Vec<String> = config::FORMATS
        .iter()
        .map(|(name, ext, _)| {
            let here = if *name == cfg.format { "  (current)" } else { "" };
            format!("{name}  .{ext}{here}")
        })
        .collect();
    let note = if cfg.convert {
        "Tracks already on disk follow when their playlist next syncs."
    } else {
        "Tracks already on disk keep the format they were downloaded in."
    };
    let Some(choice) = asker.choose_noted("Format for new downloads", note, options) else {
        return Ok(());
    };
    let picked = config::FORMATS[choice].0;
    let settle = |cfg: &mut Config| {
        cfg.format = picked.to_string();
        // Or the help overlay keeps reporting the format the run began with.
        let _ = tx.send(Msg::Settings {
            text: cfg.describe(),
            format: cfg.format.clone(),
        });
    };

    let Some(path) = cfg.config_file.clone() else {
        settle(cfg);
        let _ = tx.send(Msg::Flash(format!("format: {picked}, this run only")));
        return Ok(());
    };
    /* Before the run adopts it: a failed save used to leave the session on
       the new format while reporting that nothing had been remembered, so
       the next download disagreed with both the message and the file. */
    config::save_format(&path, picked)?;
    settle(cfg);
    let _ = tx.send(Msg::Log(format!("format {picked} written to {}", path.display())));
    let _ = tx.send(Msg::Flash(format!("format: {picked}, remembered")));
    offer_convert(cfg, tx, asker, &path, picked);
    Ok(())
}

/* Asked straight after a format pick, because that is the one moment the
   question is already in the user's head. Only when `convert` is off: with it
   on the answer is already yes and the menu's own note says so.

   Anything that fails here is reported and dropped rather than returned. The
   format change itself has already been written and is what the user asked
   for; failing the command would say otherwise. */
fn offer_convert(cfg: &mut Config, tx: &Sender<Msg>, asker: &Asker, path: &Path, picked: &str) {
    if cfg.convert {
        return;
    }
    /* A `Prompt::Choice` header is a `Block::title`, which clips, so the part
       that has to be read goes in the note. And it has to be read: once this
       is on, every later sync converts without asking again, and bringing a
       playlist to a lossless format recovers nothing while multiplying what
       it costs on disk. */
    let note = format!(
        "Every sync from now on, not just this one. YouTube serves lossy audio, so {picked} \
         recovers no quality; flac and alac only make the files several times larger."
    );
    let chosen = asker.choose_noted(
        &format!("Bring tracks already on disk to {picked} too?"),
        &note,
        vec![
            "no, leave them as they are".into(),
            format!("yes, convert them to {picked} on the next sync"),
        ],
    );
    if chosen != Some(1) {
        return;
    }
    if let Err(err) = config::save_convert(path, true) {
        let _ = tx.send(Msg::Log(format!("convert: {err}")));
        let _ = tx.send(Msg::Flash(format!("could not record convert  ·  {err}")));
        return;
    }
    cfg.convert = true;
    let _ = tx.send(Msg::Settings {
            text: cfg.describe(),
            format: cfg.format.clone(),
        });
    let _ = tx.send(Msg::Flash(
        "convert: on  ·  the next sync brings each playlist across".into(),
    ));
}

fn serve(
    cfg: &mut Config,
    tx: &Sender<Msg>,
    tracks: &mut Vec<Track>,
    cmds: Receiver<Cmd>,
    cancel: &AtomicBool,
) {
    let asker = Asker {
        tx: tx.clone(),
        enabled: true,
    };
    /* Polled here rather than on a timer of its own, because `serve` idling is
       exactly when `p` is live: during a run the keys are dead anyway, and a
       third thread to answer a question nobody can act on is not worth the
       second channel. */
    let mut watch = Watch::default();
    /* Lives for the session rather than per command, since that is what `u`
       means: the last tag write, whichever command made it. */
    let mut held: Option<Undo> = None;
    loop {
        if let Some(now) = watch.poll(player::probe()) {
            let _ = tx.send(Msg::Player(now));
        }
        let cmd = match cmds.recv_timeout(PROBE) {
            Ok(cmd) => cmd,
            // Only the far end hanging up ends the loop; a timeout re-checks.
            Err(mpsc::RecvTimeoutError::Timeout) => continue,
            Err(mpsc::RecvTimeoutError::Disconnected) => return,
        };
        /* Some commands edit what is on screen and some replace it. Only the
           ones that went to the network have an outcome worth a question at
           the end, which is what `synced` carries. */
        let synced = match cmd {
            Cmd::Open(folder, land_on) => {
                report(tx, open_row(cfg, tx, tracks, &folder, land_on.as_deref()));
                None
            }
            Cmd::SyncOne => Some(sync_open(cfg, tx, cancel, tracks)),
            Cmd::ResyncAll => Some(resync(cfg, tx, cancel, tracks, library(&cfg.dir))),
            Cmd::Url => start_url(cfg, tx, cancel, tracks, &asker),
            Cmd::Edit(index) => {
                report(tx, edit(cfg, tx, tracks, &asker, &mut held, index));
                None
            }
            Cmd::Undo => {
                report(tx, undo(cfg, tx, tracks, &mut held));
                None
            }
            Cmd::Cover(index) => {
                report(tx, cover(tx, tracks, &asker, cancel, index, cfg.apple));
                None
            }
            Cmd::Search(index) => {
                report(tx, search(cfg, tx, tracks, &asker, &mut held, index));
                None
            }
            Cmd::Retry => {
                report(tx, retry(cfg, tx, tracks, cancel, &asker));
                None
            }
            Cmd::Purge => {
                report(tx, purge(cfg, tx, tracks, &asker));
                None
            }
            Cmd::Swap(targets) => {
                report(tx, swap(cfg, tx, tracks, &mut held, &targets));
                None
            }
            Cmd::Artist(targets) => {
                report(tx, artist(cfg, tx, tracks, &asker, &mut held, &targets));
                None
            }
            Cmd::Format => {
                report(tx, set_format(cfg, tx, &asker));
                None
            }
            Cmd::Rename(folder) => {
                report(tx, rename_shelf(cfg, tx, &asker, tracks, &folder));
                None
            }
            Cmd::Reveal(folder) => {
                match player::reveal(&folder) {
                    Ok(said) => {
                        let _ = tx.send(Msg::Flash(said));
                    }
                    Err(err) => {
                        let _ = tx.send(Msg::Flash(format!("failed: {err}")));
                        let _ = tx.send(Msg::Log(err.to_string()));
                    }
                }
                None
            }
            Cmd::Toggle => {
                report(tx, player::toggle());
                None
            }
            Cmd::Remove(folder) => {
                report(tx, remove_shelf(cfg, tx, &asker, tracks, &folder));
                None
            }
            Cmd::Play(folder) => {
                match player::load(&folder) {
                    Ok(said) => {
                        let _ = tx.send(Msg::Flash(said));
                    }
                    Err(err) => {
                        let _ = tx.send(Msg::Flash(format!("cliamp: {err}")));
                        let _ = tx.send(Msg::Log(format!("cliamp: {err}")));
                    }
                }
                None
            }
        };

        if let Some(result) = synced {
            // Counts and sync times have moved, so the library screen is stale.
            let _ = tx.send(Msg::Library {
                shelves: library(&cfg.dir),
                show: false,
            });
            if !finish(cfg, tx, cancel, tracks, result) {
                return;
            }
        }
        // Last, so the keys stay dead until everything above has landed.
        let _ = tx.send(Msg::Idle);
    }
}

/// How often `serve` re-checks cliamp while it has nothing else to do. A probe
/// is about 11ms, so this is a rounding error against an idle screen.
const PROBE: Duration = Duration::from_secs(3);

fn report(tx: &Sender<Msg>, done: Result<()>) {
    if let Err(err) = done {
        let _ = tx.send(Msg::Flash(format!("failed: {err}")));
        let _ = tx.send(Msg::Log(err.to_string()));
    }
}

/* Opening a folder is not a run: nothing was downloaded and nothing can have
   gone wrong upstream, so it reports itself and asks nothing. The Done is
   still needed, since the track commands stay dead without one. */
fn open_row(
    cfg: &mut Config,
    tx: &Sender<Msg>,
    tracks: &mut Vec<Track>,
    folder: &Path,
    land_on: Option<&str>,
) -> Result<()> {
    // Re-read rather than trusting what the screen was showing: the folder may
    // have gone away between the listing and the keypress.
    let shelves = library(&cfg.dir);
    let shelf = shelves
        .into_iter()
        .find(|s| s.path == folder)
        .context("that playlist is no longer there")?;
    let summary = open_shelf(cfg, tx, tracks, &shelf)?;
    /* Matched on the filename, because that is all a search result knows and
       the index is worked out here: `open_shelf` numbers from the filename
       and numbers departures past the playlist, so the row this lands on is
       not the one the file's own number names. */
    if let Some(wanted) = land_on {
        match tracks
            .iter()
            .find(|t| t.path.as_ref().and_then(|p| p.file_name()).is_some_and(|n| n == wanted))
        {
            Some(track) => {
                let _ = tx.send(Msg::Focus(track.index));
            }
            /* Deleted between the library being read and the keypress, which
               is the same race `open_row` re-reads the library for. */
            None => {
                let _ = tx.send(Msg::Flash(format!("{wanted} is no longer in this folder")));
            }
        }
    }
    let _ = tx.send(Msg::Done {
        result: Ok(summary),
        stage: "opened",
    });
    Ok(())
}

fn start_url(
    cfg: &mut Config,
    tx: &Sender<Msg>,
    cancel: &AtomicBool,
    tracks: &mut Vec<Track>,
    asker: &Asker,
) -> Option<Result<String>> {
    let Some(url) = prompt_url(tx, Escape::Keep) else {
        let _ = tx.send(Msg::Flash("cancelled".into()));
        return None;
    };
    let _ = tx.send(Msg::Restart);
    tracks.clear();
    cfg.url = url;
    /* Load-bearing: a new playlist has no folder of its own yet, and the last
       one's would pull every track of this one into that folder. */
    cfg.folder = None;
    /* The same question the first-start prompt asks above: one menu before
       the download, Esc keeping whatever the session already had. A failed
       save reports rather than aborting the URL that was just typed. */
    report(tx, set_format(cfg, tx, asker));
    let gate = cfg.pick;
    Some(pipeline(cfg, tx, cancel, tracks, gate))
}

/// Syncs the playlist that is open against the URL its manifest recorded,
/// which is the point of opening one offline: look first, download second.
fn sync_open(
    cfg: &mut Config,
    tx: &Sender<Msg>,
    cancel: &AtomicBool,
    tracks: &mut Vec<Track>,
) -> Result<String> {
    if cfg.url.is_empty() {
        bail!("this playlist has no saved URL to sync from  ·  n syncs one by URL");
    }
    let _ = tx.send(Msg::Restart);
    tracks.clear();
    let gate = cfg.pick;
    pipeline(cfg, tx, cancel, tracks, gate)
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

    let theirs = folder_covers(tracks);
    let mut warning = String::new();
    // Nothing to refetch for a format: a retry is about tracks that failed.
    if let Err(err) = ytdlp::run(cfg, tracks, tx, &HashSet::new()) {
        warning = err.to_string();
        let _ = tx.send(Msg::Log(format!("retry: {warning}")));
    }

    let folder = folder_of(tracks);
    let playlist = folder
        .as_ref()
        .and_then(|f| f.file_name().map(|n| n.to_string_lossy().to_string()))
        .unwrap_or_default();
    /* The first pass dropped whatever it wrote, so an image here is one
       somebody chose: `written` leaves it alone, and it is on `theirs`, so
       the end of this pass leaves it alone too. */
    let mut cover = CoverState {
        written: folder.as_deref().is_some_and(has_cover),
    };
    tag_tracks(cfg, tx, cancel, tracks, &playlist, asker, &mut cover, Some(&failed));

    sync_manifest(cfg, tracks);
    let playlist_file = write_playlist(cfg, tracks)?;
    drop_folder_cover(tracks, &theirs);

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
    let _ = tx.send(Msg::Done {
        result: Ok(summary),
        stage: "finished",
    });
    Ok(())
}

/* One level and the values themselves rather than a diff: the edit worth
   taking back is the one just watched landing on the row, and a stack of them
   would be a second history to keep in step with the files on disk. */
pub struct Undo {
    /// How the UI names it on the status bar, so `u` says what it would do.
    what: String,
    /// Track index and every tag the write could have touched, as they were
    /// before it. The whole set, because `write_track` writes the whole set.
    fields: Vec<(usize, tag::Fields)>,
}

/// Tells the UI what `u` would put back, or that there is nothing. A key the
/// bar cannot say is live is a key nobody presses.
fn offer_undo(tx: &Sender<Msg>, undo: &Option<Undo>) {
    let _ = tx.send(Msg::Undoable(undo.as_ref().map(|u| u.what.clone())));
}

/// Puts the last tag write back and then has nothing left to offer: one
/// level means one level, and a `u` that redid the edit would be a toggle
/// wearing the name of an undo.
fn undo(cfg: &Config, tx: &Sender<Msg>, tracks: &mut [Track], held: &mut Option<Undo>) -> Result<()> {
    let Some(Undo { what, fields }) = held.take() else {
        return Ok(());
    };
    offer_undo(tx, held);
    let mut back = 0;
    for (index, was) in fields {
        let Some(pos) = tracks.iter().position(|t| t.index == index) else {
            continue;
        };
        /* "undone" and not the source the track had before: somebody typed
           these values back, which is exactly as much as the tags are worth
           now. The status word is provenance, and restoring `ok` would claim
           a lookup that is no longer what is in the file. */
        match write_track(cfg, tx, tracks, pos, was, "undone") {
            Ok(()) => back += 1,
            Err(err) => {
                let _ = tx.send(Msg::Log(format!("undo: track {index}: {err}")));
            }
        }
    }
    if back > 0 {
        save_manifest(cfg, tracks);
        write_playlist(cfg, tracks)?;
    }
    let _ = tx.send(Msg::Flash(format!("undid {what}")));
    Ok(())
}

/* Album and year reach the UI on their own message, so every path that fills
   them in has to send it: the worker's copy of a track is not the one the
   detail pane draws, and a run that skipped this showed no album until the
   folder was reopened from the library. */
fn offer_meta(tx: &Sender<Msg>, track: &Track) {
    let _ = tx.send(Msg::Meta {
        index: track.index,
        album: track.album.clone(),
        year: track.year,
    });
}

/* What the row currently holds, for a caller changing some of it. Every write
   writes the whole set, so a bulk change to the artist has to carry the album
   and year it is not touching or it would clear them off every track. */
fn held_fields(track: &Track) -> tag::Fields {
    tag::Fields {
        artist: track.artist.clone(),
        title: track.title.clone(),
        album: track.album.clone(),
        year: track.year,
    }
}

/// Writes one track's tags and brings everything that follows from them back
/// into line: the row, the filename, and the name the playlist will use.
fn write_track(
    cfg: &Config,
    tx: &Sender<Msg>,
    tracks: &mut [Track],
    pos: usize,
    fields: tag::Fields,
    source: &str,
) -> Result<()> {
    let path = tracks[pos].path.clone().context("track has no file")?;
    if !path.is_file() {
        bail!(
            "track {} was never downloaded  ·  r retries the failures",
            tracks[pos].index
        );
    }
    tag::set_fields(&path, &fields)?;
    let tag::Fields {
        artist,
        title,
        album,
        year,
    } = fields;

    let departed = tracks[pos].status == Status::Gone;
    /* A departed track stays departed: it is still not in the playlist, and
       calling it `manual` is what the two guards below read, so the next edit
       of the same row would rename and re-list it. */
    let status = if departed { Status::Gone } else { Status::Manual };
    let track = &mut tracks[pos];
    track.artist = artist;
    track.title = title;
    track.album = album;
    track.year = year;
    track.name = label(&track.title, &track.artist);
    track.status = status;
    track.source = source.into();
    track.note.clear();
    if !departed {
        track.listed = true;
    }
    let _ = tx.send(Msg::Update {
        index: track.index,
        status,
        source: Some(source.into()),
        note: Some(String::new()),
        name: Some(track.name.clone()),
    });
    offer_meta(tx, track);
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
    held: &mut Option<Undo>,
    targets: &[usize],
    what: &str,
    mut fields: impl FnMut(&Track) -> tag::Fields,
) -> Result<()> {
    let mut changed = 0;
    let mut failed = 0;
    // Collected as the write succeeds, so a track that could not be written
    // is not one `u` claims to put back.
    let mut before: Vec<(usize, tag::Fields)> = Vec::new();
    for target in targets {
        let Some(pos) = tracks.iter().position(|t| t.index == *target) else {
            continue;
        };
        let next = fields(&tracks[pos]);
        if let Some(missing) = match (next.artist.is_empty(), next.title.is_empty()) {
            (true, true) => Some("artist and title"),
            (true, false) => Some("artist"),
            (false, true) => Some("title"),
            _ => None,
        } {
            failed += 1;
            let _ = tx.send(Msg::Log(format!("{what}: track {target} has no {missing}")));
            continue;
        }
        let was = (tracks[pos].index, held_fields(&tracks[pos]));
        match write_track(cfg, tx, tracks, pos, next, what) {
            Ok(()) => {
                changed += 1;
                before.push(was);
            }
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
        let plural = if changed == 1 { "track" } else { "tracks" };
        *held = Some(Undo {
            what: format!("{what} {changed} {plural}"),
            fields: before,
        });
        offer_undo(tx, held);
    }
    let _ = tx.send(Msg::Flash(match failed {
        0 => format!("{what} {changed} tracks"),
        n => format!("{what} {changed} tracks, {n} could not be written"),
    }));
    Ok(())
}

fn swap(
    cfg: &Config,
    tx: &Sender<Msg>,
    tracks: &mut [Track],
    held: &mut Option<Undo>,
    targets: &[usize],
) -> Result<()> {
    bulk(cfg, tx, tracks, held, targets, "swapped", |track| tag::Fields {
        artist: track.title.clone(),
        title: track.artist.clone(),
        ..held_fields(track)
    })
}

fn artist(
    cfg: &Config,
    tx: &Sender<Msg>,
    tracks: &mut [Track],
    asker: &Asker,
    held: &mut Option<Undo>,
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
        bail!("artist cannot be empty  ·  type a name, or esc leaves them alone");
    }
    bulk(cfg, tx, tracks, held, targets, "set artist on", |track| tag::Fields {
        artist: new.clone(),
        ..held_fields(track)
    })
}

fn edit(
    cfg: &Config,
    tx: &Sender<Msg>,
    tracks: &mut [Track],
    asker: &Asker,
    held: &mut Option<Undo>,
    index: usize,
) -> Result<()> {
    let Some(pos) = tracks.iter().position(|t| t.index == index) else {
        return Ok(());
    };
    /* The video title is what the correction is being made against, so it
       goes in the form rather than being something to remember from the row
       underneath the popup. */
    let was = tracks[pos].was.clone();
    let Some(answers) = asker.form(
        &format!("track {index}"),
        &was,
        vec![
            ("artist".into(), tracks[pos].artist.clone()),
            ("title".into(), tracks[pos].title.clone()),
            ("album".into(), tracks[pos].album.clone()),
            (
                "year".into(),
                tracks[pos].year.map(|y| y.to_string()).unwrap_or_default(),
            ),
        ],
        Escape::Skip,
    ) else {
        return cancelled(tx);
    };
    let [artist, title, album, year] = answers.as_slice() else {
        return cancelled(tx);
    };
    if artist.is_empty() || title.is_empty() {
        bail!("artist and title cannot be empty  ·  esc leaves the track alone");
    }
    /* Album and year may be cleared, so empty is an answer rather than a
       refusal: only a year that is not a year is worth stopping for, since
       writing it would silently drop what was already in the file. */
    let year = match year.is_empty() {
        true => None,
        false => Some(
            year.parse::<u16>()
                .ok()
                // The range the message promises: `12345` parses as a u16 and
                // used to be written while the error said four digits.
                .filter(|y| (1000..=9999).contains(y))
                .context("a year is four digits  ·  esc leaves the track alone")?,
        ),
    };

    let previous = (tracks[pos].index, held_fields(&tracks[pos]));
    let fields = tag::Fields {
        artist: artist.clone(),
        title: title.clone(),
        album: album.clone(),
        year,
    };
    write_track(cfg, tx, tracks, pos, fields, "typed")?;
    save_manifest(cfg, tracks);
    write_playlist(cfg, tracks)?;
    *held = Some(Undo {
        what: format!("the edit of track {index}"),
        fields: vec![previous],
    });
    offer_undo(tx, held);
    let _ = tx.send(Msg::Flash(format!("saved track {index}")));
    Ok(())
}

/// Runs the text search again for one track's current artist and title:
/// Deezer first, Apple fallback. No fingerprint: the audio is unchanged, so
/// only the words are worth re-asking about. A hit is written like a run
/// match, and the row, filename and playlist follow it; a miss touches
/// nothing. No undo: the album and art a hit brings have nowhere in `Undo`
/// to go back to, and `e` is the way back instead.
fn search(
    cfg: &Config,
    tx: &Sender<Msg>,
    tracks: &mut [Track],
    asker: &Asker,
    held: &mut Option<Undo>,
    index: usize,
) -> Result<()> {
    let Some(pos) = tracks.iter().position(|t| t.index == index) else {
        return Ok(());
    };
    let path = tracks[pos].path.clone().context("track has no file")?;
    if !path.is_file() {
        bail!(
            "track {} was never downloaded  ·  r retries the failures",
            tracks[pos].index
        );
    }
    let (artist, title) = (tracks[pos].artist.clone(), tracks[pos].title.clone());
    if artist.is_empty() || title.is_empty() {
        bail!("artist and title cannot be empty  ·  e types them first");
    }
    let _ = tx.send(Msg::Stage(format!("searching for {artist} - {title}")));
    let Some(m) = tag::search(&artist, &title, cfg.apple) else {
        let _ = tx.send(Msg::Flash(format!(
            "no match for {artist} - {title}  ·  e types the tags by hand"
        )));
        return Ok(());
    };
    /* The folder image went with the pass: the hit still gets embedded, and
       no new file is left behind. */
    let mut cover = CoverState { written: true };
    let old = tracks[pos].artist.clone();
    let mut note = m.score.map(|score| format!("score {score:.2}")).unwrap_or_default();
    let (artist, title, why) = tag::apply(&path, &m, cfg, &mut cover, asker, index, &old)?;
    if !why.is_empty() {
        note = why;
    }
    let departed = tracks[pos].status == Status::Gone;
    /* A departed track stays departed, for the same reason an edit keeps it
       so: it is still not in the playlist, and re-listing it would rename it
       to a position it does not hold. */
    let status = if departed { Status::Gone } else { Status::Ok };
    /* `apply` has already written the hit's album into the file, so the row
       reads it back rather than guessing: an edit straight afterwards has to
       prefill what the track actually holds. */
    let written = tag::read(&path).ok();
    let track = &mut tracks[pos];
    track.artist = artist;
    track.title = title;
    track.album = written
        .as_ref()
        .map(|i| i.album.clone())
        .unwrap_or_default();
    track.year = written.as_ref().and_then(|i| i.year);
    track.name = label(&track.title, &track.artist);
    track.status = status;
    track.source = m.source.into();
    track.note = std::mem::take(&mut note);
    track.mbid = m.mbid.clone();
    if !departed {
        track.listed = true;
    }
    let _ = tx.send(Msg::Update {
        index: track.index,
        status,
        source: Some(track.source.clone()),
        note: Some(track.note.clone()),
        name: Some(track.name.clone()),
    });
    offer_meta(tx, track);
    if !departed {
        rename(cfg, tx, &mut tracks[pos]);
    }
    save_manifest(cfg, tracks);
    write_playlist(cfg, tracks)?;
    /* Recording no undo is not the same as leaving the last one armed: `u`
       would put the tags back that this search has just replaced, over a
       match it knows nothing about, while the bar named some earlier edit. */
    *held = None;
    offer_undo(tx, held);
    let _ = tx.send(Msg::Flash(format!(
        "track {index} matched via {}",
        tracks[pos].source
    )));
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
        // Established by the listing this function has just checked, which is
        // the one case a deletion may rest on. See `Track::departure_proven`.
        track.departure_proven = true;
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

/* Which folder images were there before a pass started. Anything on this
   list is somebody else's file: one the user dropped in by hand, or the one
   `c` wrote on an explicit choice. Read before the download, since that is
   the only moment the two can still be told apart. */
fn folder_covers(tracks: &[Track]) -> Vec<&'static str> {
    let Some(folder) = folder_of(tracks) else {
        return Vec::new();
    };
    COVER_NAMES
        .into_iter()
        .filter(|name| folder.join(name).is_file())
        .collect()
}

/* Every track already carries the art embedded, so an image this pass wrote
   is a leftover once it finishes: it duplicates those bytes for file managers
   and nothing else. Only what the pass wrote, though. Deleting one that was
   already there is not recoverable and nothing on screen would say it
   happened, so `keep` is what `folder_covers` saw on the way in. */
fn drop_folder_cover(tracks: &[Track], keep: &[&str]) {
    if let Some(folder) = folder_of(tracks) {
        for name in COVER_NAMES {
            if keep.contains(&name) {
                continue;
            }
            let _ = std::fs::remove_file(folder.join(name));
        }
    }
}

/* Deletes the files of the tracks a complete listing said had left the
   playlist. Three guards, each of which has been needed elsewhere in this
   file: only `Gone` rows whose departure was proven by a listing rather than
   read off the last playlist file, only paths under `--dir`, and a question
   that names the files before anything goes. The manifest then drops them of
   its own accord, since it keeps an entry only while the file is there, and
   the `.m3u8` never listed them. */
fn purge(cfg: &Config, tx: &Sender<Msg>, tracks: &mut Vec<Track>, asker: &Asker) -> Result<()> {
    let departed: Vec<(usize, PathBuf)> = tracks
        .iter()
        .filter(|t| t.status == Status::Gone && t.departure_proven)
        .filter_map(|t| t.path.clone().map(|p| (t.index, p)))
        .filter(|(_, p)| p.is_file())
        .collect();
    if departed.is_empty() {
        /* Either nothing has left, or what has was seen offline. The second
           is the one worth explaining: the row says `gone` and the key does
           nothing, so the message has to say what would make it work. */
        let offline = tracks.iter().any(|t| t.status == Status::Gone);
        if offline {
            bail!("these departures were read off the last playlist file  ·  S syncs and confirms them");
        }
        bail!("nothing here has left the playlist");
    }
    // The paths came from a manifest somebody could have edited by hand.
    if let Some((_, outside)) = departed.iter().find(|(_, p)| !p.starts_with(&cfg.dir)) {
        bail!("{} is outside {}  ·  nothing deleted", outside.display(), cfg.dir.display());
    }

    let names: Vec<String> = departed
        .iter()
        .map(|(_, p)| p.file_name().unwrap_or_default().to_string_lossy().to_string())
        .collect();
    let count = names.len();
    let files = if count == 1 { "file" } else { "files" };
    /* The header is a `Block::title` and clips, so the names go in the note,
       which wraps. Every one of them: a question about deleting music that
       does not say which music is not a question anybody can answer. */
    let note = format!(
        "{}  ·  the playlist no longer has them, and this cannot be undone",
        names.join(", ")
    );
    let Some(choice) = asker.choose_noted(
        &format!("Delete {count} departed {files}?"),
        &note,
        vec!["keep them".into(), format!("delete {count} {files}")],
    ) else {
        return cancelled(tx);
    };
    if choice == 0 {
        let _ = tx.send(Msg::Flash(format!("keeping {count} departed {files}")));
        return Ok(());
    }

    let mut dropped: Vec<usize> = Vec::new();
    for (index, path) in &departed {
        match std::fs::remove_file(path) {
            Ok(()) => dropped.push(*index),
            // Already gone is gone, and the row should go with it.
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => dropped.push(*index),
            Err(e) => {
                let _ = tx.send(Msg::Log(format!("purge: {}: {e}", path.display())));
            }
        }
    }
    tracks.retain(|t| !dropped.contains(&t.index));
    let _ = tx.send(Msg::Dropped(dropped.clone()));
    /* Keyed on the file being there, so the deleted entries fall out of the
       sidecar here rather than needing to be named. Written before the
       flash: a sidecar still naming a deleted file would re-list it as
       missing on the library screen. */
    save_manifest(cfg, tracks);

    let done = dropped.len();
    if done < count {
        let _ = tx.send(Msg::Flash(format!(
            "deleted {done} of {count}  ·  l has what refused"
        )));
    } else {
        let _ = tx.send(Msg::Flash(format!("deleted {done} departed {files}")));
    }
    Ok(())
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
    apple: bool,
) -> Result<()> {
    let Some(pos) = tracks.iter().position(|t| t.index == index) else {
        return Ok(());
    };
    let folder = folder_of(tracks).context("no folder to write a cover into")?;

    let _ = tx.send(Msg::Stage("finding artwork".into()));
    let found = lookup::artwork(
        &tracks[pos].artist,
        &tracks[pos].title,
        tracks[pos].mbid.as_deref(),
        apple,
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
            let _ = tx.send(Msg::Stage("fetching artwork".into()));
            lookup::fetch(url).context("could not download that cover")?
        }
        Pick::Keep => return cancelled(tx),
        Pick::OwnFile => {
            let Some(typed) = asker.input("Image file", "") else {
                return cancelled(tx);
            };
            if typed.is_empty() {
                bail!("no file given  ·  type a path to an image, or esc keeps the cover");
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
    write_manifest(cfg, tracks, false);
}

/// For a pass that actually met the playlist, so the library can say how long
/// ago that was. An edit uses `save_manifest` and leaves the time alone.
fn sync_manifest(cfg: &Config, tracks: &[Track]) {
    write_manifest(cfg, tracks, true);
}

fn write_manifest(cfg: &Config, tracks: &[Track], synced: bool) {
    let Some(folder) = folder_of(tracks) else {
        return;
    };
    /* Keyed on the file, not on `listed`: dropping a departed track here
       forgets it, and a video that comes back to the playlist would download
       again beside the copy already on disk. */
    let known: Vec<(String, PathBuf)> = tracks
        .iter()
        .filter(|t| t.path.as_deref().is_some_and(Path::is_file))
        .filter_map(|t| t.path.clone().map(|p| (t.id.clone(), p)))
        .collect();

    /* Whatever the current tracks do not account for but that is still on
       disk. A pass over part of a playlist would otherwise rewrite the whole
       manifest from that part: `--playlist-items 4`, a retry, or a folder
       opened from the library, each of which forgets files it never looked
       at and downloads them again beside the copies already there. Entries
       whose file has gone are the one thing dropped, so a folder that churns
       does not collect dead lines for good. */
    let mine: HashSet<&str> = known.iter().map(|(id, _)| id.as_str()).collect();
    let kept: Vec<(String, PathBuf)> = manifest::entries(&folder)
        .into_iter()
        .filter(|(id, file)| !mine.contains(id.as_str()) && file.is_file())
        .collect();

    let entries = kept.into_iter().chain(known);
    let _ = if synced {
        manifest::write_synced(&folder, &cfg.url, entries)
    } else {
        manifest::write(&folder, &cfg.url, entries)
    };
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
    /* The count of tracks is the count that got a file, so without this a run
       that fetched ten of two hundred reads exactly like a complete one. */
    let skipped = tracks.iter().filter(|t| t.status == Status::Skipped).count();
    if skipped > 0 {
        summary = format!("{summary}, {skipped} not downloaded");
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
        let path = track.path.as_deref().unwrap_or(Path::new(""));
        /* Relative to the playlist file, which sits in this same folder, so
           the folder plays wherever it is copied to. An absolute path is only
           valid on the machine that wrote it, and every entry breaks the
           moment the folder reaches a phone or another user's home. */
        let entry = path.strip_prefix(folder).unwrap_or(path);
        body.push_str(&format!(
            "#EXTINF:{},{}\n{}\n",
            track.duration,
            label(&track.title, &track.artist),
            /* Decomposed, because copying the folder to an iPhone decomposes
               the filenames and iOS compares bytes, where APFS does not: a
               composed entry plays here and names nothing there. Verified in
               VLC on iOS with one track listed four ways. The cost is a
               byte-exact filesystem, where this is now the wrong form. */
            decomposed(&entry.to_string_lossy())
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
pub mod tests {

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
    /* The rename is the easy half: what makes it stick is the header, which
       is what stops the next sync writing the playlist back under the name
       YouTube gives it, beside the folder that was renamed. */
    #[test]
    fn renaming_a_playlist_moves_the_folder_and_records_the_name() {
        let root = scratch("rename");
        let from = root.join("Chill Evenings");
        std::fs::create_dir_all(&from).unwrap();
        std::fs::write(from.join("01 - A.opus"), "audio").unwrap();
        std::fs::write(
            from.join("Chill Evenings.m3u8"),
            "#EXTM3U\n#EXTINF:1,A\n01 - A.opus\n",
        )
        .unwrap();
        manifest::write_synced(
            &from,
            "https://example.com",
            [("id1".to_string(), from.join("01 - A.opus"))].into_iter(),
        )
        .unwrap();

        let mut cfg = config(true);
        cfg.dir = root.clone();
        let (tx, rx) = channel();
        let asker = Asker {
            tx: tx.clone(),
            enabled: true,
        };
        let tracks: Vec<Track> = Vec::new();

        std::thread::scope(|scope| {
            let worker = scope.spawn(|| rename_shelf(&mut cfg, &tx, &asker, &tracks, &from));
            while let Ok(msg) = rx.recv_timeout(Duration::from_secs(5)) {
                if let Msg::Ask(crate::app::Prompt::Input { value, .. }, reply) = msg {
                    // Prefilled with the name it has, which is what an edit is.
                    assert_eq!(value, "Chill Evenings");
                    reply.send(Reply::Text("Evenings".into())).unwrap();
                    break;
                }
            }
            worker.join().unwrap().unwrap();
        });

        let to = root.join("Evenings");
        assert!(to.is_dir(), "the folder did not move");
        assert!(!from.exists(), "the old folder is still there");
        assert!(to.join("01 - A.opus").is_file(), "the tracks went missing");

        /* Named after its folder, and `last_playlist` finds it that way; its
           entries are relative, so the move itself left them valid. */
        assert!(to.join("Evenings.m3u8").is_file(), "the .m3u8 kept the old name");
        let listed = std::fs::read_to_string(to.join("Evenings.m3u8")).unwrap();
        assert!(listed.contains("01 - A.opus"), "{listed}");

        let after = manifest::load(&to);
        assert_eq!(after.name.as_deref(), Some("Evenings"));
        assert_eq!(after.url.as_deref(), Some("https://example.com"), "the URL went");
        assert_eq!(after.entries.len(), 1, "the entries went");

        // And the library the UI is about to draw is the renamed one.
        let shelves: Vec<Shelf> = rx
            .try_iter()
            .filter_map(|m| match m {
                Msg::Library { shelves, .. } => Some(shelves),
                _ => None,
            })
            .last()
            .expect("the library was never refreshed");
        assert_eq!(shelves.iter().map(|s| s.name.clone()).collect::<Vec<_>>(), ["Evenings"]);
        std::fs::remove_dir_all(&root).unwrap();
    }

    /* A folder renamed in a file manager is already off its upstream title
       and nothing records that, so the next sync fetches the playlist again
       under the old name. Confirming the name it has is how it gets pinned,
       and it is the one case where the answer is not a change. */
    #[test]
    fn confirming_the_name_a_folder_has_pins_it_without_moving_anything() {
        let root = scratch("pinsame");
        let folder = root.join("Evenings");
        std::fs::create_dir_all(&folder).unwrap();
        let track = folder.join("01 - A.opus");
        std::fs::write(&track, "audio").unwrap();
        manifest::write(
            &folder,
            "https://example.com",
            [("id1".to_string(), track.clone())].into_iter(),
        )
        .unwrap();

        let answer = |folder: &Path| {
            let mut cfg = config(true);
            cfg.dir = root.clone();
            let (tx, rx) = channel();
            let asker = Asker {
                tx: tx.clone(),
                enabled: true,
            };
            let tracks: Vec<Track> = Vec::new();
            std::thread::scope(|scope| {
                let worker = scope.spawn(|| rename_shelf(&mut cfg, &tx, &asker, &tracks, folder));
                while let Ok(msg) = rx.recv_timeout(Duration::from_secs(5)) {
                    if let Msg::Ask(_, reply) = msg {
                        reply.send(Reply::Text("Evenings".into())).unwrap();
                        break;
                    }
                }
                worker.join().unwrap().unwrap();
                rx.try_iter()
                    .filter_map(|m| match m {
                        Msg::Flash(said) => Some(said),
                        _ => None,
                    })
                    .last()
                    .unwrap_or_default()
            })
        };

        let said = answer(&folder);
        assert_eq!(manifest::load(&folder).name.as_deref(), Some("Evenings"));
        assert!(said.contains("keeping the name Evenings"), "{said}");
        // Nothing moved, and nothing else in the sidecar was lost.
        assert!(folder.is_dir() && track.is_file());
        assert_eq!(manifest::load(&folder).url.as_deref(), Some("https://example.com"));
        assert_eq!(manifest::load(&folder).entries.len(), 1);

        /* Already pinned, so there is nothing to do; it still says so, since
           a key that goes quiet reads as a key that is broken. */
        let again = answer(&folder);
        assert!(again.contains("already keeping Evenings"), "{again}");
        std::fs::remove_dir_all(&root).unwrap();
    }

    /* The header is only worth writing if something reads it. This is the
       line that does: without it `S` on a renamed folder downloads the
       playlist back under the name YouTube gives it. */
    #[test]
    fn opening_a_renamed_folder_pins_the_sync_to_it() {
        let root = scratch("pin");
        let folder = root.join("Evenings");
        std::fs::create_dir_all(&folder).unwrap();
        let track = folder.join("01 - A.opus");
        std::fs::write(&track, "audio").unwrap();
        manifest::write(&folder, "https://example.com", [("id1".to_string(), track.clone())].into_iter())
            .unwrap();
        let shelf = Shelf {
            path: folder.clone(),
            name: "Evenings".into(),
            url: "https://example.com".into(),
            tracks: 1,
            missing: 0,
            synced: None,
            files: Vec::new(),
        };

        // Nobody has renamed it, so the folder is still the playlist's own
        // name and a stale pin from an earlier folder has to be cleared.
        let mut cfg = config(true);
        cfg.dir = root.clone();
        cfg.folder = Some("Something Else".into());
        let mut tracks = Vec::new();
        open_shelf(&mut cfg, &channel().0, &mut tracks, &shelf).unwrap();
        assert_eq!(cfg.folder, None, "a folder kept the last one's name");

        manifest::set_name(&folder, "Evenings").unwrap();
        let mut tracks = Vec::new();
        open_shelf(&mut cfg, &channel().0, &mut tracks, &shelf).unwrap();
        assert_eq!(cfg.folder.as_deref(), Some("Evenings"));
        std::fs::remove_dir_all(&root).unwrap();
    }

    /* A name that is a path is a move, and one starting with a dot is a
       folder nobody will see again. Neither is what renaming means. */
    #[test]
    fn a_rename_refuses_a_name_that_is_not_one() {
        let root = scratch("renamebad");
        let from = root.join("Focus");
        std::fs::create_dir_all(&from).unwrap();
        std::fs::create_dir_all(root.join("Taken")).unwrap();

        let tried = |answer: &str| {
            let mut cfg = config(true);
            cfg.dir = root.clone();
            let (tx, rx) = channel();
            let asker = Asker {
                tx: tx.clone(),
                enabled: true,
            };
            let tracks: Vec<Track> = Vec::new();
            std::thread::scope(|scope| {
                let worker = scope.spawn(|| rename_shelf(&mut cfg, &tx, &asker, &tracks, &from));
                while let Ok(msg) = rx.recv_timeout(Duration::from_secs(5)) {
                    if let Msg::Ask(_, reply) = msg {
                        reply.send(Reply::Text(answer.into())).unwrap();
                        break;
                    }
                }
                worker.join().unwrap()
            })
        };

        for bad in ["../elsewhere", ".hidden"] {
            let err = tried(bad).unwrap_err().to_string();
            assert!(err.contains("slash or start with a dot"), "{bad}: {err}");
        }
        let clash = tried("Taken").unwrap_err().to_string();
        assert!(clash.contains("already a folder here"), "{clash}");
        assert!(from.is_dir(), "a refused rename moved the folder anyway");

        // The same name is not an error, it is nothing to do.
        assert!(tried("Focus").is_ok());
        assert!(from.is_dir());
        std::fs::remove_dir_all(&root).unwrap();
    }

    /* A message that says only what went wrong leaves the reader to work out
       what to do, and every one of these is reachable by pressing a key. */
    #[test]
    fn the_errors_a_user_can_reach_end_with_the_next_step() {
        let dir = scratch("fixes");
        let mut cfg = config(true);
        cfg.url.clear();
        let mut tracks = Vec::new();

        let no_url = sync_open(&mut cfg, &channel().0, &AtomicBool::new(false), &mut tracks)
            .unwrap_err()
            .to_string();
        assert!(no_url.contains("n syncs one by URL"), "{no_url}");

        // A folder of ours with nothing recorded in its sidecar.
        std::fs::write(dir.join(".earworm"), "#url\thttps://example.com\n").unwrap();
        let shelf = Shelf {
            path: dir.clone(),
            name: "Focus".into(),
            url: "u".into(),
            tracks: 0,
            missing: 0,
            synced: None,
            files: Vec::new(),
        };
        let empty = open_shelf(&mut config(true), &channel().0, &mut tracks, &shelf)
            .unwrap_err()
            .to_string();
        assert!(empty.contains("sync it to fill one in"), "{empty}");

        // Not downloaded, which is what `r` is for.
        let mut one = vec![Track::new(
            1,
            "id".into(),
            "01 - A.opus".into(),
            dir.join("01 - A.opus"),
        )];
        let missing = write_track(
            &config(true),
            &channel().0,
            &mut one,
            0,
            named("A", "B"),
            "typed",
        )
        .unwrap_err()
        .to_string();
        assert!(missing.contains("r retries the failures"), "{missing}");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /* "Next time as flac" is a thought people have at the end of a run, on a
       screen where `f` means follow. Only with a file to write it to: without
       one the choice lasts until the tool closes, which is not "next time". */
    #[test]
    fn the_finish_menu_offers_the_format_only_with_a_config_to_remember_it() {
        let rows = |config_file: Option<PathBuf>| {
            let (tx, rx) = mpsc::channel();
            let cancel = AtomicBool::new(false);
            let mut cfg = config(true);
            cfg.config_file = config_file;
            let mut tracks: Vec<Track> = Vec::new();
            std::thread::scope(|scope| {
                let worker =
                    scope.spawn(|| finish(&mut cfg, &tx, &cancel, &mut tracks, Ok("done".into())));
                let mut asked = Vec::new();
                while let Ok(msg) = rx.recv_timeout(Duration::from_secs(5)) {
                    if let Msg::Ask(crate::app::Prompt::Choice { options, .. }, reply) = msg {
                        asked = options;
                        // Esc: keeps the menu open and ends the loop.
                        reply.send(Reply::Cancel).unwrap();
                        break;
                    }
                }
                assert!(worker.join().unwrap());
                asked
            })
        };

        let with = rows(Some(PathBuf::from("/tmp/earworm-test.toml")));
        assert!(
            with.iter().any(|r| r.starts_with("format for next time")),
            "{with:?}"
        );
        // It says which format, or "next time" is a choice made blind.
        assert!(with.iter().any(|r| r.contains("(opus)")), "{with:?}");

        let without = rows(None);
        assert!(
            !without.iter().any(|r| r.contains("format")),
            "offered to remember a choice with nowhere to write it: {without:?}"
        );
    }

    /* The setting and the file have to agree. A failed save used to leave the
       session on the new format while reporting that nothing was remembered,
       so the next download disagreed with both the message and the config. */
    #[test]
    fn a_format_that_could_not_be_saved_is_not_adopted_for_the_run() {
        let dir = scratch("setformat");
        let path = dir.join("config.toml");
        // Already broken, so the save refuses before it writes anything.
        std::fs::write(&path, "nonsense\n").unwrap();

        let (tx, rx) = channel();
        let mut cfg = config(true);
        cfg.config_file = Some(path.clone());
        assert_eq!(cfg.format, "opus");

        let asker = Asker {
            tx: tx.clone(),
            enabled: true,
        };
        let failed = std::thread::scope(|scope| {
            let worker = scope.spawn(|| set_format(&mut cfg, &tx, &asker));
            while let Ok(msg) = rx.recv_timeout(Duration::from_secs(5)) {
                if let Msg::Ask(_, reply) = msg {
                    // flac, which is not what the run started on.
                    let flac = config::FORMATS.iter().position(|(n, _, _)| *n == "flac").unwrap();
                    reply.send(Reply::Choice(flac)).unwrap();
                    break;
                }
            }
            worker.join().unwrap()
        });

        let err = failed.unwrap_err().to_string();
        assert!(err.contains("does not parse"), "{err}");
        assert_eq!(cfg.format, "opus", "the run took a format the file never got");
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "nonsense\n");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn the_finish_menu_leads_with_the_guesses_the_run_left() {
        let (tx, rx) = mpsc::channel();
        let cancel = AtomicBool::new(false);
        let mut cfg = config(true);
        let mut tracks: Vec<Track> = [Status::Ok, Status::Kept, Status::Weak]
            .into_iter()
            .enumerate()
            .map(|(n, status)| {
                let mut track =
                    Track::new(n + 1, "id".into(), "name".into(), PathBuf::from("/tmp/x.opus"));
                track.status = status;
                track
            })
            .collect();

        let (asked, reviewed) = std::thread::scope(|scope| {
            let worker = scope.spawn(|| finish(&mut cfg, &tx, &cancel, &mut tracks, Ok("done".into())));
            let mut asked = Vec::new();
            let mut reviewed = false;
            // Bounded: a menu that stopped offering the row would otherwise
            // leave this waiting for a message nothing is going to send.
            while let Ok(msg) = rx.recv_timeout(Duration::from_secs(5)) {
                match msg {
                    Msg::Ask(crate::app::Prompt::Choice { options, .. }, reply) => {
                        asked = options;
                        // The first row, which is where the review has to be.
                        reply.send(Reply::Choice(0)).unwrap();
                    }
                    Msg::Review => reviewed = true,
                    _ => {}
                }
                if reviewed {
                    break;
                }
            }
            assert!(worker.join().unwrap(), "reviewing ended the session");
            (asked, reviewed)
        });

        assert!(reviewed, "the first row did something else");
        assert_eq!(asked.first().map(String::as_str), Some("review 2 tracks worth a look"), "{asked:?}");
    }

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
                    Msg::Done { result, .. } => dones.push(result),
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
    fn the_library_finds_the_folders_that_know_their_url() {
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

        let found = library(&root);
        let names: Vec<&str> = found.iter().map(|s| s.name.as_str()).collect();
        assert_eq!(names, ["Alpha", "Beta"], "wrong folders or wrong order");
        assert_eq!(found[0].url, "u-alpha");
        std::fs::remove_dir_all(&root).unwrap();
    }

    /* The preview pane draws from these and nothing else, so a `library` that
       stopped filling them would leave it blank with every UI test still
       green. Playlist order matters too: the pane shows the first screenful. */
    #[test]
    fn the_library_records_each_folders_files_and_which_are_gone() {
        let root = scratch("shelf-files");
        let folder = root.join("Focus");
        std::fs::create_dir_all(&folder).unwrap();
        let here = folder.join("01 - A.opus");
        let gone = folder.join("02 - B.opus");
        std::fs::write(&here, "audio").unwrap();
        manifest::write(
            &folder,
            "u",
            [("a".to_string(), here), ("b".to_string(), gone)].into_iter(),
        )
        .unwrap();

        let shelf = library(&root).remove(0);
        assert_eq!(
            shelf.files,
            [("01 - A.opus".to_string(), true), ("02 - B.opus".to_string(), false)],
            "the pane has nothing to draw"
        );
        // The same stat pass feeds the row's counts, so they must agree.
        assert_eq!((shelf.tracks, shelf.missing), (1, 1));
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn the_library_is_empty_for_a_directory_that_is_not_there() {
        assert!(library(Path::new("/nonexistent-earworm-library")).is_empty());
    }

    /* `D` on the library, answered "forget": the playlist goes but the audio
       stays, so the folder is left on disk as plain files. */
    #[test]
    fn forgetting_a_playlist_keeps_the_audio_but_drops_the_playlist() {
        let root = scratch("remove-forget");
        let folder = root.join("Focus");
        std::fs::create_dir_all(&folder).unwrap();
        let audio = folder.join("01 - A.opus");
        std::fs::write(&audio, "audio").unwrap();
        manifest::write(&folder, "u", [("a".to_string(), audio.clone())].into_iter()).unwrap();
        let playlist = folder.join("Focus.m3u8");
        std::fs::write(&playlist, "#EXTM3U\n").unwrap();
        let mut cfg = config(true);
        cfg.dir = root.clone();

        let (tx, rx) = mpsc::channel();
        let (result, answered, flashes) = std::thread::scope(|scope| {
            let worker = scope.spawn(|| {
                let asker = Asker { tx: tx.clone(), enabled: true };
                let mut tracks = Vec::new();
                remove_shelf(&mut cfg, &tx, &asker, &mut tracks, &folder)
            });
            let mut answered = false;
            let mut flashes = Vec::new();
            // Bounded: a menu that stopped asking would otherwise leave this
            // waiting for a message nothing is going to send.
            while let Ok(msg) = rx.recv_timeout(Duration::from_secs(5)) {
                match msg {
                    Msg::Ask(_, reply) => {
                        reply.send(Reply::Choice(1)).unwrap();
                        answered = true;
                    }
                    Msg::Flash(line) => flashes.push(line),
                    _ => {}
                }
                if answered {
                    break;
                }
            }
            let result = worker.join().unwrap();
            while let Ok(msg) = rx.try_recv() {
                if let Msg::Flash(line) = msg {
                    flashes.push(line);
                }
            }
            (result, answered, flashes)
        });

        assert!(answered, "no menu was asked");
        result.unwrap();
        assert!(!manifest::path(&folder).exists(), "the sidecar survived");
        assert!(!playlist.exists(), "the .m3u8 survived");
        assert!(audio.is_file(), "the audio went with the playlist");
        assert!(
            flashes.iter().any(|f| f.contains("audio kept")),
            "no word about what stayed: {flashes:?}"
        );
        std::fs::remove_dir_all(&root).unwrap();
    }

    /* Answered "delete": the whole folder goes, audio with it. */
    #[test]
    fn deleting_a_playlist_takes_the_folder_with_it() {
        let root = scratch("remove-delete");
        let folder = root.join("Focus");
        std::fs::create_dir_all(&folder).unwrap();
        std::fs::write(folder.join("01 - A.opus"), "audio").unwrap();
        manifest::write(&folder, "u", std::iter::empty()).unwrap();
        let mut cfg = config(true);
        cfg.dir = root.clone();

        let (tx, rx) = mpsc::channel();
        let answered = std::thread::scope(|scope| {
            let worker = scope.spawn(|| {
                let asker = Asker { tx: tx.clone(), enabled: true };
                let mut tracks = Vec::new();
                remove_shelf(&mut cfg, &tx, &asker, &mut tracks, &folder)
            });
            let mut answered = false;
            while let Ok(msg) = rx.recv_timeout(Duration::from_secs(5)) {
                if let Msg::Ask(_, reply) = msg {
                    reply.send(Reply::Choice(2)).unwrap();
                    answered = true;
                    break;
                }
            }
            worker.join().unwrap().unwrap();
            answered
        });

        assert!(answered, "no menu was asked");
        assert!(!folder.exists(), "the folder survived its own deletion");
        std::fs::remove_dir_all(&root).unwrap();
    }

    /* The first row is the safe one: keeping changes nothing on disk. */
    #[test]
    fn keeping_a_playlist_changes_nothing() {
        let root = scratch("remove-keep");
        let folder = root.join("Focus");
        std::fs::create_dir_all(&folder).unwrap();
        manifest::write(&folder, "u", std::iter::empty()).unwrap();
        let mut cfg = config(true);
        cfg.dir = root.clone();

        let (tx, rx) = mpsc::channel();
        let flashes = std::thread::scope(|scope| {
            let worker = scope.spawn(|| {
                let asker = Asker { tx: tx.clone(), enabled: true };
                let mut tracks = Vec::new();
                remove_shelf(&mut cfg, &tx, &asker, &mut tracks, &folder)
            });
            let mut flashes = Vec::new();
            while let Ok(msg) = rx.recv_timeout(Duration::from_secs(5)) {
                match msg {
                    Msg::Ask(_, reply) => {
                        reply.send(Reply::Choice(0)).unwrap();
                        break;
                    }
                    Msg::Flash(line) => flashes.push(line),
                    _ => {}
                }
            }
            worker.join().unwrap().unwrap();
            while let Ok(msg) = rx.try_recv() {
                if let Msg::Flash(line) = msg {
                    flashes.push(line);
                }
            }
            flashes
        });

        assert!(manifest::path(&folder).is_file(), "keeping removed the sidecar");
        assert!(flashes.iter().any(|f| f.contains("keeping")), "{flashes:?}");
        std::fs::remove_dir_all(&root).unwrap();
    }

    /* Removing the folder the track list is showing clears the run and lands
       back on the library, or every key on screen would act on deleted files. */
    #[test]
    fn removing_the_open_folder_returns_to_the_library() {
        let root = scratch("remove-open");
        let folder = root.join("Focus");
        std::fs::create_dir_all(&folder).unwrap();
        manifest::write(&folder, "u", std::iter::empty()).unwrap();
        let mut cfg = config(true);
        cfg.dir = root.clone();

        let (tx, rx) = mpsc::channel();
        let (tracks_left, restarted, back, empty) = std::thread::scope(|scope| {
            let worker = scope.spawn(|| {
                let asker = Asker { tx: tx.clone(), enabled: true };
                let mut tracks = vec![track_at(&folder, "01 - A.opus", Status::Have)];
                remove_shelf(&mut cfg, &tx, &asker, &mut tracks, &folder)?;
                Ok::<_, anyhow::Error>(tracks.len())
            });
            let (mut restarted, mut back, mut empty) = (false, false, false);
            while let Ok(msg) = rx.recv_timeout(Duration::from_secs(5)) {
                match msg {
                    Msg::Ask(_, reply) => {
                        reply.send(Reply::Choice(1)).unwrap();
                    }
                    Msg::Restart => restarted = true,
                    Msg::Library { shelves, show } => {
                        back = show;
                        empty = shelves.is_empty();
                    }
                    _ => {}
                }
                if restarted && back {
                    break;
                }
            }
            let tracks_left = worker.join().unwrap().unwrap();
            (tracks_left, restarted, back, empty)
        });

        assert_eq!(tracks_left, 0, "the open run kept its tracks");
        assert!(restarted, "the run state was never cleared");
        assert!(back, "the screen stayed on the deleted folder");
        assert!(empty, "the library still lists what was removed");
        std::fs::remove_dir_all(&root).unwrap();
    }

    /* The folder arrives from the UI rather than from the scan, and what this
       deletes does not come back: anything outside `--dir` is refused before
       any question is asked. */
    #[test]
    fn removing_anything_outside_the_library_is_refused() {
        let root = scratch("remove-guard");
        let mut cfg = config(true);
        cfg.dir = root.clone();
        let (tx, _rx) = mpsc::channel();
        let asker = Asker { tx: tx.clone(), enabled: false };
        let mut tracks = Vec::new();

        let err = remove_shelf(&mut cfg, &tx, &asker, &mut tracks, Path::new("/tmp/elsewhere"))
            .unwrap_err();
        assert!(err.to_string().contains("outside"), "{err:?}");
        let err = remove_shelf(&mut cfg, &tx, &asker, &mut tracks, &root).unwrap_err();
        assert!(err.to_string().contains("outside"), "{err:?}");
        std::fs::remove_dir_all(&root).unwrap();
    }

    /* Typing the first URL also offers the format: it is the only question
       before the first download, and the run adopts whatever it leaves. */
    #[test]
    fn the_first_url_prompt_also_offers_the_format() {
        let root = scratch("first-format");
        let mut cfg = config(true);
        cfg.dir = root.clone();
        cfg.config_file = None;
        // The index of "flac" in FORMATS, which is what the menu lists.
        let flac = config::FORMATS
            .iter()
            .position(|(name, _, _)| *name == "flac")
            .unwrap();

        let (tx, rx) = mpsc::channel();
        let (start, asked) = std::thread::scope(|scope| {
            let worker = scope.spawn(|| ask_start(&mut cfg, &tx));
            let mut asked = 0;
            // Bounded: a prompt that stopped asking would otherwise leave this
            // waiting for a message nothing is going to send.
            while let Ok(msg) = rx.recv_timeout(Duration::from_secs(5)) {
                let Msg::Ask(_, reply) = msg else { continue };
                asked += 1;
                if asked == 1 {
                    reply
                        .send(Reply::Text("https://www.youtube.com/playlist?list=PL1".into()))
                        .unwrap();
                } else {
                    reply.send(Reply::Choice(flac)).unwrap();
                    break;
                }
            }
            (worker.join().unwrap(), asked)
        });

        assert_eq!(asked, 2, "the format was never offered");
        assert!(matches!(start, Start::Playlist), "no playlist after two answers");
        assert_eq!(cfg.format, "flac");
        std::fs::remove_dir_all(&root).unwrap();
    }

    /* Backing out of the format menu keeps the session default: Esc is how a
       URL add stays a single question. */
    #[test]
    fn backing_out_of_the_first_format_menu_keeps_the_default() {
        let root = scratch("first-format-keep");
        let mut cfg = config(true);
        cfg.dir = root.clone();
        cfg.config_file = None;

        let (tx, rx) = mpsc::channel();
        let start = std::thread::scope(|scope| {
            let worker = scope.spawn(|| ask_start(&mut cfg, &tx));
            let mut asked = 0;
            while let Ok(msg) = rx.recv_timeout(Duration::from_secs(5)) {
                let Msg::Ask(_, reply) = msg else { continue };
                asked += 1;
                if asked == 1 {
                    reply
                        .send(Reply::Text("https://www.youtube.com/playlist?list=PL1".into()))
                        .unwrap();
                } else {
                    reply.send(Reply::Cancel).unwrap();
                    break;
                }
            }
            assert_eq!(asked, 2, "the format was never offered");
            worker.join().unwrap()
        });

        assert!(matches!(start, Start::Playlist), "cancelling the menu lost the URL");
        assert_eq!(cfg.format, config::DEFAULT_FORMAT);
        std::fs::remove_dir_all(&root).unwrap();
    }

    /* A URL from the command line arrives fully specified, so the first start
       asks nothing at all. */
    #[test]
    fn a_url_from_the_command_line_skips_the_first_questions() {
        let mut cfg = config(true);
        cfg.url = "https://www.youtube.com/playlist?list=PL1".into();
        let (tx, rx) = mpsc::channel();
        assert!(matches!(ask_start(&mut cfg, &tx), Start::Playlist));
        assert!(rx.try_recv().is_err(), "a passed URL was asked about");
        assert_eq!(cfg.format, config::DEFAULT_FORMAT);
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

    /* The headline count is tracks that got a file, so a run told to fetch one
       of three otherwise reads exactly like a complete one. */
    #[test]
    fn the_summary_says_how_many_were_never_downloaded() {
        let dir = scratch("skipped-summary");
        let mut tracks = playlist_of(&dir, &[("a", "01 - A.opus")]);
        tracks[0].listed = true;
        for (n, id) in [(2, "b"), (3, "c")] {
            let mut track = Track::new(n, id.into(), format!("{n:02} - X.opus"), dir.join("x"));
            track.status = Status::Skipped;
            tracks.push(track);
        }

        let line = summarise(&tracks, None);
        assert!(line.contains("1 tracks in"), "{line}");
        assert!(line.contains("2 not downloaded"), "{line}");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /* Both of these have a path and no file behind it, and the whole reason
       the status exists is that the pass must tell them apart: one is a
       download that went wrong, the other a download nobody asked for. */
    #[test]
    fn tagging_calls_an_absent_file_failed_but_leaves_a_skipped_one_alone() {
        let dir = scratch("skipped-tagging");
        let (tx, _rx) = channel();
        let mut tracks = vec![
            Track::new(1, "a".into(), "01 - A.opus".into(), dir.join("01 - A.opus")),
            Track::new(2, "b".into(), "02 - B.opus".into(), dir.join("02 - B.opus")),
        ];
        tracks[1].status = Status::Skipped;

        let asker = Asker { tx: tx.clone(), enabled: false };
        let mut cover = CoverState { written: true };
        let cancel = AtomicBool::new(false);
        tag_tracks(
            &config(false),
            &tx,
            &cancel,
            &mut tracks,
            "Mix",
            &asker,
            &mut cover,
            None,
        );

        assert_eq!(tracks[0].status, Status::Failed, "an absent file is a failure");
        assert_eq!(tracks[1].status, Status::Skipped, "a skipped track was re-judged");
        assert!(!tracks[1].listed, "a skipped track reached the .m3u8");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    fn channel() -> (Sender<Msg>, mpsc::Receiver<Msg>) {
        mpsc::channel()
    }

    /* The pick decides what the download fetches, and everything it leaves out
       has to end up `Skipped`: `Pending` would reach the tagging pass, find no
       file and be reported as a failure the user caused on purpose. */
    #[test]
    fn what_the_pick_leaves_out_becomes_skipped() {
        let (tx, rx) = channel();
        let mut tracks = vec![
            Track::new(1, "a".into(), "A".into(), PathBuf::from("/tmp/a")),
            Track::new(2, "b".into(), "B".into(), PathBuf::from("/tmp/b")),
            Track::new(3, "c".into(), "C".into(), PathBuf::from("/tmp/c")),
            Track::new(4, "d".into(), "D".into(), PathBuf::from("/tmp/d")),
        ];
        tracks[2].status = Status::Have;
        tracks[3].status = Status::Gone;

        // Answer before asking: the worker blocks on recv, and this is one thread.
        let answer = std::thread::spawn(move || {
            for msg in rx {
                if let Msg::Pick(reply) = msg {
                    let _ = reply.send(Reply::Picked(vec![1]));
                    break;
                }
            }
        });
        ask_which(&tx, &mut tracks);
        drop(tx);
        answer.join().unwrap();

        let statuses: Vec<Status> = tracks.iter().map(|t| t.status).collect();
        assert_eq!(
            statuses,
            [Status::Pending, Status::Skipped, Status::Have, Status::Gone],
            "a picked, on-disk or departed track was re-judged"
        );
    }

    /* The UI going away mid-question must not turn the whole playlist into a
       deliberate skip on the way out, and it can go away on either side of the
       question: before the ask lands, or after it with the answer never sent. */
    #[test]
    fn a_pick_nobody_answers_leaves_every_track_alone() {
        let track = || vec![Track::new(1, "a".into(), "A".into(), PathBuf::from("/tmp/a"))];

        let (tx, rx) = channel();
        drop(rx);
        let mut gone_before = track();
        ask_which(&tx, &mut gone_before);
        assert_eq!(gone_before[0].status, Status::Pending, "the ask never landed");

        let (tx, rx) = channel();
        // Takes the question and drops the reply channel without answering.
        let hangs_up = std::thread::spawn(move || {
            for msg in rx {
                if matches!(msg, Msg::Pick(_)) {
                    break;
                }
            }
        });
        let mut gone_after = track();
        ask_which(&tx, &mut gone_after);
        hangs_up.join().unwrap();
        assert_eq!(gone_after[0].status, Status::Pending, "the answer never came");
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

    /* The one thing earworm does that is both easy to get wrong and
       immediate. A swap over two tracks, because the bulk path is where the
       previous values are hardest to keep hold of: they have to be read off
       each track before it is written and dropped for any that failed. */
    #[test]
    fn undo_puts_the_tags_a_bulk_change_wrote_over_back() {
        let dir = scratch("undo");
        let one = dir.join("01 - A.opus");
        let two = dir.join("02 - B.opus");
        if tiny_opus(&one).is_none() || tiny_opus(&two).is_none() {
            eprintln!("no ffmpeg, skipping");
            let _ = std::fs::remove_dir_all(&dir);
            return;
        }
        let mut tracks = vec![
            Track::new(1, "a".into(), "01 - A.opus".into(), one),
            Track::new(2, "b".into(), "02 - B.opus".into(), two),
        ];
        for (pos, (artist, title)) in [("Neu!", "Hallogallo"), ("Can", "Vitamin C")]
            .into_iter()
            .enumerate()
        {
            tracks[pos].artist = artist.into();
            tracks[pos].title = title.into();
            tracks[pos].listed = true;
        }
        let cfg = config(false);
        let (tx, rx) = channel();
        let mut held: Option<Undo> = None;

        swap(&cfg, &tx, &mut tracks, &mut held, &[1, 2]).unwrap();
        assert_eq!(tracks[0].artist, "Hallogallo", "the swap did not happen");
        let offered: Vec<Option<String>> = rx
            .try_iter()
            .filter_map(|m| match m {
                Msg::Undoable(what) => Some(what),
                _ => None,
            })
            .collect();
        assert_eq!(
            offered.last().cloned().flatten().as_deref(),
            Some("swapped 2 tracks"),
            "the key was never offered: {offered:?}"
        );

        undo(&cfg, &tx, &mut tracks, &mut held).unwrap();
        assert_eq!(tracks[0].artist, "Neu!");
        assert_eq!(tracks[0].title, "Hallogallo");
        assert_eq!(tracks[1].artist, "Can");
        // The file itself, not only the row: an undo that leaves the tags on
        // disk swapped has undone the half nobody can check.
        let info = tag::read(tracks[0].path.as_deref().unwrap()).unwrap();
        assert_eq!((info.artist.as_str(), info.title.as_str()), ("Neu!", "Hallogallo"));

        /* One level means one level: a second `u` that redid the swap would
           be a toggle wearing the name of an undo. */
        assert!(held.is_none(), "a second undo is still on offer");
        let cleared: Vec<Option<String>> = rx
            .try_iter()
            .filter_map(|m| match m {
                Msg::Undoable(what) => Some(what),
                _ => None,
            })
            .collect();
        assert_eq!(cleared.first(), Some(&None), "the bar still offers u: {cleared:?}");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /* The chain behind "does a resync keep what I typed". Every link is
       tested on its own; this is the three of them in a row, because the
       guarantee lives in the join and not in any one of them:

         1. the sidecar maps the id to the renamed file, so the scan follows
            it instead of the name yt-dlp would predict,
         2. the archive names that id, so the download skips it rather than
            writing over it,
         3. the `Have` branch reads the tags off disk and neither identifies
            nor renames, so the row comes back saying what was typed.

       Driven through a stub yt-dlp that prints the path the scan would have
       predicted, which is exactly what a real resync of a renamed folder
       hands back. */
    #[test]
    fn a_resync_keeps_the_name_an_edit_gave_a_track() {
        let dir = scratch("resyncedit");
        let downloaded = dir.join("01 - Original Title.opus");
        if tiny_opus(&downloaded).is_none() {
            eprintln!("no ffmpeg, skipping");
            let _ = std::fs::remove_dir_all(&dir);
            return;
        }
        let mut cfg = config(true);
        cfg.dir = dir.parent().unwrap().to_path_buf();
        cfg.url = "https://www.youtube.com/playlist?list=PL1".into();

        // What the first run left behind: one track, tagged by the lookup.
        let mut tracks = vec![Track::new(
            1,
            "vid1".into(),
            "Original Title".into(),
            downloaded.clone(),
        )];
        tracks[0].listed = true;
        let (tx, _rx) = channel();

        // And then the edit, which renames the file and records the new name.
        write_track(
            &cfg,
            &tx,
            &mut tracks,
            0,
            named("Kraftwerk", "Autobahn"),
            "typed",
        )
        .unwrap();
        sync_manifest(&cfg, &tracks);
        let edited = tracks[0].path.clone().unwrap();
        assert_ne!(edited, downloaded, "the edit did not rename the file");
        assert!(!downloaded.exists() && edited.is_file());

        /* The resync. The stub prints what yt-dlp would: the id, the upstream
           title, and the filename the template predicts, which is the name
           the track no longer has. */
        let bin = dir.join("yt-dlp");
        std::fs::write(
            &bin,
            format!(
                "#!/bin/sh\nprintf 'vid1\t1\tOriginal Title\t{}\\n'\n",
                downloaded.display()
            ),
        )
        .unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&bin, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        let listing = crate::ytdlp::scan_with(&bin.display().to_string(), &cfg).unwrap();
        let mut found = listing.tracks;

        // 1. The sidecar won: the scan is pointing at the renamed file.
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].path.as_deref(), Some(edited.as_path()));
        assert_eq!(
            found[0].status,
            Status::Have,
            "the scan would have downloaded it again beside the renamed copy"
        );

        // 2. Which is what puts it in the archive, so the download skips it.
        let archive = dir.join("archive");
        crate::ytdlp::write_archive(&found, &archive, &HashSet::new()).unwrap();
        assert!(
            std::fs::read_to_string(&archive).unwrap().contains("vid1"),
            "the download would have written over the edited file"
        );

        // 3. And the tag pass reads what is on disk rather than looking it up.
        let asker = Asker { tx: tx.clone(), enabled: false };
        let mut cover = CoverState { written: true };
        tag_tracks(
            &cfg,
            &tx,
            &AtomicBool::new(false),
            &mut found,
            "Focus",
            &asker,
            &mut cover,
            None,
        );

        assert_eq!(found[0].artist, "Kraftwerk", "the resync took the artist back");
        assert_eq!(found[0].title, "Autobahn", "the resync took the title back");
        // The row label is title then artist, which is how the list reads.
        assert_eq!(found[0].name, "Autobahn - Kraftwerk");
        // The file too, not only the row: nothing renamed it back.
        assert!(edited.is_file(), "the resync renamed the file");
        assert!(!downloaded.exists(), "the upstream name came back");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /* The worker's copy of a track is not the one the detail pane draws, so a
       run that filled the album in and said nothing left the pane blank until
       the folder was reopened from the library: the same track, two answers,
       depending on how you got to it. */
    #[test]
    fn a_run_tells_the_ui_about_the_album_it_wrote() {
        let dir = scratch("runmeta");
        let file = dir.join("01 - A.opus");
        if tiny_opus(&file).is_none() {
            eprintln!("no ffmpeg, skipping");
            let _ = std::fs::remove_dir_all(&dir);
            return;
        }
        tag::set_fields(
            &file,
            &tag::Fields {
                artist: "Neu!".into(),
                title: "Hallogallo".into(),
                album: "Neu!".into(),
                year: Some(1972),
            },
        )
        .unwrap();

        let mut tracks = vec![Track::new(1, "a".into(), "01 - A.opus".into(), file)];
        // Already on disk, which is the branch a re-run takes for every
        // track it does not have to fetch.
        tracks[0].status = Status::Have;

        let (tx, rx) = channel();
        let asker = Asker { tx: tx.clone(), enabled: false };
        let mut cover = CoverState { written: true };
        let mut cfg = config(false);
        cfg.lookup = false;
        tag_tracks(
            &cfg,
            &tx,
            &AtomicBool::new(false),
            &mut tracks,
            "Focus",
            &asker,
            &mut cover,
            None,
        );

        let meta: Vec<(String, Option<u16>)> = rx
            .try_iter()
            .filter_map(|m| match m {
                Msg::Meta { album, year, .. } => Some((album, year)),
                _ => None,
            })
            .collect();
        assert_eq!(
            meta,
            vec![("Neu!".to_string(), Some(1972))],
            "the run kept the album to itself"
        );
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /* Every write writes the whole set, so a change to the artist has to
       carry the album and year it is not touching. Without that a swap over
       twenty tracks quietly stripped the album off all of them, and the row
       looked right because the row never showed it. */
    #[test]
    fn a_bulk_change_to_the_artist_keeps_the_album_and_the_year() {
        let dir = scratch("keepalbum");
        let file = dir.join("01 - A.opus");
        if tiny_opus(&file).is_none() {
            eprintln!("no ffmpeg, skipping");
            let _ = std::fs::remove_dir_all(&dir);
            return;
        }
        tag::set_fields(
            &file,
            &tag::Fields {
                artist: "Neu!".into(),
                title: "Hallogallo".into(),
                album: "Neu!".into(),
                year: Some(1972),
            },
        )
        .unwrap();

        let mut tracks = vec![Track::new(1, "a".into(), "01 - A.opus".into(), file.clone())];
        tracks[0].artist = "Neu!".into();
        tracks[0].title = "Hallogallo".into();
        tracks[0].album = "Neu!".into();
        tracks[0].year = Some(1972);
        tracks[0].listed = true;

        let cfg = config(false);
        let (tx, _rx) = channel();
        let mut held: Option<Undo> = None;
        swap(&cfg, &tx, &mut tracks, &mut held, &[1]).unwrap();

        // The file, not only the row: the row could be right about an album
        // the write had already cleared.
        let back = tag::read(&file).unwrap();
        assert_eq!((back.artist.as_str(), back.title.as_str()), ("Hallogallo", "Neu!"));
        assert_eq!(back.album, "Neu!", "the swap took the album with it");
        assert_eq!(back.year, Some(1972), "the swap took the year with it");
        assert_eq!(tracks[0].album, "Neu!");
        assert_eq!(tracks[0].year, Some(1972));

        // And the undo puts back all four, not the two it changed.
        undo(&cfg, &tx, &mut tracks, &mut held).unwrap();
        let back = tag::read(&file).unwrap();
        assert_eq!((back.artist.as_str(), back.title.as_str()), ("Neu!", "Hallogallo"));
        assert_eq!(back.album, "Neu!");
        assert_eq!(back.year, Some(1972));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /* The two new boxes may be cleared, so empty is an answer. A year that is
       not a year is not: writing it would drop whatever the file already had
       and say nothing. */
    #[test]
    fn the_edit_form_asks_for_album_and_year_and_refuses_a_year_that_is_not_one() {
        let dir = scratch("editmeta");
        let file = dir.join("01 - A.opus");
        if tiny_opus(&file).is_none() {
            eprintln!("no ffmpeg, skipping");
            let _ = std::fs::remove_dir_all(&dir);
            return;
        }
        tag::set_fields(
            &file,
            &tag::Fields {
                artist: "Neu!".into(),
                title: "Hallogallo".into(),
                album: "Neu!".into(),
                year: Some(1972),
            },
        )
        .unwrap();

        let answered = |reply: Vec<&str>| {
            let mut tracks = vec![Track::new(1, "a".into(), "01 - A.opus".into(), file.clone())];
            tracks[0].artist = "Neu!".into();
            tracks[0].title = "Hallogallo".into();
            tracks[0].album = "Neu!".into();
            tracks[0].year = Some(1972);
            tracks[0].listed = true;
            let cfg = config(false);
            let (tx, rx) = channel();
            let asker = Asker { tx: tx.clone(), enabled: true };
            let mut held: Option<Undo> = None;
            let values: Vec<String> = reply.iter().map(|v| (*v).to_string()).collect();
            std::thread::scope(|scope| {
                let worker =
                    scope.spawn(|| edit(&cfg, &tx, &mut tracks, &asker, &mut held, 1));
                let mut boxes = Vec::new();
                while let Ok(msg) = rx.recv_timeout(Duration::from_secs(5)) {
                    if let Msg::Ask(crate::app::Prompt::Form { fields, .. }, back) = msg {
                        boxes = fields;
                        back.send(Reply::Fields(values.clone())).unwrap();
                        break;
                    }
                }
                (boxes, worker.join().unwrap())
            })
        };

        // Prefilled from what the track holds, in the order the form asks.
        let (boxes, result) = answered(vec!["Neu!", "Hallogallo", "Neu! 2", "1973"]);
        result.unwrap();
        let labels: Vec<&str> = boxes.iter().map(|(l, _)| l.as_str()).collect();
        assert_eq!(labels, ["artist", "title", "album", "year"]);
        assert_eq!(boxes[3].1, "1972", "the year box did not prefill");
        let back = tag::read(&file).unwrap();
        assert_eq!(back.album, "Neu! 2");
        assert_eq!(back.year, Some(1973));

        // Cleared is an answer: the tag goes rather than the edit refusing.
        let (_, result) = answered(vec!["Neu!", "Hallogallo", "", ""]);
        result.unwrap();
        let back = tag::read(&file).unwrap();
        assert!(back.album.is_empty(), "the album survived being cleared");
        assert_eq!(back.year, None, "the year survived being cleared");

        // And nonsense in the year box stops the write rather than losing it.
        tag::set_fields(
            &file,
            &tag::Fields {
                artist: "Neu!".into(),
                title: "Hallogallo".into(),
                album: "Neu!".into(),
                year: Some(1972),
            },
        )
        .unwrap();
        for bad in ["last year", "12345", "999", "-1", "1.9.7.2"] {
            let (_, result) = answered(vec!["Neu!", "Hallogallo", "Neu!", bad]);
            let err = match result {
                Err(err) => err.to_string(),
                Ok(()) => panic!("{bad} was written as a year"),
            };
            assert!(err.contains("a year is four digits"), "{bad}: {err}");
            assert_eq!(
                tag::read(&file).unwrap().year,
                Some(1972),
                "{bad}: the year was lost anyway"
            );
        }
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// A real opus file, since `tag::set_fields` has to succeed for the code
    /// under test to be reached at all. `None` if ffmpeg is not installed.
    pub fn tiny_opus(at: &Path) -> Option<()> {
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

        write_track(&cfg, &tx, &mut tracks, 1, named("New", "Title"), "typed").unwrap();
        assert!(orphan.is_file(), "the departed file was moved");
        assert!(
            !dir.join("02 - New - Title.opus").exists(),
            "a departed track was given a playlist number"
        );

        write_track(&cfg, &tx, &mut tracks, 0, named("New", "Title"), "typed").unwrap();
        assert!(
            dir.join("01 - New - Title.opus").is_file(),
            "a listed track stopped being renamed, so the guard is too wide"
        );
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /* Opening a folder has to give every track the number it was downloaded
       with. Taking it from the row's position instead renumbers everything
       after a gap, and the next edit writes that wrong number onto the file. */
    #[test]
    fn opening_a_folder_numbers_tracks_from_their_filenames() {
        let dir = scratch("openshelf");
        let first = dir.join("01 - A.opus");
        let third = dir.join("03 - C.opus");
        if tiny_opus(&first).is_none() || tiny_opus(&third).is_none() {
            eprintln!("no ffmpeg, skipping");
            let _ = std::fs::remove_dir_all(&dir);
            return;
        }
        tag::set_fields(&third, &named("Cee", "Sea")).unwrap();
        // Track 2 is recorded but its file has been deleted by hand.
        manifest::write_synced(
            &dir,
            "https://youtube.com/playlist?list=PL1",
            [
                ("a".to_string(), first.clone()),
                ("b".to_string(), dir.join("02 - B.opus")),
                ("c".to_string(), third.clone()),
            ]
            .into_iter(),
        )
        .unwrap();

        let shelves = library(dir.parent().unwrap());
        let shelf = shelves
            .iter()
            .find(|s| s.path == dir)
            .expect("the folder is not in the library");
        assert_eq!(shelf.tracks, 2);
        assert_eq!(shelf.missing, 1);
        assert!(shelf.synced.is_some(), "the sync time was not recorded");

        let mut cfg = config(true);
        let mut tracks = Vec::new();
        let (tx, _rx) = channel();
        let summary = open_shelf(&mut cfg, &tx, &mut tracks, shelf).unwrap();

        assert_eq!(tracks.iter().map(|t| t.index).collect::<Vec<_>>(), [1, 3]);
        assert_eq!(tracks[1].artist, "Cee", "the tags on disk were not read");
        assert_eq!(tracks[1].name, "Sea - Cee");
        assert_eq!(tracks[1].was, tracks[1].name, "every row would claim a rename");
        assert!(tracks.iter().all(|t| t.status == Status::Have && t.listed));
        assert_eq!(cfg.url, shelf.url, "S would have no URL to sync from");
        assert!(summary.contains("1 file missing"), "{summary}");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /* A pass over part of a playlist used to rewrite the whole manifest from
       that part. The forgotten files stay on disk under names earworm chose,
       which `scan` cannot recognise, so the next sync downloads them again
       beside the copies already there. */
    #[test]
    fn a_partial_pass_does_not_forget_the_files_it_never_looked_at() {
        let dir = scratch("partial");
        let mine = dir.join("04 - D.opus");
        let others = dir.join("01 - A.opus");
        std::fs::write(&mine, "audio").unwrap();
        std::fs::write(&others, "audio").unwrap();
        manifest::write(
            &dir,
            "u",
            [
                ("a".to_string(), others.clone()),
                ("dead".to_string(), dir.join("07 - deleted.opus")),
                ("d".to_string(), mine.clone()),
            ]
            .into_iter(),
        )
        .unwrap();

        let tracks = vec![Track::new(4, "d".into(), "04 - D.opus".into(), mine)];
        save_manifest(&config(true), &tracks);

        let back = manifest::read(&dir);
        assert_eq!(back.get("a"), Some(&others), "a file outside this pass was forgotten");
        assert!(back.contains_key("d"));
        assert_eq!(
            manifest::entries(&dir).len(),
            2,
            "an entry whose file is gone was kept, so the folder collects dead lines"
        );
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// A manifest plus the `Shelf` that `library` would build for it.
    fn shelf_of(dir: &Path, files: &[(&str, &PathBuf)]) -> Shelf {
        manifest::write(
            dir,
            "https://youtube.com/playlist?list=PL1",
            files.iter().map(|(id, f)| (id.to_string(), (*f).clone())),
        )
        .unwrap();
        library(dir.parent().unwrap())
            .into_iter()
            .find(|s| s.path == dir)
            .expect("the folder is not in the library")
    }

    /* Removing a video renumbers the tracks after it and the file that left
       keeps its old name, so a folder ends up holding two `05 - `. Every
       command finds its track by index and takes the first match, so sharing
       one makes a row write tags to another row's file. */
    #[test]
    fn two_files_sharing_a_number_still_get_a_row_each() {
        let dir = scratch("collide");
        let live = dir.join("05 - New.opus");
        let departed = dir.join("05 - Old.opus");
        let unnumbered = dir.join("loose track.opus");
        for file in [&live, &departed, &unnumbered] {
            std::fs::write(file, "audio").unwrap();
        }
        let shelf = shelf_of(&dir, &[("new", &live), ("old", &departed), ("loose", &unnumbered)]);

        let mut tracks = Vec::new();
        open_shelf(&mut config(true), &channel().0, &mut tracks, &shelf).unwrap();

        let indices: Vec<usize> = tracks.iter().map(|t| t.index).collect();
        assert_eq!(indices.len(), 3);
        assert_eq!(
            indices.iter().collect::<HashSet<_>>().len(),
            3,
            "two rows share an index, so one of them edits the other's file: {indices:?}"
        );
        assert!(!indices.contains(&0), "a track kept the placeholder index");
        for track in &tracks {
            let pos = tracks.iter().position(|t| t.index == track.index);
            assert_eq!(
                tracks[pos.unwrap()].path, track.path,
                "index {} resolves to a different file",
                track.index
            );
        }
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /* An edit rebuilds the .m3u8 from `listed` and renames from `index`, and
       both are guarded on `Gone`. Reading a folder back as all-present puts
       the departed tracks into a playlist the last sync had correctly left
       them out of, and stamps a playlist number onto a file that has none.
       The edit here is the point: an earlier version of this test stopped at
       the load and passed while `write_track` undid the whole thing. */
    #[test]
    fn opening_a_folder_believes_the_playlist_file_about_what_is_in_it() {
        let dir = scratch("m3u8listed");
        let live = dir.join("01 - New.opus");
        let departed = dir.join("09 - Old.opus");
        if tiny_opus(&live).is_none() || tiny_opus(&departed).is_none() {
            eprintln!("no ffmpeg, skipping");
            let _ = std::fs::remove_dir_all(&dir);
            return;
        }
        let shelf = shelf_of(&dir, &[("new", &live), ("old", &departed)]);
        let name = dir.file_name().unwrap().to_string_lossy().to_string();
        let playlist = dir.join(format!("{name}.m3u8"));
        std::fs::write(
            &playlist,
            format!("#EXTM3U\n#EXTINF:1,New\n{}\n", live.display()),
        )
        .unwrap();

        let cfg = config(true);
        let mut tracks = Vec::new();
        open_shelf(&mut config(true), &channel().0, &mut tracks, &shelf).unwrap();
        assert_eq!(
            tracks.iter().map(|t| t.status).collect::<Vec<_>>(),
            [Status::Have, Status::Gone],
            "the file the playlist leaves out was read back as part of it"
        );
        assert_eq!(tracks[1].index, 2, "a departed row kept a playlist number");

        // Twice, because the first edit used to overwrite the departure with
        // `manual` and the second would then rename and re-list the track.
        let pos = tracks.iter().position(|t| t.status == Status::Gone).unwrap();
        for title in ["N", "N again"] {
            write_track(&cfg, &channel().0, &mut tracks, pos, named("F", title), "typed")
                .unwrap();
            assert!(departed.is_file(), "the departed file was renamed");
            assert!(!tracks[pos].listed, "the edit put it back in the playlist");
        }

        write_playlist(&cfg, &tracks).unwrap();
        let after = std::fs::read_to_string(&playlist).unwrap();
        assert!(!after.contains("09 - Old"), "it reached the .m3u8: {after}");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /* A playlist file naming nothing on disk describes some other folder, and
       both answers are then guesses. Emptying the .m3u8 is the one that loses
       something, so every track stays listed. */
    #[test]
    fn a_playlist_file_that_matches_nothing_is_not_believed() {
        let dir = scratch("m3u8stale");
        let live = dir.join("01 - New.opus");
        std::fs::write(&live, "audio").unwrap();
        let shelf = shelf_of(&dir, &[("new", &live)]);
        let name = dir.file_name().unwrap().to_string_lossy().to_string();
        std::fs::write(
            dir.join(format!("{name}.m3u8")),
            "#EXTM3U\n/somewhere/else/01 - Other.opus\n",
        )
        .unwrap();

        let mut tracks = Vec::new();
        open_shelf(&mut config(true), &channel().0, &mut tracks, &shelf).unwrap();
        assert!(tracks[0].listed);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /* An absolute path is only valid on the machine that wrote it, so a folder
       copied to a phone or another user's home plays nothing. The .m3u8 sits
       beside the tracks, so a bare name resolves wherever the folder lands. */
    #[test]
    fn the_playlist_file_names_its_tracks_relatively() {
        let dir = scratch("relative");
        let file = dir.join("01 - A - T.opus");
        std::fs::write(&file, "audio").unwrap();
        let mut tracks = vec![Track::new(1, "v1".into(), "01 - A - T.opus".into(), file)];
        tracks[0].listed = true;
        tracks[0].artist = "A".into();
        tracks[0].title = "T".into();

        let playlist = write_playlist(&config(true), &tracks).unwrap().unwrap();
        let body = std::fs::read_to_string(&playlist).unwrap();
        assert!(
            !body.contains(&dir.display().to_string()),
            "the folder's own path is in the playlist: {body}"
        );
        assert!(body.contains("\n01 - A - T.opus\n"), "{body}");

        // Still the file the loader recognises when reading a folder back.
        let names = last_playlist(&dir).expect("no usable playlist file");
        assert!(names.contains("01 - A - T.opus"));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_playlist_number_is_only_read_off_a_name_that_has_one() {
        assert_eq!(numbered("01 - A - B.opus"), Some(1));
        assert_eq!(numbered("12 - A.opus"), Some(12));
        assert_eq!(numbered("cover.jpg"), None);
        // Nothing but digits is not a track file, and 0 is not a position.
        assert_eq!(numbered("12345"), None);
        assert_eq!(numbered("00 - A.opus"), None);
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

    /* A finished folder holds tracks, the playlist and the manifest. The
       folder image only ever duplicated the art already embedded in every
       track, so the end of a pass drops it and keeps everything else. */
    #[test]
    fn a_finished_pass_leaves_no_folder_image_behind() {
        let dir = scratch("dropcover");
        let tracks = vec![track_at(&dir, "01 - a - b.opus", Status::Have)];
        for name in COVER_NAMES {
            std::fs::write(dir.join(name), "image").unwrap();
        }
        std::fs::write(dir.join("list.m3u8"), "tracks").unwrap();

        drop_folder_cover(&tracks, &[]);

        for name in COVER_NAMES {
            assert!(!dir.join(name).exists(), "{name} survived the pass");
        }
        assert!(dir.join("01 - a - b.opus").is_file(), "a track went with it");
        assert!(dir.join("list.m3u8").is_file(), "the playlist went with it");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /* An image that was there before the pass is somebody else's: one dropped
       into the folder by hand, or the one `c` wrote on an explicit choice.
       Deleting it does not come back and nothing on screen would say so. */
    #[test]
    fn a_cover_the_pass_did_not_write_survives_it() {
        let dir = scratch("keepcover");
        let tracks = vec![track_at(&dir, "01 - a - b.opus", Status::Have)];
        std::fs::write(dir.join("cover.jpg"), "theirs").unwrap();

        // What the folder held on the way in, which is the only moment the
        // two can still be told apart.
        let theirs = folder_covers(&tracks);
        assert_eq!(theirs, ["cover.jpg"]);
        // And then the download leaves one of its own beside it.
        std::fs::write(dir.join("cover.png"), "ours").unwrap();

        drop_folder_cover(&tracks, &theirs);

        assert_eq!(
            std::fs::read_to_string(dir.join("cover.jpg")).unwrap(),
            "theirs",
            "a cover nobody in this pass wrote was deleted"
        );
        assert!(!dir.join("cover.png").exists(), "the pass kept its own leftover");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /* Both bail before any lookup runs, so neither test needs the network:
       the guards are the whole point being pinned down. */
    #[test]
    fn searching_a_track_with_no_file_reports_it() {
        let dir = scratch("searchnofile");
        let (tx, _rx) = mpsc::channel();
        let asker = Asker { tx: tx.clone(), enabled: false };
        let mut tracks = vec![Track::new(1, "vid".into(), "name".into(), dir.join("gone.opus"))];
        tracks[0].artist = "Somebody".into();
        tracks[0].title = "Something".into();

        let err = search(&config(true), &tx, &mut tracks, &asker, &mut None, 1).unwrap_err();
        assert!(err.to_string().contains("never downloaded"), "{err:?}");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn searching_with_no_words_to_search_with_names_the_next_step() {
        let dir = scratch("searchempty");
        let (tx, _rx) = mpsc::channel();
        let asker = Asker { tx: tx.clone(), enabled: false };
        let mut tracks = vec![track_at(&dir, "01 - a - b.opus", Status::Failed)];
        tracks[0].artist.clear();

        let err = search(&config(true), &tx, &mut tracks, &asker, &mut None, 4).unwrap_err();
        assert!(err.to_string().contains("e types them first"), "{err:?}");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /* A hit clears the undo, since `u` would otherwise write the tags this
       search replaced back over a match it knows nothing about. A search that
       changed nothing must not: clearing at the top of the function would
       take the undo away every time the words turned out to be unsearchable.
       Only this half runs offline; the clear itself needs a real lookup. */
    #[test]
    fn a_search_that_wrote_nothing_leaves_the_undo_alone() {
        let dir = scratch("searchundo");
        let (tx, _rx) = mpsc::channel();
        let asker = Asker { tx: tx.clone(), enabled: false };
        let mut tracks = vec![track_at(&dir, "01 - a - b.opus", Status::Failed)];
        tracks[0].artist.clear();
        let mut held = Some(Undo {
            what: "the edit of track 4".into(),
            fields: vec![(4, named("Neu!", "Hallogallo"))],
        });

        assert!(search(&config(true), &tx, &mut tracks, &asker, &mut held, 4).is_err());
        assert!(held.is_some(), "a search that did nothing took the undo with it");
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

    /// Just the two fields most of these tests care about; album and year
    /// ride along empty, which is what an untagged fixture has anyway.
    pub fn named(artist: &str, title: &str) -> tag::Fields {
        tag::Fields {
            artist: artist.into(),
            title: title.into(),
            ..tag::Fields::default()
        }
    }

    pub fn config(rename: bool) -> Config {
        Config {
            url: String::new(),
            dir: PathBuf::new(),
            theme: crate::theme::Palette::default(),
            theme_name: crate::config::DEFAULT_THEME.to_string(),
            themes: crate::theme::Themes::default(),
            theme_warnings: Vec::new(),
            theme_overridden: false,
            parse: true,
            m3u8: true,
            cover: true,
            lookup: true,
            apple: true,
            fix: true,
            format: config::DEFAULT_FORMAT.into(),
            // Off, so a test that does not ask for it converts nothing.
            convert: false,
            config_file: None,
            resync: false,
            list: false,
            check: false,
            rename,
            album: true,
            pick: true,
            intro: true,
            notify: true,
            update_check: true,
            folder: None,
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


#[cfg(test)]
mod format_tests {
    use super::*;

    fn scratch(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "earworm-format-{tag}-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    /* One reply per question, in order, stopping once the last has been
       answered. `set_format` asks twice when it has a config file to write
       to, and a helper that answered only the first left the worker blocked
       on a channel nothing was going to send to. */
    fn answer(rx: &Receiver<Msg>, replies: &[Reply]) -> Vec<Msg> {
        let mut seen = Vec::new();
        let mut next = replies.iter();
        for msg in rx {
            let asked = matches!(msg, Msg::Ask(..));
            if let Msg::Ask(_, back) = &msg {
                let reply = next.next().expect("asked more questions than expected");
                back.send(reply.clone()).unwrap();
            }
            seen.push(msg);
            if asked && next.len() == 0 {
                break;
            }
        }
        seen
    }

    /// Chosen in the tool, so the next session has to start there too.
    #[test]
    fn choosing_a_format_writes_it_to_the_config_file() {
        let dir = scratch("save");
        let path = dir.join("config.toml");
        std::fs::write(&path, "# mine\ndir = \"/tmp/music\"\n").unwrap();

        let (tx, rx) = mpsc::channel();
        let mut cfg = super::tests::config(true);
        cfg.config_file = Some(path.clone());
        let asker = Asker { tx: tx.clone(), enabled: true };

        let picked = std::thread::scope(|scope| {
            let worker = scope.spawn(|| set_format(&mut cfg, &tx, &asker));
            /* The index of "mp3" in FORMATS, then no to the follow-up: this
               test is about the format alone, and the second question has
               its own. */
            let mut seen = answer(&rx, &[Reply::Choice(2), Reply::Choice(0)]);
            worker.join().unwrap().unwrap();
            // Drained after the join: the interesting messages come after the
            // answer, so stopping at the question would catch none of them.
            seen.extend(rx.try_iter());
            seen
        });

        assert_eq!(cfg.format, "mp3");
        let written = std::fs::read_to_string(&path).unwrap();
        assert!(written.contains("format = \"mp3\""), "{written:?}");
        assert!(written.contains("# mine"), "the file was rewritten: {written:?}");
        assert!(written.contains("dir = \"/tmp/music\""), "{written:?}");
        assert!(
            picked.iter().any(|m| matches!(m, Msg::Settings { text, .. } if text.contains("format mp3"))),
            "the help overlay still reports the old format"
        );
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /* The setting that decides whether a sync rewrites files already on
       disk. Asked here because this is the moment the question is in the
       user's head, and off unless it is answered yes: a format change that
       rewrote the library on its own would turn one menu pick into an
       unattended rewrite of every folder under --dir. */
    #[test]
    fn the_follow_up_is_what_turns_conversion_on() {
        let dir = scratch("convert");
        let path = dir.join("config.toml");

        let saying = |reply: Reply| {
            std::fs::write(&path, "dir = \"/tmp/music\"\n").unwrap();
            let (tx, rx) = mpsc::channel();
            let mut cfg = super::tests::config(true);
            cfg.config_file = Some(path.clone());
            let asker = Asker { tx: tx.clone(), enabled: true };
            std::thread::scope(|scope| {
                let worker = scope.spawn(|| set_format(&mut cfg, &tx, &asker));
                answer(&rx, &[Reply::Choice(3), reply]);
                worker.join().unwrap().unwrap();
            });
            (cfg.convert, std::fs::read_to_string(&path).unwrap())
        };

        let (on, written) = saying(Reply::Choice(1));
        assert!(on, "answering yes did not turn it on for this session");
        assert!(written.contains("convert = true"), "{written:?}");
        // The format is written whichever way the second question goes.
        assert!(written.contains("format = \"flac\""), "{written:?}");

        let (off, written) = saying(Reply::Choice(0));
        assert!(!off, "answering no turned it on anyway");
        assert!(!written.contains("convert = true"), "{written:?}");
        assert!(written.contains("format = \"flac\""), "{written:?}");

        // Escaping the follow-up is the same answer as no, and must not write.
        let (escaped, written) = saying(Reply::Cancel);
        assert!(!escaped);
        assert!(!written.contains("convert = true"), "{written:?}");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// Backing out of the menu must not write, or Esc becomes a way to set
    /// whatever the cursor happened to rest on.
    #[test]
    fn backing_out_changes_nothing() {
        let dir = scratch("escape");
        let path = dir.join("config.toml");
        std::fs::write(&path, "dir = \"/tmp/music\"\n").unwrap();

        let (tx, rx) = mpsc::channel();
        let mut cfg = super::tests::config(true);
        cfg.config_file = Some(path.clone());
        let asker = Asker { tx: tx.clone(), enabled: true };

        std::thread::scope(|scope| {
            let worker = scope.spawn(|| set_format(&mut cfg, &tx, &asker));
            // Backing out of the first means the second is never asked.
            answer(&rx, &[Reply::Cancel]);
            worker.join().unwrap().unwrap();
        });

        assert_eq!(cfg.format, config::DEFAULT_FORMAT);
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "dir = \"/tmp/music\"\n");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// --no-config asked for the file to be left out of the run, so there is
    /// nowhere to remember it and the session has to say so.
    #[test]
    fn without_a_config_file_the_choice_lasts_one_session() {
        let (tx, rx) = mpsc::channel();
        let mut cfg = super::tests::config(true);
        cfg.config_file = None;
        let asker = Asker { tx: tx.clone(), enabled: true };

        let seen = std::thread::scope(|scope| {
            let worker = scope.spawn(|| set_format(&mut cfg, &tx, &asker));
            /* One question only: with no config file there is nowhere to
               record a conversion either, so the follow-up is not asked. */
            let mut seen = answer(&rx, &[Reply::Choice(3)]);
            worker.join().unwrap().unwrap();
            seen.extend(rx.try_iter());
            seen
        });

        assert_eq!(cfg.format, "flac");
        assert!(
            seen.iter().any(|m| matches!(m, Msg::Flash(s) if s.contains("this run only"))),
            "nothing said the choice was not kept"
        );
    }
}
#[cfg(test)]
mod convert_tests {
    use super::*;
    use crate::app::Msg;
    use std::sync::mpsc;

    fn scratch(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "earworm-convert-{tag}-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn holding(dir: &Path, index: usize, name: &str, status: Status) -> Track {
        let path = dir.join(name);
        let mut track = Track::new(index, format!("id{index}"), name.into(), path);
        track.status = status;
        track
    }

    /* The table the whole feature turns on, every source against every
       target. Two measured facts decide it, both recorded on `bring`:
       yt-dlp's `bestaudio` is the Opus stream, so only an opus target is
       remuxed by a download and everything else is ffmpeg re-encoding that
       same Opus; and that makes m4a, mp3 and vorbis on disk two generations
       deep where opus, flac and alac are one. */
    #[test]
    fn the_source_format_decides_between_converting_and_downloading_again() {
        let dir = scratch("decide");
        /* `.m4a` is the one extension that cannot answer for itself, so those
           two need a real container with a real codec in it. The rest are
           decided on the extension and a placeholder is enough. */
        let seed = dir.join("seed.opus");
        let mpeg4 = super::tests::tiny_opus(&seed).is_some();
        let mut real: Vec<&str> = Vec::new();
        for (format, ext, _) in config::FORMATS {
            let at = dir.join(format!("{format}.{ext}"));
            if ext == "m4a" {
                if mpeg4 && tag::transcode(&seed, &at, format, Some("opus")).is_ok() {
                    real.push(format);
                }
                continue;
            }
            std::fs::write(&at, "audio").unwrap();
            real.push(format);
        }
        assert!(
            real.contains(&"opus") && real.contains(&"flac"),
            "the fixtures nothing needs ffmpeg for are missing"
        );
        let file = |format: &str| format!("{format}.{}", config::extension(format));

        for (now, _, _) in config::FORMATS {
            if !real.contains(&now) {
                eprintln!("skipped {now}: no fixture");
                continue;
            }
            for (target, _, _) in config::FORMATS {
                let name = file(now);
                let track = holding(&dir, 1, &name, Status::Have);
                let got = bring(&track, target);
                // `alac` and `m4a` are one file on disk, so the fixture cannot
                // tell them apart and only the extension is under test here.
                if config::extension(now) == config::extension(target) {
                    continue;
                }
                let want = match (now, target) {
                    // Already there.
                    (a, b) if a == b => Bring::Leave,
                    /* Two generations on disk already, so converting makes a
                       third where a download makes at most two. */
                    ("m4a" | "mp3" | "vorbis", _) => Bring::Refetch,
                    // The one target a download remuxes rather than re-encodes.
                    (_, "opus") => Bring::Refetch,
                    // One generation in, and a download would cost the same.
                    _ => Bring::Convert,
                };
                assert_eq!(got, want, "{now} to {target}");
            }
        }

        assert!(
            real.contains(&"m4a") && real.contains(&"alac"),
            "the two formats behind one extension were never tested"
        );

        // A container earworm does not write is left alone, not guessed at.
        std::fs::write(dir.join("06 - f.wav"), "audio").unwrap();
        let odd = holding(&dir, 6, "06 - f.wav", Status::Have);
        assert_eq!(bring(&odd, "opus"), Bring::Leave);

        /* Only a track with a file this sync owns. A departed one holds no
           playlist position and its index is synthetic, a skipped one was
           never fetched, a pending one is about to arrive in the target
           format anyway. */
        for status in [Status::Gone, Status::Skipped, Status::Pending, Status::Failed] {
            let track = holding(&dir, 1, "opus.opus", status);
            assert_eq!(bring(&track, "flac"), Bring::Leave, "{status:?}");
        }
        // And a row whose file is not actually there.
        let gone = holding(&dir, 9, "99 - nothing.opus", Status::Have);
        assert_eq!(bring(&gone, "flac"), Bring::Leave);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /* The pick gate says which tracks this run is for, and it only ever marks
       unpicked `Pending` tracks. Everything already on disk falls outside
       that, so a run where somebody asked for two new tracks used to
       re-download every mp3 in the folder as well, with nothing on the screen
       that asked the question even mentioning it. */
    #[test]
    fn the_pick_gate_decides_what_gets_converted_too() {
        let dir = scratch("picked");
        for name in ["01 - a.mp3", "02 - b.mp3", "03 - c.mp3"] {
            std::fs::write(dir.join(name), "audio").unwrap();
        }
        let tracks = vec![
            holding(&dir, 1, "01 - a.mp3", Status::Have),
            holding(&dir, 2, "02 - b.mp3", Status::Have),
            holding(&dir, 3, "03 - c.mp3", Status::Have),
        ];
        let mut cfg = super::tests::config(true);
        cfg.format = "flac".into();
        cfg.convert = true;

        // No gate, which is every `--resync`: the whole folder comes across.
        assert_eq!(to_bring(&cfg, &tracks, None).len(), 3);

        // Gated on one track, so that is the only one touched.
        let picked = HashSet::from([2usize]);
        let plans = to_bring(&cfg, &tracks, Some(&picked));
        assert_eq!(plans.len(), 1, "the gate was ignored: {plans:?}");
        assert_eq!(plans.get(&2), Some(&Bring::Refetch));

        // And a gate nobody marked anything at is not a gate that wants all.
        assert!(to_bring(&cfg, &tracks, Some(&HashSet::new())).is_empty());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /* An interrupted swap leaves the replacement under its own name with the
       sidecar still pointing at the old file. Refusing that forever wedged the
       track: every later sync planned the same conversion, hit the guard, and
       on the refetch path downloaded a file and threw it away. A name no track
       holds is this pass's own leftover and may be replaced. A name a track
       does hold is somebody's file and may not. */
    #[test]
    fn a_leftover_is_replaced_but_another_tracks_file_is_not() {
        let dir = scratch("collide");
        let old = dir.join("01 - A.opus");
        if super::tests::tiny_opus(&old).is_none() {
            eprintln!("no ffmpeg, skipping");
            return;
        }
        let make_fresh = |to: &Path| tag::transcode(&old, to, "flac", Some("opus")).is_ok();
        let fresh = dir.join(".earworm-convert-1.flac");
        if !make_fresh(&fresh) {
            eprintln!("no flac encoder, skipping");
            return;
        }
        // What the interrupted run left behind, under the name this wants.
        let leftover = dir.join("01 - A.flac");
        std::fs::copy(&fresh, &leftover).unwrap();
        let (tx, rx) = mpsc::channel();

        // Nothing holds that name, so it is ours and gets replaced.
        let mut track = holding(&dir, 1, "01 - A.opus", Status::Have);
        let stale = swap_in(&tx, &mut track, &fresh, "flac", &HashSet::new()).unwrap();
        assert_eq!(track.path.as_deref(), Some(leftover.as_path()));
        assert_eq!(stale.as_deref(), Some(old.as_path()));
        assert!(
            rx.try_iter().any(|m| matches!(m, Msg::Log(l) if l.contains("leftover"))),
            "replacing a file said nothing"
        );
        std::fs::remove_file(stale.unwrap()).unwrap();

        /* And now the same name, held by another track in the run. That is
           somebody's file, and the message has to say what to do about it. */
        let other = dir.join("02 - B.opus");
        super::tests::tiny_opus(&other).unwrap();
        let again = dir.join(".earworm-convert-2.flac");
        assert!(tag::transcode(&other, &again, "flac", Some("opus")).is_ok());
        let mut second = holding(&dir, 2, "02 - B.opus", Status::Have);
        // Track 1 already holds `01 - A.flac`, and this would land on it.
        second.path = Some(dir.join("01 - A.opus"));
        let owned = HashSet::from([leftover.clone()]);
        let err = swap_in(&tx, &mut second, &again, "flac", &owned).unwrap_err().to_string();
        assert!(err.contains("another track's file"), "{err}");
        assert!(err.contains("sync again"), "the error gives no next step: {err}");
        assert!(leftover.is_file(), "another track's file was taken");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /* On the refetch path the new file is whatever yt-dlp wrote. A
       postprocessor that fell back to the source container would otherwise be
       renamed, recorded in the sidecar and reported as converted while being
       nothing of the kind, and `format_on_disk` would then say `Leave` about
       it on every later sync: the row claims success for good. */
    #[test]
    fn a_swap_refuses_a_file_that_is_not_the_format_asked_for() {
        let dir = scratch("wrongfmt");
        let old = dir.join("01 - A.mp3");
        let seed = dir.join("seed.opus");
        if super::tests::tiny_opus(&seed).is_none()
            || tag::transcode(&seed, &old, "mp3", Some("opus")).is_err()
        {
            eprintln!("no ffmpeg or no mp3 encoder, skipping");
            return;
        }
        // A perfectly good audio file, just not the one the run asked for.
        let wrong = dir.join("what-yt-dlp-wrote.opus");
        std::fs::copy(&seed, &wrong).unwrap();

        let (tx, _rx) = mpsc::channel();
        let mut track = holding(&dir, 1, "01 - A.mp3", Status::Have);
        let err = swap_in(&tx, &mut track, &wrong, "flac", &HashSet::new())
            .unwrap_err()
            .to_string();
        assert!(err.contains("opus") && err.contains("flac"), "{err}");
        assert!(old.is_file(), "the old file went for a replacement in the wrong format");
        assert_eq!(track.path.as_deref(), Some(old.as_path()));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /* A folder where every transcode failed for want of an encoder used to
       report as a clean sync. The rows scroll off, `--resync` writes one line
       per folder, and that line is all anyone reads afterwards. */
    #[test]
    fn a_conversion_that_failed_is_counted_not_just_logged() {
        let dir = scratch("counted");
        // Not audio, so ffmpeg refuses every one of them.
        for name in ["01 - a.flac", "02 - b.flac"] {
            std::fs::write(dir.join(name), "this is not a flac").unwrap();
        }
        let mut tracks = vec![
            holding(&dir, 1, "01 - a.flac", Status::Have),
            holding(&dir, 2, "02 - b.flac", Status::Have),
        ];
        let mut cfg = super::tests::config(true);
        cfg.format = "mp3".into();
        cfg.convert = true;

        let plans = to_bring(&cfg, &tracks, None);
        assert_eq!(plans.len(), 2, "the fixture is not one the pass would act on");
        let (tx, _rx) = mpsc::channel();
        let cancel = AtomicBool::new(false);
        let (done, failed) = convert_tracks(&cfg, &tx, &cancel, &mut tracks, &plans);
        assert_eq!((done, failed), (0, 2), "a failure went uncounted");

        // And the files are exactly as they were.
        for track in &tracks {
            assert_eq!(track.status, Status::Have);
            assert!(track.path.as_deref().is_some_and(Path::is_file));
        }
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /* The promise the setting exists to keep. `format` means "format for new
       downloads", and a plain format change that rewrote the library would
       turn one menu pick into an unattended rewrite of every folder under
       --dir the next time cron ran a resync. */
    #[test]
    fn nothing_is_brought_across_while_convert_is_off() {
        let dir = scratch("off");
        std::fs::write(dir.join("01 - a.mp3"), "audio").unwrap();
        std::fs::write(dir.join("02 - b.opus"), "audio").unwrap();
        let tracks = vec![
            holding(&dir, 1, "01 - a.mp3", Status::Have),
            holding(&dir, 2, "02 - b.opus", Status::Have),
        ];

        let mut cfg = super::tests::config(true);
        cfg.format = "flac".into();
        cfg.convert = false;
        assert!(to_bring(&cfg, &tracks, None).is_empty(), "convert was off");

        // And that the fixture is one the feature would otherwise act on, or
        // the assertion above passes for the wrong reason.
        cfg.convert = true;
        let plans = to_bring(&cfg, &tracks, None);
        assert_eq!(plans.get(&1), Some(&Bring::Refetch));
        assert_eq!(plans.get(&2), Some(&Bring::Convert));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /* An edit is the only thing in a folder that cannot be got back, so it
       has to survive the container change: the tags off the old file, and the
       name the edit gave it with the new extension on the end. */
    #[test]
    fn a_swap_carries_the_tags_and_the_edited_name_onto_the_new_file() {
        let dir = scratch("swap");
        let old = dir.join("01 - The Name I Typed.opus");
        if super::tests::tiny_opus(&old).is_none() {
            eprintln!("no ffmpeg, skipping");
            return;
        }
        tag::set_fields(
            &old,
            &tag::Fields {
                artist: "Kraftwerk".into(),
                title: "Autobahn".into(),
                album: "Autobahn".into(),
                year: Some(1974),
            },
        )
        .unwrap();

        let fresh = dir.join("what-yt-dlp-called-it.flac");
        if tag::transcode(&old, &fresh, "flac", Some("opus")).is_err() {
            eprintln!("no flac encoder, skipping");
            return;
        }

        let (tx, rx) = mpsc::channel();
        let mut track = holding(&dir, 1, "01 - The Name I Typed.opus", Status::Have);
        let stale = swap_in(&tx, &mut track, &fresh, "flac", &HashSet::new()).unwrap();
        /* Handed back rather than removed, so the caller can move the sidecar
           onto the replacement first. Removing it here is what that caller
           does one line later. */
        assert_eq!(stale.as_deref(), Some(old.as_path()), "the old file was not handed back");
        assert!(old.is_file(), "the old file went before the sidecar could move");
        std::fs::remove_file(stale.unwrap()).unwrap();

        let landed = dir.join("01 - The Name I Typed.flac");
        assert_eq!(track.path.as_deref(), Some(landed.as_path()));
        assert!(landed.is_file(), "the new file is not under the edited name");
        assert!(!old.exists(), "the old file was left behind");
        assert!(!fresh.exists(), "the downloaded name was left behind");
        // Back on Have, so `tag_tracks` reads the file and does not identify
        // it: a lookup here would replace what was just carried across.
        assert_eq!(track.status, Status::Have);

        let back = tag::read(&landed).unwrap();
        assert_eq!(back.artist, "Kraftwerk");
        assert_eq!(back.title, "Autobahn");
        assert_eq!(back.album, "Autobahn");
        assert_eq!(back.year, Some(1974));
        assert!(
            rx.try_iter().any(|m| matches!(m, Msg::Path { index: 1, .. })),
            "the row was never told where the file went"
        );
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// Nothing is removed until the replacement has been read back, because a
    /// video pulled from YouTube since the first download cannot be re-fetched.
    #[test]
    fn a_swap_onto_an_unreadable_file_leaves_the_old_one_alone() {
        let dir = scratch("keep");
        let old = dir.join("01 - A.opus");
        if super::tests::tiny_opus(&old).is_none() {
            eprintln!("no ffmpeg, skipping");
            return;
        }
        let before = std::fs::read(&old).unwrap();
        let junk = dir.join("half-written.flac");
        std::fs::write(&junk, b"not audio at all").unwrap();

        let (tx, _rx) = mpsc::channel();
        let mut track = holding(&dir, 1, "01 - A.opus", Status::Have);
        assert!(swap_in(&tx, &mut track, &junk, "flac", &HashSet::new()).is_err());
        assert!(old.is_file(), "the old file was removed for a bad replacement");
        assert_eq!(std::fs::read(&old).unwrap(), before);
        assert_eq!(track.path.as_deref(), Some(old.as_path()));

        // An empty one is the full-disk case, and fails before the parse.
        let empty = dir.join("empty.flac");
        std::fs::write(&empty, b"").unwrap();
        assert!(swap_in(&tx, &mut track, &empty, "flac", &HashSet::new()).is_err());
        assert!(old.is_file());

        /* The case the verification is actually load-bearing for. With tags
           on the old file, writing them onto the replacement fails and takes
           the swap down with it, so the two assertions above hold whether or
           not the new file was ever checked. An old file lofty cannot read
           has no tags to carry, nothing else touches the replacement, and
           only the read-back stands between a junk file and the real one. */
        let untagged = dir.join("02 - B.opus");
        std::fs::write(&untagged, b"not a container lofty knows").unwrap();
        let mut orphan = holding(&dir, 2, "02 - B.opus", Status::Have);
        assert!(tag::read(&untagged).is_err(), "the fixture has readable tags");
        assert!(swap_in(&tx, &mut orphan, &junk, "flac", &HashSet::new()).is_err());
        assert!(untagged.is_file(), "an unreadable replacement took the old file");
        assert_eq!(std::fs::read(&untagged).unwrap(), b"not a container lofty knows");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /* `alac` and `m4a` share an extension, so the new file lands exactly where
       the old one is. Everything else about the swap has somewhere to stand;
       this case has to remove the old file first, and only once the
       replacement is written and verified somewhere else. */
    #[test]
    fn a_swap_within_one_extension_replaces_the_file_in_place() {
        let dir = scratch("inplace");
        let old = dir.join("01 - A.m4a");
        if super::tests::tiny_opus(&dir.join("seed.opus")).is_none()
            || tag::transcode(&dir.join("seed.opus"), &old, "m4a", Some("opus")).is_err()
        {
            eprintln!("no ffmpeg or no aac encoder, skipping");
            return;
        }
        assert_eq!(tag::mp4_format(&old), Some("m4a"));

        let fresh = dir.join(".earworm-convert-1.m4a");
        if tag::transcode(&old, &fresh, "alac", Some("m4a")).is_err() {
            eprintln!("no alac encoder, skipping");
            return;
        }
        let (tx, _rx) = mpsc::channel();
        let mut track = holding(&dir, 1, "01 - A.m4a", Status::Have);
        let stale = swap_in(&tx, &mut track, &fresh, "alac", &HashSet::new()).unwrap();
        // The replacement landed on the old name, so there is nothing to drop.
        assert_eq!(stale, None, "an in-place swap named a file to remove");

        assert_eq!(track.path.as_deref(), Some(old.as_path()));
        assert!(old.is_file());
        assert!(!fresh.exists(), "the scratch file was left in the folder");
        // The extension never changed, so only the codec says it worked.
        assert_eq!(tag::mp4_format(&old), Some("alac"));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /* A download that never happened leaves the track exactly as it was: the
       mp3, its manifest entry and its row. Converting it anyway would be the
       silent downgrade the plan rules out. */
    #[test]
    fn a_refetch_that_never_downloaded_keeps_the_file_it_had() {
        let dir = scratch("nofetch");
        let old = dir.join("01 - A.mp3");
        std::fs::write(&old, "audio").unwrap();
        let cfg = super::tests::config(true);
        let (tx, rx) = mpsc::channel();

        let mut tracks = vec![holding(&dir, 1, "01 - A.mp3", Status::Have)];
        // yt-dlp wrote nothing, so `path` is still what it was handed.
        let was = HashMap::from([(1usize, old.clone())]);
        let (done, missing, broke) = adopt_refetched(&cfg, &tx, &mut tracks, &was);

        assert_eq!((done, missing, broke), (0, 1, 0));
        assert!(old.is_file());
        assert_eq!(tracks[0].path.as_deref(), Some(old.as_path()));
        assert_eq!(tracks[0].status, Status::Have);
        assert!(
            rx.try_iter().any(|m| matches!(
                m,
                Msg::Update { note: Some(n), .. } if n.contains("could not refetch")
            )),
            "nothing on the row said why it was left"
        );
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /* The bug this pair of fixes exists for. A conversion renames every track
       it touches, and the `.m3u8` is written once at the end of the pass, so
       quitting part-way left it naming the old files. Opening that folder from
       the library then ran `mark_departed`, whose filename match failed for
       every converted track while the unconverted ones still matched, which
       got it past the "believe nothing" guard and called each converted track
       departed. Per CLAUDE.md that renumbers them past the playlist, stops
       them being tagged, refuses them a rename and drops them from the next
       playlist written. */
    #[test]
    fn a_half_converted_folder_is_not_read_as_a_dozen_departures() {
        let dir = scratch("halfway");
        let mut names: Vec<String> = (1..=4).map(|n| format!("{n:02} - Track {n}.opus")).collect();
        for name in &names {
            std::fs::write(dir.join(name), "audio").unwrap();
        }
        let playlist = dir.join(format!(
            "{}.m3u8",
            dir.file_name().unwrap().to_string_lossy()
        ));
        let body = format!("#EXTM3U\n{}\n", names.join("\n"));
        std::fs::write(&playlist, &body).unwrap();

        /* Two of the four converted, which is the case the guard does not
           catch: with all four renamed nothing matches and the file is not
           believed at all. */
        let mut replaced: Vec<String> = Vec::new();
        for name in names.iter_mut().take(2) {
            let old = dir.join(&*name);
            let new = old.with_extension("flac");
            std::fs::rename(&old, &new).unwrap();
            repoint_playlist(&dir, old.file_name().unwrap(), new.file_name().unwrap());
            replaced.push(name.clone());
            *name = new.file_name().unwrap().to_string_lossy().to_string();
        }

        // The playlist names four files and all four are on disk.
        let written = std::fs::read_to_string(&playlist).unwrap();
        for name in &names {
            assert!(written.contains(name.as_str()), "{name} missing from {written:?}");
            assert!(dir.join(name).is_file(), "{name} is not on disk");
        }
        for gone in &replaced {
            assert!(!written.contains(gone.as_str()), "{gone} survived in {written:?}");
        }
        // The two that were not touched keep the names they had.
        assert_eq!(written.matches(".opus").count(), 2, "{written:?}");
        assert!(written.starts_with("#EXTM3U"), "the header was lost: {written:?}");

        let mut tracks: Vec<Track> = names
            .iter()
            .enumerate()
            .map(|(n, name)| holding(&dir, n + 1, name, Status::Have))
            .collect();
        mark_departed(&dir, &mut tracks);
        assert!(
            tracks.iter().all(|t| t.status == Status::Have),
            "a converted track was called departed: {:?}",
            tracks.iter().map(|t| (t.index, t.status)).collect::<Vec<_>>()
        );
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /* The second half, for the instant `repoint_playlist` cannot cover: the
       rename has happened and the playlist write has not, or the folder was
       synced with --no-m3u8 and given one later. The stem is what a
       conversion leaves alone. */
    #[test]
    fn a_departure_is_judged_on_the_stem_so_an_extension_change_is_not_one() {
        let dir = scratch("stale");
        let playlist = dir.join(format!(
            "{}.m3u8",
            dir.file_name().unwrap().to_string_lossy()
        ));
        // Deliberately never repointed: this is the window, written out.
        std::fs::write(&playlist, "#EXTM3U\n01 - A.opus\n02 - B.opus\n").unwrap();
        for name in ["01 - A.flac", "02 - B.opus", "09 - Old.opus"] {
            std::fs::write(dir.join(name), "audio").unwrap();
        }

        let mut tracks = vec![
            holding(&dir, 1, "01 - A.flac", Status::Have),
            holding(&dir, 2, "02 - B.opus", Status::Have),
            holding(&dir, 9, "09 - Old.opus", Status::Have),
        ];
        mark_departed(&dir, &mut tracks);
        assert_eq!(tracks[0].status, Status::Have, "the converted track was called departed");
        assert_eq!(tracks[1].status, Status::Have);
        /* And a real departure is still one: a file the playlist has never
           named is what this check exists to find, and loosening the match
           must not have cost that. */
        assert_eq!(tracks[2].status, Status::Gone, "a real departure went unnoticed");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// A Hangul filename, composed here and decomposed the moment the folder is
    /// copied off the machine. Built rather than typed, so the form is the
    /// test's and not whatever normalisation this file was saved in.
    fn korean() -> String {
        let name = composed("01 - 안예은 - 봄이 온다면.opus");
        assert_ne!(name, decomposed(&name), "the fixture has nothing to decompose");
        name
    }

    fn one_listed(dir: &Path, name: &str) -> Vec<Track> {
        let mut track = Track::new(1, "v1".into(), name.into(), dir.join(name));
        track.listed = true;
        vec![track]
    }

    /* Composed entries resolve on APFS, which ignores the difference, and name
       nothing on an iPhone, which compares bytes. Verified in VLC on iOS. */
    #[test]
    fn the_playlist_names_its_tracks_decomposed() {
        let dir = scratch("nfd");
        let name = korean();
        std::fs::write(dir.join(&name), "audio").unwrap();

        let cfg = crate::worker::tests::config(true);
        let playlist = write_playlist(&cfg, &one_listed(&dir, &name)).unwrap().unwrap();
        let body = std::fs::read_to_string(&playlist).unwrap();
        assert!(body.contains(&format!("\n{}\n", decomposed(&name))), "{body:?}");
        assert!(
            !body.contains(&format!("\n{name}\n")),
            "the composed name reached the file: {body:?}"
        );
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /* The half the first one rests on. Decomposed entries against composed
       filenames miss for the Korean track while the ASCII one still matches,
       which is exactly enough to clear the "believe none of it" guard and call
       a track that never left departed. */
    #[test]
    fn a_decomposed_entry_is_not_read_as_a_departure() {
        let dir = scratch("nfdgone");
        let name = korean();
        for file in [name.as_str(), "02 - Plain.opus"] {
            std::fs::write(dir.join(file), "audio").unwrap();
        }
        let playlist = dir.join(format!("{}.m3u8", dir.file_name().unwrap().to_string_lossy()));
        std::fs::write(
            &playlist,
            format!("#EXTM3U\n{}\n02 - Plain.opus\n", decomposed(&name)),
        )
        .unwrap();

        let mut tracks = vec![
            holding(&dir, 1, &name, Status::Have),
            holding(&dir, 2, "02 - Plain.opus", Status::Have),
        ];
        mark_departed(&dir, &mut tracks);
        assert_eq!(tracks[0].status, Status::Have, "the Korean track was called departed");
        assert_eq!(tracks[1].status, Status::Have);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /* The two halves as one path, which is the bug as it was reported: sync a
       folder with Korean filenames, open it from the library, and every one of
       them reads as a departure. Each half is pinned separately above, and both
       stay green if a change moves the decomposition somewhere the other half
       does not look. This is the only test that fails when they disagree. */
    #[test]
    fn a_korean_folder_written_then_read_back_holds_no_departures() {
        let dir = scratch("nfdround");
        let name = korean();
        for file in [name.as_str(), "02 - Plain.opus"] {
            std::fs::write(dir.join(file), "audio").unwrap();
        }
        let mut tracks = vec![
            holding(&dir, 1, &name, Status::Have),
            holding(&dir, 2, "02 - Plain.opus", Status::Have),
        ];
        for track in &mut tracks {
            track.listed = true;
        }

        let cfg = crate::worker::tests::config(true);
        let playlist = write_playlist(&cfg, &tracks).unwrap().unwrap();
        assert!(playlist.is_file(), "no playlist was written");

        mark_departed(&dir, &mut tracks);
        assert!(
            tracks.iter().all(|t| t.status == Status::Have),
            "a track the playlist names was called departed: {:?}",
            tracks.iter().map(|t| (&t.name, t.status)).collect::<Vec<_>>()
        );
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /* `from` always comes off disk composed, so a byte match finds neither the
       decomposed entries this writes now nor the composed ones in every .m3u8
       written before it. Missing one leaves the converted track named by a file
       that has gone, which is the departure bug this whole pair guards. */
    #[test]
    fn a_repoint_finds_an_entry_in_either_form() {
        for (label, written) in [("composed", korean()), ("decomposed", decomposed(&korean()))] {
            let dir = scratch(&format!("nfdrepoint-{label}"));
            let name = korean();
            let playlist =
                dir.join(format!("{}.m3u8", dir.file_name().unwrap().to_string_lossy()));
            std::fs::write(&playlist, format!("#EXTM3U\n{written}\n")).unwrap();

            let to = Path::new(&name).with_extension("flac");
            let to = to.file_name().unwrap();
            repoint_playlist(&dir, Path::new(&name).as_os_str(), to);

            let body = std::fs::read_to_string(&playlist).unwrap();
            let want = decomposed(&to.to_string_lossy());
            assert!(body.contains(&want), "{label} entry not repointed: {body:?}");
            assert!(!body.contains(".opus"), "{label} old entry survived: {body:?}");
            std::fs::remove_dir_all(&dir).unwrap();
        }
    }

    /* A killed ffmpeg never reaches the cleanup in the loop's own error arm,
       so its part-written output stays in the user's music folder. Writing the
       same name again covers only a repeat of the same track into the same
       format, and a format change in between strands it for good. */
    #[test]
    fn a_killed_conversion_leaves_no_scratch_file_behind() {
        let dir = scratch("sweep");
        // What a previous run into a different format would have left.
        let orphans = [".earworm-convert-1.flac", ".earworm-convert-7.m4a"];
        for name in orphans {
            std::fs::write(dir.join(name), "half a track").unwrap();
        }
        // And the files it must not touch, hidden ones included.
        let keep = ["01 - A.opus", "cover.jpg", ".DS_Store", ".earworm"];
        for name in keep {
            std::fs::write(dir.join(name), "mine").unwrap();
        }

        sweep_scratch(&dir);
        for name in orphans {
            assert!(!dir.join(name).exists(), "{name} was left behind");
        }
        for name in keep {
            assert!(dir.join(name).is_file(), "{name} was swept away");
        }
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// Off the filenames the shelf already holds, so the library screen can
    /// ask it per frame without a stat or a worker round trip.
    #[test]
    fn the_off_format_count_reads_the_filenames_and_skips_what_is_missing() {
        let shelf = Shelf {
            path: PathBuf::from("/tmp/x"),
            name: "X".into(),
            url: "u".into(),
            tracks: 3,
            missing: 1,
            synced: None,
            files: vec![
                ("01 - a.opus".into(), true),
                ("02 - b.mp3".into(), true),
                ("03 - c.flac".into(), true),
                // Not on disk, so nothing is going to convert it.
                ("04 - d.mp3".into(), false),
            ],
        };
        assert_eq!(crate::app::off_format(&shelf, "opus"), 2);
        assert_eq!(crate::app::off_format(&shelf, "mp3"), 2);
        assert_eq!(crate::app::off_format(&shelf, "flac"), 2);
        // vorbis writes .ogg, so the name is not the extension to compare.
        assert_eq!(crate::app::off_format(&shelf, "vorbis"), 3);
    }
}
#[cfg(test)]
mod purge_tests {
    use super::*;
    use crate::app::{Msg, Reply};
    use std::sync::mpsc::{self, Receiver};

    fn scratch(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "earworm-purge-{tag}-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// One reply to the one question, then everything sent after it.
    fn answer(rx: &Receiver<Msg>, reply: Reply) -> Vec<Msg> {
        let mut seen = Vec::new();
        for msg in rx {
            let asked = matches!(msg, Msg::Ask(..));
            if let Msg::Ask(_, back) = &msg {
                back.send(reply.clone()).unwrap();
            }
            seen.push(msg);
            if asked {
                break;
            }
        }
        seen
    }

    /// A folder with two listed tracks and two departed files: one whose
    /// departure a listing proved, one read off the playlist file offline.
    fn folder() -> (PathBuf, Vec<Track>) {
        let dir = scratch("folder");
        let mut tracks = Vec::new();
        for (i, name) in ["01 - A.opus", "02 - B.opus"].iter().enumerate() {
            std::fs::write(dir.join(name), "audio").unwrap();
            let mut t = Track::new(i + 1, format!("keep{i}"), (*name).into(), dir.join(name));
            t.status = Status::Have;
            t.listed = true;
            tracks.push(t);
        }
        for (i, (name, proven)) in [("08 - Proven.opus", true), ("09 - Offline.opus", false)]
            .iter()
            .enumerate()
        {
            std::fs::write(dir.join(name), "audio").unwrap();
            let mut t = Track::new(8 + i, format!("gone{i}"), (*name).into(), dir.join(name));
            t.status = Status::Gone;
            t.departure_proven = *proven;
            tracks.push(t);
        }
        manifest::write(
            &dir,
            "u",
            tracks
                .iter()
                .map(|t| (t.id.clone(), t.path.clone().unwrap())),
        )
        .unwrap();
        (dir, tracks)
    }

    fn cfg_for(dir: &Path) -> Config {
        let mut cfg = super::tests::config(true);
        cfg.dir = dir.parent().unwrap().to_path_buf();
        cfg
    }

    fn run_purge(cfg: &Config, tracks: &mut Vec<Track>, reply: Reply) -> (Result<()>, Vec<Msg>) {
        let (tx, rx) = mpsc::channel();
        let asker = Asker { tx: tx.clone(), enabled: true };
        let (out, mut seen) = std::thread::scope(|scope| {
            let worker = scope.spawn(|| purge(cfg, &tx, tracks, &asker));
            let seen = answer(&rx, reply);
            (worker.join().unwrap(), seen)
        });
        drop(asker);
        drop(tx);
        seen.extend(rx.try_iter());
        (out, seen)
    }

    /* The whole distinction the flag exists for. A listing that asked for
       everything and got it is the one thing a deletion may rest on; the
       playlist file is the last sync's answer written down, and a narrowed
       run writes a short one. */
    #[test]
    fn only_a_complete_listing_proves_a_departure() {
        let dir = scratch("proven");
        let mut listed = vec![Track::new(1, "keep".into(), "01 - A.opus".into(), dir.join("01 - A.opus"))];
        std::fs::write(dir.join("01 - A.opus"), "audio").unwrap();
        std::fs::write(dir.join("09 - Old.opus"), "audio").unwrap();
        manifest::write(
            &dir,
            "u",
            [
                ("keep".to_string(), dir.join("01 - A.opus")),
                ("dropped".to_string(), dir.join("09 - Old.opus")),
            ]
            .into_iter(),
        )
        .unwrap();
        note_departures(&super::tests::config(true), &mpsc::channel().0, true, &mut listed);
        let gone = listed.iter().find(|t| t.status == Status::Gone).expect("no departure row");
        assert!(gone.departure_proven, "a listing's departure was not marked proven");

        // The same fact read offline, off the playlist file, is not proof.
        std::fs::write(dir.join(format!("{}.m3u8", dir.file_name().unwrap().to_string_lossy())),
            "#EXTM3U\n01 - A.opus\n").unwrap();
        let mut offline = vec![
            Track::new(1, "keep".into(), "01 - A.opus".into(), dir.join("01 - A.opus")),
            Track::new(2, "dropped".into(), "09 - Old.opus".into(), dir.join("09 - Old.opus")),
        ];
        mark_departed(&dir, &mut offline);
        assert_eq!(offline[1].status, Status::Gone, "the fixture is not a departure at all");
        assert!(!offline[1].departure_proven, "an offline read was marked proven");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn purge_deletes_the_proven_departures_and_nothing_else() {
        let (dir, mut tracks) = folder();
        let cfg = cfg_for(&dir);
        let (out, seen) = run_purge(&cfg, &mut tracks, Reply::Choice(1));
        out.unwrap();

        assert!(!dir.join("08 - Proven.opus").exists(), "the proven departure stayed");
        assert!(dir.join("09 - Offline.opus").is_file(), "an offline departure was deleted");
        assert!(dir.join("01 - A.opus").is_file() && dir.join("02 - B.opus").is_file());

        // The row went with the file, and only that row.
        assert_eq!(tracks.len(), 3);
        assert!(tracks.iter().all(|t| t.id != "gone0"));
        assert!(
            seen.iter().any(|m| matches!(m, Msg::Dropped(ids) if ids == &vec![8usize])),
            "the screen was not told which row to drop"
        );
        /* The raw lines, not `manifest::read`, which drops an entry whose
           file is gone and so would pass here whether or not the sidecar was
           rewritten. A stale line is what the library screen counts as a
           missing file until the next sync, which is the reason the save is
           there at all. */
        let left: Vec<String> = manifest::entries(&dir).into_iter().map(|(id, _)| id).collect();
        assert!(!left.contains(&"gone0".to_string()), "the sidecar still names the deleted file: {left:?}");
        assert_eq!(left.len(), 3, "{left:?}");
        // The question named the file it was about.
        let asked = seen.iter().find_map(|m| match m {
            Msg::Ask(crate::app::Prompt::Choice { header, note, .. }, _) => Some((header.clone(), note.clone())),
            _ => None,
        }).expect("nothing was asked");
        assert!(asked.0.contains("1 departed file"), "{}", asked.0);
        assert!(asked.1.contains("08 - Proven.opus"), "{}", asked.1);
        assert!(!asked.1.contains("09 - Offline"), "offered to delete an unproven one: {}", asked.1);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// Keeping and escaping are both "no", and no means no file moved.
    #[test]
    fn saying_no_or_escaping_deletes_nothing() {
        for (what, reply) in [("keep", Reply::Choice(0)), ("escape", Reply::Cancel)] {
            let (dir, mut tracks) = folder();
            let cfg = cfg_for(&dir);
            let before = tracks.len();
            let (out, seen) = run_purge(&cfg, &mut tracks, reply);
            out.unwrap();
            assert!(dir.join("08 - Proven.opus").is_file(), "{what} deleted a file");
            assert_eq!(tracks.len(), before, "{what} dropped a row");
            assert!(!seen.iter().any(|m| matches!(m, Msg::Dropped(_))));
            assert_eq!(manifest::read(&dir).len(), 4);
            std::fs::remove_dir_all(&dir).unwrap();
        }
    }

    /* The row says `gone` and the key would do nothing, so the refusal has
       to say what would make it work: a sync is what proves a departure. */
    #[test]
    fn offline_departures_alone_are_refused_with_the_next_step() {
        let (dir, mut tracks) = folder();
        // Drop the proven one, leaving only what the playlist file said.
        tracks.retain(|t| t.id != "gone0");
        let cfg = cfg_for(&dir);
        let (tx, _rx) = mpsc::channel();
        let asker = Asker { tx, enabled: true };
        let err = purge(&cfg, &asker.tx, &mut tracks, &asker).unwrap_err().to_string();
        assert!(err.contains("S syncs"), "no next step: {err}");
        assert!(dir.join("09 - Offline.opus").is_file());

        // And with no departure at all, a different answer.
        tracks.retain(|t| t.status != Status::Gone);
        let err = purge(&cfg, &asker.tx, &mut tracks, &asker).unwrap_err().to_string();
        assert!(err.contains("nothing here has left"), "{err}");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// The paths come from a manifest somebody could have edited by hand.
    #[test]
    fn a_path_outside_the_library_stops_the_whole_purge() {
        let (dir, mut tracks) = folder();
        let elsewhere = scratch("elsewhere").join("stray.opus");
        std::fs::write(&elsewhere, "audio").unwrap();
        let mut stray = Track::new(20, "stray".into(), "stray.opus".into(), elsewhere.clone());
        stray.status = Status::Gone;
        stray.departure_proven = true;
        tracks.push(stray);

        /* The playlist folder is the library here, so a sibling scratch
           directory is genuinely outside it. With `--dir` set to the temp
           root, as the other tests do, the stray would be inside after all,
           the guard would rightly stay quiet, and the worker would block on a
           question nobody in this test answers. */
        let mut cfg = super::tests::config(true);
        cfg.dir = dir.clone();
        let (tx, _rx) = mpsc::channel();
        let asker = Asker { tx, enabled: true };
        let err = purge(&cfg, &asker.tx, &mut tracks, &asker).unwrap_err().to_string();
        assert!(err.contains("outside"), "{err}");
        // Nothing at all, not merely the stray: the question was never asked.
        assert!(elsewhere.is_file());
        assert!(dir.join("08 - Proven.opus").is_file());
        std::fs::remove_dir_all(&dir).unwrap();
        std::fs::remove_dir_all(elsewhere.parent().unwrap()).unwrap();
    }
}
