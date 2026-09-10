use std::collections::HashMap;
use std::path::{Path, PathBuf};

/* Renaming a track breaks the one thing scan relies on to spot a file it has
   already downloaded: the filename yt-dlp would have chosen. This records the
   video id beside the name it ended up with, so the next run recognises the
   track however it is now called, and a playlist that gets reordered or
   retitled upstream still resolves to the right file. */
const NAME: &str = ".earworm";
/// Header key. No video id starts with a `#`, so an older manifest without
/// one still parses and a reader that predates it skips the line.
const URL: &str = "#url";

pub fn path(folder: &Path) -> PathBuf {
    folder.join(NAME)
}

/// Video id to the file it lives in, skipping entries whose file is gone so a
/// hand-deleted track downloads again rather than looking present.
pub fn read(folder: &Path) -> HashMap<String, PathBuf> {
    let mut found = HashMap::new();
    let Ok(text) = std::fs::read_to_string(path(folder)) else {
        return found;
    };
    for line in text.lines() {
        let Some((id, name)) = line.split_once('\t') else {
            continue;
        };
        if id.starts_with('#') {
            continue;
        }
        let file = folder.join(name);
        if file.is_file() {
            found.insert(id.to_string(), file);
        }
    }
    found
}

/// Where the folder came from, so it can be synced again without the URL
/// being retyped. `None` for a folder written before the header existed.
pub fn read_url(folder: &Path) -> Option<String> {
    let text = std::fs::read_to_string(path(folder)).ok()?;
    text.lines()
        .filter_map(|line| line.split_once('\t'))
        .find(|(key, _)| *key == URL)
        .map(|(_, url)| url.trim().to_string())
        .filter(|url| !url.is_empty())
}

pub fn write(
    folder: &Path,
    url: &str,
    entries: impl Iterator<Item = (String, PathBuf)>,
) -> std::io::Result<()> {
    let mut body = String::new();
    if !url.is_empty() && !url.contains(['\t', '\n']) {
        body.push_str(&format!("{URL}\t{url}\n"));
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

        assert_eq!(read_url(&dir).as_deref(), Some(url));
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

        assert_eq!(read_url(&dir), None);
        assert!(read(&dir).contains_key("vid1"));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn an_absent_manifest_is_empty_rather_than_an_error() {
        assert!(read(Path::new("/nonexistent-earworm-folder")).is_empty());
    }
}
