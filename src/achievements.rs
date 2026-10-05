//! Small rewards for using earworm, and the file that remembers which were announced.

use std::collections::BTreeSet;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread::JoinHandle;

use crate::app::{Playback, Shelf, Status, Track};
use crate::manifest::Kind;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Achievement {
    FirstPlaylist,
    TenPlaylists,
    HundredTracks,
    ThousandTracks,
    AnAlbum,
    Adopted,
    CleanRun,
    HandTagged,
    SecondThoughts,
    OnAir,
    Secret,
}

pub const ALL: [Achievement; 11] = [
    Achievement::FirstPlaylist,
    Achievement::TenPlaylists,
    Achievement::HundredTracks,
    Achievement::ThousandTracks,
    Achievement::AnAlbum,
    Achievement::Adopted,
    Achievement::CleanRun,
    Achievement::HandTagged,
    Achievement::SecondThoughts,
    Achievement::OnAir,
    Achievement::Secret,
];

pub const TOTAL: usize = ALL.len();

/// A run needs this many tracks before finishing clean means anything.
const CLEAN_RUN_MIN: usize = 10;

impl Achievement {
    // Ids are a file format: renaming one announces it again and orphans its old line.
    pub fn id(self) -> &'static str {
        match self {
            Achievement::FirstPlaylist => "first-playlist",
            Achievement::TenPlaylists => "ten-playlists",
            Achievement::HundredTracks => "hundred-tracks",
            Achievement::ThousandTracks => "thousand-tracks",
            Achievement::AnAlbum => "an-album",
            Achievement::Adopted => "adopted",
            Achievement::CleanRun => "clean-run",
            Achievement::HandTagged => "hand-tagged",
            Achievement::SecondThoughts => "second-thoughts",
            Achievement::OnAir => "on-air",
            Achievement::Secret => "secret",
        }
    }

    #[cfg(test)]
    pub fn from_id(id: &str) -> Option<Achievement> {
        ALL.into_iter().find(|a| a.id() == id)
    }

    pub fn label(self) -> &'static str {
        match self {
            Achievement::FirstPlaylist => "first crate",
            Achievement::TenPlaylists => "record shop",
            Achievement::HundredTracks => "deep cuts",
            Achievement::ThousandTracks => "crate digger",
            Achievement::AnAlbum => "front to back",
            Achievement::Adopted => "house rules",
            Achievement::CleanRun => "clean sweep",
            Achievement::HandTagged => "fine print",
            Achievement::SecondThoughts => "second thoughts",
            Achievement::OnAir => "on air",
            Achievement::Secret => "safecracker",
        }
    }

    pub fn hint(self) -> &'static str {
        match self {
            Achievement::FirstPlaylist => "synced your first playlist",
            Achievement::TenPlaylists => "ten playlists in the library",
            Achievement::HundredTracks => "100 tracks in the library",
            Achievement::ThousandTracks => "1000 tracks in the library",
            Achievement::AnAlbum => "a whole album, kept as one",
            Achievement::Adopted => "a folder you brought yourself",
            Achievement::CleanRun => "a run that finished clean",
            Achievement::HandTagged => "fixed a tag by hand",
            Achievement::SecondThoughts => "took an edit back",
            Achievement::OnAir => "played a folder in cliamp",
            Achievement::Secret => "found a cheat code",
        }
    }
}

/// Everything an achievement can be decided from, borrowed from the app.
pub struct Facts<'a> {
    pub shelves: &'a [Shelf],
    pub tracks: &'a [Track],
    /// The finished run's outcome and its stage word: only a stage of "finished" is a run.
    pub done: Option<(&'a Result<String, String>, &'a str)>,
    pub playback: &'a Playback,
    pub secrets: usize,
}

pub fn earned(f: &Facts) -> Vec<Achievement> {
    let library: usize = f.shelves.iter().map(|s| s.tracks).sum();
    // Real work: a track this run tagged. A resync of an unchanged folder leaves every track `Have`.
    let did_work = f.tracks.iter().any(|t| matches!(t.status, Status::Ok | Status::Manual));
    let run_is_clean = matches!(f.done, Some((Ok(_), "finished")))
        && f.tracks.len() >= CLEAN_RUN_MIN
        && did_work
        && !f
            .tracks
            .iter()
            .any(|t| t.status.wants_a_look() || t.status == Status::Failed);
    let on_air = f.playback.playing && f.shelves.iter().any(|s| f.playback.on(&s.path));

    let checks = [
        (Achievement::FirstPlaylist, f.shelves.iter().any(|s| s.url.is_some())),
        (Achievement::TenPlaylists, f.shelves.len() >= 10),
        (Achievement::HundredTracks, library >= 100),
        (Achievement::ThousandTracks, library >= 1000),
        (Achievement::AnAlbum, f.shelves.iter().any(|s| s.kind == Some(Kind::Album))),
        (Achievement::Adopted, f.shelves.iter().any(|s| s.url.is_none())),
        (Achievement::CleanRun, run_is_clean),
        (
            Achievement::HandTagged,
            f.tracks.iter().any(|t| t.status == Status::Manual && t.source != "undone"),
        ),
        (Achievement::SecondThoughts, f.tracks.iter().any(|t| t.source == "undone")),
        (Achievement::OnAir, on_air),
        (Achievement::Secret, f.secrets > 0),
    ];
    checks.into_iter().filter(|(_, got)| *got).map(|(a, _)| a).collect()
}

// State, not cache: a cache is safe to delete, and deleting this re-announces everything.
pub fn state_path() -> Option<PathBuf> {
    crate::config::xdg_home("XDG_STATE_HOME", ".local/state").map(|base| base.join("earworm").join("achievements"))
}

/// Ids kept verbatim, known or not, so a downgrade cannot lose a newer build's.
pub fn parse(text: &str) -> BTreeSet<String> {
    text.lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .map(String::from)
        .collect()
}

pub fn render(ids: &BTreeSet<String>) -> String {
    ids.iter().map(|id| format!("{id}\n")).collect()
}

// Only a missing file is empty: anything else unreadable must not read as "earned nothing".
pub fn read_at(path: &Path) -> io::Result<BTreeSet<String>> {
    match std::fs::read_to_string(path) {
        Ok(text) => Ok(parse(&text)),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(BTreeSet::new()),
        Err(e) => Err(e),
    }
}

/// Unions with what is on disk rather than rebuilding it, and writes nothing if the read failed.
pub fn merge_at(path: &Path, ids: &[&str]) -> anyhow::Result<()> {
    let mut all = read_at(path)?;
    all.extend(ids.iter().map(|id| id.to_string()));
    crate::config::write_atomically(path, &render(&all))
}

/// A thread that merges each batch it is sent, so the UI thread never touches the file.
pub fn writer(path: PathBuf) -> (Sender<Vec<&'static str>>, JoinHandle<()>) {
    let (tx, rx): (Sender<Vec<&'static str>>, Receiver<_>) = mpsc::channel();
    let handle = std::thread::spawn(move || {
        for batch in rx {
            let _ = merge_at(&path, &batch);
        }
    });
    (tx, handle)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::Player;

    fn dir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("earworm-ach-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn shelf(tracks: usize, url: Option<&str>, kind: Option<Kind>) -> Shelf {
        Shelf {
            tracks,
            url: url.map(String::from),
            kind,
            ..Shelf::default()
        }
    }

    fn track(index: usize, status: Status, source: &str) -> Track {
        let mut t = Track::new(index, format!("id{index}"), format!("t{index}"), PathBuf::new());
        t.status = status;
        t.source = source.into();
        t
    }

    fn got(shelves: &[Shelf], tracks: &[Track], done: Option<(&Result<String, String>, &str)>) -> Vec<Achievement> {
        earned(&Facts {
            shelves,
            tracks,
            done,
            playback: &Playback::default(),
            secrets: 0,
        })
    }

    // The pinned list: a rename here re-announces the achievement to everyone.
    #[test]
    fn ids_are_pinned_and_round_trip() {
        let ids: Vec<&str> = ALL.iter().map(|a| a.id()).collect();
        assert_eq!(
            ids,
            [
                "first-playlist", "ten-playlists", "hundred-tracks", "thousand-tracks", "an-album",
                "adopted", "clean-run", "hand-tagged", "second-thoughts", "on-air", "secret",
            ]
        );
        for a in ALL {
            assert_eq!(Achievement::from_id(a.id()), Some(a));
        }
        assert_eq!(ids.iter().collect::<BTreeSet<_>>().len(), TOTAL, "a duplicate id");
        assert_eq!(Achievement::from_id("nope"), None);
    }

    #[test]
    fn labels_and_hints_fit_the_popup() {
        for a in ALL {
            assert!(a.label().chars().count() <= 30, "{}", a.label());
            assert!(a.hint().chars().count() <= 32, "{}", a.hint());
        }
    }

    #[test]
    fn library_achievements_turn_on_at_their_boundaries() {
        let nine: Vec<Shelf> = (0..9).map(|_| shelf(11, Some("u"), None)).collect();
        let ten: Vec<Shelf> = (0..10).map(|_| shelf(10, Some("u"), None)).collect();
        assert!(!got(&nine, &[], None).contains(&Achievement::TenPlaylists));
        assert!(got(&ten, &[], None).contains(&Achievement::TenPlaylists));
        // 99 tracks is not 100, and 100 is.
        assert!(!got(&nine, &[], None).contains(&Achievement::HundredTracks));
        assert!(got(&ten, &[], None).contains(&Achievement::HundredTracks));
        let big = [shelf(999, Some("u"), None), shelf(1, Some("u"), None)];
        assert!(got(&big, &[], None).contains(&Achievement::ThousandTracks));
        assert!(!got(&[shelf(999, Some("u"), None)], &[], None).contains(&Achievement::ThousandTracks));
    }

    #[test]
    fn a_folders_url_and_kind_decide_three_of_them() {
        let both = [shelf(1, Some("u"), Some(Kind::Album)), shelf(1, None, None)];
        let found = got(&both, &[], None);
        for a in [Achievement::FirstPlaylist, Achievement::AnAlbum, Achievement::Adopted] {
            assert!(found.contains(&a), "{a:?}");
        }
        let plain = got(&[shelf(1, Some("u"), Some(Kind::Playlist))], &[], None);
        assert!(!plain.contains(&Achievement::AnAlbum) && !plain.contains(&Achievement::Adopted));
    }

    #[test]
    fn only_a_finished_run_with_nothing_to_review_is_clean() {
        let ok: Result<String, String> = Ok("done".into());
        let tidy: Vec<Track> = (1..=10).map(|n| track(n, Status::Ok, "")).collect();
        assert!(got(&[], &tidy, Some((&ok, "finished"))).contains(&Achievement::CleanRun));
        // Opening a folder settles the same way and is not a run.
        assert!(!got(&[], &tidy, Some((&ok, "opened"))).contains(&Achievement::CleanRun));
        assert!(!got(&[], &tidy[..9], Some((&ok, "finished"))).contains(&Achievement::CleanRun));
        // Nothing new was fetched or tagged, so there was no work to do cleanly.
        let idle: Vec<Track> = (1..=10).map(|n| track(n, Status::Have, "on disk")).collect();
        assert!(!got(&[], &idle, Some((&ok, "finished"))).contains(&Achievement::CleanRun));
        let mut one_new = idle.clone();
        one_new[4].status = Status::Ok;
        assert!(got(&[], &one_new, Some((&ok, "finished"))).contains(&Achievement::CleanRun));
        let failed: Result<String, String> = Err("x".into());
        assert!(!got(&[], &tidy, Some((&failed, "finished"))).contains(&Achievement::CleanRun));
        for status in [Status::Kept, Status::Weak, Status::NoMatch, Status::Failed] {
            let mut rough = tidy.clone();
            rough[3].status = status;
            assert!(!got(&[], &rough, Some((&ok, "finished"))).contains(&Achievement::CleanRun), "{status:?}");
        }
    }

    #[test]
    fn an_undo_is_second_thoughts_and_not_a_hand_tag() {
        let typed = [track(1, Status::Manual, "typed")];
        assert!(got(&[], &typed, None).contains(&Achievement::HandTagged));
        let undone = [track(1, Status::Manual, "undone")];
        let found = got(&[], &undone, None);
        assert!(found.contains(&Achievement::SecondThoughts));
        assert!(!found.contains(&Achievement::HandTagged));
    }

    #[test]
    fn on_air_needs_cliamp_playing_a_library_folder() {
        let shelves = [Shelf {
            path: PathBuf::from("/music/Focus"),
            ..Shelf::default()
        }];
        let on = |playing, folder: Option<&str>| {
            let playback = Playback {
                player: Player::Running,
                folder: folder.map(PathBuf::from),
                playing,
                ..Playback::default()
            };
            earned(&Facts { shelves: &shelves, tracks: &[], done: None, playback: &playback, secrets: 0 })
                .contains(&Achievement::OnAir)
        };
        assert!(on(true, Some("/music/Focus")));
        assert!(!on(false, Some("/music/Focus")), "paused counted");
        assert!(!on(true, Some("/music/Other")), "another folder counted");
        assert!(!on(true, None), "a radio stream counted");
    }

    #[test]
    fn a_found_cheat_is_safecracker() {
        let f = Facts { shelves: &[], tracks: &[], done: None, playback: &Playback::default(), secrets: 1 };
        assert_eq!(earned(&f), vec![Achievement::Secret]);
    }

    #[test]
    fn the_file_parses_loosely_and_renders_sorted() {
        let ids = parse("# note\n\n  on-air \nan-album\nfuture-one\n");
        assert_eq!(ids.len(), 3);
        assert_eq!(render(&ids), "an-album\nfuture-one\non-air\n");
    }

    // Break it by rebuilding from ALL: a newer build's ids would vanish.
    #[test]
    fn merging_keeps_ids_it_does_not_know() {
        let dir = dir("merge");
        let file = dir.join("nested").join("achievements");
        merge_at(&file, &["on-air"]).unwrap();
        std::fs::write(&file, "future-one\non-air\n").unwrap();
        merge_at(&file, &["an-album"]).unwrap();
        assert_eq!(std::fs::read_to_string(&file).unwrap(), "an-album\nfuture-one\non-air\n");
        let _ = std::fs::remove_dir_all(&dir);
    }

    // Break it with `unwrap_or_default`: a read glitch would replace the whole history.
    #[test]
    fn an_unreadable_file_is_never_overwritten() {
        let dir = dir("unreadable");
        let file = dir.join("achievements");
        std::fs::write(&file, [0xff, 0xfe, b'\n']).unwrap();
        assert!(merge_at(&file, &["on-air"]).is_err());
        assert_eq!(std::fs::read(&file).unwrap(), [0xff, 0xfe, b'\n'], "it wrote over a file it could not read");
        assert!(read_at(&dir.join("absent")).unwrap().is_empty(), "a missing file is empty");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_writer_merges_each_batch_and_finishes_when_dropped() {
        let dir = dir("writer");
        let file = dir.join("achievements");
        let (tx, handle) = writer(file.clone());
        tx.send(vec!["on-air"]).unwrap();
        tx.send(vec!["an-album", "on-air"]).unwrap();
        drop(tx);
        handle.join().unwrap();
        assert_eq!(std::fs::read_to_string(&file).unwrap(), "an-album\non-air\n");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
