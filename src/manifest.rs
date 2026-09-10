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
    /// In the order the file holds them, files that have since been deleted
    /// included. `read` drops both the order and the deleted ones.
    pub entries: Vec<(String, PathBuf)>,
}

pub fn load(folder: &Path) -> Sidecar {
    let mut found = Sidecar {
        url: None,
        synced: None,
        entries: Vec::new(),
    };
    let Ok(text) = std::fs::read_to_string(path(folder)) else {
        return found;
    };
    for (key, value) in text.lines().filter_map(|line| line.split_once('\t')) {
        match key {
            URL => found.url = Some(value.trim().to_string()).filter(|v| !v.is_empty()),
            SYNCED => found.synced = value.trim().parse().ok(),
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
    body(folder, url, load(folder).synced, entries)
}

/// Rewrites the entries and stamps this moment as the last sync.
pub fn write_synced(
    folder: &Path,
    url: &str,
    entries: impl Iterator<Item = (String, PathBuf)>,
) -> std::io::Result<()> {
    body(folder, url, Some(now()), entries)
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
    url: &str,
    synced: Option<u64>,
    entries: impl Iterator<Item = (String, PathBuf)>,
) -> std::io::Result<()> {
    let mut body = String::new();
    if !url.is_empty() && !url.contains(['\t', '\n']) {
        body.push_str(&format!("{URL}\t{url}\n"));
    }
    if let Some(at) = synced {
        body.push_str(&format!("{SYNCED}\t{at}\n"));
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

    #[test]
    fn an_absent_manifest_is_empty_rather_than_an_error() {
        assert!(read(Path::new("/nonexistent-earworm-folder")).is_empty());
    }
}
