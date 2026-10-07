use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;
use std::sync::mpsc::{self, Receiver, Sender};

use anyhow::{Context, Result, bail};

use crate::app::{Asker, Cmd, Escape, Msg, Reply, Shelf, Status, Track, Watch};
use crate::cheats::{Feature, LOCKED, Unlocked};
use crate::config::{self, Config, composed, decomposed};
use crate::custom;
use crate::export;
use crate::lookup;
use crate::manifest;
use crate::player;
use crate::playlists;
use crate::tag::{self, CoverState};
use crate::ytdlp;

pub fn run(mut cfg: Config, tx: Sender<Msg>, cancel: Arc<AtomicBool>, cmds: Receiver<Cmd>) {
    let mut tracks = Vec::new();
    let result = match ask_start(&mut cfg, &tx) {
        Start::Playlist => pipeline(&cfg, &tx, &cancel, &mut tracks, Pass::plain(cfg.pick)),
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
    /* A locked link from the command line lands at the prompt, typed out,
       rather than failing: the console can unlock it from there. */
    if !cfg.url.is_empty() {
        if !locked(&cfg.url, &cfg.unlocked) {
            return Start::Playlist;
        }
        let _ = tx.send(Msg::Stage("waiting for a playlist URL".into()));
        let typed = cfg.url.clone();
        return match prompt_url(tx, Escape::Quit, &cfg.unlocked, &typed) {
            Some(url) => {
                cfg.url = url;
                Start::Playlist
            }
            None => Start::Cancelled,
        };
    }
    let saved = library_all(&cfg.dir);
    if cfg.resync {
        return Start::Resync(saved);
    }
    if !saved.is_empty() {
        return Start::Library(saved);
    }

    let _ = tx.send(Msg::Stage("waiting for a playlist URL".into()));
    match prompt_url(tx, Escape::Quit, &cfg.unlocked, "") {
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
fn prompt_url(tx: &Sender<Msg>, escape: Escape, unlocked: &Unlocked, typed: &str) -> Option<String> {
    let asker = Asker {
        tx: tx.clone(),
        enabled: true,
    };
    let mut header = if locked(typed, unlocked) {
        LOCKED.to_string()
    } else {
        "YouTube playlist URL".to_string()
    };
    let mut typed = typed.to_string();
    /* The first thing a new user is ever asked, on a screen with nothing else
       on it yet. Both facts they need before typing: that one video is a
       valid answer, and what the only other key does. */
    let note = match escape {
        Escape::Quit => "a playlist or a single video  ·  esc quits",
        _ => "a playlist or a single video",
    };
    loop {
        let answer = asker.input_noted(&header, note, &typed, escape)?;
        // Asked of the answer, not the opening: the console can unlock mid-prompt.
        header = match youtube_url(&answer) {
            Some(url) if !locked(&url, unlocked) => return Some(url),
            Some(_) => LOCKED.into(),
            None if answer.trim().is_empty() => "Paste a YouTube link".into(),
            None => "Not a YouTube link".into(),
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

const MUSIC_HOST: &str = "music.youtube.com";

/// A YouTube Music link, while the console has not unlocked them.
fn locked(input: &str, unlocked: &Unlocked) -> bool {
    !unlocked.has(Feature::Music)
        && youtube_url(input).is_some_and(|url| split_url(&url).is_some_and(|(host, _)| host == MUSIC_HOST))
}

/// The lowercased host and the path after it, credentials and port dropped.
fn split_url(full: &str) -> Option<(String, &str)> {
    let rest = full.split_once("://")?.1;
    let (authority, path) = rest.split_once('/').unwrap_or((rest, ""));
    let host = authority
        .rsplit('@')
        .next()?
        .split(':')
        .next()?
        .to_ascii_lowercase();
    Some((host, path))
}

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

    let (host, path) = split_url(&full)?;
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
            .filter(|f| player::playlist_of(f).is_some())
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
                let Some(url) = prompt_url(tx, Escape::Keep, &cfg.unlocked, "") else {
                    continue;
                };
                let _ = tx.send(Msg::Restart);
                tracks.clear();
                cfg.url = url;
                // As in `start_url`: this playlist is not the last one.
                cfg.folder = None;
                let gate = cfg.pick;
                result = pipeline(cfg, tx, cancel, tracks, Pass::plain(gate)).map_err(|e| e.to_string());
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
/// manifests and the directory alone: this runs before the UI is up and must
/// not touch the network.
/* A folder earworm downloaded is known by its sidecar; a folder somebody
   copied in is known by holding audio. Both are listed, because the tags and
   the filenames are everything the library screen draws and nothing else on
   that screen depends on where the files came from. The sidecar wins when
   there is one, since it carries the playlist's order and the files a sync
   found missing, neither of which a directory read can tell you. */
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
            /* Asked before the stat pass, since a hidden folder is one
               nothing on any screen is going to draw. */
            if sidecar.hidden {
                return None;
            }
            // One stat per entry, feeding both the count and the preview.
            let files: Vec<(String, bool)> = contents(&path, &sidecar)
                .iter()
                .map(|(_, f)| {
                    let name = f.file_name().unwrap_or_default().to_string_lossy().into();
                    (name, f.is_file())
                })
                .collect();
            /* A folder with neither audio nor a recorded URL is some other
               directory under `--dir` and not a playlist at all. The URL is
               what keeps an emptied folder of ours on the list: it is still
               syncable, and dropping it would hide the one row that says
               every file went. */
            if files.is_empty() && sidecar.url.is_none() {
                return None;
            }
            let missing = files.iter().filter(|(_, here)| !here).count();
            /* Before the sidecar is taken apart below, and from the same
               resolver the sync and `open_shelf` ask, so the row cannot
               disagree with what a run would decide about the same folder. */
            let kind = manifest::kind_of(&path, &sidecar).map(|(kind, _)| kind);
            Some(Shelf {
                name: path.file_name().unwrap_or_default().to_string_lossy().into(),
                url: sidecar.url,
                kind,
                tracks: files.len() - missing,
                missing,
                synced: sidecar.synced,
                files,
                playlist: player::playlist_of(&path),
                path,
            })
        })
        .collect();
    found.sort_by(|a, b| a.name.cmp(&b.name));
    found
}

/// What the library screen draws: the folders, then the custom playlists, by name.
// `library` stays folders only. Resync, `--list`, `--check` and the duplicate checks all treat a row as a
// directory to sync or read, and a custom playlist is a list of ids that is none of those.
pub fn library_all(dir: &Path) -> Vec<Shelf> {
    let mut found = library(dir);
    let lists = playlists::list(dir);
    if lists.is_empty() {
        return found;
    }
    let others = Others::everything(dir);
    for list in lists {
        let resolved = resolve_entries(&others, dir, &list.entries);
        let files: Vec<(String, bool)> = list
            .entries
            .iter()
            .zip(&resolved)
            .map(|(entry, file)| {
                let name = file.as_deref().unwrap_or(&entry.hint).file_name().unwrap_or_default();
                (name.to_string_lossy().to_string(), file.is_some())
            })
            .collect();
        let present = files.iter().filter(|(_, here)| *here).count();
        found.push(Shelf {
            path: playlists::path_of(dir, &list.name),
            name: list.name,
            tracks: present,
            missing: files.len() - present,
            files,
            kind: Some(manifest::Kind::Custom),
            ..Shelf::default()
        });
    }
    found.sort_by(|a, b| a.name.cmp(&b.name));
    found
}

/// The folders `library` is leaving out, by name.
/* Hiding takes a folder off every screen and out of `--resync`, and the only
   word it ever gets is one flash at the moment it happens. A month later
   nothing could say why four folders were being passed over, which is the
   same silence a sync that decides without reporting would have. `--check` is
   where somebody working that out actually looks. */
pub fn hidden(dir: &Path) -> Vec<String> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut found: Vec<String> = entries
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.is_dir())
        .filter(|p| manifest::load(p).hidden)
        .map(|p| p.file_name().unwrap_or_default().to_string_lossy().into())
        .collect();
    found.sort();
    found
}

/// What a folder holds, as id and file, which is what both the library row
/// and the track list are built from.
/* The sidecar wins for a folder with a playlist behind it: it carries the
   order the playlist is in and the files a sync found missing, neither of
   which a directory read can tell you, and a file the user dropped in is the
   sync's business to place rather than something to invent a row for here.

   Without a URL the sidecar is only a tag ledger. It records which files
   happened to be in the folder the last time somebody edited a tag, which is
   no claim about anything, so the directory is authoritative and the sidecar
   supplies ids for the files it already knows. Preferring it outright meant a
   song copied into an adopted folder never appeared at all: no sync will ever
   run there to notice it, because there is no URL for one to run against. */
fn contents(folder: &Path, sidecar: &manifest::Sidecar) -> Vec<(String, PathBuf)> {
    if sidecar.url.is_some() && !sidecar.entries.is_empty() {
        return sidecar.entries.clone();
    }
    let known: HashMap<&Path, &str> = sidecar
        .entries
        .iter()
        .map(|(id, file)| (file.as_path(), id.as_str()))
        .collect();
    audio_files(folder)
        .into_iter()
        .map(|file| {
            let id = known.get(file.as_path()).map_or_else(
                || {
                    let name = file.file_name().unwrap_or_default().to_string_lossy();
                    manifest::local_id(&name)
                },
                |id| (*id).to_string(),
            );
            (id, file)
        })
        .collect()
}

/// Audio files directly in `folder`, in name order, which for a folder
/// earworm wrote is playlist order because every name carries its number.
/* Filtered on the extensions `FORMATS` can produce rather than on anything
   wider: a folder holding artwork, a text file and one `.m4a` is a one-track
   playlist, and a `.DS_Store` beside it must not be counted as a track. */
pub(crate) fn audio_files(folder: &Path) -> Vec<PathBuf> {
    let Ok(entries) = std::fs::read_dir(folder) else {
        return Vec::new();
    };
    let mut files: Vec<PathBuf> = entries
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.is_file())
        /* Hidden files are never tracks, and the extension filter alone does
           not catch them: macOS writes `._01 - A.opus` beside every file on a
           FAT or exFAT volume, which is exactly how a folder arrives off a
           USB stick, and that is a one-byte row with unreadable tags. */
        .filter(|p| {
            !p.file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.starts_with('.'))
        })
        .filter(|p| {
            p.extension()
                .and_then(|e| e.to_str())
                .is_some_and(|ext| {
                    config::FORMATS
                        .iter()
                        .any(|(_, known, _)| ext.eq_ignore_ascii_case(known))
                })
        })
        .collect();
    files.sort();
    files
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
    let sidecar = manifest::load(&shelf.path);
    let entries = contents(&shelf.path, &sidecar);
    if entries.is_empty() {
        bail!(
            "{} holds no audio earworm can read  ·  sync it to fill it in",
            shelf.name
        );
    }

    let _ = tx.send(Msg::Restart);
    let _ = tx.send(Msg::Playlist(shelf.name.clone()));
    let _ = tx.send(Msg::Folder(shelf.path.clone()));
    // Empty is how the rest of the tool spells "no URL", and `sync_open` is
    // what turns that into the question rather than into a refusal.
    cfg.url = shelf.url.clone().unwrap_or_default();
    /* Carried onto the config so a later `S` writes back into this folder
       rather than into whatever the playlist is called upstream. */
    cfg.folder = sidecar.name.clone();
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
        /* A local id in a folder that has a playlist behind it is a file the
           sync could not place: the user's own, in no playlist and never in
           one. Read back as `Have` and `listed`, `mark_departed` below finds
           it missing from the `.m3u8` the sync correctly left it out of and
           calls it `gone`, which claims a video left a playlist the file was
           never in. In a folder with no URL every id is local and every file
           is genuinely part of it, which is why the URL is half the test. */
        let own = manifest::is_local(&id) && shelf.url.is_some();
        let mut track = Track::new(index, id, name, file.clone());
        track.status = if own { Status::Local } else { Status::Have };
        track.source = if own { "not in playlist" } else { "on disk" }.into();
        track.listed = !own;
        if let Ok(info) = tag::read(&file) {
            track.artist = info.artist;
            track.title = info.title;
            track.album = info.album;
            track.year = info.year;
            track.duration = info.duration;
            track.isrc = info.isrc;
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

    /* Only for a folder with a playlist upstream. `mark_departed` reads the
       `.m3u8` as earworm's own last answer about what that playlist held, and
       in a folder earworm did not download it is not that: it is whatever
       file the user brought with them, naming whatever they chose to list.
       Believing it would call every file it leaves out departed. */
    if shelf.url.is_some() {
        mark_departed(&shelf.path, tracks);
    }

    /* Two files can carry the same number: removing a video renumbers the
       tracks after it, and the file that left keeps the name it had, so a
       folder ends up with two `05 - `. Every command finds its track by index
       and takes the first match, so sharing one makes a row write tags to
       another row's file. Tracks still in the playlist claim their own number
       first, and everything else is numbered past them, which is where a
       departed row sits after a real sync too. A local file is "everything
       else" as much as a departure is: it carries whatever number the user
       was numbering by, which collides with the playlist's, and whichever
       happens to sit first in the sidecar would otherwise take it. That is
       how a real track 5 ends up at 11, sorted to the bottom and renamed to
       `11 - ` by the first edit. */
    let mut taken: HashSet<usize> = HashSet::new();
    for track in tracks.iter_mut() {
        let live = !matches!(track.status, Status::Gone | Status::Local);
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
    // Before the tracks go out, so a reopened folder shows the same flags a sync did.
    flag_isrc_twins(&Others::new(cfg, &shelf.path), tracks);
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
    /* The same answer a sync reports, from the same resolver: offline there is
       no listing, so the name and the header are all there is. Said here too,
       because this is the screen `c` is pressed from. */
    if let Some((kind, source)) = manifest::kind_of(&shelf.path, &sidecar) {
        summary = format!("{summary}  ·  {} , from {source}", kind.label());
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
    /* A local file is left out of the playlist on purpose, by the sync that
       could not match it, so its absence from the `.m3u8` is the expected
       answer rather than evidence a video left. `Gone` would claim it was in
       the playlist once and, in the offline half, is the status `write_track`
       and `rename` are keyed on: it would be renumbered past the playlist and
       refused a rename, which is right for a departure and wrong for a file
       the user owns outright. */
    for track in tracks
        .iter_mut()
        .filter(|t| t.status != Status::Local)
        .filter(|t| !holds(t))
    {
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
            "no playlist folder under {} yet; sync one by URL first",
            cfg.dir.display()
        );
    }

    let mut synced = 0;
    let mut listed = 0;
    let mut skipped = 0;
    for (n, shelf) in found.iter().enumerate() {
        if cancel.load(Ordering::SeqCst) {
            break;
        }
        let name = &shelf.name;
        /* Attaching a URL to a folder earworm did not download is an
           interactive act: the matching that stops it downloading the folder
           again is only as good as the tags, and the pick gate is the last
           defence against getting it wrong. An unattended walk of the library
           has nobody at the gate, so it passes this folder by and says so. */
        let Some(url) = shelf.url.clone() else {
            skipped += 1;
            let _ = tx.send(Msg::Log(format!(
                "{name}: no playlist URL  ·  open it and press S to attach one"
            )));
            continue;
        };
        let _ = tx.send(Msg::Stage(format!("{} of {}: {name}", n + 1, found.len())));
        // Each folder starts from an empty list, so its rows are its own.
        tracks.clear();
        let _ = tx.send(Msg::Restart);
        cfg.url = url;
        cfg.folder = manifest::load(&shelf.path).name;

        match pipeline(cfg, tx, cancel, tracks, Pass::plain(false)) {
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

    let mut summary = format!("{synced} of {} playlists, {listed} tracks", found.len());
    if skipped > 0 {
        let word = if skipped == 1 { "folder has" } else { "folders have" };
        summary = format!("{summary}  ·  {skipped} {word} no URL  ·  l has which");
    }
    Ok(summary)
}

/// What kind of pass this is, as named fields: two loose bools at six call
/// sites is one transposition away from an unattended walk of the library
/// reconciling folders nobody is watching.
#[derive(Clone, Copy)]
struct Pass {
    /// `gate` is separate from `cfg.pick` because `--resync` walks the whole
    /// library unattended: a question per folder is the one thing that would
    /// stop it, and the user asked for the sync, not for each playlist in it.
    gate: bool,
    /// The one pass that matches files on disk to videos by name and tag
    /// rather than by id: the sync that gives a URL to a folder earworm did
    /// not download. Every later sync of that folder is an ordinary one,
    /// because the ids it matched are real by then.
    attaching: bool,
}

impl Pass {
    fn plain(gate: bool) -> Self {
        Pass {
            gate,
            attaching: false,
        }
    }

    /* The gate is forced on, whatever asked for the sync. The matching is a
       guess made from tags the user typed and titles YouTube chose, and the
       cost of getting it wrong is the whole playlist downloading again beside
       the folder. Somebody reading "12 to download" against a folder they
       know holds twelve is the last defence, and CLAUDE.md calls the `false`
       that `resync` passes load-bearing: this is the one thing that overrides
       it, and `resync` never reaches here because it skips a folder with no
       URL outright. */
    fn attaching() -> Self {
        Pass {
            gate: true,
            attaching: true,
        }
    }
}

fn pipeline(
    cfg: &Config,
    tx: &Sender<Msg>,
    cancel: &AtomicBool,
    tracks: &mut Vec<Track>,
    pass: Pass,
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
    /* Before the tagging pass, which is the first thing the answer changes,
       and after the scan, which is where the only automatic signal is. */
    let mut kind = settle_kind(listing.playlist_id.as_deref(), folder_of(tracks).as_deref());
    let asker = Asker {
        tx: tx.clone(),
        enabled: cfg.fix,
    };

    if pass.attaching {
        reconcile(tx, tracks);
    }
    note_departures(cfg, tx, listing.complete, tracks);
    /* After the departures, which are read off the one video, and before the
       list is shown, so the screen only ever holds the chapters. */
    let split = settle_split(cfg, &asker, &pass, listing.chapters, tracks);
    // Only when nothing has answered: a `[playlist]` name is somebody's choice, and the name wins on reopening anyway.
    if split.is_some() && kind.is_none() {
        kind = Some((manifest::Kind::Album, "its chapters"));
    }
    let album = matches!(kind, Some((manifest::Kind::Album, _)));
    // Chapters are cut from one video and have no id to look up,
    // so a split pass neither copies nor compares titles.
    let here = folder_of(tracks);
    let others = Others::new(cfg, here.as_deref().unwrap_or(Path::new("")));
    let known = match here.as_deref().filter(|_| split.is_none()) {
        Some(here) => {
            let wanted: HashSet<&str> = tracks
                .iter()
                .filter(|t| t.status == Status::Pending)
                .map(|t| t.id.as_str())
                .collect();
            let known = library_index(&others, &cfg.format, &wanted);
            // `library` stats every file, so ask it only when a row
            // is a download the id index could not account for.
            let unplaced = tracks
                .iter()
                .any(|t| t.status == Status::Pending && !known.contains_key(&t.id));
            let shelves = if unplaced { library(&cfg.dir) } else { Vec::new() };
            mark_twins(tracks, &known, &shelves, here);
            known
        }
        None => HashMap::new(),
    };
    let _ = tx.send(Msg::Tracks(tracks.clone()));

    // Held on to, because `to_bring` has to honour the same answer.
    let picked = pass.gate.then(|| ask_which(tx, tracks)).flatten();
    // After the gate, so a track nobody picked is not copied either.
    let copied = copy_known(cfg, tx, tracks, &known);

    let have = tracks.iter().filter(|t| t.status == Status::Have).count();
    let _ = tx.send(Msg::Stage(format!(
        "fetching  ({have} of {} already on disk)",
        tracks.len()
    )));

    /* Decided here and not a moment later: this is the last point at which
       every track either holds the file an earlier run left or holds nothing,
       and the download is about to overwrite `path` for anything it fetches.
       Empty unless `convert` is on. */
    // A chapter is cut from the video's own file and has no download of its own to bring across.
    let plans = if split.is_some() {
        HashMap::new()
    } else {
        to_bring(cfg, tracks, picked.as_ref())
    };
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
    if let Some(split) = &split {
        warning = fetch_split(cfg, tx, cancel, tracks, split);
    } else if let Err(err) = ytdlp::run(cfg, tracks, tx, &refetch) {
        warning = err.to_string();
        let _ = tx.send(Msg::Log(format!("download: {warning}")));
    }

    let (adopted, missing, broke) = adopt_refetched(cfg, tx, tracks, &was);
    let (encoded, unencoded) = convert_tracks(cfg, tx, cancel, tracks, &plans);
    let converted = adopted + encoded;
    // Both halves failing is one thing to the reader: it did not come across.
    let stuck = broke + unencoded;

    let _ = tx.send(Msg::Stage("tagging".into()));
    /* An image that was here before the download is the user's, and `apply`
       writing over it is the same loss as deleting it: `drop_folder_cover`
       keeps the name and nothing keeps the bytes. `theirs` is that same
       answer, taken before the download for the same reason. */
    let mut cover = CoverState {
        written: !theirs.is_empty(),
        one_sleeve: album,
        ..CoverState::default()
    };
    let extras = tag_tracks(cfg, tx, cancel, tracks, &playlist, &asker, &mut cover, None);
    if album && cfg.loudness && !cancel.load(Ordering::SeqCst) {
        album_loudness(tx, tracks);
    }

    for (index, twin) in flag_isrc_twins(&others, tracks) {
        let _ = tx.send(Msg::Twin { index, twin });
    }
    record_kind(tx, folder_of(tracks).as_deref(), kind);
    sync_manifest(cfg, tracks);
    let playlist_file = write_playlist(cfg, tracks)?;
    drop_folder_cover(tracks, &theirs, album);
    let mut summary = summarise(tracks, playlist_file.as_deref());
    if let Some(line) = extras.said() {
        summary = format!("{summary}, {line}");
    }
    /* Said out loud, because the rows scroll off and a --resync's one line
       per folder is the only record anyone reads afterwards. */
    if copied > 0 {
        summary = format!("{summary}, {copied} copied from other playlists");
    }
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
    if let Some((kind, source)) = kind {
        summary = format!("{summary}  ·  {} , from {source}", kind.label());
    }
    if !warning.is_empty() {
        summary = format!("{summary}   [{warning}]");
    }
    Ok(summary)
}

/// A single video that is being cut into its chapters.
struct Split {
    chapters: Vec<ytdlp::Chapter>,
    /// The video as the scan saw it, which is what the download is about: the
    /// chapters are cut out of this file and never fetched themselves.
    video: Track,
}

/* Whether this one video becomes its chapters, and if so the tracks that
   replace it in `tracks`. A video with two or more chapters is not
   necessarily an album: tutorials and podcasts carry timestamps too, so the
   answer is asked, and never assumed.

   Asked once. A folder that already holds this video's chapters is split
   again without a question, and one that holds the whole file was either
   declined or downloaded before this existed, so it is left alone. That is
   the only record of the choice, and it carries through every writer of the
   sidecar for free. Nothing asks without a gate: `--resync` and `--no-pick`
   walk unattended, and the safe answer for them is the video as it was. */
fn settle_split(
    cfg: &Config,
    asker: &Asker,
    pass: &Pass,
    chapters: Vec<ytdlp::Chapter>,
    tracks: &mut Vec<Track>,
) -> Option<Split> {
    let video = tracks.first()?.clone();
    let folder = video.path.as_deref()?.parent()?.to_path_buf();
    let split = manifest::is_split(&folder, &video.id);
    // Whatever a cut that was killed left, found on the next sync and not only when something is cut.
    if split {
        sweep_scratch(&folder);
    }
    if chapters.is_empty() {
        // Upstream lost its chapters: the cuts on disk are still the folder, and fetching the video again would put it back beside them.
        let held = held_chapters(&folder, &video.id);
        // One video only: a playlist that also has chapter entries for its first track is not that video.
        if !split || held.is_empty() || tracks.len() != 1 {
            return None;
        }
        *tracks = held;
        return Some(Split { chapters, video });
    }
    if !split {
        if video.status == Status::Have || !pass.gate {
            return None;
        }
        let shown: Vec<&str> = chapters.iter().take(3).map(|c| c.title.as_str()).collect();
        let note = format!(
            "{} chapters, starting {}. Splitting makes one track each, in an album folder.",
            chapters.len(),
            shown.join(", ")
        );
        let options = vec![
            format!("Split into {} tracks", chapters.len()),
            "Keep as one track".to_string(),
        ];
        if asker.choose_noted("This video has chapters", &note, options) != Some(0) {
            return None;
        }
    }
    *tracks = chapter_tracks(cfg, &video, &chapters, &folder);
    Some(Split { chapters, video })
}

fn held_chapters(folder: &Path, video: &str) -> Vec<Track> {
    let prefix = format!("{video}#");
    let mut held: Vec<(usize, PathBuf)> = manifest::entries(folder)
        .into_iter()
        .filter(|(_, file)| file.is_file())
        .filter_map(|(id, file)| Some((id.strip_prefix(&prefix)?.parse().ok()?, file)))
        .collect();
    held.sort();
    held.into_iter()
        .map(|(number, file)| {
            let stem = file.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_default();
            let mut track = Track::new(number, manifest::chapter_id(video, number), strip_number(&stem).to_string(), file);
            track.status = Status::Have;
            track
        })
        .collect()
}

// Numbered from one in listing order, and found again by the manifest on a later sync, which is what keeps a rename from costing the cut.
// An id is a position, so a creator who inserts a chapter would otherwise hand every later position the wrong
// file. The span each cut came from is recorded, and a chapter takes the file whose span it matches, wherever
// it sat before; a position whose recorded span no longer matches is cut again. A folder split before spans
// were recorded has none, and keeps trusting the position.
fn chapter_tracks(cfg: &Config, video: &Track, chapters: &[ytdlp::Chapter], folder: &Path) -> Vec<Track> {
    let ext = config::extension(&cfg.format);
    let known = manifest::read(folder);
    let prefix = format!("{}#", video.id);
    let recorded: Vec<(String, f64, f64)> =
        manifest::load(folder).chapters.into_iter().filter(|(id, _, _)| id.starts_with(&prefix)).collect();
    // A second either way is a rounding of the same moment, which is how a listing and a cut can differ.
    let near = |start: f64, end: f64, chapter: &ytdlp::Chapter| (start - chapter.start).abs() <= 1.0 && (end - chapter.end).abs() <= 1.0;
    let mut claimed: HashSet<PathBuf> = HashSet::new();
    let tracks: Vec<Track> = chapters
        .iter()
        .enumerate()
        .map(|(i, chapter)| {
            let number = i + 1;
            let id = manifest::chapter_id(&video.id, number);
            let mut path = folder.join(chapter_file(number, &chapter.title, ext));
            // The cut reads the video's file, so a chapter may not be named like it, and a file by that name is the video and not the chapter.
            if video.path.as_ref() == Some(&path) {
                path = folder.join(chapter_file(number, &format!("{} (chapter)", chapter.title), ext));
            }
            let mut track = Track::new(number, id.clone(), chapter.title.clone(), path);
            track.duration = (chapter.end - chapter.start).round() as u64;
            let own = recorded.iter().find(|(held, _, _)| *held == id);
            let by_span = recorded
                .iter()
                .filter(|(_, start, end)| near(*start, *end, chapter))
                .filter_map(|(held, _, _)| known.get(held))
                .find(|file| !claimed.contains(*file));
            match (own, by_span) {
                (Some((_, start, end)), _) if near(*start, *end, chapter) => {
                    if let Some(file) = known.get(&id) {
                        track.path = Some(file.clone());
                        track.status = Status::Have;
                    }
                }
                (_, Some(file)) => {
                    track.path = Some(file.clone());
                    track.status = Status::Have;
                }
                // The position holds a cut of some other span: cut this one, and leave that file where it is.
                (Some(_), None) => {}
                (None, None) => match known.get(&id) {
                    Some(file) => {
                        track.path = Some(file.clone());
                        track.status = Status::Have;
                    }
                    None if track.path.as_deref().is_some_and(Path::is_file) => track.status = Status::Have,
                    None => {}
                },
            }
            if track.status == Status::Have && let Some(file) = &track.path {
                claimed.insert(file.clone());
            }
            track
        })
        .collect();
    // So a folder split before spans were kept, or a chapter that moved, is described from now on.
    for (track, chapter) in tracks.iter().zip(chapters).filter(|(t, _)| t.status == Status::Have) {
        let held = recorded.iter().find(|(id, _, _)| *id == track.id);
        if held.is_none_or(|(_, start, end)| !near(*start, *end, chapter)) {
            let _ = manifest::set_chapter(folder, &track.id, chapter.start, chapter.end);
        }
    }
    tracks
}

/* The download for a split video: the whole file once, then each chapter cut
   out of it. Returns the warning the run's summary carries. The video is
   shown as one row while it downloads and the chapters replace it afterwards,
   because yt-dlp reports progress by the video's index and a chapter row
   would otherwise claim to be downloading the whole thing.

   The full file goes once every chapter has a file of its own, and not
   before: a cut that failed leaves it for the next sync to try again from
   without fetching it twice. */
fn fetch_split(
    cfg: &Config,
    tx: &Sender<Msg>,
    cancel: &AtomicBool,
    tracks: &mut [Track],
    split: &Split,
) -> String {
    if !tracks.iter().any(|t| t.status == Status::Pending) {
        return String::new();
    }
    let mut whole = vec![split.video.clone()];
    let mut warning = String::new();
    let _ = tx.send(Msg::Tracks(whole.clone()));
    if whole[0].status != Status::Have
        && let Err(err) = ytdlp::run(cfg, &mut whole, tx, &HashSet::new())
    {
        warning = err.to_string();
        let _ = tx.send(Msg::Log(format!("download: {warning}")));
    }
    let _ = tx.send(Msg::Tracks(tracks.to_vec()));
    let Some(full) = whole[0].path.clone().filter(|p| p.is_file()) else {
        return warning;
    };

    // Whatever an earlier cut was killed in the middle of.
    if let Some(folder) = full.parent() {
        sweep_scratch(folder);
        // Before the first cut: stopped in between, the folder holds the whole file and nothing else says it was split.
        if let Err(err) = manifest::set_split(folder, &split.video.id) {
            let _ = tx.send(Msg::Log(format!("split: could not record the choice: {err}")));
        }
    }
    let artist = tag::read(&full).map(|info| info.artist).unwrap_or_default();
    // The cut drops the picture, and the video's thumbnail is what a chapter the lookup cannot place falls back on.
    let art = tag::read_cover(&full);
    // By position and not by iterator: the manifest write after each cut reads every track.
    for i in 0..tracks.len() {
        if tracks[i].status != Status::Pending {
            continue;
        }
        if cancel.load(Ordering::SeqCst) {
            break;
        }
        let index = tracks[i].index;
        let (Some(chapter), Some(target)) = (split.chapters.get(index - 1), tracks[i].path.clone()) else {
            continue;
        };
        /* Cut under a scratch name and renamed into place, so a kill in the
           middle of ffmpeg leaves nothing at the name a later sync reads as a
           finished chapter. */
        let ext = target.extension().and_then(|e| e.to_str()).unwrap_or_default();
        let scratch = target.with_file_name(format!("{SCRATCH}{index}.{ext}"));
        let cut = if target == full {
            // The cut reads the file it would write over.
            Err(anyhow::anyhow!("a chapter is named like the video's own file"))
        } else {
            tag::cut(&full, &scratch, chapter.start, chapter.end, &cfg.format)
                .and_then(|()| {
                    // What the lookup has to go on: the chapter's name, under the video's artist.
                    let fields = tag::Fields {
                        artist: artist.clone(),
                        title: chapter.title.clone(),
                        ..tag::Fields::default()
                    };
                    tag::set_fields(&scratch, &fields)?;
                    art.as_deref().map_or(Ok(()), |image| tag::set_cover(&scratch, image))
                })
                .and_then(|()| std::fs::rename(&scratch, &target).map_err(Into::into))
        };
        if cut.is_err() {
            let _ = std::fs::remove_file(&scratch);
        }
        match cut {
            Ok(()) => {
                tracks[i].status = Status::Downloaded;
                let _ = tx.send(Msg::Update {
                    index,
                    status: Status::Downloaded,
                    source: None,
                    note: None,
                    name: None,
                });
                let _ = tx.send(Msg::Path { index, path: target });
                /* After each chapter, because the entry is the only record
                   that this folder was split: stopped half way, the next sync
                   still finds it, resumes from the full file and does not
                   call the video whole with chapter files lying beside it. */
                write_manifest(cfg, tracks, false);
                if let Some(folder) = full.parent() {
                    let _ = manifest::set_chapter(folder, &tracks[i].id, chapter.start, chapter.end);
                }
            }
            Err(err) => {
                tracks[i].status = Status::Failed;
                let _ = tx.send(Msg::Log(format!("cut: chapter {index}: {err}")));
                let _ = tx.send(Msg::Update {
                    index,
                    status: Status::Failed,
                    source: None,
                    note: Some(format!("cut: {err}")),
                    name: None,
                });
            }
        }
    }
    if whole_is_done(tracks, &full) {
        let _ = std::fs::remove_file(&full);
    }
    warning
}

// The full file goes when every chapter the user wants has a cut of its own. A failed chapter may be recorded
// against the video's own file, which is there and is not its cut, so the status is read as well as the file.
// A chapter left unpicked has no file and is no reason to keep the video: it downloads again if it is ever wanted,
// as an unpicked track does anywhere else.
fn whole_is_done(tracks: &[Track], full: &Path) -> bool {
    tracks.iter().all(|t| {
        t.status == Status::Skipped
            || (t.status != Status::Failed && t.path.as_deref().is_some_and(|p| p != full && p.is_file()))
    })
}

// Runs after the tagging pass, because it needs every track's own gain first.
fn album_loudness(tx: &Sender<Msg>, tracks: &[Track]) {
    let paths: Vec<&Path> = tracks
        .iter()
        .filter(|t| !matches!(t.status, Status::Gone | Status::Skipped | Status::Local))
        .filter_map(|t| t.path.as_deref())
        .filter(|p| p.is_file())
        .collect();
    let (changed, skipped) = tag::write_album_loudness(&paths);
    if changed > 0 {
        let _ = tx.send(Msg::Log(format!("loudness: album gain on {changed} tracks")));
    }
    if skipped > 0 {
        let _ = tx.send(Msg::Log(format!("loudness: album: {skipped} tracks left out, tags unreadable")));
    }
}

/// The prefix YouTube Music gives a release, which is the one automatic signal
/// worth acting on.
const ALBUM_PREFIX: &str = "OLAK5uy_";

/// What this folder is, and what said so, recording detection's answer.
/* Detection only ever says "album": nothing it can see distinguishes a
   playlist from an album reached by a `PL` link, and the two mistakes are not
   equal. Calling an album a playlist leaves today's behaviour in place;
   calling a playlist an album stamps one sleeve across a dozen records, which
   is the bug this exists to prevent. So silence means playlist and only the
   `OLAK5uy_` prefix, or the user, promotes a folder.

   Resolves and writes nothing: the answer is wanted before the download and
   the folder does not exist until the download makes it. `record_kind` is the
   other half, and runs once there is somewhere to put it. */
fn settle_kind(
    playlist_id: Option<&str>,
    folder: Option<&Path>,
) -> Option<(manifest::Kind, &'static str)> {
    let folder = folder?;
    let named = manifest::kind_of(folder, &manifest::load(folder));
    if let Some((kind, "the folder name")) = named {
        return Some((kind, "the folder name"));
    }
    if playlist_id.is_some_and(|id| id.starts_with(ALBUM_PREFIX)) {
        return Some((manifest::Kind::Album, "the playlist id"));
    }
    named
}

/// Writes down what detection worked out, once the folder is there to hold it.
/* Only what the listing decided. A name that says it needs no header, and one
   left behind by a marker somebody has since deleted would go on deciding
   with nothing on screen to explain it; `an earlier sync` is the header
   already saying the same thing.

   After the download, because before it a brand new playlist's folder does
   not exist and the write fails for that reason alone: the header was then
   never recorded on the one run that discovered it, the library showed no
   album until somebody synced a second time, and a retry in the same session
   re-resolved `playlist` and deleted the `cover.jpg` the run had just kept.
   A folder still missing here is a run where nothing downloaded at all, which
   is not worth a line in the log. */
fn record_kind(tx: &Sender<Msg>, folder: Option<&Path>, kind: Option<(manifest::Kind, &str)>) {
    let (Some(folder), Some((kind, "the playlist id" | "its chapters"))) = (folder, kind) else {
        return;
    };
    if !folder.is_dir() {
        return;
    }
    if let Err(err) = manifest::set_kind(folder, kind) {
        let _ = tx.send(Msg::Log(format!("recording the album marker: {err}")));
    }
}

/// Matches the files already in the folder to the videos the listing named,
/// on the one sync that gives a folder earworm did not download a URL.
/* The whole feature turns on this. Without it, attaching a URL downloads the
   entire playlist again beside the copies already there, because `scan`
   matches by video id and these files have none.

   Three rules, in this order, and the order is the confidence:

     1. The predicted filename, which `scan` has already tried: a track that
        matched it is `Have` before this runs. That catches a folder somebody
        made with earworm and then lost the sidecar from.
     2. The title, against the file's title tag and then against its filename
        stem with any leading number stripped. Composed and lowercased, the
        way `mark_departed` and `shelf_matches` already normalise, or every
        accented or Hangul title misses while the ASCII ones match.
     3. Duration, within two seconds, and only to break a tie between two
        files whose titles both matched. Never on its own: every other song
        is three minutes long, and a duration match alone would hand a video
        whatever file happens to be the same length.

   A rule that finds more than one file and cannot break the tie matches
   nothing, which costs a download rather than the wrong file. Everything left
   over is `Local`: on disk, in no playlist, and not `Gone`, which would claim
   a video left and let `D` delete it. */
fn reconcile(tx: &Sender<Msg>, tracks: &mut Vec<Track>) {
    let Some(folder) = folder_of(tracks) else {
        return;
    };
    /* Every audio file in the folder that no track already holds. Not the
       sidecar's `~` entries: a folder nobody has edited has no sidecar at
       all, and the files are just as much there. */
    let claimed: HashSet<PathBuf> = tracks.iter().filter_map(|t| t.path.clone()).collect();
    let mut spare: Vec<PathBuf> = audio_files(&folder)
        .into_iter()
        .filter(|f| !claimed.contains(f))
        .collect();
    if spare.is_empty() {
        return;
    }

    // Read once: a tag read per file per candidate would be one per pair.
    let facts: Vec<(PathBuf, Vec<String>, u64)> = spare
        .iter()
        .map(|file| {
            let info = tag::read(file).ok();
            let stem = file.file_stem().unwrap_or_default().to_string_lossy().to_string();
            let mut names = Vec::new();
            if let Some(info) = &info {
                names.push(normalised(&info.title));
            }
            names.push(normalised(strip_number(&stem)));
            names.retain(|n| !n.is_empty());
            (file.clone(), names, info.map_or(0, |i| i.duration))
        })
        .collect();

    let mut taken: HashSet<PathBuf> = HashSet::new();
    let mut matched = 0;
    for track in tracks.iter_mut().filter(|t| t.status == Status::Pending) {
        let wanted = normalised(&track.name);
        if wanted.is_empty() {
            continue;
        }
        let hits: Vec<&(PathBuf, Vec<String>, u64)> = facts
            .iter()
            .filter(|(file, _, _)| !taken.contains(file))
            .filter(|(_, names, _)| names.contains(&wanted))
            .collect();
        let found = match hits.len() {
            0 => continue,
            1 => hits[0],
            /* Two files claiming one title, which is where duration earns its
               place. Still only a tiebreaker: one candidate within two
               seconds or nothing, because two files that are both the right
               title and both the right length are not something a guess
               should choose between. */
            _ => {
                let close: Vec<&&(PathBuf, Vec<String>, u64)> = hits
                    .iter()
                    .filter(|(_, _, secs)| secs.abs_diff(track.duration) <= 2)
                    .collect();
                if close.len() != 1 {
                    let _ = tx.send(Msg::Log(format!(
                        "attach: {} matches {} files in this folder and none by duration  ·  \
                         it will download again",
                        track.name,
                        hits.len()
                    )));
                    continue;
                }
                close[0]
            }
        };
        taken.insert(found.0.clone());
        track.path = Some(found.0.clone());
        track.status = Status::Have;
        matched += 1;
    }

    /* Numbered past the playlist for the same reason a departure is: the
       index is what every command finds a track by, and a local file holds no
       playlist position to collide over. */
    spare.retain(|f| !taken.contains(f));
    spare.sort();
    let next = tracks.iter().map(|t| t.index).max().unwrap_or(0);
    let local = spare.len();
    for (offset, file) in spare.into_iter().enumerate() {
        let name = file.file_name().unwrap_or_default().to_string_lossy().to_string();
        let id = manifest::local_id(&name);
        let mut track = Track::new(next + offset + 1, id, name, file.clone());
        track.status = Status::Local;
        track.source = "not in playlist".into();
        track.listed = false;
        if let Ok(info) = tag::read(&file) {
            track.artist = info.artist;
            track.title = info.title;
            track.album = info.album;
            track.year = info.year;
            track.duration = info.duration;
            if !track.title.is_empty() {
                track.name = label(&track.title, &track.artist);
            }
        }
        track.was = track.name.clone();
        tracks.push(track);
    }

    let word = if local == 1 { "file" } else { "files" };
    let _ = tx.send(Msg::Log(format!(
        "attach: {matched} of the playlist's tracks were already here, \
         {local} {word} it does not list"
    )));
}

// The pick gate's `copy from` is no longer true once the copy is done or has failed.
fn unmark(tx: &Sender<Msg>, track: &mut Track) {
    if track.twin.take().is_some() {
        let _ = tx.send(Msg::Twin { index: track.index, twin: None });
    }
}

/// A copy of a video that another folder already holds.
struct Held {
    file: PathBuf,
    folder: String,
}

/// One other playlist folder under `--dir`, as its sidecar holds it.
pub(crate) struct Other {
    pub(crate) name: String,
    pub(crate) sidecar: manifest::Sidecar,
}

/// The other playlist folders under `--dir`, read at most once per pass.
// One read serves the id index, the title check and the ISRC check; a walk per question
// made a `--resync` cost the square of the library. Lazy, so a pass with nothing to ask never reads.
// Name order makes one video in three folders always copy from the same one.
pub(crate) struct Others {
    dir: PathBuf,
    here: Option<std::ffi::OsString>,
    hidden: bool,
    read: std::cell::OnceCell<Vec<Other>>,
}

impl Others {
    fn new(cfg: &Config, here: &Path) -> Self {
        Others::at(&cfg.dir, here)
    }

    pub(crate) fn at(dir: &Path, here: &Path) -> Self {
        Others {
            dir: dir.to_path_buf(),
            here: here.file_name().map(ToOwned::to_owned),
            hidden: false,
            read: std::cell::OnceCell::new(),
        }
    }

    // For a custom playlist, which keeps playing a hidden folder's files: hiding takes a row off the library
    // and nothing else, so what resolves an entry has to see the folder too.
    pub(crate) fn everything(dir: &Path) -> Self {
        Others { hidden: true, ..Others::at(dir, Path::new("")) }
    }

    pub(crate) fn all(&self) -> &[Other] {
        self.read.get_or_init(|| {
            let Ok(entries) = std::fs::read_dir(&self.dir) else {
                return Vec::new();
            };
            let mut folders: Vec<PathBuf> = entries
                .flatten()
                .map(|e| e.path())
                .filter(|p| p.is_dir() && p.file_name() != self.here.as_deref())
                .filter(|p| !p.file_name().is_some_and(|n| n.to_string_lossy().starts_with('.')))
                .collect();
            folders.sort();
            folders
                .into_iter()
                .map(|folder| (folder.file_name().unwrap_or_default().to_string_lossy().to_string(), manifest::load(&folder)))
                .filter(|(_, sidecar)| self.hidden || !sidecar.hidden)
                .map(|(name, sidecar)| Other { name, sidecar })
                .collect()
        })
    }
}

// Ids this pass wants, matched to files other folders hold. Built per pass, not per `--resync`,
// so the second folder sees what the first just fetched. Only the chosen format counts, or
// `convert` would have a second format to undo; `~` and chapter ids are no listing's.
fn library_index(others: &Others, format: &str, wanted: &HashSet<&str>) -> HashMap<String, Held> {
    let mut found: HashMap<String, Held> = HashMap::new();
    if wanted.is_empty() {
        return found;
    }
    for other in others.all() {
        for (id, file) in &other.sidecar.entries {
            if manifest::is_local(id) || manifest::is_chapter(id) {
                continue;
            }
            if !wanted.contains(id.as_str()) || found.contains_key(id) || !file.is_file() {
                continue;
            }
            if format_on_disk(file) != Some(format) {
                continue;
            }
            found.insert(id.clone(), Held { file: file.clone(), folder: other.name.clone() });
        }
    }
    found
}

// Where each entry of a custom playlist is now, or `None` for one that is gone.
// The id resolves it, so a rename of the file or its folder breaks nothing. A video id is global and
// the entry's folder only breaks a tie; a `~` id is a filename, so it only means something in its own folder.
pub(crate) fn resolve_entries(others: &Others, root: &Path, entries: &[playlists::Entry]) -> Vec<Option<PathBuf>> {
    let mut held: HashMap<&str, Vec<(&str, &PathBuf)>> = HashMap::new();
    for other in others.all() {
        for (id, file) in &other.sidecar.entries {
            held.entry(id.as_str()).or_default().push((other.name.as_str(), file));
        }
    }
    entries
        .iter()
        .map(|entry| {
            let home = entry.hint.components().next().map(|c| c.as_os_str().to_string_lossy().to_string());
            let in_home = |folder: &str| home.as_deref() == Some(folder);
            let found = held.get(entry.id.as_str()).and_then(|files| {
                let mut live = files.iter().filter(|(_, file)| file.is_file());
                let chosen = live.clone().find(|(folder, _)| in_home(folder));
                if manifest::is_local(&entry.id) { chosen } else { chosen.or_else(|| live.next()) }.map(|(_, file)| (*file).clone())
            });
            // The last resort, for a file no sidecar lists, and never one that leaves `--dir`.
            found.or_else(|| {
                Some(root.join(&entry.hint)).filter(|p| playlists::safe_hint(&entry.hint) && p.is_file())
            })
        })
        .collect()
}

// A copy and not a hardlink, so editing a tag in one folder leaves the other alone. Named after the
// source, which has the name its lookup gave it, with this playlist's number in front: a `Have` track
// is never renamed. Through a scratch name, so a killed copy is not a truncated file the scan trusts.
fn copy_known(cfg: &Config, tx: &Sender<Msg>, tracks: &mut [Track], known: &HashMap<String, Held>) -> usize {
    let Some(folder) = folder_of(tracks) else {
        return 0;
    };
    let mut copied = 0;
    // A row the gate left out is not copied, so its "copy from" would describe something that will not happen.
    for track in tracks.iter_mut().filter(|t| t.status == Status::Skipped) {
        if track.twin.as_deref().is_some_and(|t| t.starts_with("copy from ")) {
            unmark(tx, track);
        }
    }
    for track in tracks.iter_mut().filter(|t| t.status == Status::Pending) {
        let Some(held) = known.get(&track.id) else {
            continue;
        };
        let stem = held.file.file_stem().unwrap_or_default().to_string_lossy();
        let ext = held.file.extension().unwrap_or_default().to_string_lossy();
        let name = format!("{:02} - {}.{ext}", track.index, strip_number(&stem));
        let dest = folder.join(&name);
        if dest.exists() {
            let _ = tx.send(Msg::Log(format!(
                "copy: {name} is already here, so track {} downloads as usual",
                track.index
            )));
            unmark(tx, track);
            continue;
        }
        let scratch = folder.join(format!("{SCRATCH}{name}"));
        let done = std::fs::create_dir_all(&folder)
            .map(|()| sweep_scratch(&folder))
            .and_then(|()| std::fs::copy(&held.file, &scratch))
            .and_then(|_| std::fs::rename(&scratch, &dest));
        if let Err(err) = done {
            let _ = std::fs::remove_file(&scratch);
            let _ = tx.send(Msg::Log(format!("copy: track {}: {err}", track.index)));
            unmark(tx, track);
            continue;
        }
        track.path = Some(dest.clone());
        track.status = Status::Have;
        track.source = "copied".into();
        track.note = format!("from {}", held.folder);
        unmark(tx, track);
        let _ = tx.send(Msg::Path { index: track.index, path: dest });
        let _ = tx.send(Msg::Update {
            index: track.index,
            status: Status::Have,
            source: Some(track.source.clone()),
            note: Some(track.note.clone()),
            name: None,
        });
        let _ = tx.send(Msg::Log(format!(
            "copied track {} from {} instead of downloading it",
            track.index, held.folder
        )));
        copied += 1;
    }
    // Recorded now and not at the end of the pass: quit during the download and an unrecorded copy is a file
    // the next scan does not know, so the track downloads again beside it.
    if copied > 0 {
        write_manifest(cfg, tracks, false);
    }
    copied
}

// The whole title and what follows the last ` - `, since one side often has the artist and the other not.
fn title_keys(text: &str) -> (String, String) {
    let whole = normalised(strip_number(text));
    let tail = whole.rsplit_once(" - ").map_or_else(String::new, |(_, t)| t.trim().to_string());
    (whole, tail)
}

/// Marks a likely copy of the same song for the pick gate. A warning and never a skip.
// Two ids in this listing match on title and a duration within 2s, `reconcile`'s rules. Library files
// match on title only, since their length costs a tag read. Two tails never meet, or every `Intro` would.
// An id the library holds says it will be copied instead, which is what the sync does.
fn mark_twins(tracks: &mut [Track], known: &HashMap<String, Held>, shelves: &[Shelf], here: &Path) {
    let keys: Vec<(String, String)> = tracks.iter().map(|t| title_keys(&t.name)).collect();
    // First folder wins, as it did when this was a scan in folder order.
    let mut whole_of: HashMap<String, &str> = HashMap::new();
    let mut tail_of: HashMap<String, &str> = HashMap::new();
    for shelf in shelves.iter().filter(|s| s.path.file_name() != here.file_name()) {
        for (name, present) in &shelf.files {
            if !present {
                continue;
            }
            let stem = Path::new(name).file_stem().unwrap_or_default().to_string_lossy();
            let (whole, tail) = title_keys(&stem);
            if whole.is_empty() {
                continue;
            }
            if !tail.is_empty() {
                tail_of.entry(tail).or_insert(shelf.name.as_str());
            }
            whole_of.entry(whole).or_insert(shelf.name.as_str());
        }
    }
    for i in 0..tracks.len() {
        // Said first, because it is the one answer that changes what happens: unpicking this row saves a copy, not a download.
        if tracks[i].status == Status::Pending
            && let Some(held) = known.get(&tracks[i].id)
        {
            tracks[i].twin = Some(format!("copy from {}", held.folder));
            continue;
        }
        if !matches!(tracks[i].status, Status::Pending | Status::Have) || keys[i].0.is_empty() {
            continue;
        }
        let earlier = (0..i).find(|&j| {
            matches!(tracks[j].status, Status::Pending | Status::Have)
                && keys[j].0 == keys[i].0
                && tracks[j].duration.abs_diff(tracks[i].duration) <= 2
        });
        if let Some(j) = earlier {
            tracks[i].twin = Some(format!("same as track {}", tracks[j].index));
            continue;
        }
        if tracks[i].status != Status::Pending {
            continue;
        }
        let (whole, tail) = &keys[i];
        let hit = whole_of
            .get(whole)
            .or_else(|| whole_of.get(tail).filter(|_| !tail.is_empty()))
            .or_else(|| tail_of.get(whole));
        if let Some(folder) = hit {
            tracks[i].twin = Some(format!("maybe in {folder}"));
        }
    }
}

// Two videos with one ISRC are one recording. Returns what changed; `open_shelf` has nobody to tell.
// An ISRC match replaces a title-based marker, since it is the stronger answer, and the title guess is
// dropped only when the other side holds a different ISRC. Nothing recorded on the other side contradicts
// nothing, and most files have none. The same video in two folders is what a copy makes, so it is not a twin.
fn flag_isrc_twins(others: &Others, tracks: &mut [Track]) -> Vec<(usize, Option<String>)> {
    let mut changed = Vec::new();
    if tracks.iter().all(|t| t.isrc.is_none()) {
        return changed;
    }
    // Every holder of an ISRC, in folder order: a copy of this very video is not a twin, and must not hide a real one.
    let mut elsewhere: HashMap<&str, Vec<(&str, &str)>> = HashMap::new();
    for other in others.all() {
        let files: HashMap<&str, &Path> =
            other.sidecar.entries.iter().map(|(id, file)| (id.as_str(), file.as_path())).collect();
        for (id, isrc) in &other.sidecar.isrcs {
            if files.get(id.as_str()).is_some_and(|file| file.is_file()) {
                elsewhere.entry(isrc.as_str()).or_default().push((other.name.as_str(), id.as_str()));
            }
        }
    }
    let isrc_of: HashMap<usize, String> = tracks.iter().filter_map(|t| t.isrc.clone().map(|i| (t.index, i))).collect();
    let mut seen: HashMap<String, (usize, String)> = HashMap::new();
    for track in tracks.iter_mut() {
        let Some(isrc) = track.isrc.clone() else {
            continue;
        };
        let mut twin = if let Some((index, _)) = seen.get(&isrc).filter(|(_, id)| *id != track.id) {
            Some(format!("same recording as track {index}"))
        } else {
            elsewhere
                .get(isrc.as_str())
                .and_then(|held| held.iter().find(|(_, id)| *id != track.id))
                .map(|(folder, _)| format!("same recording in {folder}"))
        };
        if twin.is_none() && !title_guess_contradicted(others, track, &isrc, &isrc_of) {
            twin.clone_from(&track.twin);
        }
        seen.entry(isrc).or_insert((track.index, track.id.clone()));
        if track.twin != twin {
            track.twin.clone_from(&twin);
            changed.push((track.index, twin));
        }
    }
    changed
}

// Whether the other side of a title guess is known to hold a different ISRC. "copy from" is never a guess.
fn title_guess_contradicted(others: &Others, track: &Track, isrc: &str, isrc_of: &HashMap<usize, String>) -> bool {
    let Some(guess) = track.twin.as_deref() else {
        return false;
    };
    if let Some(index) = guess.strip_prefix("same as track ").and_then(|n| n.parse::<usize>().ok()) {
        return isrc_of.get(&index).is_some_and(|other| other != isrc);
    }
    let Some(folder) = guess.strip_prefix("maybe in ") else {
        return false;
    };
    let (whole, tail) = title_keys(&track.name);
    let Some(other) = others.all().into_iter().find(|o| o.name == folder) else {
        return false;
    };
    // The files in that folder the title matched, and what the sidecar says their recordings are.
    let held: Vec<&str> = other
        .sidecar
        .entries
        .iter()
        .filter(|(_, file)| {
            let stem = file.file_stem().unwrap_or_default().to_string_lossy();
            let (w, t) = title_keys(&stem);
            !w.is_empty() && (w == whole || (!tail.is_empty() && w == tail) || (!t.is_empty() && t == whole))
        })
        .filter_map(|(id, _)| other.sidecar.isrcs.iter().find(|(known, _)| known == id).map(|(_, i)| i.as_str()))
        .collect();
    !held.is_empty() && held.iter().all(|other| *other != isrc)
}

/// Lowercased, composed and trimmed, which is what every other comparison in
/// this file does and for the same reason: matched raw, every accented or
/// Hangul title misses while the ASCII ones still match.
fn normalised(text: &str) -> String {
    composed(text.trim()).to_lowercase()
}

/// A filename stem without the playlist number earworm and most other people
/// put in front of it, so `03 - Ping Pong` can be compared with `Ping Pong`.
/* Only a leading run of digits followed by a separator: a title that opens
   with a number, `1979` or `99 Luftballons`, keeps it, because there is no
   separator after it. */
fn strip_number(stem: &str) -> &str {
    let rest = stem.trim_start_matches(|c: char| c.is_ascii_digit());
    if rest.len() == stem.len() {
        return stem;
    }
    let trimmed = rest.trim_start();
    match trimmed.strip_prefix(['-', '.', '_']) {
        Some(after) => after.trim_start(),
        None => stem,
    }
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
pub(crate) fn format_on_disk(path: &Path) -> Option<&'static str> {
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
   - `alac` on disk, target `m4a`: convert. Both are `.m4a`, so a refetch's
     file lands on the very path the old one holds and yt-dlp skips or
     rewrites it in place.
   - target `opus` or `m4a`: a download copies YouTube's own stream (the AAC
     one for m4a, through `NATIVE_M4A`), so a fetch is one generation against
     the two a conversion would cost. Fetch it again unless the file already is one.
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
        // A refetch to `m4a` names its file where this alac one already sits, so yt-dlp would skip or rewrite it.

        Some("alac") if format == "m4a" => Bring::Convert,
        Some(_) if matches!(format, "opus" | "m4a") => Bring::Refetch,
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

    let kept = tag::Kept::read(&old);

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

    if let Some(warning) = kept.write(new)? {
        // Not fatal: a track with the right audio and no art beats no track.
        let _ = tx.send(Msg::Log(format!("convert: cover: {warning}")));
    }
    // The sidecar line follows the file's tag, so a replacement that did not keep the ISRC drops it too.
    if kept.isrc().is_some() && tag::read(new).ok().and_then(|i| i.isrc).is_none() {
        if let Some(folder) = old.parent() {
            let _ = manifest::drop_isrc(folder, &track.id);
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
                save_manifest(cfg, tx, tracks);
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
                save_manifest(cfg, tx, tracks);
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
) -> Extras {
    let mut extras = Extras::default();
    for track in tracks.iter_mut() {
        if cancel.load(Ordering::SeqCst) {
            break;
        }
        /* None of these has a file this run is entitled to touch: a departed
           one is somebody else's playlist position, a skipped one was never
           fetched, and a local one is a file the playlist knows nothing about,
           so a lookup here would retag somebody's own music off a playlist it
           is not in. Falling through marks the first two `Failed` for having
           no file. */
        if wanted.is_some_and(|only| !only.contains(&track.index))
            || matches!(track.status, Status::Gone | Status::Skipped | Status::Local)
        {
            continue;
        }
        let Some(path) = track.path.clone() else {
            continue;
        };
        if !path.is_file() {
            // Already failed with a reason of its own, such as a cut that went wrong, and "no file" would replace it.
            if track.status == Status::Failed {
                continue;
            }
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
                // So a file from an earlier run is compared too, and its line reaches the sidecar.
                track.isrc = info.isrc;
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
            extras.add(enrich(cfg, tx, track, &path, playlist));
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
                // The file is the truth, so a line for a recording it no longer holds goes.
                let isrc = info.as_ref().map_or(out.isrc, |i| i.isrc.clone());
                if isrc.is_none()
                    && track.isrc.is_some()
                    && let Some(folder) = path.parent()
                {
                    let _ = manifest::drop_isrc(folder, &track.id);
                }
                track.isrc = isrc;
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
                extras.add(enrich(cfg, tx, track, &path, playlist));
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
    extras
}

// What the optional per-track passes managed, for the summary: a miss is not a failure, so it is counted and not flagged.
#[derive(Default, Clone, Copy, Debug, PartialEq)]
struct Extras {
    lyrics_found: usize,
    lyrics_missed: usize,
    // Not asked because the service stopped answering, which is not the same as nothing being there.
    lyrics_unavailable: usize,
}

impl Extras {
    fn add(&mut self, other: Extras) {
        self.lyrics_found += other.lyrics_found;
        self.lyrics_missed += other.lyrics_missed;
        self.lyrics_unavailable += other.lyrics_unavailable;
    }

    // Only tracks that were searched for count: one that already had words, or had nothing to ask with, was never asked about.
    fn said(&self) -> Option<String> {
        let asked = self.lyrics_found + self.lyrics_missed;
        let mut said = Vec::new();
        if asked > 0 {
            said.push(format!("lyrics found for {} of {asked} searched", self.lyrics_found));
        }
        if self.lyrics_unavailable > 0 {
            said.push(format!(
                "lyrics service not answering, {} left for the next sync",
                self.lyrics_unavailable
            ));
        }
        (!said.is_empty()).then(|| said.join(", "))
    }
}

// Per-track extras written after identification: loudness and lyrics. Runs for tracks already
// on disk too, so a resync backfills old folders. Both are off by default.
fn enrich(cfg: &Config, tx: &Sender<Msg>, track: &Track, path: &Path, playlist: &str) -> Extras {
    enrich_with(cfg, tx, track, path, playlist, lookup::lyrics)
}

fn real_album<'a>(album: &'a str, playlist: &str) -> &'a str {
    if album == playlist { "" } else { album }
}

// Takes the lyrics fetch so a test can count requests without a network.
fn enrich_with(
    cfg: &Config,
    tx: &Sender<Msg>,
    track: &Track,
    path: &Path,
    playlist: &str,
    fetch: impl Fn(&str, &str, &str, u64) -> lookup::Lookup,
) -> Extras {
    let mut extras = Extras::default();
    if cfg.loudness {
        match tag::write_loudness(path) {
            Ok(true) => {
                let _ = tx.send(Msg::Log(format!("loudness: track {} measured", track.index)));
            }
            Ok(false) => {}
            // A failure here costs the gain tag and nothing else, so it is a log line and not a status.
            Err(err) => {
                let _ = tx.send(Msg::Log(format!("loudness: track {}: {err}", track.index)));
            }
        }
    }
    if cfg.lyrics {
        // Asked before the request: a track that has words never costs one, and a rerun is free.
        match tag::extra(path, tag::Extra::Lyrics) {
            Ok(Some(_)) => {}
            // The album fill is the playlist's name, which LRCLIB's exact lookup would only ever miss on.
            Ok(None) => match fetch(&track.artist, &track.title, real_album(&track.album, playlist), track.duration) {
                lookup::Lookup::Found(text) => match tag::set_extra(path, tag::Extra::Lyrics, &text) {
                    Ok(()) => {
                        extras.lyrics_found += 1;
                        let _ = tx.send(Msg::Log(format!("lyrics: track {} found", track.index)));
                    }
                    Err(err) => {
                        let _ = tx.send(Msg::Log(format!("lyrics: track {}: {err}", track.index)));
                    }
                },
                lookup::Lookup::Missing => extras.lyrics_missed += 1,
                lookup::Lookup::Unavailable => extras.lyrics_unavailable += 1,
                lookup::Lookup::Skipped => {}
            },
            Err(err) => {
                let _ = tx.send(Msg::Log(format!("lyrics: track {}: {err}", track.index)));
            }
        }
    }
    extras
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
    not_custom(cfg, folder)?;
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

    // A custom playlist names its tracks by folder, so a local id would be orphaned by the move.
    if let Err(err) = playlists::rehome(&cfg.dir, &current, &name) {
        let _ = tx.send(Msg::Log(format!("custom playlists: {err}")));
    }

    /* The .m3u8 is named after its folder and `last_playlist` finds it that
       way. Its entries are relative, so the move itself left them valid. */
    let old_playlist = to.join(format!("{current}.m3u8"));
    if old_playlist.is_file() {
        let _ = std::fs::rename(&old_playlist, to.join(format!("{name}.m3u8")));
        /* And the header that says which playlist file is earworm's own moves
           with it. Left naming the old filename, `write_playlist` in a folder
           with no URL finds a file it does not recognise and refuses for
           good: earworm would stop updating its own playlist after a rename,
           silently, for the rest of that folder's life. */
        if manifest::load(&to).playlist.as_deref() == Some(format!("{current}.m3u8").as_str()) {
            let _ = manifest::set_playlist(&to, &format!("{name}.m3u8"));
        }
    }
    manifest::set_name(&to, &name)
        .with_context(|| format!("recording the name in {}", manifest::path(&to).display()))?;

    if pin_open_run(cfg, tracks, folder, &name) {
        // The run on screen is this folder, so it has moved under it.
        let _ = tx.send(Msg::Playlist(name.clone()));
        let _ = tx.send(Msg::Folder(to.clone()));
    }
    let _ = tx.send(Msg::Library {
        shelves: library_all(&cfg.dir),
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
/// What `D` was asked to do, so a row's meaning does not depend on its
/// position in a menu that is a row shorter for some folders.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Remove {
    Keep,
    Forget,
    Hide,
    Delete,
}

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
    not_custom(cfg, folder)?;
    let name = folder
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .context("that folder has no name to remove")?;
    /* The rows carry their answers rather than the reader counting them: a
       folder earworm did not download has nothing to forget, so that row is
       absent and a bare index would mean a different thing on the two
       folders this menu opens on. */
    let synced = manifest::path(folder).is_file();
    let audio = audio_files(folder).len();
    let mut rows = vec![("keep it".to_string(), Remove::Keep)];
    if synced {
        rows.push((format!("forget {name}"), Remove::Forget));
    }
    rows.push((format!("hide {name} from earworm"), Remove::Hide));
    /* Tracks rather than files, because that is what the number counts:
       `remove_dir_all` takes the whole folder, `cover.jpg` and the sidecar
       with it, and a count of six against a word meaning everything would be
       the one row on this menu that understates itself. */
    let tracks_word = if audio == 1 { "track" } else { "tracks" };
    rows.push((
        format!("delete {name} and its {audio} {tracks_word}"),
        Remove::Delete,
    ));

    /* Says what each row costs, since every one of them but the first takes
       something away and only the last is undoable by re-downloading. */
    let note = if synced {
        "forget drops the playlist and stops syncing it  ·  hide takes the row off the library  ·  both keep the audio  ·  delete takes the folder as well"
    } else {
        "earworm did not download this folder, so there is nothing to forget  ·  hide takes the row off the library and keeps the audio  ·  delete takes the files with it"
    };
    let Some(choice) = asker.choose_noted(
        &format!("Remove {name}?"),
        note,
        rows.iter().map(|(label, _)| label.clone()).collect(),
    ) else {
        return cancelled(tx);
    };
    /* Out of range can only be a bug in the menu above, and keeping is the
       safe way to be wrong, but silently safe is how the last index drift
       went unnoticed for a version. */
    debug_assert!(choice < rows.len(), "answer {choice} is off a menu of {}", rows.len());
    let chosen = rows.get(choice).map(|(_, act)| *act).unwrap_or(Remove::Keep);
    if chosen == Remove::Keep {
        let _ = tx.send(Msg::Flash(format!("keeping {name}")));
        return Ok(());
    }
    /* Hiding leaves the folder exactly as it is, so cliamp's copy of it is
       still playable and still correct: only the row goes. */
    if chosen == Remove::Hide {
        manifest::hide(folder).with_context(|| format!("hiding {}", folder.display()))?;
        /* The line to remove, not the file: a found folder's sidecar holds
           nothing else and deleting it would be a clean undo, but a folder
           earworm downloaded keeps its URL, its rename and every id in that
           same file, and throwing it away re-downloads the lot beside the
           copies on disk. One message has to be true of both. */
        let _ = tx.send(Msg::Flash(format!(
            "hid {name}  ·  the audio is untouched  ·  remove the #hidden line from its .earworm to bring it back"
        )));
    }
    /* Before either removal, since the stored name is built from the path
       this folder has now: afterwards there is nothing left that names it. */
    if chosen != Remove::Hide {
        player::forget(folder);
    }
    if chosen == Remove::Forget {
        /* Already gone is gone: the row was listed from a manifest that
           someone may have deleted by hand since. */
        for file in [manifest::path(folder), folder.join(format!("{name}.m3u8"))] {
            match std::fs::remove_file(&file) {
                Ok(()) => {}
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                Err(e) => bail!("removing {}: {e}", file.display()),
            }
        }
    }
    if chosen == Remove::Delete {
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
        shelves: library_all(&cfg.dir),
        show: open,
    });
    /* Forgetting takes the sidecar, not the music, and a folder holding
       audio is on the library list whether earworm downloaded it or not. So
       the row does not go: it drops back to a folder earworm merely found,
       which `R` passes by and `S` would have to be given a URL to sync. Say
       that, or the key looks as though it half worked.

       On `chosen`, not on the index: read as `choice == 1` this said "forgot"
       after a hide on a found folder and "deleted" after a hide on a synced
       one, which is a claim that earworm destroyed a folder still entirely on
       disk. Hiding has already said its piece above and wants no second line. */
    match chosen {
        Remove::Forget => {
            let _ = tx.send(Msg::Flash(format!(
                "forgot {name}  ·  audio kept  ·  no longer synced"
            )));
        }
        Remove::Delete => {
            let _ = tx.send(Msg::Flash(format!("deleted {name}")));
        }
        Remove::Keep | Remove::Hide => {}
    }
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
            Cmd::NewPlaylist => {
                report(tx, custom::new(&cfg.dir, tx, &asker));
                None
            }
            Cmd::AddToPlaylist(picks) => {
                report(tx, custom::add(&cfg.dir, tx, &asker, &picks));
                None
            }
            Cmd::OpenPlaylist(name) => {
                report(tx, custom::open(&cfg.dir, tx, &name));
                None
            }
            Cmd::MoveEntry { name, id, up } => {
                report(tx, custom::move_entry(&cfg.dir, tx, &name, &id, up));
                None
            }
            Cmd::RemoveEntry { name, id } => {
                report(tx, custom::remove_entry(&cfg.dir, tx, &name, &id));
                None
            }
            Cmd::RenamePlaylist(name) => {
                report(tx, custom::rename(&cfg.dir, tx, &asker, &name));
                None
            }
            Cmd::DeletePlaylist(name) => {
                report(tx, custom::delete(&cfg.dir, tx, &asker, &name));
                None
            }
            Cmd::Export(target) => {
                report(tx, export::run(&cfg.dir, tx, &asker, cancel, &target));
                None
            }
            Cmd::PlayPlaylist(name) => {
                match custom::play(&cfg.dir, &name) {
                    Ok((said, folders)) => {
                        let _ = tx.send(Msg::OnAir(Some((name, folders))));
                        let _ = tx.send(Msg::Flash(said));
                    }
                    Err(err) => {
                        let _ = tx.send(Msg::Flash(format!("cliamp: {err}")));
                        let _ = tx.send(Msg::Log(format!("cliamp: {err}")));
                    }
                }
                None
            }
            Cmd::Play(folder) => {
                match player::load(&folder) {
                    Ok(said) => {
                        let _ = tx.send(Msg::OnAir(None));
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
                shelves: library_all(&cfg.dir),
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

// A custom playlist's row carries its file as the path, and these commands treat a path as a directory to move or delete.
// The keys route a custom row elsewhere, and this is the second lock: `remove_dir_all` does not come back.
fn not_custom(cfg: &Config, path: &Path) -> Result<()> {
    if playlists::in_store(&cfg.dir, path) {
        bail!("that is a custom playlist, not a folder  ·  its own keys never touch audio");
    }
    Ok(())
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
    not_custom(cfg, folder)?;
    // Re-read rather than trusting what the screen was showing: the folder may
    // have gone away between the listing and the keypress.
    let shelves = library_all(&cfg.dir);
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
    let Some(url) = prompt_url(tx, Escape::Keep, &cfg.unlocked, "") else {
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
    Some(pipeline(cfg, tx, cancel, tracks, Pass::plain(gate)))
}

/// Syncs the playlist that is open against the URL its manifest recorded,
/// which is the point of opening one offline: look first, download second.
/* With no URL recorded, this is where one is given. A folder earworm did not
   download has files and no playlist behind them, and attaching is the
   interactive act that gives it one: `S` on the open folder and nowhere else,
   because the matching that follows needs somebody at the pick gate. The
   folder is pinned by name first, or the download writes the whole playlist
   into a second folder named after the playlist upstream and leaves this one
   sitting beside it. */
fn sync_open(
    cfg: &mut Config,
    tx: &Sender<Msg>,
    cancel: &AtomicBool,
    tracks: &mut Vec<Track>,
) -> Result<String> {
    let attaching = cfg.url.is_empty();
    let folder = folder_of(tracks).context("no folder is open to sync")?;
    if attaching {
        let name = folder.file_name().unwrap_or_default().to_string_lossy().to_string();
        let Some(url) = prompt_url(tx, Escape::Keep, &cfg.unlocked, "") else {
            bail!("cancelled  ·  nothing was downloaded");
        };
        cfg.url = url;
        /* Before the run, because `output_template` reads it: without it the
           download writes the whole playlist into a second folder named after
           the playlist upstream and leaves this one sitting beside it. */
        cfg.folder = Some(name);
    }
    let _ = tx.send(Msg::Restart);
    tracks.clear();
    let pass = if attaching {
        Pass::attaching()
    } else {
        Pass::plain(cfg.pick)
    };
    /* Nothing is written here. `write_manifest` records `cfg.folder` as it
       writes the sidecar, so the `#name` header arrives with the `#url` in one
       file, from the run that earned both. A sync that failed on a typo'd URL
       or a dead network therefore leaves a folder earworm did not download
       exactly as it found it, which matters because a hidden file appearing
       in somebody's music folder is meant to be said out loud. */
    pipeline(cfg, tx, cancel, tracks, pass)
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
    /* Nothing to refetch for a format: a retry is about tracks that failed.
       Not for a split video: its tracks have no download of their own, and
       running yt-dlp over them would fetch the whole video again and hand its
       file to track 1. A sync cuts any chapter that is missing. */
    if tracks.iter().any(|t| manifest::is_chapter(&t.id)) {
        let _ = tx.send(Msg::Log(
            "retry: chapters are cut when the playlist syncs, so this only tags again".into(),
        ));
    } else if let Err(err) = ytdlp::run(cfg, tracks, tx, &HashSet::new()) {
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
    /* The same answer the first pass used, re-resolved rather than carried:
       a retry is a separate command and the folder may have been renamed
       between the two. */
    let album = folder
        .as_deref()
        .and_then(|f| manifest::kind_of(f, &manifest::load(f)))
        .is_some_and(|(kind, _)| kind == manifest::Kind::Album);
    let mut cover = CoverState {
        written: folder.as_deref().is_some_and(has_cover),
        one_sleeve: album,
        ..CoverState::default()
    };
    let extras = tag_tracks(cfg, tx, cancel, tracks, &playlist, asker, &mut cover, Some(&failed));
    // Album gain is a mean over the folder, so a retried track changes it for every file.
    if album && cfg.loudness && !cancel.load(Ordering::SeqCst) {
        album_loudness(tx, tracks);
    }

    sync_manifest(cfg, tracks);
    let playlist_file = write_playlist(cfg, tracks)?;
    drop_folder_cover(tracks, &theirs, album);

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
    if let Some(line) = extras.said() {
        summary = format!("{summary}, {line}");
    }
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
        save_manifest(cfg, tx, tracks);
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
        save_manifest(cfg, tx, tracks);
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
    save_manifest(cfg, tx, tracks);
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
    /* One track, so there is no sleeve to share, and `written` keeps this
       from touching the folder image either way. */
    let mut cover = CoverState {
        written: true,
        ..CoverState::default()
    };
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
    let isrc = written.as_ref().and_then(|i| i.isrc.clone());
    if isrc.is_none()
        && track.isrc.is_some()
        && let Some(folder) = path.parent()
    {
        let _ = manifest::drop_isrc(folder, &track.id);
    }
    track.isrc = isrc;
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
    save_manifest(cfg, tx, tracks);
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
    /// Take the folder image away and give every track back the art its own
    /// lookup finds, which is the only way out of a `c` that has already
    /// stamped one picture across the whole folder.
    Restore,
}

fn cover_options(found: &[lookup::Artwork], current: Option<String>) -> Vec<(String, Pick)> {
    let mut rows: Vec<(String, Pick)> = found
        .iter()
        .map(|a| (a.label.clone(), Pick::Url(a.url.clone())))
        .collect();
    let removes = current.is_some();
    if let Some(desc) = current {
        rows.push((format!("keep the current cover  {desc}"), Pick::Keep));
    }
    /* Beside `keep` because both rows are about what the folder already has,
       where every other row puts a new picture on it. The label names the
       removal only when there is a file to remove: a row offering to delete
       something that is not there reads as a row that does not know what it
       is looking at. */
    rows.push((
        if removes {
            "remove the folder cover  ·  restore each track's own art".to_string()
        } else {
            "restore each track's own art".to_string()
        },
        Pick::Restore,
    ));
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
        /* A file earworm did not download is in no listing, ever, so its
           absence from one says nothing: this is the id earworm invented for
           it. Without this, attaching a URL to a folder somebody copied in
           would call every file it holds departed, mark the departures proven
           off a complete listing, and offer the lot to `D`. */
        .filter(|(id, _)| !manifest::is_local(id))
        /* A chapter's id is one earworm made from the video's, so no listing
           names it either. Reading it as departed would offer the whole split
           album to `D` the moment a listing arrives that holds the video. */
        .filter(|(id, _)| !manifest::is_chapter(id))
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
/* An album is the other exception. Its `cover.jpg` is that album's sleeve
   rather than an arbitrary track's, and one image matching every file in the
   folder is what a file manager and most players want. On a playlist the same
   file is the first track's art wearing the folder's name, which is what this
   was written to clear up. */
fn drop_folder_cover(tracks: &[Track], keep: &[&str], album: bool) {
    if album {
        return;
    }
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
    save_manifest(cfg, tx, tracks);

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
        Pick::Restore => return restore_art(tx, tracks, asker, cancel, &folder, apple),
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
    /* Every file's art has just changed behind a path that has not, and the
       detail pane keys its decoded cover on the path. Without this the pane
       goes on showing the sleeve that was replaced. */
    let _ = tx.send(Msg::Artwork);
    let _ = tx.send(Msg::Flash(format!("cover set from {label}")));
    Ok(())
}

/* Takes the one image off the folder and gives every track back the art its
   own lookup finds.

   The counterpart to the rest of that menu, which puts a single picture on
   every file in the folder: right for an album, wrong for a playlist, where
   the tracks come from a dozen different records and the shared sleeve is the
   only thing hiding it. Nothing on disk can put that back, because the embed
   overwrote each track's own art rather than sitting beside it, so the art is
   fetched again per track.

   One track's failure is logged and counted rather than stopping the rest,
   and the flash reports both halves: a pass that gave eleven of twelve tracks
   their sleeve back and said nothing about the twelfth is the half-report
   this file has already paid for once. */
fn restore_art(
    tx: &Sender<Msg>,
    tracks: &[Track],
    asker: &Asker,
    cancel: &AtomicBool,
    folder: &Path,
    apple: bool,
) -> Result<()> {
    let had = has_cover(folder);
    /* `is_file` and not merely a recorded path. A `Gone` track keeps the path
       of a file that is not there, and so does anything moved outside
       earworm: without this each one costs a lookup before failing at the
       write, and lands in the same count as a track whose art could not be
       found. Two unrelated failures sharing one number is the reason the
       conversion halves report separately. */
    let targets: Vec<(&Track, &Path)> = tracks
        .iter()
        .filter(|t| t.listed)
        .filter_map(|t| t.path.as_deref().filter(|p| p.is_file()).map(|p| (t, p)))
        .collect();

    /* Asked because every one of those files is rewritten with art nobody has
       seen yet, and there is no undo for a picture: `u` holds tag values. The
       rest of this menu at least shows you the image you are choosing. Only
       when there are files to rewrite, since with none the row does exactly
       what its label says and nothing is at risk. */
    if !targets.is_empty() {
        let count = targets.len();
        let word = if count == 1 { "track" } else { "tracks" };
        let name = folder.file_name().unwrap_or_default().to_string_lossy();
        let Some(choice) = asker.choose_noted(
            &format!("Restore art on {count} {word} in {name}?"),
            "each track is looked up again and gets its own album art  ·  whatever art they carry now is replaced, and this cannot be undone",
            /* "change nothing" rather than "keep their art": this row has two
               halves and refusing it abandons both, so an option naming only
               the art would leave the reader expecting the folder image to go
               anyway. */
            vec!["change nothing".into(), format!("restore {count} {word}")],
        ) else {
            return cancelled(tx);
        };
        if choice == 0 {
            let _ = tx.send(Msg::Flash("nothing changed".into()));
            return Ok(());
        }
    }

    for name in COVER_NAMES {
        let _ = std::fs::remove_file(folder.join(name));
    }

    let (mut given, mut missed) = (0usize, 0usize);
    for (done, (track, path)) in targets.iter().enumerate() {
        if cancel.load(Ordering::SeqCst) {
            break;
        }
        let _ = tx.send(Msg::Stage(format!(
            "restoring art  {}/{}",
            done + 1,
            targets.len()
        )));
        let Some(image) = own_art(track, apple) else {
            missed += 1;
            let _ = tx.send(Msg::Log(format!(
                "cover: track {}: no artwork found for {}",
                track.index, track.name
            )));
            continue;
        };
        match tag::set_cover(path, &image) {
            Ok(()) => given += 1,
            Err(err) => {
                missed += 1;
                let _ = tx.send(Msg::Log(format!("cover: track {}: {err}", track.index)));
            }
        }
    }

    let _ = tx.send(Msg::Artwork);
    // Every arm claims the removal only when there was something to remove.
    let gone = if had { "folder cover removed  ·  " } else { "" };
    let _ = tx.send(Msg::Flash(match (given, missed) {
        (0, 0) if had => "folder cover removed".to_string(),
        (0, 0) => "nothing here to restore".to_string(),
        (_, 0) => format!("{gone}{given} tracks back on their own art"),
        _ => format!("{gone}{given} restored, {missed} unchanged  ·  l has why"),
    }));
    Ok(())
}

/// The first candidate that actually downloads, in the order the menu offers
/// them: picking by some other rule here would be a second opinion about
/// which art is best from the one list. `find_map` rather than the first URL,
/// so a Cover Art Archive release group with no front image falls through to
/// Deezer instead of costing the track its sleeve.
///
/// Uncapped on purpose. `COVER_MAX` is Deezer's `cover_xl`, which is the
/// largest thing this list can return, so there is nothing here to shrink.
fn own_art(track: &Track, apple: bool) -> Option<Vec<u8>> {
    lookup::artwork(&track.artist, &track.title, track.mbid.as_deref(), apple)
        .iter()
        .find_map(|found| lookup::fetch(&found.url))
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
    fit_stem(&format!("{index:02} - {} - {}", clean(artist), clean(title)), ext)
}

/// A chapter's file before the lookup has named it: the number and the
/// chapter's own title.
fn chapter_file(index: usize, title: &str, ext: &str) -> String {
    fit_stem(&format!("{index:02} - {}", clean(title)), ext)
}

fn fit_stem(stem: &str, ext: &str) -> String {
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

/* Adoption is this function running for the first time in a folder earworm
   did not download: an edit is the moment the user asks earworm to change
   that folder, and the sidecar is what carries the change. It is said once,
   because a hidden file appearing in somebody's music folder should not be
   something they find out about by looking. A folder with a URL is one
   earworm downloaded, whose sidecar is no news at all. */
fn save_manifest(cfg: &Config, tx: &Sender<Msg>, tracks: &[Track]) {
    let fresh = folder_of(tracks).filter(|f| !manifest::path(f).is_file());
    write_manifest(cfg, tracks, false);
    if let Some(folder) = fresh.filter(|_| cfg.url.is_empty()) {
        let name = folder.file_name().unwrap_or_default().to_string_lossy();
        let _ = tx.send(Msg::Flash(format!(
            "adopted {name}  ·  earworm keeps a .earworm here now"
        )));
    }
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
    /* By file as well as by id, which is what lets an id change under a file.
       Attaching a URL rewrites a local `~id` to the video id the sync matched
       it to, and the entry it replaces names the same file: kept on id alone,
       the sidecar would hold two lines for one track, and the next sync would
       read the stale one back as a file the playlist does not have. */
    let ours: HashSet<&Path> = known.iter().map(|(_, file)| file.as_path()).collect();
    let kept: Vec<(String, PathBuf)> = manifest::entries(&folder)
        .into_iter()
        .filter(|(id, file)| {
            !mine.contains(id.as_str()) && !ours.contains(file.as_path()) && file.is_file()
        })
        .collect();

    /* The run knows which folder it is writing into and the sidecar may not
       yet: attaching a URL to a folder earworm did not download pins
       `cfg.folder` so the download lands here rather than under the playlist's
       upstream title, and the header is what makes the next sync land here
       too. Recorded by whichever write happens to be first, so it arrives with
       the `#url` in one file rather than needing a second step afterwards
       that a failed sync would have to remember not to take. Never overwritten:
       a name already in the file is one somebody chose with `e`. */
    if let Some(name) = cfg.folder.as_deref()
        && manifest::load(&folder).name.is_none()
    {
        let _ = manifest::set_name(&folder, name);
    }

    let found: Vec<(String, String)> = tracks
        .iter()
        .filter(|t| t.path.as_deref().is_some_and(Path::is_file))
        .filter_map(|t| t.isrc.clone().map(|isrc| (t.id.clone(), isrc)))
        .collect();
    let entries = kept.into_iter().chain(known);
    let _ = if synced {
        manifest::write_synced(&folder, &cfg.url, entries)
    } else {
        manifest::write(&folder, &cfg.url, entries)
    };
    // After the entries, which are what say which ids still have a line.
    let _ = manifest::set_isrcs(&folder, &found);
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
    let file = format!("{name}.m3u8");
    let playlist = folder.join(&file);

    /* A folder with a playlist upstream is one earworm owns the `.m3u8` of:
       the sync decides what the playlist holds, so it rewrites the file every
       pass. A folder somebody copied in may have brought its own, and it may
       be named exactly what earworm would name one, after the folder. So in a
       folder with no URL the file is written only when there is nothing there
       or when the sidecar says earworm put it there, and the header is
       recorded at the moment of writing. Replacing somebody's playlist does
       not come back and nothing on screen would say it happened. */
    /* Only asked while the folder has no playlist behind it. A sync takes the
       `.m3u8` over: from then on the playlist upstream is what the folder
       holds and in what order, which is earworm's answer to write down, and
       `docs/library.md` says so where somebody can read it before attaching. */
    let ours = cfg.url.is_empty().then(|| {
        !playlist.exists() || manifest::load(folder).playlist.as_deref() == Some(file.as_str())
    });
    if ours == Some(false) {
        return Ok(None);
    }

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
    // Only where the question can arise: a folder with a URL never asks it,
    // and a header written there would be noise in every sidecar earworm has.
    if ours == Some(true) {
        let _ = manifest::set_playlist(folder, &file);
    }
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

    #[test]
    fn a_youtube_music_link_is_locked_until_the_code_is_found() {
        let unlocked = Unlocked::default();
        for link in [
            "https://music.youtube.com/playlist?list=OLAK5uy_x",
            "music.youtube.com/watch?v=abc",
            "https://MUSIC.youtube.com/playlist?list=PL1",
        ] {
            assert!(locked(link, &unlocked), "let through {link}");
        }
        for link in ["https://www.youtube.com/playlist?list=PL1", "https://youtu.be/abc", "hello"] {
            assert!(!locked(link, &unlocked), "locked {link}");
        }
        unlocked.grant(Feature::Music);
        assert!(!locked("https://music.youtube.com/playlist?list=PL1", &unlocked));
    }

    // The header under the prompt says what the tool is waiting on.
    #[test]
    fn a_locked_link_off_the_command_line_waits_at_the_prompt() {
        let link = "https://music.youtube.com/playlist?list=PL1";
        let mut cfg = config(true);
        cfg.url = link.into();
        let (tx, rx) = channel();
        std::thread::scope(|scope| {
            let worker = scope.spawn(|| matches!(ask_start(&mut cfg, &tx), Start::Cancelled));
            let mut staged = false;
            while let Ok(msg) = rx.recv_timeout(Duration::from_secs(5)) {
                match msg {
                    Msg::Stage(s) => staged = s == "waiting for a playlist URL",
                    Msg::Ask(crate::app::Prompt::Input { header, value, .. }, reply) => {
                        assert!(staged, "the header never said it was waiting");
                        assert_eq!((header.as_str(), value.as_str()), (LOCKED, link));
                        reply.send(Reply::Cancel).unwrap();
                        break;
                    }
                    _ => {}
                }
            }
            assert!(worker.join().unwrap(), "Esc on the first prompt did not cancel");
        });
    }

    /* The unlock is read off each answer, not once when the prompt opens:
       the console is used while this question is still on screen. */
    #[test]
    fn the_url_prompt_refuses_a_locked_link_and_takes_it_once_unlocked() {
        let link = "https://music.youtube.com/playlist?list=PL1";
        let unlocked = Unlocked::default();
        let (tx, rx) = channel();
        std::thread::scope(|scope| {
            // Typed already, as a link off the command line arrives.
            let worker = scope.spawn(|| prompt_url(&tx, Escape::Quit, &unlocked, link));
            let mut asked = Vec::new();
            while let Ok(msg) = rx.recv_timeout(Duration::from_secs(5)) {
                let Msg::Ask(crate::app::Prompt::Input { header, value, .. }, reply) = msg else {
                    continue;
                };
                asked.push((header, value));
                if asked.len() == 2 {
                    unlocked.grant(Feature::Music);
                }
                reply.send(Reply::Text(link.into())).unwrap();
                if asked.len() == 2 {
                    break;
                }
            }
            assert_eq!(worker.join().unwrap().as_deref(), Some(link));
            assert_eq!(
                asked,
                vec![(LOCKED.to_string(), link.to_string()), (LOCKED.to_string(), link.to_string())],
                "opened unlocked, or refusing lost the text"
            );
        });
    }
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
        // A custom playlist that names this folder, which the move must not orphan.
        let mine = playlists::Playlist {
            name: "Mix".into(),
            entries: vec![playlists::Entry { id: "~01 - A.opus".into(), hint: "Chill Evenings/01 - A.opus".into() }],
        };
        playlists::save(&root, &mine).unwrap();

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
        assert_eq!(
            playlists::load(&root, "Mix").unwrap().entries[0].hint,
            PathBuf::from("Evenings/01 - A.opus"),
            "a custom playlist still names the old folder"
        );

        // And the library the UI is about to draw is the renamed one.
        let shelves: Vec<Shelf> = rx
            .try_iter()
            .filter_map(|m| match m {
                Msg::Library { shelves, .. } => Some(shelves),
                _ => None,
            })
            .last()
            .expect("the library was never refreshed");
        // The folder under its new name, and the custom playlist beside it, which is still there.
        assert_eq!(shelves.iter().map(|s| s.name.clone()).collect::<Vec<_>>(), ["Evenings", "Mix"]);
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
            url: Some("https://example.com".into()),
            tracks: 1,
            ..Shelf::default()
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

        /* `S` with no folder open at all. With one open and no URL it asks
           for one instead of refusing, which is what attaching is. */
        let no_folder = sync_open(&mut cfg, &channel().0, &AtomicBool::new(false), &mut tracks)
            .unwrap_err()
            .to_string();
        assert!(no_folder.contains("no folder is open"), "{no_folder}");

        /* And refusing that question says the download did not happen, rather
           than leaving the reader to guess from a bare "cancelled". The
           receiver is dropped rather than left on a temporary, or the ask
           lands in a channel nobody is reading and blocks for good. */
        let mut open = vec![Track::new(1, "~a.opus".into(), "A".into(), dir.join("a.opus"))];
        let (dead, hung_up) = channel();
        drop(hung_up);
        let declined = sync_open(&mut cfg, &dead, &AtomicBool::new(false), &mut open)
            .unwrap_err()
            .to_string();
        assert!(declined.contains("nothing was downloaded"), "{declined}");

        // A folder of ours with nothing recorded in its sidecar.
        std::fs::write(dir.join(".earworm"), "#url\thttps://example.com\n").unwrap();
        let shelf = Shelf {
            path: dir.clone(),
            name: "Focus".into(),
            url: Some("u".into()),
            ..Shelf::default()
        };
        let empty = open_shelf(&mut config(true), &channel().0, &mut tracks, &shelf)
            .unwrap_err()
            .to_string();
        assert!(empty.contains("sync it to fill it in"), "{empty}");

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

    /* Attaching a URL to a folder earworm did not download runs a matching
       pass whose only backstop is somebody at the pick gate reading it. An
       unattended walk of the library has nobody there, so it must pass the
       folder by rather than download the whole playlist beside the files
       already in it. Nothing here reaches yt-dlp: every folder is URL-less,
       so a skip that was not happening would be a failed network call. */
    /* The row's pill has to come from the same resolver the sync asks, or
       the library can say one thing about a folder and a run another. Both
       sources, since the name and the header reach it by different paths. */
    #[test]
    fn a_library_row_carries_the_kind_from_the_name_or_the_header() {
        let root = scratch("library-kind");
        for name in ["Autobahn [album]", "Chill Evenings", "Kraftwerk"] {
            let folder = root.join(name);
            std::fs::create_dir_all(&folder).unwrap();
            std::fs::write(folder.join("01 - A.opus"), "audio").unwrap();
        }
        // What a sync that met an OLAK5uy_ listing leaves behind.
        std::fs::write(
            manifest::path(&root.join("Kraftwerk")),
            "#url\thttps://x\n#kind\talbum\n",
        )
        .unwrap();

        let kind = |name: &str| {
            library(&root)
                .into_iter()
                .find(|s| s.name == name)
                .unwrap()
                .kind
        };
        assert_eq!(kind("Autobahn [album]"), Some(manifest::Kind::Album));
        assert_eq!(kind("Kraftwerk"), Some(manifest::Kind::Album));
        assert_eq!(kind("Chill Evenings"), None, "a plain folder claimed a kind");
    }

    #[test]
    fn a_resync_passes_by_a_folder_with_no_url_and_says_which() {
        let root = scratch("resync-skip");
        for name in ["Copied", "Adopted"] {
            let folder = root.join(name);
            std::fs::create_dir_all(&folder).unwrap();
            std::fs::write(folder.join("01 - A.opus"), "audio").unwrap();
        }
        // The adopted shape: a sidecar with entries and no `#url`.
        std::fs::write(
            manifest::path(&root.join("Adopted")),
            "~01 - A.opus\t01 - A.opus\n",
        )
        .unwrap();

        let found = library(&root);
        assert_eq!(found.len(), 2, "the folders are not on the list at all");

        let (tx, rx) = mpsc::channel();
        let mut cfg = config(true);
        cfg.dir = root.clone();
        let cancel = AtomicBool::new(false);
        let mut tracks = Vec::new();
        let summary = resync(&mut cfg, &tx, &cancel, &mut tracks, found).unwrap();
        drop(tx);

        assert!(
            summary.starts_with("0 of 2 playlists"),
            "something was synced: {summary}"
        );
        assert!(summary.contains("2 folders have no URL"), "{summary}");
        let logs: Vec<String> = rx
            .into_iter()
            .filter_map(|m| match m {
                Msg::Log(line) => Some(line),
                _ => None,
            })
            .collect();
        for name in ["Copied", "Adopted"] {
            assert!(
                logs.iter().any(|l| l.starts_with(&format!("{name}: no playlist URL"))),
                "{name} was passed by with nothing saying why: {logs:?}"
            );
        }
        std::fs::remove_dir_all(&root).unwrap();
    }

    /* Every folder that holds a playlist, in a stable order, and no crash on
       a directory holding anything else. A recorded URL is what makes a row
       syncable, not what makes it a row: a folder somebody copied in holds
       the same tags and filenames and the library draws nothing else. */
    #[test]
    fn the_library_finds_every_folder_and_says_which_know_their_url() {
        let root = scratch("library");
        for (name, url) in [("Beta", "u-beta"), ("Alpha", "u-alpha")] {
            let folder = root.join(name);
            std::fs::create_dir_all(&folder).unwrap();
            manifest::write(&folder, url, std::iter::empty()).unwrap();
        }
        // A sidecar written before the `#url` header existed: it has entries
        // and no URL, which is the adopted folder's shape as well.
        let legacy = root.join("NoUrl");
        std::fs::create_dir_all(&legacy).unwrap();
        std::fs::write(legacy.join("file.opus"), "audio").unwrap();
        std::fs::write(manifest::path(&legacy), "vid\tfile.opus\n").unwrap();
        // Nothing of ours in it at all, which is the found folder.
        let copied_dir = root.join("Copied");
        std::fs::create_dir_all(&copied_dir).unwrap();
        std::fs::write(copied_dir.join("01 - A.m4a"), "audio").unwrap();
        std::fs::write(copied_dir.join("cover.jpg"), "not audio").unwrap();
        /* The AppleDouble macOS writes beside every file on a FAT or exFAT
           volume, which is exactly how a folder arrives off a USB stick. It
           carries the audio extension and is not audio, so the extension
           filter alone counts it as a track. */
        std::fs::write(copied_dir.join("._01 - A.m4a"), "resource fork").unwrap();
        std::fs::write(copied_dir.join(".DS_Store"), "junk").unwrap();
        // A directory under --dir that is not a playlist at all.
        std::fs::create_dir_all(root.join("Scratch")).unwrap();
        std::fs::write(root.join("Scratch/notes.txt"), "text").unwrap();
        std::fs::write(root.join("loose.opus"), "not a folder").unwrap();

        let found = library(&root);
        let names: Vec<&str> = found.iter().map(|s| s.name.as_str()).collect();
        assert_eq!(
            names,
            ["Alpha", "Beta", "Copied", "NoUrl"],
            "wrong folders or wrong order"
        );
        assert_eq!(found[0].url.as_deref(), Some("u-alpha"));
        let copied = &found[2];
        assert_eq!(copied.url, None, "a folder nobody synced claims a URL");
        assert_eq!(
            copied.files,
            vec![("01 - A.m4a".to_string(), true)],
            "something that is not a track was counted as one"
        );
        assert_eq!(copied.tracks, 1);
        assert_eq!(found[3].url, None, "a sidecar with no #url claims one");
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
                    Msg::Ask(prompt, reply) => {
                        reply.send(pick(&prompt, "forget")).unwrap();
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

    /* Driving the menu once and handing back what it offered, since what is
       on it is the thing these two are about. */
    fn remove_menu(
        cfg: &mut Config,
        folder: &Path,
        answer: &str,
    ) -> (Vec<String>, String, Vec<String>) {
        let (tx, rx) = mpsc::channel();
        std::thread::scope(|scope| {
            let worker = scope.spawn(|| {
                let asker = Asker { tx: tx.clone(), enabled: true };
                let mut tracks = Vec::new();
                remove_shelf(cfg, &tx, &asker, &mut tracks, folder)
            });
            let (mut rows, mut note, mut flashes) = (Vec::new(), String::new(), Vec::new());
            while let Ok(msg) = rx.recv_timeout(Duration::from_secs(5)) {
                match msg {
                    Msg::Ask(prompt, reply) => {
                        if let crate::app::Prompt::Choice { options, note: said, .. } = &prompt {
                            rows.clone_from(options);
                            note.clone_from(said);
                        }
                        reply.send(pick(&prompt, answer)).unwrap();
                    }
                    Msg::Flash(line) => flashes.push(line),
                    _ => {}
                }
                if !rows.is_empty() && worker.is_finished() {
                    break;
                }
            }
            worker.join().unwrap().unwrap();
            while let Ok(msg) = rx.try_recv() {
                if let Msg::Flash(line) = msg {
                    flashes.push(line);
                }
            }
            (rows, note, flashes)
        })
    }

    /* `D` used to refuse a folder earworm did not download outright, on the
       grounds that the only row left would be deleting somebody's music. It
       left no way to get such a row off the library at all, which is worse:
       the answer is a menu without the row that cannot apply, not no menu. */
    #[test]
    fn a_found_folder_is_offered_every_row_that_means_something() {
        let root = scratch("remove-found");
        let folder = root.join("deep-focus-chillstep");
        std::fs::create_dir_all(&folder).unwrap();
        for n in 1..=6 {
            std::fs::write(folder.join(format!("Chillstep #{n}.mp3")), "audio").unwrap();
        }
        let mut cfg = config(true);
        cfg.dir = root.clone();

        let (rows, note, _) = remove_menu(&mut cfg, &folder, "keep");
        assert!(
            !rows.iter().any(|r| r.starts_with("forget")),
            "offered to forget a sidecar that is not there: {rows:?}"
        );
        assert!(rows.iter().any(|r| r.starts_with("hide")), "{rows:?}");
        /* The count is the thing worth reading twice, and the header clips,
           so it goes in the row and the note carries the rest. */
        assert!(
            rows.iter().any(|r| r.contains("6 tracks")),
            "the delete row did not say what it would take: {rows:?}"
        );
        assert!(note.contains("nothing to forget"), "{note:?}");
        assert!(folder.is_dir(), "keeping removed something");
        std::fs::remove_dir_all(&root).unwrap();
    }

    /* The row goes and the music does not, which is the whole point of it:
       every other answer on that menu takes something off the disk. */
    #[test]
    fn hiding_a_folder_takes_the_row_and_leaves_every_file() {
        let root = scratch("remove-hide");
        let folder = root.join("deep-focus-chillstep");
        std::fs::create_dir_all(&folder).unwrap();
        let files: Vec<PathBuf> = (1..=3)
            .map(|n| folder.join(format!("Chillstep #{n}.mp3")))
            .collect();
        for file in &files {
            std::fs::write(file, "audio").unwrap();
        }
        let mut cfg = config(true);
        cfg.dir = root.clone();
        assert_eq!(library(&root).len(), 1, "the folder was not on the list to start with");

        let (_, _, flashes) = remove_menu(&mut cfg, &folder, "hide");

        assert!(library(&root).is_empty(), "the row survived being hidden");
        assert_eq!(
            hidden(&root),
            vec!["deep-focus-chillstep".to_string()],
            "nothing could say which folder had gone"
        );
        for file in &files {
            assert!(file.is_file(), "hiding took {} with it", file.display());
        }
        assert!(manifest::load(&folder).hidden, "nothing recorded it as hidden");
        /* The whole list, not `any`: read off the row index rather than the
           answer, the line after this one claimed the folder had been forgotten
           on a found folder and deleted on a synced one, and `any` cannot see a
           message that is there and wrong. */
        assert_eq!(flashes.len(), 1, "hiding said something else as well: {flashes:?}");
        /* A row that vanished with no way back is a folder somebody has lost,
           so the flash carries the undo: the line, not the file, since that
           file is where a synced folder keeps its URL and every id. */
        assert!(flashes[0].contains("#hidden"), "no word on how to bring it back: {flashes:?}");

        // And the way back is exactly what it says.
        let body = std::fs::read_to_string(manifest::path(&folder)).unwrap();
        let kept: String = body.lines().filter(|l| !l.starts_with("#hidden")).collect();
        std::fs::write(manifest::path(&folder), kept).unwrap();
        assert_eq!(library(&root).len(), 1, "dropping the line did not bring it back");
        std::fs::remove_dir_all(&root).unwrap();
    }

    /* The remove menu is a row shorter for a folder earworm did not download,
       so a test answering by index answers a different question depending on
       the fixture. Picked by the word the row opens with, which is what the
       reader picks by as well. */
    fn pick(prompt: &crate::app::Prompt, word: &str) -> Reply {
        let crate::app::Prompt::Choice { options, .. } = prompt else {
            panic!("that was not a menu");
        };
        let at = options
            .iter()
            .position(|row| row.starts_with(word))
            .unwrap_or_else(|| panic!("no {word:?} row in {options:?}"));
        Reply::Choice(at)
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
                if let Msg::Ask(prompt, reply) = msg {
                    reply.send(pick(&prompt, "delete")).unwrap();
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
                    Msg::Ask(prompt, reply) => {
                        reply.send(pick(&prompt, "keep")).unwrap();
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
            let (mut restarted, mut back, mut empty) = (false, false, Vec::new());
            while let Ok(msg) = rx.recv_timeout(Duration::from_secs(5)) {
                match msg {
                    Msg::Ask(prompt, reply) => {
                        reply.send(pick(&prompt, "forget")).unwrap();
                    }
                    Msg::Restart => restarted = true,
                    Msg::Library { shelves, show } => {
                        back = show;
                        empty = shelves.iter().map(|s| s.url.clone()).collect();
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
        /* Forgetting takes the sidecar and leaves the audio, and a folder
           holding audio is on the list either way: what it loses is its URL,
           which is what makes `R` pass it by and `S` ask for one. Deleting is
           the answer that takes the row, and it takes the music with it. */
        assert_eq!(empty, vec![None], "forgetting kept the playlist URL");
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
        assert_eq!(cfg.format, "opus");
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
        assert_eq!(cfg.format, "opus");
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
        let mut cover = CoverState { written: true, ..CoverState::default() };
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

    /* Adoption. The first edit in a folder earworm did not download writes a
       hidden file into somebody's music folder, which is a thing they should
       be told once rather than find out about by looking. Once, because a
       line on every edit is nagging, and the ids in it have to be earworm's
       own or every guard above has nothing to bite on.

       And the playlist: `write_playlist` runs on every edit and names its
       file after the folder, which in a folder somebody copied in may already
       be their own. Replacing it does not come back. */
    #[test]
    fn the_first_edit_adopts_the_folder_and_leaves_the_users_playlist_alone() {
        let dir = scratch("adopt");
        let file = dir.join("01 - A.opus");
        if tiny_opus(&file).is_none() {
            eprintln!("no ffmpeg, skipping");
            let _ = std::fs::remove_dir_all(&dir);
            return;
        }
        /* Named exactly what earworm would name its own, which is the case
           that cannot be told apart without a record of who wrote it. */
        let name = dir.file_name().unwrap().to_string_lossy().to_string();
        let theirs = dir.join(format!("{name}.m3u8"));
        let hand_written = "#EXTM3U\n# my favourites, in my order\n01 - A.opus\n";
        std::fs::write(&theirs, hand_written).unwrap();

        let shelf = library(dir.parent().unwrap())
            .into_iter()
            .find(|s| s.path == dir)
            .expect("the folder is not in the library");
        let mut cfg = config(false);
        cfg.dir = dir.parent().unwrap().to_path_buf();
        let mut tracks = Vec::new();
        open_shelf(&mut cfg, &channel().0, &mut tracks, &shelf).unwrap();
        assert!(cfg.url.is_empty(), "a folder nobody synced carries a URL");

        let flashes = |rx: &Receiver<Msg>| -> Vec<String> {
            rx.try_iter()
                .filter_map(|m| match m {
                    Msg::Flash(said) => Some(said),
                    _ => None,
                })
                .collect()
        };

        let (tx, rx) = channel();
        write_track(&cfg, &tx, &mut tracks, 0, named("Neu!", "Hallogallo"), "typed").unwrap();
        save_manifest(&cfg, &tx, &tracks);
        write_playlist(&cfg, &tracks).unwrap();
        let said = flashes(&rx);
        assert!(
            said.iter().any(|f| f.starts_with("adopted ")),
            "a sidecar appeared in the folder with nothing saying so: {said:?}"
        );

        // The sidecar is there, and its ids are earworm's own.
        let ids: Vec<String> = manifest::entries(&dir).into_iter().map(|(id, _)| id).collect();
        assert_eq!(ids.len(), 1);
        assert!(manifest::is_local(&ids[0]), "the edit invented a video id: {ids:?}");

        // And their playlist file is exactly as they left it.
        assert_eq!(
            std::fs::read_to_string(&theirs).unwrap(),
            hand_written,
            "the first edit replaced a playlist file earworm did not write"
        );

        // Said once. A second edit is no longer news about the folder.
        let (tx, rx) = channel();
        write_track(&cfg, &tx, &mut tracks, 0, named("Neu!", "Negativland"), "typed").unwrap();
        save_manifest(&cfg, &tx, &tracks);
        write_playlist(&cfg, &tracks).unwrap();
        let said = flashes(&rx);
        assert!(
            !said.iter().any(|f| f.starts_with("adopted ")),
            "every edit says it again: {said:?}"
        );
        assert_eq!(
            std::fs::read_to_string(&theirs).unwrap(),
            hand_written,
            "the second edit replaced the playlist the first one spared"
        );
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /* The other half of that rule: with no playlist file in the folder there
       is nothing to spare, so earworm writes one and records that it is its
       own. Without the record the next edit would find a file there, take it
       for the user's, and leave the playlist stale for good. */
    #[test]
    fn an_adopted_folder_with_no_playlist_file_gets_one_earworm_then_keeps() {
        let dir = scratch("adopt-m3u8");
        let file = dir.join("01 - A.opus");
        if tiny_opus(&file).is_none() {
            eprintln!("no ffmpeg, skipping");
            let _ = std::fs::remove_dir_all(&dir);
            return;
        }
        let name = dir.file_name().unwrap().to_string_lossy().to_string();
        let ours = dir.join(format!("{name}.m3u8"));

        let shelf = library(dir.parent().unwrap())
            .into_iter()
            .find(|s| s.path == dir)
            .expect("the folder is not in the library");
        let mut cfg = config(false);
        cfg.dir = dir.parent().unwrap().to_path_buf();
        let mut tracks = Vec::new();
        open_shelf(&mut cfg, &channel().0, &mut tracks, &shelf).unwrap();

        let (tx, _rx) = channel();
        write_track(&cfg, &tx, &mut tracks, 0, named("Neu!", "Hallogallo"), "typed").unwrap();
        save_manifest(&cfg, &tx, &tracks);
        write_playlist(&cfg, &tracks).unwrap();
        assert!(ours.is_file(), "no playlist was written at all");
        assert_eq!(
            manifest::load(&dir).playlist.as_deref(),
            Some(format!("{name}.m3u8").as_str()),
            "earworm wrote a playlist and did not record that it was its own"
        );

        write_track(&cfg, &tx, &mut tracks, 0, named("Neu!", "Negativland"), "typed").unwrap();
        write_playlist(&cfg, &tracks).unwrap();
        let after = std::fs::read_to_string(&ours).unwrap();
        assert!(
            after.contains("Negativland"),
            "earworm stopped updating its own playlist: {after}"
        );
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// A listed video the scan found no file for, which is what `reconcile`
    /// is handed.
    fn listed(index: usize, id: &str, title: &str, secs: u64, folder: &Path) -> Track {
        let mut track = Track::new(
            index,
            id.into(),
            title.into(),
            folder.join(format!("{index:02} - {title}.opus")),
        );
        track.duration = secs;
        track
    }

    /* The matching, rule by rule. Everything here turns on one thing: a file
       already in the folder must not be downloaded again, and a file the
       playlist does not have must not be called departed. */
    #[test]
    fn attaching_a_url_matches_the_files_already_in_the_folder() {
        let dir = scratch("reconcile");
        let tagged = dir.join("07 - whatever they called it.opus");
        /* Numbered 05 where the playlist puts it at 2, so the filename the
           scan predicts is not this one: rule 1 would otherwise cover rule 2
           and the test would pass with the title matching removed. */
        let by_name = dir.join("05 - Hallogallo.opus");
        let theirs = dir.join("99 - Something Else.opus");
        for file in [&tagged, &by_name, &theirs] {
            if tiny_opus(file).is_none() {
                eprintln!("no ffmpeg, skipping");
                let _ = std::fs::remove_dir_all(&dir);
                return;
            }
        }
        // The title tag wins over the filename, which here says nothing.
        tag::set_fields(
            &tagged,
            &tag::Fields {
                artist: "Stereolab".into(),
                title: "Ping Pong".into(),
                ..tag::Fields::default()
            },
        )
        .unwrap();

        let mut tracks = vec![
            // Matched on its title tag, against a file named nothing like it.
            listed(1, "vid1", "Ping Pong", 0, &dir),
            // Matched on the filename stem, with the number stripped off.
            listed(2, "vid2", "Hallogallo", 0, &dir),
            // In the playlist and genuinely not here: this one downloads.
            listed(3, "vid3", "Brand New", 0, &dir),
        ];
        let (tx, _rx) = channel();
        reconcile(&tx, &mut tracks);

        assert_eq!(tracks[0].status, Status::Have, "the title tag was not read");
        assert_eq!(tracks[0].path.as_deref(), Some(tagged.as_path()));
        assert_eq!(tracks[1].status, Status::Have, "the filename was not matched");
        assert_eq!(tracks[1].path.as_deref(), Some(by_name.as_path()));
        assert_eq!(
            tracks[2].status,
            Status::Pending,
            "a track the folder does not hold was called present"
        );

        /* And the file the playlist never had: on the list, settled, and not
           `Gone`, which would claim a video left and let `D` delete it. */
        assert_eq!(tracks.len(), 4, "the user's own file vanished from the list");
        let local = &tracks[3];
        assert_eq!(local.status, Status::Local);
        assert!(local.status.settled(), "the run would never finish");
        assert!(!local.listed, "it would be written into the .m3u8");
        assert!(!local.departure_proven, "D was offered somebody's own music");
        assert_eq!(local.path.as_deref(), Some(theirs.as_path()));
        assert!(manifest::is_local(&local.id), "{}", local.id);
        // Numbered past the playlist, or it collides with a real position.
        assert!(local.index > 3, "index {}", local.index);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /* Duration is a tiebreaker and never a rule of its own: every other song
       is three minutes, and matching on length alone would hand a video
       whatever file happens to be the same size. So it only ever chooses
       between files whose titles already matched. */
    #[test]
    fn two_files_with_one_title_are_told_apart_by_length_and_not_by_order() {
        let dir = scratch("reconcile-tie");
        // Three files of one title, numbered where the playlist does not put
        // them, so no predicted filename resolves any of this.
        let short = dir.join("11 - Intro.opus");
        let same = dir.join("12 - Intro.opus");
        let long = dir.join("13 - Intro.opus");
        if opus_of(&short, "1").is_none()
            || opus_of(&same, "1").is_none()
            || opus_of(&long, "6").is_none()
        {
            eprintln!("no ffmpeg, skipping");
            let _ = std::fs::remove_dir_all(&dir);
            return;
        }
        let (short_secs, long_secs) = (
            tag::read(&short).unwrap().duration,
            tag::read(&long).unwrap().duration,
        );
        if short_secs.abs_diff(long_secs) <= 2 {
            eprintln!("ffmpeg gave the fixtures the same length, skipping");
            let _ = std::fs::remove_dir_all(&dir);
            return;
        }

        /* The long one is third in the folder, so a match by position would
           take the first file and pass for the wrong reason. */
        let mut tracks = vec![listed(1, "vid1", "Intro", long_secs, &dir)];
        let (tx, _rx) = channel();
        reconcile(&tx, &mut tracks);
        assert_eq!(tracks[0].path.as_deref(), Some(long.as_path()), "matched by order");
        assert_eq!(tracks[0].status, Status::Have);

        /* And with two files equally good, nothing is matched: the track
           downloads, which costs a duplicate, where the wrong file costs the
           user their own music renamed to something it is not. */
        let mut blind = vec![listed(1, "vid1", "Intro", short_secs, &dir)];
        let (tx, rx) = channel();
        reconcile(&tx, &mut blind);
        drop(tx);
        assert_eq!(
            blind[0].status,
            Status::Pending,
            "one of two equally good files was picked anyway"
        );
        let logs: Vec<String> = rx
            .into_iter()
            .filter_map(|m| match m {
                Msg::Log(line) => Some(line),
                _ => None,
            })
            .collect();
        assert!(
            logs.iter().any(|l| l.contains("matches 3 files")),
            "the download was left unexplained: {logs:?}"
        );
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /* A matched file changes id: the `~` earworm invented becomes the video
       id the sync placed it against. Both lines name the same file, so
       keeping the old one would leave the sidecar holding two entries for one
       track, and the next sync would read the stale one back as a file the
       playlist does not have. */
    #[test]
    fn a_matched_file_has_its_invented_id_rewritten_in_place() {
        let dir = scratch("reconcile-id");
        // Numbered where the playlist does not put it, so the match has to be
        // the title rather than the filename the scan predicts.
        let file = dir.join("05 - Hallogallo.opus");
        if tiny_opus(&file).is_none() {
            eprintln!("no ffmpeg, skipping");
            let _ = std::fs::remove_dir_all(&dir);
            return;
        }
        // The sidecar an edit left behind, before any URL was attached.
        let local = manifest::local_id("05 - Hallogallo.opus");
        manifest::write(&dir, "", [(local.clone(), file.clone())].into_iter()).unwrap();

        let mut tracks = vec![listed(1, "vid1", "Hallogallo", 0, &dir)];
        let (tx, _rx) = channel();
        reconcile(&tx, &mut tracks);
        assert_eq!(tracks[0].status, Status::Have);
        tracks[0].listed = true;

        let mut cfg = config(true);
        cfg.url = "https://www.youtube.com/playlist?list=PL1".into();
        sync_manifest(&cfg, &tracks);

        let after = manifest::entries(&dir);
        assert_eq!(
            after,
            vec![("vid1".to_string(), file.clone())],
            "the sidecar kept a second line for the same file"
        );
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /* A file the playlist does not have is the one thing in the folder
       earworm has no claim on at all. The lookup must not retag it off a
       playlist it is not in, the `.m3u8` must not list it, and `D` must never
       reach it: `Gone` would give it all three, which is why it is not
       `Gone`. */
    #[test]
    fn a_local_file_is_not_retagged_not_listed_and_not_deletable() {
        let dir = scratch("local-left-alone");
        let listed_file = dir.join("01 - Mine.opus");
        let theirs = dir.join("77 - Theirs.opus");
        if tiny_opus(&listed_file).is_none() || tiny_opus(&theirs).is_none() {
            eprintln!("no ffmpeg, skipping");
            let _ = std::fs::remove_dir_all(&dir);
            return;
        }
        let was = tag::Fields {
            artist: "Somebody".into(),
            title: "Their Own Track".into(),
            album: "Their Own Record".into(),
            year: Some(1998),
        };
        tag::set_fields(&theirs, &was).unwrap();

        let mut listed_track =
            Track::new(1, "vid1".into(), "Mine".into(), listed_file.clone());
        listed_track.status = Status::Have;
        listed_track.listed = true;
        let mut local = Track::new(2, manifest::local_id("77 - Theirs.opus"), "Theirs".into(), theirs.clone());
        local.status = Status::Local;
        let mut tracks = vec![listed_track, local];

        let cfg = config(true);
        let (tx, _rx) = channel();
        // `enabled: false`, so any question the lookup asked would be refused
        // rather than hanging, and the row would come back changed.
        let asker = Asker { tx: tx.clone(), enabled: false };
        let mut cover = CoverState { written: true, ..CoverState::default() };
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

        assert_eq!(
            tracks[1].status,
            Status::Local,
            "the tag pass claimed a file the playlist does not have"
        );
        let after = tag::read(&theirs).unwrap();
        assert_eq!(after.artist, was.artist, "their tags were rewritten");
        assert_eq!(after.title, was.title);
        assert_eq!(after.album, was.album, "the playlist name was stamped on it");

        let playlist = write_playlist(&cfg, &tracks).unwrap().unwrap();
        let body = std::fs::read_to_string(&playlist).unwrap();
        assert!(body.contains("01 - Mine"), "the playlist lost its own track: {body}");
        assert!(
            !body.contains("Theirs"),
            "a file the playlist does not have was written into it: {body}"
        );

        // And `D` is not even drawn: it reads `Gone` plus a proven departure,
        // and a local row is neither.
        let (ui_tx, _ui_rx) = std::sync::mpsc::channel();
        let mut app = crate::app::App::new(ui_tx, String::new());
        app.tracks = tracks.clone();
        app.done = Some(Ok(String::new()));
        assert!(!app.can_purge(), "D was offered somebody's own music");
        assert_eq!(app.purgeable(), 0);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /* The gate is forced on for an attaching sync whatever asked for it.
       CLAUDE.md calls the `false` that `resync` passes load-bearing, and this
       is the one thing that overrides it: the matching above is a guess, and
       somebody reading "12 to download" against a folder they know holds
       twelve is the last defence against it being wrong. A literal at one
       call site still compiles and still passes the suite when flipped, which
       is exactly why it is pinned here. */
    #[test]
    fn an_attaching_sync_always_stops_at_the_pick_gate() {
        assert!(Pass::attaching().gate, "the matching would download unwatched");
        assert!(Pass::attaching().attaching);
        assert!(!Pass::plain(false).gate);
        assert!(!Pass::plain(false).attaching, "an ordinary sync reconciles");
        assert!(!Pass::plain(true).attaching);
    }

    /* A filename's leading number is earworm's own convention and most other
       people's, so it has to come off before a title is compared. A title
       that opens with a number is not one: there is no separator after it. */
    #[test]
    fn a_leading_track_number_comes_off_and_a_numeric_title_does_not() {
        for (stem, want) in [
            ("03 - Ping Pong", "Ping Pong"),
            ("03 Ping Pong", "03 Ping Pong"),
            ("03. Ping Pong", "Ping Pong"),
            ("03_Ping Pong", "Ping Pong"),
            ("1979", "1979"),
            ("99 Luftballons", "99 Luftballons"),
            ("Ping Pong", "Ping Pong"),
        ] {
            assert_eq!(strip_number(stem), want, "{stem}");
        }
    }

    /* Reopening an attached folder has to give a local file back the status
       the sync gave it. Read back as `Have` and `listed`, `mark_departed` then
       finds it missing from the `.m3u8` the sync correctly left it out of and
       calls it `gone`: the row claims a video left a playlist the file was
       never in, which is the one thing `Status::Local` exists to avoid. */
    #[test]
    fn a_local_file_is_still_local_when_the_folder_is_reopened() {
        let root = scratch("local-reopen");
        let dir = root.join("Focus");
        std::fs::create_dir_all(&dir).unwrap();
        let mine = dir.join("01 - Mine.opus");
        let theirs = dir.join("77 - Theirs.opus");
        if tiny_opus(&mine).is_none() || tiny_opus(&theirs).is_none() {
            eprintln!("no ffmpeg, skipping");
            let _ = std::fs::remove_dir_all(&root);
            return;
        }
        /* What an attaching sync leaves behind: a URL, the video it matched,
           and the file it could not place keeping the id earworm invented. */
        let local = manifest::local_id("77 - Theirs.opus");
        manifest::write_synced(
            &dir,
            "https://www.youtube.com/playlist?list=PL1",
            [
                ("vid1".to_string(), mine.clone()),
                (local.clone(), theirs.clone()),
            ]
            .into_iter(),
        )
        .unwrap();
        // The playlist that sync wrote, which correctly leaves the local file
        // out because it was never in the playlist.
        std::fs::write(dir.join("Focus.m3u8"), "#EXTM3U\n01 - Mine.opus\n").unwrap();

        let mut cfg = config(true);
        cfg.dir = root.clone();
        let shelf = library(&root).into_iter().find(|s| s.path == dir).unwrap();
        assert!(shelf.url.is_some(), "the fixture is not an attached folder");
        let mut tracks = Vec::new();
        open_shelf(&mut cfg, &channel().0, &mut tracks, &shelf).unwrap();

        let back = tracks
            .iter()
            .find(|t| t.path.as_deref() == Some(theirs.as_path()))
            .expect("the local file is not on the list at all");
        assert_eq!(
            back.status,
            Status::Local,
            "reopening turned the user's own file into a departure"
        );
        assert!(!back.listed, "it would be written into the next .m3u8");
        assert!(!back.departure_proven);
        // And the track the playlist does have is unaffected.
        let real = tracks
            .iter()
            .find(|t| t.path.as_deref() == Some(mine.as_path()))
            .expect("the playlist's own track went missing");
        assert_eq!(real.status, Status::Have);
        assert!(real.listed);
        std::fs::remove_dir_all(&root).unwrap();
    }

    /* The read-back path, which is where the last three bugs in this feature
       lived: every other `Local` test asserts on what `reconcile` leaves in
       memory, and the folder is reopened from disk far more often than it is
       attached. */
    #[test]
    fn reopening_an_attached_folder_keeps_the_playlists_own_numbers() {
        let root = scratch("local-number");
        let dir = root.join("Focus");
        std::fs::create_dir_all(&dir).unwrap();
        /* Both numbered 05: the user was numbering by their own scheme and it
           collides with the playlist's, which is the ordinary case rather than
           a contrived one. The local entry sits first in the sidecar, where a
           sync that could not place it would have written it. */
        let theirs = dir.join("05 - Theirs.opus");
        let mine = dir.join("05 - Mine.opus");
        for file in [&theirs, &mine] {
            if tiny_opus(file).is_none() {
                eprintln!("no ffmpeg, skipping");
                let _ = std::fs::remove_dir_all(&root);
                return;
            }
        }
        let local = manifest::local_id("05 - Theirs.opus");
        manifest::write_synced(
            &dir,
            "https://www.youtube.com/playlist?list=PL1",
            [
                (local.clone(), theirs.clone()),
                ("vid5".to_string(), mine.clone()),
            ]
            .into_iter(),
        )
        .unwrap();
        std::fs::write(dir.join("Focus.m3u8"), "#EXTM3U\n05 - Mine.opus\n").unwrap();

        let mut cfg = config(true);
        cfg.dir = root.clone();
        let shelf = library(&root).into_iter().find(|s| s.path == dir).unwrap();
        let mut tracks = Vec::new();
        open_shelf(&mut cfg, &channel().0, &mut tracks, &shelf).unwrap();

        let real = tracks
            .iter()
            .find(|t| t.path.as_deref() == Some(mine.as_path()))
            .expect("the playlist's own track went missing");
        assert_eq!(
            real.index, 5,
            "a local file took the playlist's number, so the real track sorts \
             to the bottom and the first edit renames it to that position"
        );
        let own = tracks
            .iter()
            .find(|t| t.path.as_deref() == Some(theirs.as_path()))
            .expect("the local file went missing");
        assert_eq!(own.status, Status::Local);
        assert!(own.index > 5, "a local row sits inside the playlist: {}", own.index);
        std::fs::remove_dir_all(&root).unwrap();
    }

    /* Without a URL the sidecar is only a tag ledger: it says which files
       happened to be there the last time somebody edited a tag, which is no
       claim about the folder. Preferred outright, a song copied into an
       adopted folder never appeared anywhere, and nothing would ever pick it
       up, because no sync can run against a folder with no URL. */
    #[test]
    fn a_song_added_to_an_adopted_folder_shows_up() {
        let root = scratch("adopted-grows");
        let dir = root.join("Copied");
        std::fs::create_dir_all(&dir).unwrap();
        let first = dir.join("01 - A.opus");
        if tiny_opus(&first).is_none() {
            eprintln!("no ffmpeg, skipping");
            let _ = std::fs::remove_dir_all(&root);
            return;
        }
        // Adopted: a sidecar naming the one file that was there at the time.
        let known = manifest::local_id("01 - A.opus");
        manifest::write(&dir, "", [(known.clone(), first.clone())].into_iter()).unwrap();

        // And then a second song is dropped in.
        let second = dir.join("02 - B.opus");
        if tiny_opus(&second).is_none() {
            let _ = std::fs::remove_dir_all(&root);
            return;
        }

        let shelf = library(&root)
            .into_iter()
            .find(|s| s.path == dir)
            .expect("the folder is not in the library");
        assert_eq!(shelf.tracks, 2, "the library row still counts one track");

        let mut cfg = config(true);
        cfg.dir = root.clone();
        let mut tracks = Vec::new();
        open_shelf(&mut cfg, &channel().0, &mut tracks, &shelf).unwrap();
        assert_eq!(tracks.len(), 2, "the new song is invisible on the track list");
        // The file the sidecar already knew keeps the id it was given, or the
        // entry it has would be orphaned beside a second one for one file.
        let old = tracks.iter().find(|t| t.path.as_deref() == Some(first.as_path())).unwrap();
        assert_eq!(old.id, known, "the adopted file was given a second id");
        let new = tracks.iter().find(|t| t.path.as_deref() == Some(second.as_path())).unwrap();
        assert!(manifest::is_local(&new.id), "{}", new.id);
        assert_eq!(new.status, Status::Have, "a folder with no URL has no local rows");
        assert!(new.listed);
        std::fs::remove_dir_all(&root).unwrap();
    }

    /* The folder a run is writing into has to reach the sidecar, or the next
       sync fetches the playlist again under its upstream title and leaves this
       folder beside it. Written by whichever manifest write happens first, so
       it arrives with the `#url` from the run that earned both rather than
       needing a second step afterwards that a failed sync must remember not to
       take. Never over a name somebody chose with `e`. */
    #[test]
    fn a_run_records_the_folder_it_is_writing_into() {
        let dir = scratch("pins-folder");
        let file = dir.join("01 - A.opus");
        std::fs::write(&file, "audio").unwrap();
        let mut tracks = vec![Track::new(1, "vid1".into(), "A".into(), file.clone())];
        tracks[0].status = Status::Have;

        let mut cfg = config(true);
        cfg.url = "https://www.youtube.com/playlist?list=PL1".into();
        cfg.folder = Some("Neon Venom".into());
        sync_manifest(&cfg, &tracks);

        let after = manifest::load(&dir);
        assert_eq!(after.name.as_deref(), Some("Neon Venom"), "the folder is not pinned");
        assert_eq!(after.url.as_deref(), Some("https://www.youtube.com/playlist?list=PL1"));
        assert_eq!(after.entries.len(), 1, "the header ate the entry");

        // And a name already recorded is one somebody chose, so it stands.
        cfg.folder = Some("Something Else".into());
        sync_manifest(&cfg, &tracks);
        assert_eq!(
            manifest::load(&dir).name.as_deref(),
            Some("Neon Venom"),
            "a later run overwrote the name the user chose"
        );
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /* A sync that fails on a typo'd URL or a dead network must leave the
       folder as it found it. The sidecar is the first thing ever written into
       a folder earworm did not download, and the rule is that a hidden file
       appearing in somebody's music folder is said out loud. */
    #[test]
    fn an_attach_that_fails_writes_nothing_into_the_folder() {
        let root = scratch("attach-fails");
        let dir = root.join("Copied");
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("01 - A.opus");
        std::fs::write(&file, "audio").unwrap();

        let mut cfg = config(true);
        cfg.dir = root.clone();
        cfg.url.clear();
        let mut tracks = vec![Track::new(
            1,
            manifest::local_id("01 - A.opus"),
            "A".into(),
            file.clone(),
        )];

        /* The URL question is refused, which is the cheapest failure to drive
           and takes the same road out as a failed listing. The receiver is
           dropped rather than left on a temporary, or the ask lands in a
           channel nobody reads and blocks for good. */
        let (dead, hung_up) = channel();
        drop(hung_up);
        assert!(sync_open(&mut cfg, &dead, &AtomicBool::new(false), &mut tracks).is_err());
        assert!(
            !manifest::path(&dir).is_file(),
            "a failed attach left a sidecar behind with nothing saying so"
        );
        std::fs::remove_dir_all(&root).unwrap();
    }

    /* A rename moves the playlist file with the folder, and the header saying
       which playlist file is earworm's own has to move with it. Left naming
       the old filename, `write_playlist` in a folder with no URL finds a file
       it does not recognise and refuses for good: earworm stops updating its
       own playlist after the rename, silently, for good. */
    #[test]
    fn renaming_an_adopted_folder_keeps_earworms_claim_on_its_playlist() {
        let root = scratch("rename-m3u8");
        let dir = root.join("Copied");
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("01 - A.opus");
        if tiny_opus(&file).is_none() {
            eprintln!("no ffmpeg, skipping");
            let _ = std::fs::remove_dir_all(&root);
            return;
        }

        let mut cfg = config(false);
        cfg.dir = root.clone();
        let shelf = library(&root).into_iter().find(|s| s.path == dir).unwrap();
        let mut tracks = Vec::new();
        open_shelf(&mut cfg, &channel().0, &mut tracks, &shelf).unwrap();

        // The first edit adopts the folder and writes earworm's own playlist.
        let (tx, _rx) = channel();
        write_track(&cfg, &tx, &mut tracks, 0, named("Neu!", "Hallogallo"), "typed").unwrap();
        save_manifest(&cfg, &tx, &tracks);
        write_playlist(&cfg, &tracks).unwrap();
        assert_eq!(manifest::load(&dir).playlist.as_deref(), Some("Copied.m3u8"));

        // Then the rename, answered by the UI on another thread.
        let (tx, rx) = mpsc::channel();
        let moved = std::thread::scope(|scope| {
            let worker = scope.spawn(|| {
                let asker = Asker { tx: tx.clone(), enabled: true };
                let mut own = config(false);
                own.dir = root.clone();
                rename_shelf(&mut own, &tx, &asker, &tracks, &dir)
            });
            while let Ok(msg) = rx.recv_timeout(Duration::from_secs(5)) {
                if let Msg::Ask(_, reply) = msg {
                    reply.send(Reply::Text("Neon Venom".into())).unwrap();
                    break;
                }
            }
            worker.join().unwrap()
        });
        moved.unwrap();

        let to = root.join("Neon Venom");
        assert!(to.is_dir(), "the folder did not move");
        assert_eq!(
            manifest::load(&to).playlist.as_deref(),
            Some("Neon Venom.m3u8"),
            "the claim was left naming a file that is no longer there"
        );

        /* And it still updates: without the header moving, this write is
           refused and the playlist names the old title for good. */
        let mut after = Vec::new();
        let shelf = library(&root).into_iter().find(|s| s.path == to).unwrap();
        let mut cfg = config(false);
        cfg.dir = root.clone();
        open_shelf(&mut cfg, &channel().0, &mut after, &shelf).unwrap();
        let (tx, _rx) = channel();
        write_track(&cfg, &tx, &mut after, 0, named("Neu!", "Negativland"), "typed").unwrap();
        write_playlist(&cfg, &after).unwrap();
        let body = std::fs::read_to_string(to.join("Neon Venom.m3u8")).unwrap();
        assert!(
            body.contains("Negativland"),
            "earworm stopped updating its own playlist after the rename: {body}"
        );
        std::fs::remove_dir_all(&root).unwrap();
    }

    /* Detection only ever promotes, never demotes, and the reason is that the
       two mistakes are not equal. Calling an album a playlist leaves today's
       behaviour in place; calling a playlist an album stamps one sleeve
       across a dozen unrelated records, which is the bug this exists to stop.
       Nothing in a flat listing separates a playlist from an album reached by
       a `PL` link, so silence has to mean playlist. */
    #[test]
    fn only_a_youtube_music_release_id_promotes_a_folder_to_an_album() {
        let root = scratch("detect-kind");
        let dir = root.join("Autobahn");
        std::fs::create_dir_all(&dir).unwrap();
        let (tx, _rx) = channel();

        // A hand-made playlist, whatever is in it.
        assert_eq!(settle_kind(Some("PLabc123"), Some(&dir)), None);
        assert_eq!(settle_kind(None, Some(&dir)), None);

        // A YouTube Music release, which is the one signal worth acting on.
        let said = settle_kind(Some("OLAK5uy_kAbCdEf"), Some(&dir));
        assert_eq!(said, Some((manifest::Kind::Album, "the playlist id")));
        assert!(
            !manifest::path(&dir).is_file(),
            "resolving wrote a header, which is `record_kind`'s job and not this one"
        );

        record_kind(&tx, Some(&dir), said);
        assert_eq!(
            manifest::load(&dir).kind,
            Some(manifest::Kind::Album),
            "the answer was not recorded, so opening the folder offline loses it"
        );

        /* And the recorded answer is what a later sync of the same folder
           reports even when the listing no longer says so, because offline
           that header is all there is. */
        assert_eq!(
            settle_kind(Some("PLabc123"), Some(&dir)),
            Some((manifest::Kind::Album, "an earlier sync"))
        );
        std::fs::remove_dir_all(&root).unwrap();
    }

    /* The folder does not exist until the download makes it, and the answer
       is wanted before the download: written where it is decided, the one run
       that discovered the album recorded nothing, the library showed no album
       until a second sync, and `r` in the same session re-resolved `playlist`
       and deleted the `cover.jpg` the run had just kept. */
    #[test]
    fn the_album_header_waits_for_the_folder_the_download_makes() {
        let root = scratch("kind-timing");
        let dir = root.join("Autobahn");
        let (tx, rx) = channel();

        let said = settle_kind(Some("OLAK5uy_kAbCdEf"), Some(&dir));
        assert_eq!(
            said,
            Some((manifest::Kind::Album, "the playlist id")),
            "a folder that is not there yet still has an answer"
        );
        // A run where nothing arrived says nothing: there is no folder to blame.
        record_kind(&tx, Some(&dir), said);
        drop(tx);
        let logs: Vec<String> = rx
            .iter()
            .filter_map(|m| match m {
                Msg::Log(line) => Some(line),
                _ => None,
            })
            .collect();
        assert!(logs.is_empty(), "a missing folder was reported as a failure: {logs:?}");

        std::fs::create_dir_all(&dir).unwrap();
        let (tx, _rx) = channel();
        record_kind(&tx, Some(&dir), said);
        assert_eq!(
            manifest::load(&dir).kind,
            Some(manifest::Kind::Album),
            "the header never arrived, so the library and a retry both read playlist"
        );
        std::fs::remove_dir_all(&root).unwrap();
    }

    /* The name is the override, so it has to beat the listing as well as the
       header, and it must leave no header behind: one that outlived a deleted
       marker would go on deciding with nothing on screen to explain it. */
    #[test]
    fn a_marked_folder_name_beats_detection_and_records_nothing() {
        let root = scratch("named-kind");
        let dir = root.join("Some Compilation [playlist]");
        std::fs::create_dir_all(&dir).unwrap();
        let (tx, _rx) = channel();

        let said = settle_kind(Some("OLAK5uy_kAbCdEf"), Some(&dir));
        assert_eq!(
            said,
            Some((manifest::Kind::Playlist, "the folder name")),
            "the release id overrode the name somebody typed"
        );
        record_kind(&tx, Some(&dir), said);
        assert!(
            !manifest::path(&dir).is_file(),
            "a marked name left a header behind, which outlives the marker"
        );
        std::fs::remove_dir_all(&root).unwrap();
    }

    /* The trap this whole feature is shaped around. An edit in a folder
       earworm did not download writes a sidecar, and the ids in it have to be
       invented. Attach a URL later and every one of them reaches three places
       that read ids and expect video ids:

         1. `scan` matches a listed video against the sidecar. A `~` id
            matching one would hand a local file to an unrelated video.
         2. `write_archive` names ids for yt-dlp, which knows nothing of them.
         3. `note_departures` calls every manifest id not in the listing
            departed, off a complete listing, and marks it proven, which is
            what lets `D` delete it. Every file the user copied in would be
            offered for deletion.

       Driven through a stub yt-dlp printing a listing that names neither
       file, which is the shape of attaching an unrelated URL and the worst
       case for all three. */
    #[test]
    fn a_local_id_is_never_matched_archived_or_called_departed() {
        let dir = scratch("localids");
        let theirs = dir.join("01 - Copied.opus");
        let predicted = dir.join("01 - Brand New.opus");
        if tiny_opus(&theirs).is_none() {
            eprintln!("no ffmpeg, skipping");
            let _ = std::fs::remove_dir_all(&dir);
            return;
        }
        let mut cfg = config(true);
        cfg.dir = dir.parent().unwrap().to_path_buf();
        cfg.url = "https://www.youtube.com/playlist?list=PL1".into();

        // The sidecar an edit in a found folder leaves behind: no URL yet, and
        // one entry whose id earworm invented from the filename.
        let local = manifest::local_id("01 - Copied.opus");
        manifest::write(&dir, "", [(local.clone(), theirs.clone())].into_iter()).unwrap();
        assert!(
            std::fs::read_to_string(manifest::path(&dir)).unwrap().contains(&local),
            "the sidecar did not keep the local id"
        );

        let bin = dir.join("yt-dlp");
        std::fs::write(
            &bin,
            format!(
                "#!/bin/sh\nprintf 'vid1\t1\tBrand New\t213\tPL1\tNA\t{}\\n'\n",
                predicted.display()
            ),
        )
        .unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&bin, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        let listing = crate::ytdlp::scan_with(&bin.display().to_string(), &cfg).unwrap();
        assert!(listing.complete, "an incomplete listing silences the departure half");
        let mut found = listing.tracks;

        // 1. The listed video is a download, not the file sitting on disk.
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].status, Status::Pending);
        assert_eq!(found[0].path.as_deref(), Some(predicted.as_path()));

        /* 2. The file the user copied in is not called departed, off a
              listing that never had any reason to name it. Before the local
              row joins the list, because that is the order a sync runs in and
              because a row already on it is excluded by the plain "not in the
              current tracks" filter, which would make this prove nothing. */
        let (tx, rx) = channel();
        note_departures(&cfg, &tx, listing.complete, &mut found);
        drop(tx);
        assert_eq!(
            found.len(),
            1,
            "the copied file was added as a departure: {:?}",
            found.iter().map(|t| (t.name.clone(), t.status)).collect::<Vec<_>>()
        );
        assert!(
            !found.iter().any(|t| t.departure_proven),
            "a file earworm never downloaded was offered to D"
        );
        /* Not merely silent about it: a doubt message would mean the guard
           above never ran and something else swallowed the departure. */
        let logs: Vec<String> = rx
            .into_iter()
            .filter_map(|m| match m {
                Msg::Log(line) => Some(line),
                _ => None,
            })
            .collect();
        assert!(logs.is_empty(), "the departure was doubted rather than skipped: {logs:?}");

        /* 3. And yt-dlp is handed nothing it cannot read. The local file is
              on the list here because that is what a reconciling sync builds:
              the listing's tracks plus the files on disk it could not place,
              and `write_archive` names every id whose file exists. */
        let mut local_track = Track::new(2, local.clone(), "Copied".into(), theirs.clone());
        local_track.status = Status::Have;
        found.push(local_track);
        let archive = dir.join("archive");
        crate::ytdlp::write_archive(&found, &archive, &HashSet::new()).unwrap();
        let written = std::fs::read_to_string(&archive).unwrap();
        assert!(
            !written.contains('~'),
            "an id earworm invented reached the yt-dlp archive: {written:?}"
        );

        /* And the entry survives the sync that did not match it: dropped
           here, the next one would have no record of the file at all. */
        sync_manifest(&cfg, &found);
        let after: Vec<String> = manifest::entries(&dir).into_iter().map(|(id, _)| id).collect();
        assert!(after.contains(&local), "the local entry was rewritten away: {after:?}");
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
                "#!/bin/sh\nprintf 'vid1\t1\tOriginal Title\t213\tPL1\tNA\t{}\\n'\n",
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
        let mut cover = CoverState { written: true, ..CoverState::default() };
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
        let mut cover = CoverState { written: true, ..CoverState::default() };
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
        opus_of(at, "0.2")
    }

    /// A fixture of a chosen length, for the matching that breaks a tie on it.
    pub fn opus_of(at: &Path, seconds: &str) -> Option<()> {
        let made = std::process::Command::new("ffmpeg")
            .args(["-v", "error", "-f", "lavfi", "-i", "anullsrc=r=48000:cl=mono"])
            .args(["-t", seconds, "-c:a", "libopus", "-y"])
            .arg(at)
            .status()
            .ok()?;
        made.success().then_some(())
    }

    /* The line that routes the album tag through the folder's one answer,
       which nothing else reaches: `tag::apply` needs a `Match`, and during a
       run that only ever comes off the network. Built by hand here, with the
       cover off so the same call touches nothing. Track two matched a
       compilation, which is exactly the case the sharing exists for: without
       it the file wears "Now 47" beside the record's own sleeve. */
    #[test]
    fn an_albums_files_all_read_back_the_same_album_tag() {
        let dir = scratch("album-tag");
        let (one, two) = (dir.join("01 - A.opus"), dir.join("02 - B.opus"));
        if tiny_opus(&one).is_none() || tiny_opus(&two).is_none() {
            eprintln!("no ffmpeg, skipping");
            return;
        }
        let (tx, _rx) = channel();
        let asker = Asker { tx, enabled: false };
        let mut cfg = config(false);
        cfg.cover = false;
        let found = |title: &str, album: &str| crate::lookup::Match {
            title: title.into(),
            artist: "Kraftwerk".into(),
            album: Some(album.into()),
            mbid: None,
            cover_url: None,
            score: None,
            isrc: None,
            source: "test",
        };

        let mut cover = CoverState {
            one_sleeve: true,
            ..CoverState::default()
        };
        for (path, m) in [
            (&one, found("Autobahn", "Autobahn")),
            (&two, found("Kometenmelodie", "Now 47")),
        ] {
            tag::apply(path, &m, &cfg, &mut cover, &asker, 1, "").unwrap();
        }

        assert_eq!(tag::read(&one).unwrap().album, "Autobahn");
        assert_eq!(
            tag::read(&two).unwrap().album,
            "Autobahn",
            "one record's files disagree about which record they are"
        );
        std::fs::remove_dir_all(&dir).unwrap();
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
        assert_eq!(Some(cfg.url.clone()), shelf.url, "S would have no URL to sync from");
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
        save_manifest(&config(true), &channel().0, &tracks);

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

    /* Nothing in a folder earworm did not download says what is in it but
       the files themselves, so the track list is built from the directory and
       the tags. Read-only: opening must write nothing, because the folder is
       the user's until they change something in it. */
    #[test]
    fn a_folder_earworm_never_downloaded_opens_from_its_own_files() {
        let dir = scratch("found-open");
        let files = ["03 - Third.opus", "01 - First.opus", "02 - Second.opus"];
        for name in files {
            if tiny_opus(&dir.join(name)).is_none() {
                eprintln!("no ffmpeg, skipping");
                let _ = std::fs::remove_dir_all(&dir);
                return;
            }
        }
        // Not audio, and not a track.
        std::fs::write(dir.join("cover.jpg"), "image").unwrap();
        tag::set_fields(
            &dir.join("01 - First.opus"),
            &tag::Fields {
                artist: "Stereolab".into(),
                title: "Ping Pong".into(),
                album: "Mars Audiac Quintet".into(),
                year: Some(1994),
            },
        )
        .unwrap();

        let shelf = library(dir.parent().unwrap())
            .into_iter()
            .find(|s| s.path == dir)
            .expect("the folder is not in the library");
        assert_eq!(shelf.url, None);

        let mut tracks = Vec::new();
        let summary = open_shelf(&mut config(true), &channel().0, &mut tracks, &shelf).unwrap();
        assert!(summary.contains("3 tracks"), "{summary}");

        // Numbered off the filenames, which is playlist order for a folder
        // anybody made with a numbering scheme, ours included.
        assert_eq!(
            tracks.iter().map(|t| t.index).collect::<Vec<_>>(),
            [1, 2, 3],
            "the rows were numbered by position rather than by name"
        );
        assert!(
            tracks.iter().all(|t| t.status == Status::Have && t.listed),
            "a file sitting in the folder was reported as anything but present"
        );
        /* The ids are earworm's own, which is what stops a later sync
           matching them against a listing or handing them to yt-dlp. */
        assert!(
            tracks.iter().all(|t| manifest::is_local(&t.id)),
            "a found file was given an id a listing could match: {:?}",
            tracks.iter().map(|t| t.id.clone()).collect::<Vec<_>>()
        );
        let first = &tracks[0];
        assert_eq!(first.artist, "Stereolab");
        assert_eq!(first.title, "Ping Pong");
        assert_eq!(first.album, "Mars Audiac Quintet");
        assert_eq!(first.year, Some(1994));
        // An untagged file falls back to its filename rather than to "? - ?".
        assert!(tracks[1].name.contains("Second"), "{}", tracks[1].name);

        assert!(
            !manifest::path(&dir).is_file(),
            "opening wrote a sidecar into a folder nobody has edited"
        );
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /* `mark_departed` reads the `.m3u8` as earworm's own last answer about
       the playlist, and in a folder earworm did not download it is nothing of
       the kind: it is whatever file the user brought, listing whatever they
       chose. Believed, it calls every file it leaves out departed, and a
       departed row is never renamed, never re-tagged, and dropped from the
       next playlist written. */
    #[test]
    fn a_found_folders_own_playlist_file_does_not_depart_the_rest_of_it() {
        let dir = scratch("found-m3u8");
        for name in ["01 - A.opus", "02 - B.opus", "03 - C.opus"] {
            if tiny_opus(&dir.join(name)).is_none() {
                eprintln!("no ffmpeg, skipping");
                let _ = std::fs::remove_dir_all(&dir);
                return;
            }
        }
        /* Named after the folder, so it is exactly the file `last_playlist`
           looks for, and listing half the folder, which is what a hand-made
           playlist of favourites looks like. */
        let name = dir.file_name().unwrap().to_string_lossy().to_string();
        std::fs::write(
            dir.join(format!("{name}.m3u8")),
            "#EXTM3U\n01 - A.opus\n",
        )
        .unwrap();

        let shelf = library(dir.parent().unwrap())
            .into_iter()
            .find(|s| s.path == dir)
            .expect("the folder is not in the library");
        let mut tracks = Vec::new();
        open_shelf(&mut config(true), &channel().0, &mut tracks, &shelf).unwrap();

        assert_eq!(tracks.len(), 3);
        assert!(
            tracks.iter().all(|t| t.status != Status::Gone),
            "the user's own playlist file was believed about a folder earworm did not sync: {:?}",
            tracks.iter().map(|t| (t.name.clone(), t.status)).collect::<Vec<_>>()
        );
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
        save_manifest(&config(true), &channel().0, &tracks);

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

        drop_folder_cover(&tracks, &[], false);

        for name in COVER_NAMES {
            assert!(!dir.join(name).exists(), "{name} survived the pass");
        }
        assert!(dir.join("01 - a - b.opus").is_file(), "a track went with it");
        assert!(dir.join("list.m3u8").is_file(), "the playlist went with it");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /* An album is the other exception, and the reason is that the image is
       no longer arbitrary. On a playlist `cover.jpg` is whichever track
       resolved art first, wearing the folder's name, and dropping it is right.
       On an album it is that album's sleeve and matches every file in the
       folder, which is what a file manager shows and what most players want. */
    #[test]
    fn an_albums_folder_image_is_the_albums_sleeve_and_stays() {
        let dir = scratch("albumcover");
        let tracks = vec![track_at(&dir, "01 - a - b.opus", Status::Have)];
        for name in COVER_NAMES {
            std::fs::write(dir.join(name), "sleeve").unwrap();
        }

        drop_folder_cover(&tracks, &[], true);

        for name in COVER_NAMES {
            assert!(
                dir.join(name).is_file(),
                "{name} was dropped from an album, which is its own sleeve"
            );
        }
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

        drop_folder_cover(&tracks, &theirs, false);

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
            icons: crate::icons::Icons::Text,
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
            // The fixtures are all `.opus`, whatever the shipped default is.
            format: "opus".into(),
            // Off, so a test that does not ask for it converts nothing.
            convert: false,
            loudness: false,
            lyrics: false,
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
            achievements: true,
            folder: None,
            extra: Vec::new(),
            acoustid_key: None,
            unlocked: Default::default(),
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
        assert_eq!(rows.len(), 5);
        assert_eq!(rows[0].1, Pick::Url("http://a".into()));
        assert_eq!(rows[1].1, Pick::Url("http://b".into()));
        assert_eq!(rows[2].1, Pick::Keep);
        assert_eq!(rows[3].1, Pick::Restore);
        assert_eq!(rows[4].1, Pick::OwnFile);
    }

    /// The empty case is the one that breaks silently: with nothing found,
    /// every row is one of the fixed ones, and an index still expecting a
    /// leading artwork row resolves the pick to the wrong action entirely.
    #[test]
    fn nothing_found_leaves_only_the_rows_that_need_no_artwork() {
        let rows = cover_options(&[], None);
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].1, Pick::Restore);
        assert_eq!(rows[1].1, Pick::OwnFile);
    }

    #[test]
    fn keep_row_is_absent_without_a_current_cover() {
        let rows = cover_options(&[art("Deezer  A", "http://a")], None);
        assert!(!rows.iter().any(|(_, pick)| *pick == Pick::Keep));
        assert_eq!(rows.last().map(|(_, pick)| pick), Some(&Pick::OwnFile));
    }

    /* The row deletes a file, so its label is a claim about the folder. With
       no image there it would be offering to remove one that is not there,
       which reads as a menu that has not looked. */
    #[test]
    fn the_restore_row_names_the_folder_image_only_when_there_is_one() {
        let label = |current| {
            cover_options(&[], current)
                .into_iter()
                .find(|(_, pick)| *pick == Pick::Restore)
                .map(|(label, _)| label)
                .expect("no restore row was offered")
        };
        assert!(
            label(Some("(cover.jpg  500x500)".into())).starts_with("remove the folder cover"),
            "the row did not say it would remove the image that is there"
        );
        let bare = label(None);
        assert!(
            !bare.contains("remove"),
            "the row offered to remove a folder cover that does not exist: {bare}"
        );
        assert!(bare.contains("own art"), "the row stopped saying what it does");
    }

    /* Both names, because `tag::apply` writes a JPEG and `c` can leave a PNG:
       taking only one away leaves the folder still carrying a shared sleeve,
       which is the entire thing this row exists to remove. No tracks, so
       nothing here reaches the network. */
    #[test]
    fn restoring_takes_every_folder_image_away() {
        let dir = scratch("restore");
        std::fs::create_dir_all(&dir).unwrap();
        for name in COVER_NAMES {
            std::fs::write(dir.join(name), b"not really an image").unwrap();
        }
        let (tx, rx) = std::sync::mpsc::channel();
        let asker = Asker { tx: tx.clone(), enabled: false };
        restore_art(&tx, &[], &asker, &AtomicBool::new(false), &dir, false).unwrap();

        for name in COVER_NAMES {
            assert!(!dir.join(name).exists(), "{name} survived the restore");
        }
        let said: Vec<String> = rx
            .try_iter()
            .filter_map(|msg| match msg {
                Msg::Flash(text) => Some(text),
                _ => None,
            })
            .collect();
        assert!(
            said.iter().any(|text| text.contains("folder cover removed")),
            "nothing said the cover had gone: {said:?}"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /* `cover_options` offering the row and `restore_art` doing the work prove
       nothing about the line between them, which is free to call anything at
       all: swapping it for `cancelled` leaves the folder image in place and
       every other test still green. The same lesson `scan_template` taught,
       so this drives the real menu and answers it.

       No network: a track with no artist, title or MusicBrainz id gives
       `lookup::artwork` nothing to search with, so it returns an empty list
       without a request, both for the menu and for the track itself. */
    #[test]
    fn choosing_the_restore_row_is_wired_to_the_restore() {
        let dir = scratch("restore-wired");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("cover.jpg"), b"a shared sleeve").unwrap();
        let audio = dir.join("01 - A.opus");
        std::fs::write(&audio, b"audio").unwrap();
        let mut track = Track::new(1, "v1".into(), "01 - A.opus".into(), audio);
        track.listed = true;
        let mut tracks = vec![track];

        /* Asked of the same function the menu is built from, so reordering
           the rows moves the answer with them rather than silently picking
           whichever row slid into that position. */
        let at = cover_options(&[], current_cover(&dir))
            .iter()
            .position(|(_, pick)| *pick == Pick::Restore)
            .expect("the menu offered no restore row");

        let (tx, rx) = mpsc::channel();
        let asked = std::thread::scope(|scope| {
            let worker = scope.spawn(|| {
                let asker = Asker { tx: tx.clone(), enabled: true };
                cover(&tx, &mut tracks, &asker, &AtomicBool::new(false), 1, false)
            });
            let mut asked = 0;
            /* Bounded, or a menu that stopped asking waits here for good, and
               stopped at `Artwork`, which is the last thing the restore sends
               before its flash: waiting for the channel to close instead
               means waiting out the timeout on every green run. */
            while let Ok(msg) = rx.recv_timeout(Duration::from_secs(5)) {
                match msg {
                    Msg::Ask(_, reply) => {
                        // The row itself, then the confirmation behind it.
                        let answer = if asked == 0 { at } else { 1 };
                        reply.send(Reply::Choice(answer)).unwrap();
                        asked += 1;
                    }
                    Msg::Artwork => break,
                    _ => {}
                }
            }
            worker.join().unwrap().unwrap();
            asked
        });

        assert_eq!(asked, 2, "the row did not ask before rewriting the files");
        assert!(
            !dir.join("cover.jpg").exists(),
            "the row was picked and the folder cover stayed"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /* The other answer, and the one that has to leave the folder exactly as it
       was: this row has two halves and refusing it must abandon both. Dropping
       the early return still passes the test above, which answers "restore",
       so the refusal needs its own. */
    #[test]
    fn refusing_the_confirmation_changes_nothing_at_all() {
        let dir = scratch("restore-refused");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("cover.jpg"), b"a shared sleeve").unwrap();
        let audio = dir.join("01 - A.opus");
        std::fs::write(&audio, b"audio").unwrap();
        let mut track = Track::new(1, "v1".into(), "01 - A.opus".into(), audio.clone());
        track.listed = true;
        let tracks = vec![track];

        let (tx, rx) = mpsc::channel();
        std::thread::scope(|scope| {
            let worker = scope.spawn(|| {
                let asker = Asker { tx: tx.clone(), enabled: true };
                restore_art(&tx, &tracks, &asker, &AtomicBool::new(false), &dir, false)
            });
            while let Ok(msg) = rx.recv_timeout(Duration::from_secs(5)) {
                match msg {
                    Msg::Ask(_, reply) => reply.send(Reply::Choice(0)).unwrap(),
                    Msg::Flash(_) => break,
                    _ => {}
                }
            }
            worker.join().unwrap().unwrap();
        });

        assert!(
            dir.join("cover.jpg").is_file(),
            "refusing still took the folder cover away"
        );
        assert_eq!(
            std::fs::read(&audio).unwrap(),
            b"audio",
            "refusing still rewrote a track"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /* A recorded path is not a file on disk. A `Gone` track keeps the path of
       one that has been deleted, and without the `is_file` filter each of
       those costs a lookup and a failed write, then lands in the same count
       as a track whose art could not be found.

       The disabled asker is what makes that visible here: it refuses every
       question, so a folder that wrongly believes it has a file to rewrite
       puts a confirmation in the way, gets refused, and keeps its cover. With
       the filter there is nothing to confirm and the cover goes. */
    #[test]
    fn a_track_whose_file_has_gone_is_never_looked_up() {
        let dir = scratch("restore-gone");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("cover.jpg"), b"a shared sleeve").unwrap();
        let mut track = Track::new(1, "v1".into(), "01 - A.opus".into(), dir.join("01 - A.opus"));
        track.listed = true;

        let (tx, _rx) = mpsc::channel();
        let asker = Asker { tx: tx.clone(), enabled: false };
        restore_art(&tx, &[track], &asker, &AtomicBool::new(false), &dir, false).unwrap();

        assert!(
            !dir.join("cover.jpg").exists(),
            "a track with no file behind it was treated as one worth rewriting"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /* The files change behind paths that do not, so the pane has no other way
       to learn its decoded cover is stale. Without this it goes on showing the
       sleeve that was just replaced, which is the one thing that would make
       the whole row look as though it had done nothing. */
    #[test]
    fn a_restore_tells_the_pane_its_cover_is_out_of_date() {
        let dir = scratch("restore-says");
        std::fs::create_dir_all(&dir).unwrap();
        let (tx, rx) = std::sync::mpsc::channel();
        let asker = Asker { tx: tx.clone(), enabled: false };
        restore_art(&tx, &[], &asker, &AtomicBool::new(false), &dir, false).unwrap();
        assert!(
            rx.try_iter().any(|msg| matches!(msg, Msg::Artwork)),
            "the pane was never told the art had changed"
        );
        let _ = std::fs::remove_dir_all(&dir);
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

        assert_eq!(cfg.format, "opus");
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
                    // The targets a download copies rather than re-encodes.
                    (_, "opus" | "m4a") => Bring::Refetch,
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

        // The pair sharing an extension: a refetch would land on the file already there.
        let alac = holding(&dir, 2, "alac.m4a", Status::Have);
        assert_eq!(bring(&alac, "m4a"), Bring::Convert);

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
    // The swap used to carry four fields and the picture, so lyrics somebody typed and the ISRC the sidecar's
    // `#isrc` line is built from were gone from every converted file.
    #[test]
    fn a_swap_carries_the_lyrics_and_the_isrc_too() {
        let dir = scratch("swap-extras");
        let old = dir.join("01 - Song.opus");
        if super::tests::tiny_opus(&old).is_none() {
            eprintln!("no ffmpeg, skipping");
            return;
        }
        tag::set_fields(&old, &tag::Fields { artist: "A".into(), title: "Song".into(), album: String::new(), year: None }).unwrap();
        tag::set_extra(&old, tag::Extra::Lyrics, "words I typed").unwrap();
        tag::set_isrc(&old, "GBAYE9200070").unwrap();
        // A fresh download holds none of it, which is what a refetch hands back.
        let fresh = dir.join("fresh.flac");
        if tag::transcode(&old, &fresh, "flac", Some("opus")).is_err() {
            eprintln!("no flac encoder, skipping");
            return;
        }
        tag::set_extra(&fresh, tag::Extra::Lyrics, "").unwrap();

        let (tx, _rx) = mpsc::channel();
        let mut track = holding(&dir, 1, "01 - Song.opus", Status::Have);
        swap_in(&tx, &mut track, &fresh, "flac", &HashSet::new()).unwrap();
        let landed = dir.join("01 - Song.flac");
        assert_eq!(tag::extra(&landed, tag::Extra::Lyrics).unwrap().as_deref(), Some("words I typed"));
        assert_eq!(tag::read(&landed).unwrap().isrc.as_deref(), Some("GBAYE9200070"));
        std::fs::remove_dir_all(&dir).unwrap();
    }

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
            url: Some("u".into()),
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
            ..Shelf::default()
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

    // Off by default, so a plain sync must leave a file's tags exactly as they were.
    #[test]
    fn enrich_writes_loudness_only_when_it_is_on() {
        let dir = std::env::temp_dir().join(format!("earworm-enrich-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("01 - A.flac");
        let made = std::process::Command::new("ffmpeg")
            .args(["-v", "error", "-f", "lavfi", "-i", "sine=f=440:d=2", "-ac", "2", "-y"])
            .arg(&file)
            .status();
        if !made.is_ok_and(|s| s.success()) {
            eprintln!("skipped: ffmpeg could not make a fixture");
            std::fs::remove_dir_all(&dir).unwrap();
            return;
        }
        let track = Track::new(1, "a".into(), "01 - A.flac".into(), file.clone());
        let (tx, rx) = mpsc::channel();
        let mut cfg = super::tests::config(false);

        super::enrich(&cfg, &tx, &track, &file, "");
        assert!(tag::extra(&file, tag::Extra::TrackGain).unwrap().is_none(), "written with loudness off");

        cfg.loudness = true;
        super::enrich(&cfg, &tx, &track, &file, "");
        assert!(tag::extra(&file, tag::Extra::TrackGain).unwrap().is_some(), "not written with loudness on");
        assert!(rx.try_iter().any(|m| matches!(m, Msg::Log(l) if l.contains("loudness"))));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn the_playlist_name_filled_in_as_an_album_is_not_asked_about() {
        assert_eq!(super::real_album("Focus", "Focus"), "");
        assert_eq!(super::real_album("Autobahn", "Focus"), "Autobahn");
        assert_eq!(super::real_album("", "Focus"), "");
    }

    #[test]
    fn enrich_asks_for_lyrics_only_for_a_track_without_them() {
        let dir = std::env::temp_dir().join(format!("earworm-lyrics-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("01 - A.flac");
        let made = std::process::Command::new("ffmpeg")
            .args(["-v", "error", "-f", "lavfi", "-i", "anullsrc=r=44100:cl=stereo", "-t", "0.2", "-y"])
            .arg(&file)
            .status();
        if !made.is_ok_and(|s| s.success()) {
            eprintln!("skipped: ffmpeg could not make a fixture");
            std::fs::remove_dir_all(&dir).unwrap();
            return;
        }
        let mut track = Track::new(1, "a".into(), "01 - A.flac".into(), file.clone());
        track.artist = "Kraftwerk".into();
        track.title = "Autobahn".into();
        track.duration = 213;
        let (tx, _rx) = mpsc::channel();
        let calls = std::cell::Cell::new(0);
        let fetch = |artist: &str, title: &str, _album: &str, duration: u64| {
            calls.set(calls.get() + 1);
            assert_eq!((artist, title, duration), ("Kraftwerk", "Autobahn", 213));
            lookup::Lookup::Found("Fahren fahren fahren".to_string())
        };
        let mut cfg = super::tests::config(false);

        super::enrich_with(&cfg, &tx, &track, &file, "", fetch);
        assert_eq!(calls.get(), 0, "asked with lyrics off");

        cfg.lyrics = true;
        let found = super::enrich_with(&cfg, &tx, &track, &file, "", fetch);
        assert_eq!(calls.get(), 1);
        assert_eq!((found.lyrics_found, found.lyrics_missed), (1, 0));
        assert_eq!(tag::extra(&file, tag::Extra::Lyrics).unwrap().as_deref(), Some("Fahren fahren fahren"));

        // Already has them: a rerun is free, and what is there stays.
        tag::set_extra(&file, tag::Extra::Lyrics, "mine").unwrap();
        let had = super::enrich_with(&cfg, &tx, &track, &file, "", fetch);
        assert_eq!(had, super::Extras::default(), "a track that had lyrics was counted");
        assert_eq!(calls.get(), 1, "asked again for a track that had lyrics");
        assert_eq!(tag::extra(&file, tag::Extra::Lyrics).unwrap().as_deref(), Some("mine"));

        // No answer writes nothing.
        tag::set_extra(&file, tag::Extra::Lyrics, "").unwrap();
        let missed = super::enrich_with(&cfg, &tx, &track, &file, "", |_: &str, _: &str, _: &str, _: u64| lookup::Lookup::Missing);
        assert_eq!((missed.lyrics_found, missed.lyrics_missed), (0, 1));
        assert_eq!(tag::extra(&file, tag::Extra::Lyrics).unwrap(), None);

        // A service that did not answer is neither a hit nor a miss, and a track not asked about is neither.
        let down = super::enrich_with(&cfg, &tx, &track, &file, "", |_: &str, _: &str, _: &str, _: u64| lookup::Lookup::Unavailable);
        assert_eq!(down, super::Extras { lyrics_unavailable: 1, ..super::Extras::default() });
        let skipped = super::enrich_with(&cfg, &tx, &track, &file, "", |_: &str, _: &str, _: &str, _: u64| lookup::Lookup::Skipped);
        assert_eq!(skipped, super::Extras::default());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn the_summary_counts_lyrics_only_for_tracks_that_were_searched() {
        let none = super::Extras::default();
        assert_eq!(none.said(), None, "said something when nothing was asked");
        let mut sum = none;
        sum.add(super::Extras { lyrics_found: 1, lyrics_missed: 0, lyrics_unavailable: 0 });
        sum.add(super::Extras { lyrics_found: 0, lyrics_missed: 2, lyrics_unavailable: 0 });
        sum.add(none);
        assert_eq!(sum.said().as_deref(), Some("lyrics found for 1 of 3 searched"));
        // Every track missing is still worth saying: it is the line that explains empty tags.
        let all = super::Extras { lyrics_found: 0, lyrics_missed: 4, lyrics_unavailable: 0 };
        assert_eq!(all.said().as_deref(), Some("lyrics found for 0 of 4 searched"));
        // An outage is said on its own, and beside the count when some were asked before it began.
        let down = super::Extras { lyrics_unavailable: 7, ..none };
        assert_eq!(down.said().as_deref(), Some("lyrics service not answering, 7 left for the next sync"));
        let both = super::Extras { lyrics_found: 2, lyrics_missed: 1, lyrics_unavailable: 5 };
        assert_eq!(
            both.said().as_deref(),
            Some("lyrics found for 2 of 3 searched, lyrics service not answering, 5 left for the next sync")
        );
    }
}

#[cfg(test)]
mod chapter_tests {
    use super::*;
    use crate::app::Reply;
    use std::sync::mpsc::Receiver;

    fn chapter(title: &str, start: f64, end: f64) -> ytdlp::Chapter {
        ytdlp::Chapter { title: title.into(), start, end }
    }

    fn three() -> Vec<ytdlp::Chapter> {
        vec![chapter("Intro", 0.0, 4.0), chapter("Middle", 4.0, 8.0), chapter("End", 8.0, 12.0)]
    }

    fn folder(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("earworm-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    // A video the scan predicted a path for, with or without the file there.
    fn video(dir: &Path, present: bool) -> Track {
        let path = dir.join("01 - Album.opus");
        let mut track = Track::new(1, "vid".into(), "Album".into(), path.clone());
        if present {
            // A test that made real audio there first keeps it.
            if !path.exists() {
                std::fs::write(&path, "audio").unwrap();
            }
            track.status = Status::Have;
        }
        track
    }

    // Answers the one question with `reply` and reports whether it was asked at all.
    fn settle(pass: Pass, reply: Reply, tracks: &mut Vec<Track>) -> (Option<Split>, bool) {
        let (tx, rx): (Sender<Msg>, Receiver<Msg>) = mpsc::channel();
        let asker = Asker { tx, enabled: true };
        let cfg = super::tests::config(false);
        let mut asked = false;
        let split = std::thread::scope(|scope| {
            let worker = scope.spawn(|| settle_split(&cfg, &asker, &pass, three(), tracks));
            while !worker.is_finished() {
                if let Ok(Msg::Ask(_, back)) = rx.recv_timeout(Duration::from_millis(50)) {
                    asked = true;
                    back.send(reply.clone()).unwrap();
                }
            }
            worker.join().unwrap()
        });
        (split, asked)
    }

    #[test]
    fn a_video_with_chapters_is_split_only_when_somebody_says_so() {
        let dir = folder("splitask");
        let mut tracks = vec![video(&dir, false)];

        let (split, asked) = settle(Pass::plain(true), Reply::Choice(0), &mut tracks);
        assert!(asked && split.is_some());
        let ids: Vec<&str> = tracks.iter().map(|t| t.id.as_str()).collect();
        assert_eq!(ids, ["vid#01", "vid#02", "vid#03"]);
        assert_eq!(tracks.iter().map(|t| t.index).collect::<Vec<_>>(), [1, 2, 3]);
        assert_eq!(tracks[1].path.as_deref(), Some(dir.join("02 - Middle.opus").as_path()));
        assert_eq!(tracks[1].duration, 4);
        assert!(tracks.iter().all(|t| t.status == Status::Pending));

        for no in [Reply::Choice(1), Reply::Cancel] {
            let mut tracks = vec![video(&dir, false)];
            let (split, asked) = settle(Pass::plain(true), no, &mut tracks);
            assert!(asked && split.is_none());
            assert_eq!(tracks.len(), 1, "declining still replaced the video");
        }
        std::fs::remove_dir_all(&dir).unwrap();
    }

    // The safe answer for a pass nobody is watching is the video as it was.
    #[test]
    fn nothing_is_asked_without_a_gate_or_for_a_video_already_whole() {
        let dir = folder("splitquiet");
        let mut tracks = vec![video(&dir, false)];
        let (split, asked) = settle(Pass::plain(false), Reply::Choice(0), &mut tracks);
        assert!(!asked && split.is_none() && tracks.len() == 1, "an unattended pass asked or split");

        // The file is here: declined before, or downloaded before this existed.
        let mut tracks = vec![video(&dir, true)];
        let (split, asked) = settle(Pass::plain(true), Reply::Choice(0), &mut tracks);
        assert!(!asked && split.is_none() && tracks.len() == 1, "a whole video was asked about again");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /* The chapter entries in the sidecar are the whole record of the choice,
       so a folder that has them splits again with nobody there to ask, and
       finds each file under the name it has been renamed to. */
    #[test]
    fn a_folder_that_was_split_is_split_again_without_asking() {
        let dir = folder("splitagain");
        let renamed = dir.join("02 - Band - The Middle.opus");
        std::fs::write(&renamed, "audio").unwrap();
        manifest::write(&dir, "https://youtu.be/vid", [("vid#02".to_string(), renamed.clone())].into_iter()).unwrap();

        let mut tracks = vec![video(&dir, false)];
        let (split, asked) = settle(Pass::plain(false), Reply::Cancel, &mut tracks);
        assert!(!asked && split.is_some());
        assert_eq!(tracks.len(), 3);
        assert_eq!(tracks[1].status, Status::Have);
        assert_eq!(tracks[1].path.as_deref(), Some(renamed.as_path()), "the rename was lost");
        assert_eq!(tracks[0].status, Status::Pending, "a chapter with no file is not downloaded");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    // A listing that holds the video must not call its own chapters gone.
    #[test]
    fn a_chapter_is_never_called_departed() {
        let dir = folder("chapgone");
        let file = dir.join("01 - Intro.opus");
        std::fs::write(&file, "audio").unwrap();
        manifest::write(&dir, "https://youtu.be/vid", [("vid#01".to_string(), file)].into_iter()).unwrap();
        let mut listed = vec![video(&dir, false)];
        note_departures(&super::tests::config(true), &mpsc::channel().0, true, &mut listed);
        assert_eq!(listed.len(), 1, "a chapter was added as a departure");
        assert!(listed.iter().all(|t| t.status != Status::Gone));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    fn tone(at: &Path, seconds: u32) -> bool {
        std::process::Command::new("ffmpeg")
            .args(["-v", "error", "-f", "lavfi", "-i"])
            .arg(format!("sine=f=440:d={seconds}"))
            .args(["-ac", "2", "-c:a", "libopus", "-y"])
            .arg(at)
            .status()
            .is_ok_and(|s| s.success())
    }

    /* The video is already here, so no yt-dlp runs and the test reaches the
       cutting alone. The full file is removed once every chapter has its own,
       and is the one thing left if a cut fails. */
    #[test]
    fn a_split_video_is_cut_into_its_chapters_and_the_whole_file_goes() {
        let dir = folder("splitcut");
        let whole = dir.join("01 - Album.opus");
        if !tone(&whole, 12) {
            eprintln!("skipped: ffmpeg could not make a fixture");
            std::fs::remove_dir_all(&dir).unwrap();
            return;
        }
        tag::set_fields(&whole, &tag::Fields { artist: "Band".into(), title: "Album".into(), ..Default::default() }).unwrap();
        // The video's thumbnail, which the cuts have to carry or a chapter the lookup cannot place has no art.
        let jpg = dir.join("thumb.jpg");
        let made = std::process::Command::new("ffmpeg")
            .args(["-v", "error", "-f", "lavfi", "-i", "testsrc=size=64x64:duration=1:rate=1", "-frames:v", "1", "-y"])
            .arg(&jpg)
            .status()
            .is_ok_and(|s| s.success());
        if made {
            tag::set_cover(&whole, &std::fs::read(&jpg).unwrap()).unwrap();
        }
        let cfg = super::tests::config(false);
        let mut tracks = chapter_tracks(&cfg, &video(&dir, true), &three(), &dir);
        let split = Split { chapters: three(), video: Track::new(1, "vid".into(), "Album".into(), whole.clone()) };
        let mut split = split;
        split.video.status = Status::Have;
        let (tx, rx) = mpsc::channel();

        let warning = fetch_split(&cfg, &tx, &AtomicBool::new(false), &mut tracks, &split);
        assert_eq!(warning, "");
        for (track, title) in tracks.iter().zip(["Intro", "Middle", "End"]) {
            assert_eq!(track.status, Status::Downloaded, "{title}");
            let path = track.path.as_deref().unwrap();
            assert!(path.is_file(), "{title} was not written");
            let info = tag::read(path).unwrap();
            assert_eq!((info.title.as_str(), info.artist.as_str()), (title, "Band"));
            assert!((3..=5).contains(&info.duration), "{title} is {}s", info.duration);
            assert_eq!(tag::read_cover(path).is_some(), made, "{title} lost the video's picture");
        }
        assert!(!whole.exists(), "the full-length file was left behind");
        assert!(rx.try_iter().any(|m| matches!(m, Msg::Path { index: 3, .. })));
        // The record of the split is written by the cut itself, before anything else gets the chance to be interrupted.
        assert!(manifest::is_split(&dir, "vid"));
        assert_eq!(manifest::entries(&dir).len(), 3);
        let leftovers = std::fs::read_dir(&dir).unwrap().flatten().filter(|e| e.file_name().to_string_lossy().starts_with(SCRATCH)).count();
        assert_eq!(leftovers, 0, "a scratch file was left in the folder");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_failed_cut_keeps_the_whole_file_for_the_next_sync() {
        let dir = folder("splitfail");
        let whole = dir.join("01 - Album.opus");
        if !tone(&whole, 12) {
            eprintln!("skipped: ffmpeg could not make a fixture");
            std::fs::remove_dir_all(&dir).unwrap();
            return;
        }
        let cfg = super::tests::config(false);
        // The last chapter runs past the end of the file, which is a cut that comes out empty.
        let chapters = vec![chapter("Intro", 0.0, 4.0), chapter("Gone", 100.0, 104.0)];
        let mut tracks = chapter_tracks(&cfg, &video(&dir, true), &chapters, &dir);
        let mut split = Split { chapters, video: Track::new(1, "vid".into(), "Album".into(), whole.clone()) };
        split.video.status = Status::Have;
        let (tx, _rx) = mpsc::channel();

        fetch_split(&cfg, &tx, &AtomicBool::new(false), &mut tracks, &split);
        assert_eq!(tracks[0].status, Status::Downloaded);
        assert_eq!(tracks[1].status, Status::Failed);
        assert!(tracks[1].note.is_empty() || tracks[1].note.starts_with("cut"));
        assert!(whole.is_file(), "the video was removed with a chapter still missing");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    // The cut reads the file it would write over, so a chapter named like the video is given another name.
    #[test]
    fn a_chapter_named_like_the_video_does_not_overwrite_it() {
        let dir = folder("splitclash");
        let whole = dir.join("01 - Album.opus");
        if !tone(&whole, 8) {
            eprintln!("skipped: ffmpeg could not make a fixture");
            std::fs::remove_dir_all(&dir).unwrap();
            return;
        }
        let before = std::fs::read(&whole).unwrap();
        let cfg = super::tests::config(false);
        let chapters = vec![chapter("Album", 0.0, 4.0), chapter("Two", 4.0, 8.0)];
        // Forced, since the names above no longer collide: a track whose recorded file is the video's own.
        let mut forced = chapter_tracks(&cfg, &video(&dir, true), &chapters, &dir);
        forced[0].path = Some(whole.clone());
        forced[0].status = Status::Pending;
        let guard = Split { chapters: chapters.clone(), video: Track::new(1, "vid".into(), "Album".into(), whole.clone()) };
        let mut guard = guard;
        guard.video.status = Status::Have;
        fetch_split(&cfg, &mpsc::channel().0, &AtomicBool::new(false), &mut forced, &guard);
        assert_eq!(forced[0].status, Status::Failed);
        assert_eq!(std::fs::read(&whole).unwrap(), before, "the video was overwritten");
        // The forced run recorded its second cut, and the second half of this test starts from nothing.
        let _ = std::fs::remove_file(dir.join(".earworm"));
        let _ = std::fs::remove_file(dir.join("02 - Two.opus"));

        let mut tracks = chapter_tracks(&cfg, &video(&dir, true), &chapters, &dir);
        assert_eq!(tracks[0].status, Status::Pending, "the video's own file was taken for the chapter");
        assert_ne!(tracks[0].path.as_deref(), Some(whole.as_path()), "the chapter would overwrite the video");
        let mut split = Split { chapters, video: Track::new(1, "vid".into(), "Album".into(), whole.clone()) };
        split.video.status = Status::Have;
        let (tx, _rx) = mpsc::channel();
        fetch_split(&cfg, &tx, &AtomicBool::new(false), &mut tracks, &split);
        assert_eq!(tracks[0].status, Status::Downloaded);
        assert!(tracks[0].path.as_deref().is_some_and(Path::is_file));
        // Every chapter has a file of its own, so the video itself is what goes, and only after the cuts.
        assert!(!whole.exists());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_chapter_id_is_the_whole_shape_and_not_just_a_hash() {
        assert!(manifest::is_chapter("dQw4w9WgXcQ#01"));
        assert!(manifest::is_chapter("dQw4w9WgXcQ#12"));
        assert!(!manifest::is_chapter("dQw4w9WgXcQ"));
        // A local id is a filename, and a filename can say `#2`.
        assert!(!manifest::is_chapter(&manifest::local_id("Song #2.mp3")));
        assert!(!manifest::is_chapter(&manifest::local_id("track#1")));
        assert!(!manifest::is_chapter("#01"));
        assert!(!manifest::is_chapter("abc#"));
        assert!(!manifest::is_chapter("abc#x1"));
    }

    // A killed ffmpeg leaves scratch behind, and a later sync clears it instead of reading it as a track.
    #[test]
    fn a_cut_that_was_killed_leaves_nothing_a_later_sync_reads_as_a_chapter() {
        let dir = folder("splitscratch");
        let whole = dir.join("01 - Album.opus");
        if !tone(&whole, 12) {
            eprintln!("skipped: ffmpeg could not make a fixture");
            std::fs::remove_dir_all(&dir).unwrap();
            return;
        }
        let stale = dir.join(format!("{SCRATCH}2.opus"));
        std::fs::write(&stale, "half a chapter").unwrap();
        let cfg = super::tests::config(false);
        let mut tracks = chapter_tracks(&cfg, &video(&dir, true), &three(), &dir);
        let mut split = Split { chapters: three(), video: Track::new(1, "vid".into(), "Album".into(), whole.clone()) };
        split.video.status = Status::Have;
        fetch_split(&cfg, &mpsc::channel().0, &AtomicBool::new(false), &mut tracks, &split);
        assert!(!stale.exists(), "a leftover scratch file survived");
        assert!(tracks.iter().all(|t| t.status == Status::Downloaded));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /* Stopped half way: the chapters already cut are in the sidecar, the full
       file is still here, and the next sync finds both. Without the entry
       written per chapter, the video would read as whole and the cut files as
       somebody's own music. */
    #[test]
    fn a_split_that_stopped_part_way_resumes_from_the_full_file() {
        let dir = folder("splitresume");
        let whole = dir.join("01 - Album.opus");
        if !tone(&whole, 12) {
            eprintln!("skipped: ffmpeg could not make a fixture");
            std::fs::remove_dir_all(&dir).unwrap();
            return;
        }
        let cfg = super::tests::config(false);
        let broken = vec![chapter("Intro", 0.0, 4.0), chapter("Middle", 100.0, 104.0), chapter("End", 8.0, 12.0)];
        let mut tracks = chapter_tracks(&cfg, &video(&dir, true), &broken, &dir);
        let mut split = Split { chapters: broken, video: Track::new(1, "vid".into(), "Album".into(), whole.clone()) };
        split.video.status = Status::Have;
        fetch_split(&cfg, &mpsc::channel().0, &AtomicBool::new(false), &mut tracks, &split);
        assert_eq!(
            tracks.iter().map(|t| t.status).collect::<Vec<_>>(),
            [Status::Downloaded, Status::Failed, Status::Downloaded]
        );
        assert!(whole.is_file(), "the full file went with a chapter still missing");

        // The next sync: the scan sees the whole file, the sidecar says it was split.
        let mut next = vec![video(&dir, true)];
        let (again, asked) = settle(Pass::plain(false), Reply::Cancel, &mut next);
        assert!(!asked && again.is_some(), "a stopped split read as a whole video");
        assert_eq!(next.iter().map(|t| t.status).collect::<Vec<_>>(), [Status::Have, Status::Pending, Status::Have]);
        std::fs::remove_dir_all(&dir).unwrap();
    }
    // Break it by moving `set_split` below the cut loop: a kill before the first cut leaves the video looking whole.
    #[test]
    fn the_choice_to_split_is_recorded_before_anything_is_cut() {
        let dir = folder("splitrecord");
        let whole = dir.join("01 - Album.opus");
        std::fs::write(&whole, "audio").unwrap();
        let cfg = super::tests::config(false);
        let mut tracks = chapter_tracks(&cfg, &video(&dir, true), &three(), &dir);
        let mut split = Split { chapters: three(), video: video(&dir, true) };
        split.video.status = Status::Have;
        // Cancelled before the first cut, which is where quitting during the first ffmpeg lands.
        fetch_split(&cfg, &mpsc::channel().0, &AtomicBool::new(true), &mut tracks, &split);
        assert!(tracks.iter().all(|t| t.status == Status::Pending), "something was cut");
        assert!(manifest::is_split(&dir, "vid"), "nothing says the video was to be split");

        // The next gated sync does not ask again and does not keep one whole track.
        let mut next = vec![video(&dir, true)];
        let (again, asked) = settle(Pass::plain(true), Reply::Choice(1), &mut next);
        assert!(!asked && again.is_some(), "the answer was lost");
        assert_eq!(next.len(), 3);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    // Break it by taking the span match out of `chapter_tracks`: every position keeps whatever file it had.
    #[test]
    fn a_chapter_inserted_upstream_takes_the_file_its_span_matches_and_cuts_only_the_new_one() {
        let dir = folder("spans");
        let (middle, end) = (dir.join("01 - Middle.opus"), dir.join("02 - End.opus"));
        std::fs::write(&middle, "m").unwrap();
        std::fs::write(&end, "e").unwrap();
        manifest::write(&dir, "u", [("vid#01".to_string(), middle.clone()), ("vid#02".to_string(), end.clone())].into_iter()).unwrap();
        manifest::set_chapter(&dir, "vid#01", 4.0, 8.0).unwrap();
        manifest::set_chapter(&dir, "vid#02", 8.0, 12.0).unwrap();

        // The creator added an Intro in front: positions shift, spans do not.
        let cfg = super::tests::config(false);
        let tracks = chapter_tracks(&cfg, &video(&dir, false), &three(), &dir);
        let got: Vec<(&str, Status, Option<&Path>)> = tracks.iter().map(|t| (t.name.as_str(), t.status, t.path.as_deref())).collect();
        assert_eq!(got[0].1, Status::Pending, "{got:?}");
        assert_eq!((got[1].1, got[1].2), (Status::Have, Some(middle.as_path())), "{got:?}");
        assert_eq!((got[2].1, got[2].2), (Status::Have, Some(end.as_path())), "{got:?}");
        // And the new positions describe themselves from now on.
        let spans = manifest::load(&dir).chapters;
        assert!(spans.iter().any(|(id, s, e)| id == "vid#02" && *s == 4.0 && *e == 8.0), "{spans:?}");
        assert!(spans.iter().any(|(id, s, e)| id == "vid#03" && *s == 8.0 && *e == 12.0), "{spans:?}");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_position_whose_span_moved_is_cut_again_and_the_old_file_is_left_alone() {
        let dir = folder("spanmoved");
        let one = dir.join("01 - Intro.opus");
        std::fs::write(&one, "old cut").unwrap();
        manifest::write(&dir, "u", [("vid#01".to_string(), one.clone())].into_iter()).unwrap();
        manifest::set_chapter(&dir, "vid#01", 0.0, 4.0).unwrap();
        // The timestamp was corrected upstream by ten seconds.
        let cfg = super::tests::config(false);
        let moved = vec![chapter("Intro", 0.0, 14.0)];
        let tracks = chapter_tracks(&cfg, &video(&dir, false), &moved, &dir);
        assert_eq!(tracks[0].status, Status::Pending, "the old cut was kept for a longer chapter");
        assert!(one.is_file(), "the old file was removed");
        // A second either way is rounding and not a change.
        let rounded = vec![chapter("Intro", 0.0, 4.6)];
        assert_eq!(chapter_tracks(&cfg, &video(&dir, false), &rounded, &dir)[0].status, Status::Have);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    // Break it by deleting the `Failed` half: a failed chapter is recorded against the video's own file, which is there.
    #[test]
    fn the_full_file_stays_while_a_chapter_failed_and_goes_when_the_rest_are_done_or_unpicked() {
        let dir = folder("wholedone");
        let (full, cut, other) = (dir.join("full.opus"), dir.join("cut.opus"), dir.join("other.opus"));
        for f in [&full, &cut, &other] {
            std::fs::write(f, "x").unwrap();
        }
        let track = |index: usize, status: Status, path: &Path| {
            let mut t = Track::new(index, format!("vid#0{index}"), "T".into(), path.to_path_buf());
            t.status = status;
            t
        };
        // Failed, yet pointing at a file that exists and is not the video's.
        let failed = [track(1, Status::Downloaded, &cut), track(2, Status::Failed, &other)];
        assert!(!whole_is_done(&failed, &full), "a failed chapter did not keep the video");
        // Failed against the video's own file.
        let against_full = [track(1, Status::Downloaded, &cut), track(2, Status::Failed, &full)];
        assert!(!whole_is_done(&against_full, &full));
        // Unpicked: no file, and no reason to keep a whole video for it.
        let unpicked = [track(1, Status::Downloaded, &cut), track(2, Status::Skipped, &dir.join("02.opus"))];
        assert!(whole_is_done(&unpicked, &full), "an unpicked chapter kept the whole video for good");
        // And a chapter with no file at all, not unpicked, still counts against it.
        let missing = [track(1, Status::Downloaded, &cut), track(2, Status::Pending, &dir.join("02.opus"))];
        assert!(!whole_is_done(&missing, &full));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    // Break it by moving the empty-chapters return back above the `is_split` check.
    #[test]
    fn a_folder_that_was_split_stays_split_when_upstream_loses_its_chapters() {
        let dir = folder("splitgone");
        let one = dir.join("01 - Intro.opus");
        let two = dir.join("02 - Middle.opus");
        std::fs::write(&one, "a").unwrap();
        std::fs::write(&two, "b").unwrap();
        manifest::write(
            &dir,
            "https://youtu.be/vid",
            [("vid#01".to_string(), one), ("vid#02".to_string(), two)].into_iter(),
        )
        .unwrap();
        // The scan sees the video, whose full file went after the split.
        let mut tracks = vec![video(&dir, false)];
        let (tx, _rx) = mpsc::channel();
        let asker = Asker { tx, enabled: false };
        let cfg = super::tests::config(false);
        let split = settle_split(&cfg, &asker, &Pass::plain(false), Vec::new(), &mut tracks);
        assert!(split.is_some());
        let names: Vec<&str> = tracks.iter().map(|t| t.name.as_str()).collect();
        assert_eq!(names, ["Intro", "Middle"]);
        assert!(tracks.iter().all(|t| t.status == Status::Have), "the video would be fetched again");

        // A folder never split, with no chapters, is the plain video.
        let other = folder("splitnever");
        let mut plain = vec![video(&other, false)];
        assert!(settle_split(&cfg, &asker, &Pass::plain(false), Vec::new(), &mut plain).is_none());
        assert_eq!(plain.len(), 1);
        std::fs::remove_dir_all(&dir).unwrap();
        std::fs::remove_dir_all(&other).unwrap();
    }

    // A playlist whose first track is a split video's id is a playlist, whatever its sidecar also holds.
    #[test]
    fn held_chapters_replace_a_single_video_and_never_a_playlist() {
        let dir = folder("splitplaylist");
        let one = dir.join("01 - Intro.opus");
        std::fs::write(&one, "a").unwrap();
        manifest::write(&dir, "https://youtu.be/vid", [("vid#01".to_string(), one)].into_iter()).unwrap();
        let mut tracks = vec![video(&dir, false), Track::new(2, "other".into(), "Other".into(), dir.join("02 - Other.opus"))];
        let (tx, _rx) = mpsc::channel();
        let asker = Asker { tx, enabled: false };
        let cfg = super::tests::config(false);
        assert!(settle_split(&cfg, &asker, &Pass::plain(false), Vec::new(), &mut tracks).is_none());
        assert_eq!(tracks.len(), 2, "the playlist was replaced by one video's chapters");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    // Break it by removing the `Failed` early exit in `tag_tracks`: the reason becomes "no file".
    #[test]
    fn a_failed_cut_keeps_its_reason_through_the_tagging_pass() {
        let dir = folder("cutreason");
        let mut track = Track::new(1, "vid#01".into(), "Intro".into(), dir.join("01 - Intro.opus"));
        track.status = Status::Failed;
        let (tx, rx) = mpsc::channel();
        let asker = Asker { tx: tx.clone(), enabled: false };
        let cfg = super::tests::config(false);
        let mut tracks = vec![track];
        tag_tracks(
            &cfg,
            &tx,
            &AtomicBool::new(false),
            &mut tracks,
            "Album",
            &asker,
            &mut CoverState::default(),
            None,
        );
        drop(tx);
        assert!(
            !rx.try_iter().any(|m| matches!(m, Msg::Update { note: Some(n), .. } if n == "no file")),
            "the cut's reason was replaced"
        );
        std::fs::remove_dir_all(&dir).unwrap();
    }
}

#[cfg(test)]
mod duplicate_tests {
    use super::*;
    use crate::worker::tests::{config, tiny_opus};
    use std::sync::mpsc::channel;

    fn library_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("earworm-dup-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    // A folder holding one file the sidecar names, as a finished sync leaves it.
    fn folder_with(dir: &Path, name: &str, id: &str, file: &str) -> PathBuf {
        let folder = dir.join(name);
        std::fs::create_dir_all(&folder).unwrap();
        std::fs::write(folder.join(file), "audio").unwrap();
        manifest::write(&folder, "u", [(id.to_string(), folder.join(file))].into_iter()).unwrap();
        folder
    }

    fn cfg_for(dir: &Path) -> Config {
        let mut cfg = config(false);
        cfg.dir = dir.to_path_buf();
        cfg
    }

    fn index_for(dir: &Path, here: &Path, wanted: &HashSet<&str>) -> HashMap<String, Held> {
        let cfg = cfg_for(dir);
        library_index(&Others::new(&cfg, here), &cfg.format, wanted)
    }

    fn pending(index: usize, id: &str, name: &str, folder: &Path) -> Track {
        Track::new(index, id.into(), name.into(), folder.join(format!("{index:02} - {name}.opus")))
    }

    // The path short of the scan: index, copy under this playlist's number, then the archive names the id.
    #[test]
    fn a_video_another_folder_holds_is_copied_and_the_archive_skips_it() {
        let dir = library_dir("copy");
        let a = folder_with(&dir, "A", "vid1", "01 - Artist - Song.opus");
        let b = dir.join("B");
        let mut tracks = vec![pending(3, "vid1", "Raw Title", &b)];

        let known = index_for(&dir, &b, &HashSet::from(["vid1"]));
        let (tx, _rx) = channel();
        assert_eq!(copy_known(&cfg_for(&dir), &tx, &mut tracks, &known), 1);

        let path = tracks[0].path.clone().unwrap();
        assert_eq!(path, b.join("03 - Artist - Song.opus"));
        assert_eq!(tracks[0].status, Status::Have);
        assert_eq!(tracks[0].source, "copied");
        assert!(a.join("01 - Artist - Song.opus").is_file(), "the source was moved");
        assert!(!std::fs::read_dir(&b).unwrap().flatten().any(|e| {
            e.file_name().to_string_lossy().starts_with(SCRATCH)
        }), "a scratch file was left behind");

        let archive = dir.join("archive.txt");
        ytdlp::write_archive(&tracks, &archive, &HashSet::new()).unwrap();
        assert!(std::fs::read_to_string(&archive).unwrap().contains("youtube vid1"));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn only_a_file_in_the_right_format_under_a_real_id_is_a_source() {
        let dir = library_dir("index");
        folder_with(&dir, "A", "wrongfmt", "01 - X.mp3");
        folder_with(&dir, "C", "~Song.opus", "Song.opus");
        let own = folder_with(&dir, "B", "mine", "01 - Own.opus");
        let wanted = HashSet::from(["wrongfmt", "~Song.opus", "mine"]);

        let known = index_for(&dir, &own, &wanted);
        assert!(known.is_empty(), "{:?}", known.keys().collect::<Vec<_>>());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn the_same_video_in_three_folders_copies_from_the_first_by_name() {
        let dir = library_dir("order");
        folder_with(&dir, "Z", "vid", "01 - Z.opus");
        folder_with(&dir, "A", "vid", "01 - A.opus");
        let here = dir.join("M");
        let known = index_for(&dir, &here, &HashSet::from(["vid"]));
        assert_eq!(known["vid"].folder, "A");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    // A run quit during the download used to leave the copy with no sidecar line, so the next scan did not know it.
    #[test]
    fn a_copy_is_recorded_in_the_sidecar_as_soon_as_it_is_made() {
        let dir = library_dir("recorded");
        folder_with(&dir, "A", "vid1", "01 - Artist - Song.opus");
        let b = dir.join("B");
        let mut tracks = vec![pending(3, "vid1", "Raw Title", &b)];
        let known = index_for(&dir, &b, &HashSet::from(["vid1"]));
        let (tx, _rx) = channel();
        assert_eq!(copy_known(&cfg_for(&dir), &tx, &mut tracks, &known), 1);
        let held: Vec<(String, PathBuf)> = manifest::entries(&b);
        assert_eq!(held.len(), 1, "the copy is on disk and nothing names it: {held:?}");
        assert_eq!(held[0].0, "vid1");
        assert_eq!(Some(held[0].1.as_path()), tracks[0].path.as_deref());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_name_already_taken_is_left_alone_and_the_track_downloads() {
        let dir = library_dir("taken");
        folder_with(&dir, "A", "vid1", "01 - Song.opus");
        let b = dir.join("B");
        std::fs::create_dir_all(&b).unwrap();
        std::fs::write(b.join("02 - Song.opus"), "somebody else's").unwrap();
        let mut tracks = vec![pending(2, "vid1", "Song", &b)];
        let known = index_for(&dir, &b, &HashSet::from(["vid1"]));
        let (tx, _rx) = channel();
        assert_eq!(copy_known(&cfg_for(&dir), &tx, &mut tracks, &known), 0);
        assert_eq!(tracks[0].status, Status::Pending);
        assert_eq!(std::fs::read_to_string(b.join("02 - Song.opus")).unwrap(), "somebody else's");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    fn shelf_with(name: &str, files: &[&str]) -> Shelf {
        Shelf {
            path: PathBuf::from("/music").join(name),
            name: name.into(),
            files: files.iter().map(|f| ((*f).to_string(), true)).collect(),
            ..Shelf::default()
        }
    }

    #[test]
    fn two_rows_with_one_title_and_length_are_flagged_and_two_lengths_are_not() {
        let b = PathBuf::from("/music/B");
        let mut tracks = vec![
            pending(1, "a", "Song", &b),
            pending(2, "b", "Song", &b),
            pending(3, "c", "Other", &b),
            pending(4, "d", "Other", &b),
        ];
        (tracks[0].duration, tracks[1].duration) = (200, 201);
        (tracks[2].duration, tracks[3].duration) = (200, 260);
        mark_twins(&mut tracks, &HashMap::new(), &[], &b);
        assert_eq!(tracks[1].twin.as_deref(), Some("same as track 1"));
        assert_eq!(tracks[0].twin, None);
        assert_eq!(tracks[3].twin, None, "a live cut a minute longer is not a twin");
    }

    #[test]
    fn a_title_in_another_folder_is_a_maybe_and_a_known_id_says_it_will_be_copied() {
        let b = PathBuf::from("/music/B");
        let mut tracks = vec![
            pending(1, "a", "Song", &b),
            pending(2, "b", "Artist - Song", &b),
            pending(3, "held", "Lullaby", &b),
            pending(4, "c", "Live - Intro", &b),
        ];
        let held = Held { file: PathBuf::from("/x"), folder: "A".into() };
        let known = HashMap::from([("held".to_string(), held)]);
        let shelves = [
            shelf_with(
                "A",
                &["01 - Artist - Song.opus", "02 - Artist - Intro.opus", "03 - Artist - Lullaby.opus"],
            ),
            // The folder being synced is not somewhere a twin can be.
            shelf_with("B", &["01 - Song.opus"]),
        ];
        mark_twins(&mut tracks, &known, &shelves, &b);
        assert_eq!(tracks[0].twin.as_deref(), Some("maybe in A"));
        assert_eq!(tracks[1].twin.as_deref(), Some("maybe in A"));
        assert_eq!(tracks[2].twin.as_deref(), Some("copy from A"), "the sync copies this one");
        // Two tails never meet: every `Intro` would match every other.
        assert_eq!(tracks[3].twin, None);
    }

    #[test]
    fn a_hit_from_the_lookup_writes_its_isrc_into_the_tag() {
        let dir = library_dir("isrc");
        let file = dir.join("01 - A.opus");
        if tiny_opus(&file).is_none() {
            eprintln!("no ffmpeg, skipping");
            return;
        }
        let (tx, _rx) = channel();
        let asker = Asker { tx, enabled: false };
        let mut cfg = config(false);
        cfg.cover = false;
        let m = crate::lookup::Match {
            title: "Autobahn".into(),
            artist: "Kraftwerk".into(),
            album: None,
            mbid: None,
            cover_url: None,
            score: None,
            isrc: Some("GB01A0900374".into()),
            source: "test",
        };
        tag::apply(&file, &m, &cfg, &mut CoverState::default(), &asker, 1, "").unwrap();

        use lofty::file::TaggedFileExt;
        let tagged = lofty::read_from_path(&file).unwrap();
        let isrc = tagged
            .primary_tag()
            .and_then(|t| t.get_string(lofty::tag::ItemKey::Isrc).map(str::to_string));
        assert_eq!(isrc.as_deref(), Some("GB01A0900374"));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn two_videos_with_one_isrc_are_one_recording_but_one_video_twice_is_not() {
        let dir = library_dir("isrc-twin");
        let a = folder_with(&dir, "A", "old", "01 - Song.opus");
        manifest::set_isrcs(&a, &[("old".into(), "GB01A0900374".into())]).unwrap();
        let b = dir.join("B");
        let mut tracks = vec![
            pending(1, "new", "Song", &b),
            pending(2, "old", "Song", &b),
        ];
        tracks[0].isrc = Some("GB01A0900374".into());
        tracks[1].isrc = Some("GB01A0900374".into());
        let cfg = cfg_for(&dir);
        let marked = flag_isrc_twins(&Others::new(&cfg, &b), &mut tracks);

        assert_eq!(tracks[0].twin.as_deref(), Some("same recording in A"));
        // The same video in both folders is what a copy makes.
        assert_eq!(tracks[1].twin.as_deref(), Some("same recording as track 1"));
        assert_eq!(marked.len(), 2);
        std::fs::remove_dir_all(&dir).unwrap();
    }
    #[test]
    fn a_hit_with_no_isrc_clears_the_one_a_different_recording_left() {
        let dir = library_dir("isrc-stale");
        let file = dir.join("01 - A.opus");
        if tiny_opus(&file).is_none() {
            eprintln!("no ffmpeg, skipping");
            return;
        }
        let (tx, _rx) = channel();
        let asker = Asker { tx, enabled: false };
        let mut cfg = config(false);
        cfg.cover = false;
        let hit = |title: &str, isrc: Option<&str>| crate::lookup::Match {
            title: title.into(),
            artist: "Kraftwerk".into(),
            album: None,
            mbid: None,
            cover_url: None,
            score: None,
            isrc: isrc.map(Into::into),
            source: "test",
        };
        let mut cover = CoverState::default();
        tag::apply(&file, &hit("Autobahn", Some("GB01A0900374")), &cfg, &mut cover, &asker, 1, "").unwrap();
        assert_eq!(tag::read(&file).unwrap().isrc.as_deref(), Some("GB01A0900374"));
        tag::apply(&file, &hit("Kometenmelodie", None), &cfg, &mut cover, &asker, 1, "").unwrap();
        assert_eq!(tag::read(&file).unwrap().isrc, None, "the old song's ISRC outlived the song");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    // The flag follows the file, so a track from an earlier run is compared and a reopened folder shows what a sync did.
    #[test]
    fn a_track_already_on_disk_is_compared_by_the_isrc_in_its_tag() {
        let dir = library_dir("isrc-have");
        let a = folder_with(&dir, "A", "old", "01 - Song.opus");
        manifest::set_isrcs(&a, &[("old".into(), "GB01A0900374".into())]).unwrap();
        let b = dir.join("B");
        std::fs::create_dir_all(&b).unwrap();
        let file = b.join("01 - Song.opus");
        if tiny_opus(&file).is_none() {
            eprintln!("no ffmpeg, skipping");
            return;
        }
        let (tx, _rx) = channel();
        let asker = Asker { tx, enabled: false };
        let mut cfg = config(false);
        cfg.cover = false;
        let m = crate::lookup::Match {
            title: "Song".into(),
            artist: "Artist".into(),
            album: None,
            mbid: None,
            cover_url: None,
            score: None,
            isrc: Some("GB01A0900374".into()),
            source: "test",
        };
        tag::apply(&file, &m, &cfg, &mut CoverState::default(), &asker, 1, "").unwrap();
        let mut tracks = vec![Track::new(1, "new".into(), "Song".into(), file.clone())];
        tracks[0].status = Status::Have;
        let cfg = cfg_for(&dir);
        let (tx, _rx) = channel();
        let asker = Asker { tx: tx.clone(), enabled: false };
        let cancel = AtomicBool::new(false);
        tag_tracks(&cfg, &tx, &cancel, &mut tracks, "B", &asker, &mut CoverState::default(), None);
        assert_eq!(tracks[0].isrc.as_deref(), Some("GB01A0900374"), "the Have branch did not read it");
        flag_isrc_twins(&Others::new(&cfg, &b), &mut tracks);
        assert_eq!(tracks[0].twin.as_deref(), Some("same recording in A"));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_copy_clears_the_marker_the_pick_gate_gave_it() {
        let dir = library_dir("copy-twin");
        folder_with(&dir, "A", "vid1", "01 - Song.opus");
        let b = dir.join("B");
        let mut tracks = vec![pending(1, "vid1", "Song", &b)];
        let known = index_for(&dir, &b, &HashSet::from(["vid1"]));
        mark_twins(&mut tracks, &known, &[], &b);
        assert_eq!(tracks[0].twin.as_deref(), Some("copy from A"));
        let (tx, rx) = channel();
        copy_known(&cfg_for(&dir), &tx, &mut tracks, &known);
        assert_eq!(tracks[0].twin, None);
        assert!(rx.try_iter().any(|m| matches!(m, Msg::Twin { twin: None, .. })));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn the_rest_of_the_library_is_read_once_however_many_questions_ask() {
        let dir = library_dir("once");
        folder_with(&dir, "A", "vid1", "01 - Song.opus");
        let cfg = cfg_for(&dir);
        let here = dir.join("B");
        let others = Others::new(&cfg, &here);
        let first = others.all().as_ptr();
        // A sidecar that changes after the first read is not read again.
        std::fs::remove_file(manifest::path(&dir.join("A"))).unwrap();
        assert_eq!(others.all().as_ptr(), first);
        assert_eq!(others.all().len(), 1);
        std::fs::remove_dir_all(&dir).unwrap();
    }
    // An acoustid or Apple hit has no ISRC to give, and that is no reason to lose the one the file has.
    #[test]
    fn a_hit_with_no_isrc_keeps_the_one_the_same_song_already_has() {
        let dir = library_dir("isrc-keep");
        let file = dir.join("01 - A.opus");
        if tiny_opus(&file).is_none() {
            eprintln!("no ffmpeg, skipping");
            return;
        }
        let (tx, _rx) = channel();
        let asker = Asker { tx, enabled: false };
        let mut cfg = config(false);
        cfg.cover = false;
        let hit = |isrc: Option<&str>| crate::lookup::Match {
            title: "Autobahn".into(),
            artist: "Kraftwerk".into(),
            album: None,
            mbid: None,
            cover_url: None,
            score: None,
            isrc: isrc.map(Into::into),
            source: "test",
        };
        let mut cover = CoverState::default();
        tag::apply(&file, &hit(Some("GB01A0900374")), &cfg, &mut cover, &asker, 1, "").unwrap();
        tag::apply(&file, &hit(None), &cfg, &mut cover, &asker, 1, "").unwrap();
        assert_eq!(tag::read(&file).unwrap().isrc.as_deref(), Some("GB01A0900374"));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_copy_that_does_not_happen_does_not_leave_its_marker() {
        let dir = library_dir("copy-skip");
        folder_with(&dir, "A", "vid1", "01 - Song.opus");
        let b = dir.join("B");
        std::fs::create_dir_all(&b).unwrap();
        std::fs::write(b.join("02 - Song.opus"), "somebody else's").unwrap();
        let mut tracks = vec![pending(2, "vid1", "Song", &b)];
        let known = index_for(&dir, &b, &HashSet::from(["vid1"]));
        mark_twins(&mut tracks, &known, &[], &b);
        let (tx, rx) = channel();
        assert_eq!(copy_known(&cfg_for(&dir), &tx, &mut tracks, &known), 0);
        assert_eq!(tracks[0].twin, None, "it still says it was copied");
        assert!(rx.try_iter().any(|m| matches!(m, Msg::Twin { twin: None, .. })));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    // The ISRC is the stronger answer, so it replaces a title guess it contradicts.
    #[test]
    fn an_isrc_replaces_the_title_marker_it_contradicts() {
        let dir = library_dir("isrc-replaces");
        let a = folder_with(&dir, "A", "old", "01 - Song.opus");
        manifest::set_isrcs(&a, &[("old".into(), "AAAA".into())]).unwrap();
        let b = dir.join("B");
        let mut tracks = vec![pending(1, "new", "Song", &b)];
        tracks[0].twin = Some("maybe in A".into());
        tracks[0].isrc = Some("BBBB".into());
        let cfg = cfg_for(&dir);
        let changed = flag_isrc_twins(&Others::new(&cfg, &b), &mut tracks);
        assert_eq!(tracks[0].twin, None);
        assert_eq!(changed, [(1, None)]);
        std::fs::remove_dir_all(&dir).unwrap();
    }
    // A copy of this very video in one folder must not stand in for the real twin in the next.
    #[test]
    fn a_copy_of_this_video_does_not_hide_a_twin_in_another_folder() {
        let dir = library_dir("isrc-holders");
        let a = folder_with(&dir, "A", "new", "01 - Song.opus");
        manifest::set_isrcs(&a, &[("new".into(), "GBAYE9200070".into())]).unwrap();
        let b = folder_with(&dir, "B", "other", "01 - Song.opus");
        manifest::set_isrcs(&b, &[("other".into(), "GBAYE9200070".into())]).unwrap();
        let c = dir.join("C");
        let mut tracks = vec![pending(1, "new", "Song", &c)];
        tracks[0].isrc = Some("GBAYE9200070".into());
        flag_isrc_twins(&Others::new(&cfg_for(&dir), &c), &mut tracks);
        assert_eq!(tracks[0].twin.as_deref(), Some("same recording in B"), "A holds this very video");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    // A row the gate left out is not copied, so its marker described something that would not happen.
    #[test]
    fn an_unpicked_row_loses_its_copy_marker() {
        let dir = library_dir("unpicked-copy");
        folder_with(&dir, "A", "vid1", "01 - Song.opus");
        let b = dir.join("B");
        let mut tracks = vec![pending(1, "vid1", "Song", &b), pending(2, "vid2", "Other", &b)];
        tracks[0].twin = Some("copy from A".into());
        tracks[0].status = Status::Skipped;
        tracks[1].twin = Some("maybe in A".into());
        tracks[1].status = Status::Skipped;
        let known = index_for(&dir, &b, &HashSet::from(["vid1"]));
        let (tx, rx) = channel();
        copy_known(&cfg_for(&dir), &tx, &mut tracks, &known);
        assert_eq!(tracks[0].twin, None);
        assert_eq!(tracks[1].twin.as_deref(), Some("maybe in A"), "a title guess is still a guess");
        assert!(rx.try_iter().any(|m| matches!(m, Msg::Twin { index: 1, twin: None })), "the screen was not told");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    // A guess is dropped only for a recording that is known to differ. Most files have no ISRC at all.
    #[test]
    fn a_title_guess_survives_an_isrc_that_nothing_contradicts() {
        let dir = library_dir("isrc-keeps");
        let a = folder_with(&dir, "A", "old", "01 - Song.opus");
        let b = dir.join("B");
        let cfg = cfg_for(&dir);
        // The other file has no ISRC recorded.
        let mut tracks = vec![pending(1, "new", "Song", &b)];
        tracks[0].twin = Some("maybe in A".into());
        tracks[0].isrc = Some("BBBB".into());
        assert!(flag_isrc_twins(&Others::new(&cfg, &b), &mut tracks).is_empty());
        assert_eq!(tracks[0].twin.as_deref(), Some("maybe in A"));

        // A copy is not a guess either, and an ISRC does not clear it.
        tracks[0].twin = Some("copy from A".into());
        flag_isrc_twins(&Others::new(&cfg, &b), &mut tracks);
        assert_eq!(tracks[0].twin.as_deref(), Some("copy from A"));

        // Within the listing: same title, and the other track has a different recording.
        let mut pair = vec![pending(1, "x", "Song", &b), pending(2, "y", "Song", &b)];
        pair[1].twin = Some("same as track 1".into());
        pair[1].isrc = Some("BBBB".into());
        flag_isrc_twins(&Others::new(&cfg, &b), &mut pair);
        assert_eq!(pair[1].twin.as_deref(), Some("same as track 1"), "track 1 has no ISRC to contradict it");
        pair[0].isrc = Some("AAAA".into());
        flag_isrc_twins(&Others::new(&cfg, &b), &mut pair);
        assert_eq!(pair[1].twin, None, "a different ISRC on the other side contradicts the guess");
        let _ = a;
        std::fs::remove_dir_all(&dir).unwrap();
    }

    // The whole reopen path on real files: tag, sidecar, `open_shelf`, then the screen.
    #[test]
    fn a_reopened_folder_shows_the_twin_marker_on_screen() {
        use ratatui::{Terminal, backend::TestBackend};
        let dir = library_dir("screen");
        let (tx, rx) = channel();
        let asker = Asker { tx: tx.clone(), enabled: false };
        let mut cfg = cfg_for(&dir);
        cfg.cover = false;
        let tagged = |file: &Path| {
            if tiny_opus(file).is_none() {
                return false;
            }
            let m = crate::lookup::Match {
                title: "Creep".into(),
                artist: "Radiohead".into(),
                album: None,
                mbid: None,
                cover_url: None,
                score: None,
                isrc: Some("GBAYE9200070".into()),
                source: "test",
            };
            tag::apply(file, &m, &cfg, &mut CoverState::default(), &asker, 1, "").unwrap();
            true
        };
        let (a, c) = (dir.join("A"), dir.join("C"));
        std::fs::create_dir_all(&a).unwrap();
        std::fs::create_dir_all(&c).unwrap();
        let (fa, fc) = (a.join("01 - Creep.opus"), c.join("01 - Creep.opus"));
        if !tagged(&fa) || !tagged(&fc) {
            eprintln!("no ffmpeg, skipping");
            return;
        }
        manifest::write(&a, "u", [("old".to_string(), fa)].into_iter()).unwrap();
        manifest::set_isrcs(&a, &[("old".into(), "GBAYE9200070".into())]).unwrap();
        manifest::write(&c, "u", [("new".to_string(), fc)].into_iter()).unwrap();

        let shelf = library(&dir).into_iter().find(|s| s.name == "C").unwrap();
        let mut tracks = Vec::new();
        open_shelf(&mut cfg, &tx, &mut tracks, &shelf).unwrap();

        let (ui_tx, _ui_rx) = channel();
        let mut app = crate::app::App::new(ui_tx, "settings".into());
        app.intro_done = true;
        for msg in rx.try_iter() {
            app.apply(msg);
        }
        let mut terminal = Terminal::new(TestBackend::new(120, 10)).unwrap();
        terminal.draw(|f| crate::ui::draw(f, &mut app)).unwrap();
        let buffer = terminal.backend().buffer().clone();
        let screen: String = (0..10)
            .map(|y| (0..120).map(|x| buffer[(x, y)].symbol().to_string()).collect::<String>())
            .collect::<Vec<_>>()
            .join("\n");
        assert!(screen.contains("≈ same recording in A"), "{screen}");
        std::fs::remove_dir_all(&dir).unwrap();
    }
    // Two folders holding one video are what a copy makes, so reopening either shows no marker.
    #[test]
    fn a_reopened_folder_whose_video_is_also_elsewhere_shows_no_marker() {
        use ratatui::{Terminal, backend::TestBackend};
        let dir = library_dir("screen-control");
        let (tx, rx) = channel();
        let asker = Asker { tx: tx.clone(), enabled: false };
        let mut cfg = cfg_for(&dir);
        cfg.cover = false;
        let (a, b) = (dir.join("A"), dir.join("B"));
        for folder in [&a, &b] {
            std::fs::create_dir_all(folder).unwrap();
            let file = folder.join("01 - Creep.opus");
            if tiny_opus(&file).is_none() {
                eprintln!("no ffmpeg, skipping");
                let _ = std::fs::remove_dir_all(&dir);
                return;
            }
            let m = crate::lookup::Match {
                title: "Creep".into(),
                artist: "Radiohead".into(),
                album: None,
                mbid: None,
                cover_url: None,
                score: None,
                isrc: Some("GBAYE9200070".into()),
                source: "test",
            };
            tag::apply(&file, &m, &cfg, &mut CoverState::default(), &asker, 1, "").unwrap();
            manifest::write(folder, "u", [("same".to_string(), file)].into_iter()).unwrap();
            manifest::set_isrcs(folder, &[("same".into(), "GBAYE9200070".into())]).unwrap();
        }
        let shelf = library(&dir).into_iter().find(|s| s.name == "A").unwrap();
        let mut tracks = Vec::new();
        open_shelf(&mut cfg, &tx, &mut tracks, &shelf).unwrap();
        let (ui_tx, _ui_rx) = channel();
        let mut app = crate::app::App::new(ui_tx, "settings".into());
        app.intro_done = true;
        for msg in rx.try_iter() {
            app.apply(msg);
        }
        let mut terminal = Terminal::new(TestBackend::new(120, 10)).unwrap();
        terminal.draw(|f| crate::ui::draw(f, &mut app)).unwrap();
        let buffer = terminal.backend().buffer().clone();
        let screen: String = (0..10)
            .map(|y| (0..120).map(|x| buffer[(x, y)].symbol().to_string()).collect::<String>())
            .collect::<Vec<_>>()
            .join("\n");
        assert!(screen.contains("Creep") && !screen.contains('≈'), "{screen}");
        std::fs::remove_dir_all(&dir).unwrap();
    }
}

#[cfg(test)]
mod playlist_tests {
    use super::*;
    use crate::playlists::Entry;

    fn root(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("earworm-resolve-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn folder(dir: &Path, name: &str, files: &[(&str, &str)]) {
        let folder = dir.join(name);
        std::fs::create_dir_all(&folder).unwrap();
        for (_, file) in files {
            std::fs::write(folder.join(file), "audio").unwrap();
        }
        let entries = files.iter().map(|(id, file)| (id.to_string(), folder.join(file)));
        manifest::write(&folder, "u", entries).unwrap();
    }

    fn entry(id: &str, hint: &str) -> Entry {
        Entry { id: id.into(), hint: hint.into() }
    }

    fn cfg_in(dir: &Path) -> Config {
        let mut cfg = crate::worker::tests::config(false);
        cfg.dir = dir.to_path_buf();
        cfg
    }

    fn resolve(dir: &Path, entries: &[Entry]) -> Vec<Option<PathBuf>> {
        let cfg = cfg_in(dir);
        resolve_entries(&Others::new(&cfg, Path::new("")), dir, entries)
    }

    // The exit test for the storage decision: an edit renames the file and the entry still finds it.
    #[test]
    fn an_entry_still_resolves_after_its_file_and_its_folder_are_renamed() {
        let dir = root("rename");
        folder(&dir, "A", &[("vid1", "01 - Old.opus")]);
        let entries = [entry("vid1", "A/01 - Old.opus")];
        assert_eq!(resolve(&dir, &entries), [Some(dir.join("A/01 - Old.opus"))]);

        // What `write_track` does: the file moves and the sidecar follows.
        std::fs::rename(dir.join("A/01 - Old.opus"), dir.join("A/01 - New.opus")).unwrap();
        manifest::write(&dir.join("A"), "u", [("vid1".to_string(), dir.join("A/01 - New.opus"))].into_iter()).unwrap();
        assert_eq!(resolve(&dir, &entries), [Some(dir.join("A/01 - New.opus"))]);

        // And what `rename_shelf` does: the whole folder moves.
        std::fs::rename(dir.join("A"), dir.join("Renamed")).unwrap();
        assert_eq!(resolve(&dir, &entries), [Some(dir.join("Renamed/01 - New.opus"))]);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_video_in_two_folders_prefers_the_one_the_entry_came_from() {
        let dir = root("two");
        folder(&dir, "A", &[("vid", "01 - A.opus")]);
        folder(&dir, "B", &[("vid", "01 - B.opus")]);
        assert_eq!(resolve(&dir, &[entry("vid", "B/01 - B.opus")]), [Some(dir.join("B/01 - B.opus"))]);
        // The home folder lost it, so the other copy answers rather than the entry going dead.
        std::fs::remove_file(dir.join("B/01 - B.opus")).unwrap();
        assert_eq!(resolve(&dir, &[entry("vid", "B/01 - B.opus")]), [Some(dir.join("A/01 - A.opus"))]);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    // Two folders can both hold a local file with one name, and it is only that folder's.
    #[test]
    fn a_local_id_only_means_something_in_its_own_folder() {
        let dir = root("local");
        folder(&dir, "A", &[("~Song.opus", "Song.opus")]);
        folder(&dir, "B", &[("~Song.opus", "Song.opus")]);
        assert_eq!(resolve(&dir, &[entry("~Song.opus", "B/Song.opus")]), [Some(dir.join("B/Song.opus"))]);
        std::fs::remove_file(dir.join("B/Song.opus")).unwrap();
        assert_eq!(resolve(&dir, &[entry("~Song.opus", "B/Song.opus")]), [None], "it borrowed A's song");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_chapter_resolves_and_a_gone_entry_is_none_not_a_guess() {
        let dir = root("chapter");
        folder(&dir, "A", &[("vid#02", "02 - Cut.opus")]);
        std::fs::write(dir.join("A/unlisted.opus"), "audio").unwrap();
        let got = resolve(
            &dir,
            &[entry("vid#02", "A/02 - Cut.opus"), entry("gone", "A/missing.opus"), entry("unlisted", "A/unlisted.opus")],
        );
        assert_eq!(got, [Some(dir.join("A/02 - Cut.opus")), None, Some(dir.join("A/unlisted.opus"))]);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn the_playlist_store_is_not_a_folder_to_read_from() {
        let dir = root("store");
        folder(&dir, ".playlists", &[("vid", "x.opus")]);
        assert_eq!(resolve(&dir, &[entry("vid", "Gone/x.opus")]), [None]);
        std::fs::remove_dir_all(&dir).unwrap();
    }
    #[test]
    fn a_hint_that_leaves_the_library_is_never_resolved() {
        let dir = root("escape");
        let outside = std::env::temp_dir().join(format!("earworm-outside-{}.opus", std::process::id()));
        std::fs::write(&outside, "not music").unwrap();
        let up = format!("../{}", outside.file_name().unwrap().to_string_lossy());
        let got = resolve(&dir, &[entry("a", &up), entry("b", &outside.to_string_lossy()), entry("c", "")]);
        assert_eq!(got, [None, None, None]);
        std::fs::remove_file(&outside).unwrap();
        std::fs::remove_dir_all(&dir).unwrap();
    }

    // The case `an_entry_still_resolves_after_its_file_and_its_folder_are_renamed` leaves out: a local id.
    #[test]
    fn a_local_entry_follows_its_folder_through_rehome() {
        let dir = root("rehome");
        folder(&dir, "A", &[("~Song.opus", "Song.opus")]);
        let list = playlists::Playlist { name: "Mix".into(), entries: vec![entry("~Song.opus", "A/Song.opus")] };
        playlists::save(&dir, &list).unwrap();

        std::fs::rename(dir.join("A"), dir.join("Renamed")).unwrap();
        let stale = playlists::load(&dir, "Mix").unwrap().entries;
        assert_eq!(resolve(&dir, &stale), [None], "the old hint should not resolve");

        assert_eq!(playlists::rehome(&dir, "A", "Renamed").unwrap(), 1);
        let fresh = playlists::load(&dir, "Mix").unwrap().entries;
        assert_eq!(resolve(&dir, &fresh), [Some(dir.join("Renamed/Song.opus"))]);
        std::fs::remove_dir_all(&dir).unwrap();
    }
    #[test]
    fn a_custom_playlist_is_a_library_row_but_never_a_folder() {
        let dir = root("rows");
        folder(&dir, "A", &[("a1", "01 - One.opus")]);
        folder(&dir, "B", &[("b1", "01 - Two.opus")]);
        let mix = playlists::Playlist {
            name: "Mix".into(),
            entries: vec![entry("b1", "B/01 - Two.opus"), entry("gone", "A/05 - Lost.opus"), entry("a1", "A/01 - One.opus")],
        };
        playlists::save(&dir, &mix).unwrap();

        let all = library_all(&dir);
        let row = all.iter().find(|s| s.name == "Mix").expect("the playlist is not on the library");
        assert_eq!(row.kind, Some(manifest::Kind::Custom));
        assert_eq!((row.tracks, row.missing), (2, 1));
        assert_eq!(row.path, playlists::path_of(&dir, "Mix"), "a folder command would be handed the wrong thing");
        // In the playlist's order, with the missing one named and flagged.
        assert_eq!(
            row.files,
            [("01 - Two.opus".to_string(), true), ("05 - Lost.opus".to_string(), false), ("01 - One.opus".to_string(), true)]
        );
        assert_eq!(all.iter().map(|s| s.name.as_str()).collect::<Vec<_>>(), ["A", "B", "Mix"]);

        // Resync, `--list`, `--check` and the duplicate checks walk this, and a list is none of those.
        assert_eq!(library(&dir).iter().map(|s| s.name.as_str()).collect::<Vec<_>>(), ["A", "B"]);
        std::fs::remove_dir_all(&dir).unwrap();
    }
    // `remove_dir_all` does not come back, and a custom row's path is a file in the store, not a folder.
    #[test]
    fn no_folder_command_will_act_on_a_custom_playlists_path() {
        let dir = root("lock");
        folder(&dir, "A", &[("a1", "01 - One.opus")]);
        let list = playlists::Playlist { name: "Mix".into(), entries: vec![entry("a1", "A/01 - One.opus")] };
        playlists::save(&dir, &list).unwrap();
        let mut cfg = cfg_in(&dir);
        let (tx, _rx) = std::sync::mpsc::channel();
        let asker = Asker { tx: tx.clone(), enabled: false };
        let mut tracks = Vec::new();

        for target in [playlists::path_of(&dir, "Mix"), playlists::folder(&dir)] {
            for (what, err) in [
                ("remove", remove_shelf(&mut cfg, &tx, &asker, &mut tracks, &target).unwrap_err()),
                ("rename", rename_shelf(&mut cfg, &tx, &asker, &tracks, &target).unwrap_err()),
                ("open", open_row(&mut cfg, &tx, &mut tracks, &target, None).unwrap_err()),
            ] {
                assert!(err.to_string().contains("custom playlist"), "{what}: {err}");
            }
        }
        assert!(playlists::load(&dir, "Mix").is_some(), "a folder command took the list");
        assert!(dir.join("A/01 - One.opus").is_file());
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
