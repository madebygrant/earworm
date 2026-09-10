use std::collections::HashMap;
use std::path::{Path, PathBuf};

/* Renaming a track breaks the one thing scan relies on to spot a file it has
   already downloaded: the filename yt-dlp would have chosen. This records the
   video id beside the name it ended up with, so the next run recognises the
   track however it is now called, and a playlist that gets reordered or
   retitled upstream still resolves to the right file. */
const NAME: &str = ".earworm";

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
        let file = folder.join(name);
        if file.is_file() {
            found.insert(id.to_string(), file);
        }
    }
    found
}

pub fn write(folder: &Path, entries: impl Iterator<Item = (String, PathBuf)>) -> std::io::Result<()> {
    let mut body = String::new();
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
        write(&dir, [("vid1".to_string(), dir.join("missing.opus"))].into_iter()).unwrap();
        assert!(read(&dir).is_empty());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn an_absent_manifest_is_empty_rather_than_an_error() {
        assert!(read(Path::new("/nonexistent-earworm-folder")).is_empty());
    }
}
