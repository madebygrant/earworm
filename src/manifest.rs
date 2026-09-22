use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

/* Renaming a track breaks the one thing scan relies on to spot a file it has
   already downloaded: the filename yt-dlp would have chosen. This records the
   video id beside the name it ended up with, so the next run recognises the
   track however it is now called, and a playlist that gets reordered or
   retitled upstream still resolves to the right file. */
const NAME: &str = ".earworm";
/// Header key. No video id starts with a `#`, so an older manifest without
/// one still parses and a reader that predates it skips the line.
const URL: &str = "#url";
/// Unix seconds of the last sync, as a second `#` header. Stored raw so the
/// reader never has to agree with the writer about a date format or a zone.
const SYNCED: &str = "#synced";
/// The folder this playlist lives in, once somebody has chosen it. Without
/// this the folder is named from the playlist's title on every download, so a
/// rename would be undone by the next sync: yt-dlp would write the upstream
/// name again and the whole playlist would arrive beside the renamed copy.
const FOLDER: &str = "#name";
/// The playlist file earworm wrote in this folder, by name.
/* A folder earworm downloaded owns its `.m3u8` outright: the sync is what
   decides the playlist, so it rewrites the file every pass. A folder somebody
   copied in may have brought one, and it may be named exactly what earworm
   would name its own, after the folder. Without a record of which, the first
   edit in such a folder either refuses for good and lets the playlist go
   stale, or replaces a file the user wrote and nothing says it happened.
   This is the record: earworm rewrites the playlist it named here and leaves
   every other one alone. */
const PLAYLIST: &str = "#m3u8";
/// What detection decided this folder is, so a folder opened offline gets the
/// same answer as one being synced. Written only by detection: a name that
/// carries a marker records nothing, or the two would drift and the stale one
/// would go on deciding after the marker was removed.
const KIND: &str = "#kind";
/// Set when somebody has told earworm to stop listing this folder. The files
/// stay exactly where they are; only the row goes.
const HIDDEN: &str = "#hidden";

/// Marks an id earworm invented rather than one YouTube gave it.
/* A folder earworm did not download has no video ids, and the first edit in
   one has to write a sidecar all the same. An invented id that looked like a
   video id would be matched against a listing by `scan`, missed by
   `note_departures`, and handed to yt-dlp in the archive: the whole playlist
   would download again beside the files already there, and every local file
   would be offered for deletion as departed.

   On the id rather than in a header, because a folder can be half reconciled:
   a sync matches eight files, cannot place two, and those two keep their local
   ids beside eight real ones. A header is one bit for the whole folder, and
   would have to be cleared at exactly the right moment or left on for good.

   `~` and not `#`, which starts a header and would make the entry vanish. No
   video id starts with either, and a manifest written before this existed
   holds no `~` at all, so it reads exactly as it always did. */
const LOCAL: char = '~';

/// The id to give a file earworm did not download, from its current filename.
/* The filename at the moment of adoption. Unique within a folder, so it is a
   usable key; assigned once and never changed, so a later rename moves the
   filename column and leaves this alone, exactly as a video id behaves; and
   readable in the file, which a hash would not be. */
pub fn local_id(filename: &str) -> String {
    format!("{LOCAL}{filename}")
}

/// Whether this id is one earworm invented, which means no listing will ever
/// name it: not to be matched against one, departed for missing one, or
/// handed to yt-dlp.
pub fn is_local(id: &str) -> bool {
    id.starts_with(LOCAL)
}

/// Whether a folder holds one record or a collection of them.
/* An album's tracks share a sleeve, so looking each one up separately returns
   whatever release that track matched and one record ends up holding three
   covers. See docs/album-or-playlist.md. */
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Kind {
    Album,
    Playlist,
}

impl Kind {
    pub fn label(self) -> &'static str {
        match self {
            Kind::Album => "album",
            Kind::Playlist => "playlist",
        }
    }

    // Anything else is a header from a later version or a hand edit, and
    // "no answer" is the safe reading of both.
    fn parse(value: &str) -> Option<Kind> {
        match value.trim().to_lowercase().as_str() {
            "album" => Some(Kind::Album),
            "playlist" => Some(Kind::Playlist),
            _ => None,
        }
    }
}

/// The kind a folder's name declares, or `None` for a name that says nothing.
/* Read and never written, so removing the marker hands the folder back to
   detection. The bracket's whole contents must be the word: brackets in music
   folder names are usually a year, a format or an edition, and a substring
   match would read `[Album Version]` and half a library besides as albums. */
pub fn kind_from_name(name: &str) -> Option<Kind> {
    let mut found: Option<Kind> = None;
    let mut rest = name;
    while let Some(open) = rest.find('[') {
        let after = &rest[open + 1..];
        // An unclosed bracket ends the scan: there is no group to read.
        let Some(close) = after.find(']') else {
            return found;
        };
        let inside = after[..close].trim().to_lowercase();
        let kind = match inside.as_str() {
            "album" => Some(Kind::Album),
            "playlist" => Some(Kind::Playlist),
            _ => None,
        };
        if let Some(kind) = kind {
            if found.is_some_and(|had| had != kind) {
                return None;
            }
            found = Some(kind);
        }
        rest = &after[close + 1..];
    }
    found
}

/// A folder's name with the marker for `kind` taken out, for a screen that is
/// already saying that kind some other way.
/* Only the kind passed in, so a row never drops a word it is not showing
   elsewhere: the library marks an album with a pill and leaves a `[playlist]`
   to say itself in the name. The same bracket rule as `kind_from_name`,
   because a display that hid a group that one did not read would be a second
   answer to what counts as a marker. A name that is nothing but the marker
   comes back whole: an empty name column says less than a repeated word. */
pub fn without_marker(name: &str, kind: Kind) -> String {
    let mut out = String::with_capacity(name.len());
    let mut rest = name;
    let mut took = false;
    while let Some(open) = rest.find('[') {
        let after = &rest[open + 1..];
        let Some(close) = after.find(']') else { break };
        if after[..close].trim().to_lowercase() == kind.label() {
            out.push_str(&rest[..open]);
            took = true;
        } else {
            out.push_str(&rest[..open + close + 2]);
        }
        rest = &after[close + 1..];
    }
    /* A name with no marker in it comes back untouched rather than tidied:
       the collapse below is for the gap a removed group leaves, and running
       it over every name would quietly rewrite whatever spacing somebody
       chose for a folder this never had anything to do with. */
    if !took {
        return name.into();
    }
    out.push_str(rest);
    let trimmed = out.split_whitespace().collect::<Vec<_>>().join(" ");
    if trimmed.is_empty() { name.into() } else { trimmed }
}

/// What a folder is, and what said so, or `None` when nothing has.
/* One derivation, so a sync and a folder opened offline cannot disagree. The
   name wins because it is the one answer a person typed, and it is re-read
   every time so deleting the marker takes effect at once. The header is
   detection's own record, which is all there is offline. */
pub fn kind_of(folder: &Path, sidecar: &Sidecar) -> Option<(Kind, &'static str)> {
    let named = folder
        .file_name()
        .and_then(|n| kind_from_name(&n.to_string_lossy()));
    match (named, sidecar.kind) {
        (Some(kind), _) => Some((kind, "the folder name")),
        (None, Some(kind)) => Some((kind, "an earlier sync")),
        (None, None) => None,
    }
}

pub fn path(folder: &Path) -> PathBuf {
    folder.join(NAME)
}

/// Video id to the file it lives in, skipping entries whose file is gone so a
/// hand-deleted track downloads again rather than looking present.
pub fn read(folder: &Path) -> HashMap<String, PathBuf> {
    entries(folder)
        .into_iter()
        .filter(|(_, file)| file.is_file())
        .collect()
}

/// Everything one manifest holds, from a single read. The library wants all
/// three of these for every folder it lists.
pub struct Sidecar {
    /// Where the folder came from, so it can be synced again without the URL
    /// being retyped. `None` for a folder written before the header existed.
    pub url: Option<String>,
    /// Unix seconds of the last sync, or `None` for a folder last written
    /// before that header existed.
    pub synced: Option<u64>,
    /// The folder name to keep using, for a playlist somebody has renamed.
    /// `None` means the folder is still whatever the playlist is called
    /// upstream, which is what yt-dlp will name it again.
    pub name: Option<String>,
    /// What detection last decided, or `None` for a folder nothing has
    /// decided about. Beaten by a marker in the folder's name, which is why
    /// `kind_of` is what callers ask rather than reading this.
    pub kind: Option<Kind>,
    /// The playlist file earworm wrote here, if it wrote one. `None` means any
    /// `.m3u8` in the folder came from somewhere else and is not earworm's to
    /// replace.
    pub playlist: Option<String>,
    /// Whether the library is to leave this folder out. Set by `D`, cleared
    /// only by hand, and the audio is untouched either way.
    pub hidden: bool,
    /// In the order the file holds them, files that have since been deleted
    /// included. `read` drops both the order and the deleted ones.
    pub entries: Vec<(String, PathBuf)>,
}

/// Everything above the entries, so the four writers can each change one of
/// them and carry the rest without restating the list at every call.
struct Headers {
    url: String,
    synced: Option<u64>,
    name: Option<String>,
    playlist: Option<String>,
    kind: Option<Kind>,
    hidden: bool,
}

impl Headers {
    /// What is recorded now, which is what every write starts from: dropping a
    /// header somebody else wrote would quietly undo their setting.
    fn of(was: &Sidecar) -> Self {
        Headers {
            url: was.url.clone().unwrap_or_default(),
            synced: was.synced,
            name: was.name.clone(),
            playlist: was.playlist.clone(),
            kind: was.kind,
            hidden: was.hidden,
        }
    }
}

pub fn load(folder: &Path) -> Sidecar {
    let mut found = Sidecar {
        url: None,
        synced: None,
        name: None,
        kind: None,
        playlist: None,
        hidden: false,
        entries: Vec::new(),
    };
    let Ok(text) = std::fs::read_to_string(path(folder)) else {
        return found;
    };
    for (key, value) in text.lines().filter_map(|line| line.split_once('\t')) {
        match key {
            URL => found.url = Some(value.trim().to_string()).filter(|v| !v.is_empty()),
            SYNCED => found.synced = value.trim().parse().ok(),
            FOLDER => found.name = Some(value.trim().to_string()).filter(|v| !v.is_empty()),
            PLAYLIST => found.playlist = Some(value.trim().to_string()).filter(|v| !v.is_empty()),
            KIND => found.kind = Kind::parse(value),
            // Present is hidden, whatever it says, like `NO_COLOR`.
            HIDDEN => found.hidden = true,
            // A header this reader predates, which is why they carry a `#`.
            _ if key.starts_with('#') => {}
            _ => found.entries.push((key.to_string(), folder.join(value))),
        }
    }
    found
}

pub fn entries(folder: &Path) -> Vec<(String, PathBuf)> {
    load(folder).entries
}

/// Rewrites the entries and keeps whatever sync time is already recorded: a
/// tag edit changes what is in the folder, not when it last met the playlist.
pub fn write(
    folder: &Path,
    url: &str,
    entries: impl Iterator<Item = (String, PathBuf)>,
) -> std::io::Result<()> {
    let was = load(folder);
    let headers = Headers {
        url: url.to_string(),
        ..Headers::of(&was)
    };
    body(folder, &headers, entries)
}

/// Records the folder a renamed playlist is to keep, leaving everything else
/// the sidecar holds exactly as it was.
pub fn set_name(folder: &Path, name: &str) -> std::io::Result<()> {
    let was = load(folder);
    let headers = Headers {
        name: Some(name.to_string()),
        ..Headers::of(&was)
    };
    body(folder, &headers, was.entries.into_iter())
}

/// Records the playlist file as earworm's own, so later passes may rewrite it.
/* Only the writer that created the file calls this, which is what keeps the
   header a statement about what earworm did rather than a claim about what is
   in the folder. */
pub fn set_playlist(folder: &Path, name: &str) -> std::io::Result<()> {
    let was = load(folder);
    if was.playlist.as_deref() == Some(name) {
        return Ok(());
    }
    let headers = Headers {
        playlist: Some(name.to_string()),
        ..Headers::of(&was)
    };
    body(folder, &headers, was.entries.into_iter())
}

/// Records what detection decided, leaving everything else as it was.
pub fn set_kind(folder: &Path, kind: Kind) -> std::io::Result<()> {
    let was = load(folder);
    if was.kind == Some(kind) {
        return Ok(());
    }
    let headers = Headers {
        kind: Some(kind),
        ..Headers::of(&was)
    };
    body(folder, &headers, was.entries.into_iter())
}

/// Takes a folder off the library without touching a byte of its audio.
/* The one answer `D` had for a folder earworm did not download was deleting
   the music, so a row somebody wanted rid of could only be got rid of by
   losing the files. Recorded in the sidecar rather than in the config because
   it is a fact about that folder: move the folder and it stays hidden, and
   the way back is to delete the `.earworm`, which is the same way back as
   everything else this file records. */
pub fn hide(folder: &Path) -> std::io::Result<()> {
    let was = load(folder);
    if was.hidden {
        return Ok(());
    }
    let headers = Headers {
        hidden: true,
        ..Headers::of(&was)
    };
    body(folder, &headers, was.entries.into_iter())
}

/// Rewrites the entries and stamps this moment as the last sync.
pub fn write_synced(
    folder: &Path,
    url: &str,
    entries: impl Iterator<Item = (String, PathBuf)>,
) -> std::io::Result<()> {
    let was = load(folder);
    let headers = Headers {
        url: url.to_string(),
        synced: Some(now()),
        ..Headers::of(&was)
    };
    body(folder, &headers, entries)
}

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// How long ago a stored sync time was, in the coarsest unit that still says
/// something: the useful question is whether it was today or months back.
pub fn ago(synced: u64) -> String {
    let seconds = now().saturating_sub(synced);
    match seconds {
        // A clock that has gone backwards, or a sync from this same minute.
        0..=59 => "just now".into(),
        s if s < 3600 => format!("{}m ago", s / 60),
        s if s < 86_400 => format!("{}h ago", s / 3600),
        s if s < 86_400 * 365 => format!("{}d ago", s / 86_400),
        s => format!("{}y ago", s / (86_400 * 365)),
    }
}

fn body(
    folder: &Path,
    headers: &Headers,
    entries: impl Iterator<Item = (String, PathBuf)>,
) -> std::io::Result<()> {
    let mut body = String::new();
    let url = &headers.url;
    if !url.is_empty() && !url.contains(['\t', '\n']) {
        body.push_str(&format!("{URL}\t{url}\n"));
    }
    if let Some(at) = headers.synced {
        body.push_str(&format!("{SYNCED}\t{at}\n"));
    }
    if let Some(name) = headers.name.as_ref().filter(|n| !n.contains(['\t', '\n'])) {
        body.push_str(&format!("{FOLDER}\t{name}\n"));
    }
    if let Some(file) = headers.playlist.as_ref().filter(|n| !n.contains(['\t', '\n'])) {
        body.push_str(&format!("{PLAYLIST}\t{file}\n"));
    }
    if let Some(kind) = headers.kind {
        body.push_str(&format!("{KIND}\t{}\n", kind.label()));
    }
    if headers.hidden {
        body.push_str(&format!("{HIDDEN}\tyes\n"));
    }
    for (id, file) in entries {
        let Some(name) = file.file_name().map(|n| n.to_string_lossy().to_string()) else {
            continue;
        };
        // A tab or newline in either field would make the line unparseable.
        if id.contains(['\t', '\n']) || name.contains(['\t', '\n']) {
            continue;
        }
        body.push_str(&format!("{id}\t{name}\n"));
    }
    std::fs::write(path(folder), body)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("earworm-manifest-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    /* The folder is named from the playlist's title on every download, so a
       rename only survives if the sidecar says what the folder is called. */
    #[test]
    fn a_recorded_name_survives_every_other_write_to_the_sidecar() {
        let dir = scratch("named");
        std::fs::write(dir.join("01 - A.opus"), "x").unwrap();
        let entry = || [("id1".to_string(), dir.join("01 - A.opus"))].into_iter();

        write_synced(&dir, "https://example.com", entry()).unwrap();
        assert_eq!(load(&dir).name, None, "a folder nobody renamed claims a name");

        set_name(&dir, "Morning").unwrap();
        let after = load(&dir);
        assert_eq!(after.name.as_deref(), Some("Morning"));
        // Everything else the sidecar held is still there.
        assert_eq!(after.url.as_deref(), Some("https://example.com"));
        assert!(after.synced.is_some(), "the sync time went with the rename");
        assert_eq!(after.entries.len(), 1);

        /* Every later write has to carry it: a tag edit or a sync that
           dropped the header would put the next download back under the
           upstream title, beside the folder that was renamed. */
        write(&dir, "https://example.com", entry()).unwrap();
        assert_eq!(load(&dir).name.as_deref(), Some("Morning"));
        write_synced(&dir, "https://example.com", entry()).unwrap();
        assert_eq!(load(&dir).name.as_deref(), Some("Morning"));

        // A reader that predates the header still loads the entries.
        let text = std::fs::read_to_string(path(&dir)).unwrap();
        assert!(text.contains("#name\tMorning"), "{text}");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn round_trips_ids_to_their_current_filenames() {
        let dir = scratch("round");
        std::fs::write(dir.join("01 - A - B.opus"), "x").unwrap();
        write(
            &dir,
            "https://youtube.com/playlist?list=PL1",
            [("vid1".to_string(), dir.join("01 - A - B.opus"))].into_iter(),
        )
        .unwrap();
        let back = read(&dir);
        assert_eq!(back.get("vid1"), Some(&dir.join("01 - A - B.opus")));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn forgets_entries_whose_file_was_deleted() {
        let dir = scratch("gone");
        write(&dir, "u", [("vid1".to_string(), dir.join("missing.opus"))].into_iter()).unwrap();
        assert!(read(&dir).is_empty());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /* The header has to be invisible to the entry parser, or the URL line
       would come back as a video called "#url". */
    #[test]
    fn the_url_round_trips_without_becoming_an_entry() {
        let dir = scratch("url");
        std::fs::write(dir.join("01 - A.opus"), "x").unwrap();
        let url = "https://youtube.com/playlist?list=PLabc";
        write(&dir, url, [("vid1".to_string(), dir.join("01 - A.opus"))].into_iter()).unwrap();

        assert_eq!(load(&dir).url.as_deref(), Some(url));
        let files = read(&dir);
        assert_eq!(files.len(), 1);
        assert!(files.contains_key("vid1"));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /* A folder written before the header existed still has to load, or every
       playlist downloaded so far would re-download. */
    #[test]
    fn a_manifest_without_a_url_still_reads() {
        let dir = scratch("legacy");
        std::fs::write(dir.join("01 - A.opus"), "x").unwrap();
        std::fs::write(path(&dir), "vid1\t01 - A.opus\n").unwrap();

        assert_eq!(load(&dir).url, None);
        assert!(read(&dir).contains_key("vid1"));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /* The file's own order is the only record of the order tracks were
       written in: `read` returns a map and loses it. */
    #[test]
    fn entries_keep_the_order_they_were_written_in() {
        let dir = scratch("order");
        let files: Vec<PathBuf> = ["03 - C.opus", "01 - A.opus", "02 - B.opus"]
            .iter()
            .map(|n| dir.join(n))
            .collect();
        for file in &files {
            std::fs::write(file, "x").unwrap();
        }
        let ids = ["c", "a", "b"];
        write(
            &dir,
            "u",
            ids.iter()
                .map(|s| s.to_string())
                .zip(files.iter().cloned()),
        )
        .unwrap();

        let back: Vec<String> = entries(&dir).into_iter().map(|(id, _)| id).collect();
        assert_eq!(back, ids);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /* A deleted file has to reach the library as a missing track, and reach
       `scan` as a track to download again. Same manifest, two answers. */
    #[test]
    fn entries_keep_deleted_files_that_read_drops() {
        let dir = scratch("deleted");
        write(&dir, "u", [("vid1".to_string(), dir.join("missing.opus"))].into_iter()).unwrap();
        assert_eq!(entries(&dir).len(), 1);
        assert!(read(&dir).is_empty());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_tag_edit_keeps_the_sync_time_and_a_sync_moves_it() {
        let dir = scratch("synced");
        std::fs::write(dir.join("01 - A.opus"), "x").unwrap();
        let entry = || [("vid1".to_string(), dir.join("01 - A.opus"))].into_iter();

        write(&dir, "u", entry()).unwrap();
        assert_eq!(load(&dir).synced, None, "never synced, so nothing to say");

        write_synced(&dir, "u", entry()).unwrap();
        let at = load(&dir).synced.expect("the sync stamped a time");

        write(&dir, "u", entry()).unwrap();
        assert_eq!(load(&dir).synced, Some(at), "an edit moved the sync time");
        assert!(read(&dir).contains_key("vid1"), "the header ate the entry");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn ago_reads_in_whole_units() {
        assert_eq!(ago(now()), "just now");
        assert_eq!(ago(now() - 90 * 60), "1h ago");
        assert_eq!(ago(now() - 3 * 86_400), "3d ago");
        // A clock that moved backwards must not print a huge number.
        assert_eq!(ago(now() + 500), "just now");
    }

    /* The "not a marker" half is the feature. Brackets in music folder names
       are nearly always a year, a format or an edition, so a rule that was any
       looser would read a whole library as albums and give every folder in it
       one sleeve. */
    #[test]
    fn a_folder_name_declares_its_kind_only_when_it_says_exactly_that() {
        for (name, want) in [
            ("Autobahn [album]", Some(Kind::Album)),
            ("Chill Evenings [playlist]", Some(Kind::Playlist)),
            // Case and padding are typing, not meaning.
            ("Autobahn [ALBUM]", Some(Kind::Album)),
            ("Autobahn [ Album ]", Some(Kind::Album)),
            // Anywhere in the name, because the end is usually the year.
            ("Autobahn [album] [1974]", Some(Kind::Album)),
            ("[album] Autobahn", Some(Kind::Album)),
            // The ordinary contents of a bracket in a music folder's name.
            ("Greatest Hits [Album Version]", None),
            ("Autobahn [Deluxe Edition]", None),
            ("Autobahn [1974]", None),
            ("Autobahn [FLAC]", None),
            // Not a bracket group at all.
            ("Albumania", None),
            ("Autobahn album", None),
            ("Autobahn", None),
            // Nothing to read, and nothing to panic on either.
            ("", None),
            ("Autobahn [album", None),
            ("Autobahn ]album[", None),
            /* Whoever wrote both meant neither, and guessing from the order
               they appear in would be a coin toss wearing a rule. */
            ("Autobahn [album] [playlist]", None),
            ("Autobahn [playlist] [album]", None),
            // The same answer twice is still one answer.
            ("Autobahn [album] [album]", Some(Kind::Album)),
        ] {
            assert_eq!(kind_from_name(name), want, "{name:?}");
        }
    }

    /* What the library row shows once the pill is saying the same thing. It
       takes out only the marker it was asked for and reads a bracket group
       exactly as `kind_from_name` does, or the screen and the answer would
       disagree about what a marker is. */
    #[test]
    fn a_name_gives_up_only_the_marker_it_is_asked_for() {
        for (name, kind, want) in [
            ("Autobahn [album]", Kind::Album, "Autobahn"),
            ("[album] Autobahn", Kind::Album, "Autobahn"),
            ("Autobahn [ALBUM]", Kind::Album, "Autobahn"),
            ("Autobahn [album] [1974]", Kind::Album, "Autobahn [1974]"),
            ("Autobahn [album] [album]", Kind::Album, "Autobahn"),
            // The other kind's marker is somebody else's to show.
            ("Chill [playlist]", Kind::Album, "Chill [playlist]"),
            // Not markers, so not touched.
            ("Greatest Hits [Album Version]", Kind::Album, "Greatest Hits [Album Version]"),
            ("Autobahn [1974]", Kind::Album, "Autobahn [1974]"),
            ("Autobahn [album", Kind::Album, "Autobahn [album"),
            // An empty name column says less than a repeated word.
            ("[album]", Kind::Album, "[album]"),
            ("별 [album]", Kind::Album, "별"),
            // Nothing was taken, so nothing else is tidied either.
            ("Kraftwerk  Autobahn", Kind::Album, "Kraftwerk  Autobahn"),
        ] {
            assert_eq!(without_marker(name, kind), want, "{name:?}");
        }
    }

    /* The marker is matched on a lowercased copy, and `to_lowercase` does not
       preserve byte length. Compared as whole strings rather than indexed, so
       this is a guard on that staying true. */
    #[test]
    fn a_marker_survives_a_name_with_multi_byte_characters() {
        assert_eq!(kind_from_name("별 [album]"), Some(Kind::Album));
        assert_eq!(kind_from_name("Café [playlist] 2024"), Some(Kind::Playlist));
        assert_eq!(kind_from_name("[Ålbum]"), None);
    }

    /* Two records of one fact drift, and this drift is silent: a header left
       behind by a marker somebody has since deleted would go on deciding with
       nothing on screen saying why. So the name is re-read every time and
       always wins, and the header is only ever detection's own note. */
    #[test]
    fn the_folder_name_outranks_the_recorded_kind_and_deleting_it_takes_effect() {
        let root = scratch("kindorder");
        let plain = root.join("Autobahn");
        let marked = root.join("Autobahn [playlist]");
        std::fs::create_dir_all(&plain).unwrap();
        std::fs::create_dir_all(&marked).unwrap();

        // Nobody has said anything yet.
        assert_eq!(kind_of(&plain, &load(&plain)), None);

        // Detection's note is the answer while the name is silent.
        set_kind(&plain, Kind::Album).unwrap();
        assert_eq!(
            kind_of(&plain, &load(&plain)),
            Some((Kind::Album, "an earlier sync"))
        );

        /* The same recorded answer under a name that disagrees. The header is
           what a rename leaves behind, and the name is what somebody typed. */
        set_kind(&marked, Kind::Album).unwrap();
        assert_eq!(
            kind_of(&marked, &load(&marked)),
            Some((Kind::Playlist, "the folder name"))
        );

        // A name with no marker falls back rather than overriding with nothing.
        assert_eq!(
            kind_of(&plain, &load(&plain)),
            Some((Kind::Album, "an earlier sync")),
            "an unmarked name erased the recorded answer"
        );
        std::fs::remove_dir_all(&root).unwrap();
    }

    /* A fifth header beside four that were added the same way, so the same
       two things have to hold: the entry parser must not see it, and a
       sidecar written before it existed must still load. */
    #[test]
    fn the_kind_header_round_trips_without_disturbing_anything_else() {
        let dir = scratch("kindheader");
        std::fs::write(dir.join("01 - A.opus"), "x").unwrap();
        let entry = || [("vid1".to_string(), dir.join("01 - A.opus"))].into_iter();
        write_synced(&dir, "https://example.com", entry()).unwrap();
        set_name(&dir, "Autobahn").unwrap();
        set_kind(&dir, Kind::Album).unwrap();

        let after = load(&dir);
        assert_eq!(after.kind, Some(Kind::Album));
        assert_eq!(after.url.as_deref(), Some("https://example.com"));
        assert_eq!(after.name.as_deref(), Some("Autobahn"));
        assert!(after.synced.is_some());
        assert_eq!(after.entries.len(), 1, "the header ate the entry");
        assert!(read(&dir).contains_key("vid1"));

        // Every other writer carries it, or a tag edit would forget it.
        write(&dir, "https://example.com", entry()).unwrap();
        assert_eq!(load(&dir).kind, Some(Kind::Album));
        write_synced(&dir, "https://example.com", entry()).unwrap();
        assert_eq!(load(&dir).kind, Some(Kind::Album));
        set_name(&dir, "Autobahn").unwrap();
        assert_eq!(load(&dir).kind, Some(Kind::Album));

        // A value from a later version, or a hand edit, is no answer.
        std::fs::write(path(&dir), "#kind\tboxset\nvid1\t01 - A.opus\n").unwrap();
        assert_eq!(load(&dir).kind, None);
        assert!(read(&dir).contains_key("vid1"), "the bad header ate the entry");

        // And a sidecar written before the header existed still loads.
        std::fs::write(path(&dir), "vid1\t01 - A.opus\n").unwrap();
        assert_eq!(load(&dir).kind, None);
        assert!(read(&dir).contains_key("vid1"));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn an_absent_manifest_is_empty_rather_than_an_error() {
        assert!(read(Path::new("/nonexistent-earworm-folder")).is_empty());
    }
}
