use std::path::{Path, PathBuf};

use anyhow::{Result, bail};

use crate::portable::{self, Form};

// Under `--dir` and hidden, so no download-folder rule (resync, `D`, kind detection, adoption) has to learn an exception.
const FOLDER: &str = ".playlists";
const EXT: &str = "playlist";
const NAME: &str = "#name";
// Leaves room for the extension and the temporary name an atomic write adds, inside a 255-byte filename.
const MAX_NAME: usize = 200;

/// One track of a custom playlist: the video id, and where it was last seen.
// The id resolves it, so renaming the file does not break the entry. The hint is relative to `--dir`:
// a local `~` id only means something in its folder, and it is the last resort for an unlisted file.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry {
    pub id: String,
    pub hint: PathBuf,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Playlist {
    pub name: String,
    pub entries: Vec<Entry>,
}

pub fn folder(root: &Path) -> PathBuf {
    root.join(FOLDER)
}

/// The file a playlist is stored in, which is also what its library row calls its path.
pub fn path_of(root: &Path, name: &str) -> PathBuf {
    file_for(root, name)
}

/// Whether a path is inside the store, so a command that means "a folder" can refuse it.
// A custom playlist's library row carries its file as the path, and a folder command would treat it as a directory.
pub fn in_store(root: &Path, path: &Path) -> bool {
    path.starts_with(folder(root))
}

fn file_for(root: &Path, name: &str) -> PathBuf {
    folder(root).join(format!("{}.{EXT}", portable::name(name, Form::Composed)))
}

/// Whether a hint stays inside `--dir`: relative, and never stepping up out of it.
// The file is hand-editable and travels between machines, and an absolute path replaces the root in a join.
pub fn safe_hint(hint: &Path) -> bool {
    !hint.as_os_str().is_empty()
        && hint.components().all(|c| matches!(c, std::path::Component::Normal(_)))
}

// A tab or newline would split a line, and a `#` first character would make it a header.
fn writable(entry: &Entry) -> bool {
    let bad = |s: &str| s.contains(['\t', '\n']);
    !entry.id.is_empty()
        && !entry.id.starts_with('#')
        && !bad(&entry.id)
        && !bad(&entry.hint.to_string_lossy())
        && safe_hint(&entry.hint)
}

fn parse(text: &str) -> Option<Playlist> {
    let mut name = None;
    let mut entries = Vec::new();
    for (key, value) in text.lines().filter_map(|line| line.split_once('\t')) {
        match key {
            NAME => name = Some(value.trim().to_string()).filter(|n| !n.is_empty()),
            // A header from a later version: skipped, like the sidecar's.
            _ if key.starts_with('#') => {}
            _ => {
                let entry = Entry { id: key.to_string(), hint: PathBuf::from(value) };
                // One that leaves `--dir` is dropped here, so nothing downstream ever holds it.
                if safe_hint(&entry.hint) {
                    entries.push(entry);
                }
            }
        }
    }
    Some(Playlist { name: name?, entries })
}

pub fn load(root: &Path, name: &str) -> Option<Playlist> {
    let found = parse(&std::fs::read_to_string(file_for(root, name)).ok()?)?;
    (found.name == name).then_some(found)
}

/// The path cliamp's stored name is built from. Only its last component matters, and it is the list's name.
pub fn key(root: &Path, name: &str) -> PathBuf {
    folder(root).join(name)
}

/// Gives a playlist another name, keeping its tracks and order.
// The file can be the same one under both names (`mix` and `Mix` on a case-insensitive disk), which is
// a rename and not a clash. Any other file is refused if it is held, and the old one goes only after the new one is written.
pub fn rename(root: &Path, old: &str, new: &str) -> Result<()> {
    let Some(mut list) = load(root, old) else {
        bail!("no playlist called \"{old}\"");
    };
    if old == new {
        return Ok(());
    }
    // `save` replaces a playlist of the same name, which is right for an edit and wrong for a rename.
    if load(root, new).is_some() {
        bail!("there is already a playlist called \"{new}\"");
    }
    // On a case-insensitive disk `Mix` and `mix` are one file whose paths differ, so what the new path holds decides.
    let held_by_old = std::fs::read_to_string(file_for(root, new)).ok().and_then(|t| parse(&t)).is_some_and(|p| p.name == old);
    if file_for(root, old) == file_for(root, new) || held_by_old {
        list.name = new.to_string();
        return write(root, &list);
    }
    list.name = new.to_string();
    save(root, &list)?;
    delete(root, old)
}

/// Writes the `.m3u8` a player is handed, with `../Folder/file` entries for the tracks that exist.
// For playback inside the library only: the entries are relative to the store, so they hold wherever
// `--dir` is. Export rewrites them, because a car stereo or a phone will not follow a `../`.
pub fn write_m3u8(root: &Path, list: &Playlist, files: &[Option<PathBuf>]) -> Result<PathBuf> {
    let mut text = String::from("#EXTM3U\n");
    let mut any = false;
    for file in files.iter().flatten() {
        let rel = file.strip_prefix(root).map_or_else(|_| file.display().to_string(), |r| format!("../{}", r.display()));
        text.push_str(&rel);
        text.push('\n');
        any = true;
    }
    if !any {
        bail!("none of \"{}\" is on disk  ·  nothing to play", list.name);
    }
    let path = file_for(root, &list.name).with_extension("m3u8");
    std::fs::create_dir_all(folder(root))?;
    crate::config::write_atomically(&path, &text)?;
    Ok(path)
}

/// Files in the store that do not parse, by name, for the caller to report.
// `list` leaves them out, and a list that vanishes without a word reads as lost.
pub fn broken(root: &Path) -> Vec<String> {
    let Ok(entries) = std::fs::read_dir(folder(root)) else {
        return Vec::new();
    };
    let mut found: Vec<String> = entries
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|e| e == EXT))
        .filter(|p| std::fs::read_to_string(p).ok().is_none_or(|t| parse(&t).is_none()))
        .filter_map(|p| p.file_name().map(|n| n.to_string_lossy().to_string()))
        .collect();
    found.sort();
    found
}

/// Points every entry that lived in folder `old` at folder `new`, after a rename.
/// Returns how many entries moved.
// A local id resolves only in the folder named by its hint, so without this a renamed folder orphans them.
pub fn rehome(root: &Path, old: &str, new: &str) -> Result<usize> {
    let mut moved = 0;
    for mut playlist in list(root) {
        let mut changed = false;
        for entry in &mut playlist.entries {
            let mut parts = entry.hint.components();
            if parts.next().is_some_and(|c| c.as_os_str() == old) {
                entry.hint = Path::new(new).join(parts.as_path());
                changed = true;
                moved += 1;
            }
        }
        if changed {
            save(root, &playlist)?;
        }
    }
    Ok(moved)
}

/// Every custom playlist, in name order. A file that does not parse is left out.
pub fn list(root: &Path) -> Vec<Playlist> {
    let Ok(entries) = std::fs::read_dir(folder(root)) else {
        return Vec::new();
    };
    let mut found: Vec<Playlist> = entries
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|e| e == EXT))
        .filter_map(|p| parse(&std::fs::read_to_string(p).ok()?))
        .collect();
    found.sort_by_key(|p| p.name.to_lowercase());
    found
}

/// Writes the playlist, replacing its own file and refusing another's.
// Two names can map to one filename (`a?b` and `a*b`), and taking the other's would replace somebody's playlist.
pub fn save(root: &Path, playlist: &Playlist) -> Result<()> {
    let name = &playlist.name;
    // Trimmed on read, so a name with edge spaces would save and then never load.
    if name.trim().is_empty() || name.trim() != name || name.contains(['\t', '\n']) {
        bail!("a playlist needs a name with no tabs, line breaks or spaces at either end");
    }
    if portable::name(name, Form::Composed).len() > MAX_NAME {
        bail!("that name is too long  ·  {MAX_NAME} bytes at most");
    }
    let path = file_for(root, &playlist.name);
    if let Some(held) = std::fs::read_to_string(&path).ok().and_then(|t| parse(&t))
        && held.name != playlist.name
    {
        bail!("\"{}\" already uses that file name  ·  give this one another name", held.name);
    }
    write(root, playlist)
}

fn write(root: &Path, playlist: &Playlist) -> Result<()> {
    std::fs::create_dir_all(folder(root))?;
    let mut text = format!("{NAME}\t{}\n", playlist.name);
    for entry in playlist.entries.iter().filter(|e| writable(e)) {
        text.push_str(&format!("{}\t{}\n", entry.id, entry.hint.display()));
    }
    crate::config::write_atomically(&file_for(root, &playlist.name), &text)
}

/// Removes the list and its `.m3u8`, and nothing else. The audio is in the
/// folders, which this never reads.
// A file that does not parse can be removed too, or it would sit there unseen for good. One that parses
// under another name is that playlist's, which shares this file name.
pub fn delete(root: &Path, name: &str) -> Result<()> {
    let path = file_for(root, name);
    let held = std::fs::read_to_string(&path).ok().map(|t| parse(&t));
    match held {
        Some(Some(found)) if found.name == name => {}
        Some(None) => {}
        _ => bail!("no playlist called \"{name}\""),
    }
    std::fs::remove_file(&path)?;
    let _ = std::fs::remove_file(path.with_extension("m3u8"));
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn root(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("earworm-playlists-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn entry(id: &str, hint: &str) -> Entry {
        Entry { id: id.into(), hint: hint.into() }
    }

    #[test]
    fn a_playlist_round_trips_in_order_with_its_header_next_to_an_unknown_one() {
        let dir = root("round");
        let made = Playlist {
            name: "Road trip: 2026?".into(),
            entries: vec![entry("zzz", "B/02 - Z.opus"), entry("~Song name.opus", "A/Song name.opus"), entry("aaa", "A/01 - A.opus")],
        };
        save(&dir, &made).unwrap();
        assert_eq!(load(&dir, "Road trip: 2026?"), Some(made.clone()));
        assert_eq!(list(&dir), [made]);

        // A header a later version added must not become an entry.
        let path = file_for(&dir, "Road trip: 2026?");
        let text = std::fs::read_to_string(&path).unwrap();
        std::fs::write(&path, format!("#colour\tred\n{text}")).unwrap();
        assert_eq!(load(&dir, "Road trip: 2026?").unwrap().entries.len(), 3);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn an_entry_that_cannot_be_written_is_left_out_rather_than_corrupting_the_file() {
        let dir = root("bad");
        let made = Playlist {
            name: "Mix".into(),
            entries: vec![entry("ok", "A/a.opus"), entry("tab\tid", "A/b.opus"), entry("#hash", "A/c.opus"), entry("nl", "A/d\n.opus")],
        };
        save(&dir, &made).unwrap();
        assert_eq!(load(&dir, "Mix").unwrap().entries, [entry("ok", "A/a.opus")]);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    // The list travels between machines and is hand-editable, so a hint is untrusted.
    #[test]
    fn an_entry_that_leaves_the_library_never_loads_or_saves() {
        assert!(safe_hint(Path::new("A/01 - A.opus")));
        for bad in ["../x.opus", "A/../../x.opus", "/etc/hosts", ""] {
            assert!(!safe_hint(Path::new(bad)), "{bad:?}");
        }
        let dir = root("hint");
        std::fs::create_dir_all(folder(&dir)).unwrap();
        std::fs::write(file_for(&dir, "Mix"), "#name\tMix\nok\tA/a.opus\nup\t../x.opus\nabs\t/etc/hosts\n").unwrap();
        assert_eq!(load(&dir, "Mix").unwrap().entries, [entry("ok", "A/a.opus")]);
        save(&dir, &Playlist { name: "Mix".into(), entries: vec![entry("up", "../x.opus"), entry("ok", "A/a.opus")] }).unwrap();
        assert!(!std::fs::read_to_string(file_for(&dir, "Mix")).unwrap().contains("../"));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_rename_keeps_the_tracks_and_refuses_a_name_another_playlist_holds() {
        let dir = root("rename");
        let tracks = vec![entry("a", "A/a.opus"), entry("b", "A/b.opus")];
        save(&dir, &Playlist { name: "Old".into(), entries: tracks.clone() }).unwrap();
        save(&dir, &Playlist { name: "Other".into(), entries: Vec::new() }).unwrap();

        rename(&dir, "Old", "New").unwrap();
        assert!(load(&dir, "Old").is_none());
        assert_eq!(load(&dir, "New").unwrap().entries, tracks);

        // A clash leaves both exactly as they were.
        let err = rename(&dir, "New", "Other").unwrap_err().to_string();
        assert!(err.contains("already"), "{err}");
        assert_eq!(load(&dir, "New").map(|p| p.entries), Some(tracks), "the rename lost the list");
        assert_eq!(load(&dir, "Other").map(|p| p.entries.len()), Some(0), "the rename replaced the other list");

        // Only the case changes, which is the same file on a case-insensitive disk and not a clash.
        rename(&dir, "New", "new").unwrap();
        assert!(load(&dir, "new").is_some());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn the_m3u8_names_tracks_relative_to_the_store_and_leaves_out_the_missing() {
        let dir = root("m3u8");
        let list = Playlist { name: "Mix".into(), entries: vec![entry("a", "A/a.opus"), entry("b", "B/b.opus")] };
        let files = [Some(dir.join("A/01 - Café.opus")), None, Some(dir.join("B/02 - B.opus"))];
        let path = write_m3u8(&dir, &list, &files).unwrap();
        let text = std::fs::read_to_string(path).unwrap();
        assert_eq!(text, "#EXTM3U\n../A/01 - Café.opus\n../B/02 - B.opus\n");
        assert!(write_m3u8(&dir, &list, &[None, None]).unwrap_err().to_string().contains("nothing to play"));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_name_that_would_not_survive_a_round_trip_is_refused() {
        let dir = root("names");
        for bad in [" Mix", "Mix ", "  ", "tab\tname"] {
            assert!(save(&dir, &Playlist { name: bad.into(), entries: Vec::new() }).is_err(), "{bad:?}");
        }
        let long = Playlist { name: "x".repeat(MAX_NAME + 1), entries: Vec::new() };
        assert!(save(&dir, &long).unwrap_err().to_string().contains("too long"));
        // The longest allowed really does write, temporary name and all.
        save(&dir, &Playlist { name: "x".repeat(MAX_NAME), entries: Vec::new() }).unwrap();
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_file_that_does_not_parse_is_reported_and_can_be_removed() {
        let dir = root("broken");
        std::fs::create_dir_all(folder(&dir)).unwrap();
        let path = file_for(&dir, "Torn");
        std::fs::write(&path, "no header here\n").unwrap();
        assert!(list(&dir).is_empty());
        assert_eq!(broken(&dir), ["Torn.playlist"]);
        delete(&dir, "Torn").unwrap();
        assert!(!path.exists() && broken(&dir).is_empty());

        // A good playlist under another name that shares the file is not Torn's to delete.
        save(&dir, &Playlist { name: "a?b".into(), entries: Vec::new() }).unwrap();
        assert!(delete(&dir, "a*b").is_err());
        assert!(load(&dir, "a?b").is_some(), "deleted another playlist's file");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn two_names_that_share_a_file_name_do_not_replace_each_other() {
        let dir = root("clash");
        let one = Playlist { name: "a?b".into(), entries: vec![entry("x", "A/x.opus")] };
        save(&dir, &one).unwrap();
        let two = Playlist { name: "a*b".into(), entries: Vec::new() };
        let err = save(&dir, &two).unwrap_err().to_string();
        assert!(err.contains("a?b") && err.contains("another name"), "{err}");
        assert_eq!(load(&dir, "a?b").unwrap().entries.len(), 1, "the first was replaced");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    // The list is the only thing it may remove: the audio is in the folders, and these stay.
    #[test]
    fn deleting_a_playlist_takes_its_files_and_never_the_audio() {
        let dir = root("delete");
        std::fs::create_dir_all(dir.join("A")).unwrap();
        std::fs::write(dir.join("A/01 - A.opus"), "audio").unwrap();
        let made = Playlist { name: "Mix".into(), entries: vec![entry("a", "A/01 - A.opus")] };
        save(&dir, &made).unwrap();
        std::fs::write(file_for(&dir, "Mix").with_extension("m3u8"), "#EXTM3U\n").unwrap();

        delete(&dir, "Mix").unwrap();
        assert!(list(&dir).is_empty());
        assert!(!file_for(&dir, "Mix").with_extension("m3u8").exists());
        assert!(dir.join("A/01 - A.opus").is_file(), "the audio went with the list");
        assert!(delete(&dir, "Mix").is_err());
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
