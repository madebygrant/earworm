use std::path::Path;
use std::sync::mpsc::Sender;

use anyhow::{Context, Result, bail};

use crate::app::{Asker, Msg, PlaylistRow};
use crate::config::composed;
use crate::manifest;
use crate::playlists::{self, Entry, Playlist};
use crate::tag;
use crate::worker::{self, Others};

// Keeps `library_all`'s counts honest after any change to a list.
fn refresh_library(dir: &Path, tx: &Sender<Msg>) {
    let _ = tx.send(Msg::Library { shelves: worker::library_all(dir), show: false });
}

// A list that does not parse is left out of the library, and a list that vanishes without a word reads as lost.
fn say_broken(dir: &Path, tx: &Sender<Msg>) {
    for file in playlists::broken(dir) {
        let _ = tx.send(Msg::Log(format!(
            "playlist: {file} in {} cannot be read  ·  it is hidden until it is fixed or deleted",
            playlists::folder(dir).display()
        )));
    }
}

fn load(dir: &Path, name: &str) -> Result<Playlist> {
    playlists::load(dir, name).with_context(|| format!("no playlist called \"{name}\"  ·  it may have been deleted"))
}

/// The screen's rows for a list, with the label read from the file's tags when it is there.
pub fn rows_of(dir: &Path, list: &Playlist) -> Vec<PlaylistRow> {
    let others = Others::everything(dir);
    let resolved = worker::resolve_entries(&others, dir, &list.entries);
    list.entries
        .iter()
        .zip(resolved)
        .map(|(entry, path)| {
            let stem = |p: &Path| p.file_stem().unwrap_or_default().to_string_lossy().to_string();
            let label = path
                .as_deref()
                .and_then(|p| tag::read(p).ok())
                .filter(|info| !info.title.is_empty())
                .map_or_else(
                    || stem(path.as_deref().unwrap_or(&entry.hint)),
                    |info| if info.artist.is_empty() { info.title } else { format!("{} - {}", info.artist, info.title) },
                );
            let at = path.as_deref().unwrap_or(&entry.hint);
            PlaylistRow {
                id: entry.id.clone(),
                label,
                folder: at.parent().and_then(Path::file_name).unwrap_or_default().to_string_lossy().to_string(),
                file: at.file_name().unwrap_or_default().to_string_lossy().to_string(),
                path,
            }
        })
        .collect()
}

fn show(dir: &Path, tx: &Sender<Msg>, list: &Playlist, open: bool, focus: Option<String>) {
    let _ = tx.send(Msg::Custom { name: list.name.clone(), rows: rows_of(dir, list), show: open, focus });
    refresh_library(dir, tx);
}

// Moves and removals send the order alone: the screen has the labels, and re-reading every tag on each
// keypress made a long list sluggish.
fn order(tx: &Sender<Msg>, list: &Playlist, focus: Option<String>) {
    let ids = list.entries.iter().map(|e| e.id.clone()).collect();
    let _ = tx.send(Msg::Order { name: list.name.clone(), ids, focus });
}

// An entry follows its id, and the hint is only the fallback for a file nothing lists. So whenever the list is
// read, each entry that resolves gets the hint it has now and the id its folder's sidecar knows it by: a
// `~` id is dropped when a sync matches the file to a video, and a hint that is never refreshed then points
// at the old name once the file is edited. Saved only when something changed.
fn refreshed(dir: &Path, mut list: Playlist) -> Playlist {
    let others = Others::everything(dir);
    let files = worker::resolve_entries(&others, dir, &list.entries);
    let mut changed = false;
    let mut taken: std::collections::HashSet<String> = list.entries.iter().map(|e| e.id.clone()).collect();
    for (entry, file) in list.entries.iter_mut().zip(files) {
        let Some(file) = file else {
            continue;
        };
        if let (Ok(rel), Some(name)) = (file.strip_prefix(dir), file.file_name()) {
            if let Some(home) = rel.components().next() {
                let hint = Path::new(home.as_os_str()).join(name);
                if hint != entry.hint && playlists::safe_hint(&hint) {
                    entry.hint = hint;
                    changed = true;
                }
            }
        }
        let known = others.all().iter().flat_map(|o| &o.sidecar.entries).find(|(_, p)| *p == file).map(|(id, _)| id.clone());
        if let Some(id) = known.filter(|id| *id != entry.id && !taken.contains(id)) {
            taken.insert(id.clone());
            entry.id = id;
            changed = true;
        }
    }
    if changed {
        // Not worth failing the open for: the list is as good as it was.
        let _ = playlists::save(dir, &list);
    }
    list
}

pub fn open(dir: &Path, tx: &Sender<Msg>, name: &str) -> Result<()> {
    say_broken(dir, tx);
    show(dir, tx, &refreshed(dir, load(dir, name)?), true, None);
    Ok(())
}

pub fn new(dir: &Path, tx: &Sender<Msg>, asker: &Asker) -> Result<()> {
    let Some(name) = asker.input_noted("Playlist name", "a list built from tracks in your other folders", "", crate::app::Escape::Keep) else {
        return Ok(());
    };
    let name = name.trim().to_string();
    if name.is_empty() {
        return Ok(());
    }
    if playlists::load(dir, &name).is_some() {
        bail!("there is already a playlist called \"{name}\"");
    }
    say_broken(dir, tx);
    playlists::save(dir, &Playlist { name: name.clone(), entries: Vec::new() })?;
    refresh_library(dir, tx);
    let _ = tx.send(Msg::Select(playlists::path_of(dir, &name)));
    let _ = tx.send(Msg::PickTracks(name));
    Ok(())
}

/// The id a playlist can follow for a file, adopting its folder when it has none.
// A file in a folder earworm never edited has no sidecar entry, so an entry for it would only be its path and
// the first rename would lose it. Writing the entry is what `save_manifest` does at the first edit, done now.
fn id_for(folder: &Path, file: &str) -> Result<(String, bool)> {
    let sidecar = manifest::load(folder);
    let want = composed(file);
    let named = |p: &Path| p.file_name().is_some_and(|n| composed(&n.to_string_lossy()) == want);
    if let Some((id, _)) = sidecar.entries.iter().find(|(_, p)| named(p)) {
        return Ok((id.clone(), false));
    }
    if !folder.join(file).is_file() {
        bail!("{file} is not in {}", folder.display());
    }
    // A renamed file keeps its old `~` id, so a new file may be named like the one the id was made for.
    let mut id = manifest::local_id(file);
    let mut n = 2;
    while sidecar.entries.iter().any(|(held, _)| *held == id) {
        id = format!("{} ({n})", manifest::local_id(file));
        n += 1;
    }
    let entries = sidecar.entries.iter().cloned().chain([(id.clone(), folder.join(file))]);
    manifest::write(folder, sidecar.url.as_deref().unwrap_or(""), entries)
        .with_context(|| format!("recording {file} in {}", manifest::path(folder).display()))?;
    Ok((id, true))
}

pub fn add(dir: &Path, tx: &Sender<Msg>, asker: &Asker, picks: &[(std::path::PathBuf, String)], into: Option<&str>) -> Result<()> {
    if picks.is_empty() {
        return Ok(());
    }
    say_broken(dir, tx);
    let lists = playlists::list(dir);
    if let Some(name) = into {
        let list = load(dir, name)?;
        return append(dir, tx, list, picks, true, 0);
    }
    let mut options: Vec<String> = lists.iter().map(|l| format!("{}  ({} tracks)", l.name, l.entries.len())).collect();
    options.push("a new playlist".into());
    let n = picks.len();
    let header = format!("Add {n} {}", if n == 1 { "track" } else { "tracks" });
    let Some(choice) = asker.choose(&header, options) else {
        return Ok(());
    };
    let list = if let Some(held) = lists.get(choice) {
        held.clone()
    } else {
        let Some(name) = asker.input("Playlist name", "") else {
            return Ok(());
        };
        let name = name.trim().to_string();
        if name.is_empty() {
            return Ok(());
        }
        playlists::load(dir, &name).unwrap_or(Playlist { name, entries: Vec::new() })
    };
    append(dir, tx, list, picks, false, 0)
}

fn append(dir: &Path, tx: &Sender<Msg>, mut list: Playlist, picks: &[(std::path::PathBuf, String)], open: bool, removed: usize) -> Result<()> {
    let (mut added, mut already, mut adopted) = (0, 0, Vec::new());
    for (folder, file) in picks {
        // Only a folder directly under `--dir`: the hint names it by its place there.
        let Some(home) = folder.strip_prefix(dir).ok().filter(|rel| rel.components().count() == 1) else {
            let _ = tx.send(Msg::Log(format!("playlist: {} is not a folder under {}", folder.display(), dir.display())));
            continue;
        };
        let (id, fresh) = match id_for(folder, file) {
            Ok(found) => found,
            Err(err) => {
                let _ = tx.send(Msg::Log(format!("playlist: {err}")));
                continue;
            }
        };
        if fresh {
            adopted.push(home.display().to_string());
        }
        if list.entries.iter().any(|e| e.id == id) {
            already += 1;
            continue;
        }
        let entry = Entry { id, hint: home.join(file) };
        // `save` drops what it cannot write, so counting it as added would be a claim about nothing.
        if !playlists::writable(&entry) {
            let _ = tx.send(Msg::Log(format!("playlist: {file} has a name a list cannot hold")));
            continue;
        }
        list.entries.push(entry);
        added += 1;
    }
    playlists::save(dir, &list)?;
    adopted.sort();
    adopted.dedup();
    for folder in &adopted {
        let _ = tx.send(Msg::Log(format!("playlist: {folder} now keeps a .earworm so a playlist can follow its files")));
    }
    let said = match adopted.as_slice() {
        [] => String::new(),
        [one] => format!("  ·  {one} now keeps a .earworm"),
        many => format!("  ·  {} folders now keep a .earworm", many.len()),
    };
    if added > 0 {
        let _ = tx.send(Msg::FoundUnmark);
    }
    if open {
        show(dir, tx, &refreshed(dir, list.clone()), true, None);
    } else {
        refresh_library(dir, tx);
    }
    let kept = if already > 0 { format!("  ·  {already} already there") } else { String::new() };
    let took = match (added, removed) {
        (_, 0) => format!("added {added} to {}", list.name),
        (0, _) => format!("removed {removed} from {}", list.name),
        _ => format!("added {added} to and removed {removed} from {}", list.name),
    };
    let _ = tx.send(Msg::Flash(format!("{took}{kept}{said}")));
    Ok(())
}

// The picker's answer for an open list: tracks to add and tracks to take off, both by folder and filename.
// A removal drops the entry only, never the audio.
pub fn edit(dir: &Path, tx: &Sender<Msg>, name: &str, add: &[(std::path::PathBuf, String)], remove: &[(std::path::PathBuf, String)]) -> Result<()> {
    say_broken(dir, tx);
    let mut list = load(dir, name)?;
    let want: std::collections::HashSet<(std::path::PathBuf, String)> =
        remove.iter().map(|(f, n)| (f.clone(), composed(n))).collect();
    let files = worker::resolve_entries(&Others::everything(dir), dir, &list.entries);
    let before = list.entries.len();
    let mut files = files.into_iter();
    list.entries.retain(|_| {
        let gone = files.next().flatten().is_some_and(|p| match (p.parent(), p.file_name()) {
            (Some(folder), Some(file)) => want.contains(&(folder.to_path_buf(), composed(&file.to_string_lossy()))),
            _ => false,
        });
        !gone
    });
    let removed = before - list.entries.len();
    append(dir, tx, list, add, true, removed)
}

pub fn move_entry(dir: &Path, tx: &Sender<Msg>, name: &str, id: &str, up: bool) -> Result<()> {
    let mut list = load(dir, name)?;
    let Some(at) = list.entries.iter().position(|e| e.id == id) else {
        bail!("that track is no longer in {name}");
    };
    let to = if up { at.checked_sub(1) } else { Some(at + 1).filter(|n| *n < list.entries.len()) };
    if let Some(to) = to {
        list.entries.swap(at, to);
        playlists::save(dir, &list)?;
    }
    order(tx, &list, Some(id.to_string()));
    Ok(())
}

pub fn remove_entry(dir: &Path, tx: &Sender<Msg>, name: &str, id: &str) -> Result<()> {
    let mut list = load(dir, name)?;
    list.entries.retain(|e| e.id != id);
    playlists::save(dir, &list)?;
    order(tx, &list, None);
    // A removal changes the list's count in the library; a move does not, so only this one refreshes.
    refresh_library(dir, tx);
    Ok(())
}

pub fn rename(dir: &Path, tx: &Sender<Msg>, asker: &Asker, name: &str) -> Result<()> {
    load(dir, name)?;
    let Some(new) = asker.input_noted("Playlist name", "renames the list  ·  its tracks stay where they are", name, crate::app::Escape::Keep) else {
        return Ok(());
    };
    let new = new.trim().to_string();
    if new.is_empty() || new == name {
        return Ok(());
    }
    playlists::rename(dir, name, &new)?;
    // cliamp's stored copy is keyed on the old name and nothing else will name it again.
    crate::player::forget_list(&playlists::key(dir, name));
    // Before the library, whose refresh would otherwise see the old name gone and clear the marker.
    let _ = tx.send(Msg::Renamed { old: name.to_string(), new: new.clone() });
    refresh_library(dir, tx);
    let _ = tx.send(Msg::Select(playlists::path_of(dir, &new)));
    let _ = tx.send(Msg::Flash(format!("renamed to {new}")));
    Ok(())
}

pub fn delete(dir: &Path, tx: &Sender<Msg>, asker: &Asker, name: &str) -> Result<()> {
    let list = load(dir, name)?;
    let note = format!("the list only  ·  its {} tracks stay in their folders", list.entries.len());
    let options = vec!["Delete the list".to_string(), "Keep it".to_string()];
    if asker.choose_noted(&format!("Delete {name}?"), &note, options) != Some(0) {
        return Ok(());
    }
    playlists::delete(dir, name)?;
    crate::player::forget_list(&playlists::key(dir, name));
    refresh_library(dir, tx);
    let _ = tx.send(Msg::Flash(format!("deleted {name}  ·  no audio was touched")));
    Ok(())
}

/// The message and the folders the list's files sit in, which is how a playing track is tied back to the list.
pub fn play(dir: &Path, name: &str) -> Result<(String, Vec<std::path::PathBuf>)> {
    let list = refreshed(dir, load(dir, name)?);
    let others = Others::everything(dir);
    let files = worker::resolve_entries(&others, dir, &list.entries);
    let m3u8 = playlists::write_m3u8(dir, &list, &files)?;
    let said = crate::player::load_list(&m3u8, &playlists::key(dir, name))?;
    let mut folders: Vec<std::path::PathBuf> = files.iter().flatten().filter_map(|f| f.parent().map(Path::to_path_buf)).collect();
    folders.sort();
    folders.dedup();
    Ok((said, folders))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::Reply;
    use std::path::PathBuf;
    use std::sync::mpsc::{Receiver, channel};
    use std::time::Duration;

    fn root(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("earworm-custom-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    // A folder the way a finished sync leaves it: audio, and a sidecar naming each file.
    fn folder(dir: &Path, name: &str, files: &[(&str, &str)]) -> PathBuf {
        let folder = dir.join(name);
        std::fs::create_dir_all(&folder).unwrap();
        for (_, file) in files {
            std::fs::write(folder.join(file), "audio").unwrap();
        }
        let entries = files.iter().map(|(id, file)| (id.to_string(), folder.join(file)));
        manifest::write(&folder, "u", entries).unwrap();
        folder
    }

    /// Runs `work` while this thread answers each question it asks, in order.
    fn answering<T: Send>(
        answers: Vec<Reply>,
        work: impl FnOnce(&Sender<Msg>, &Asker) -> T + Send,
    ) -> (T, Receiver<Msg>) {
        let (tx, rx) = channel();
        let asker = Asker { tx: tx.clone(), enabled: true };
        let mut answers = answers.into_iter();
        let mut kept = Vec::new();
        let mut out = None;
        std::thread::scope(|scope| {
            let worker = scope.spawn(|| work(&tx, &asker));
            while let Ok(msg) = rx.recv_timeout(Duration::from_millis(1500)) {
                match msg {
                    Msg::Ask(_, reply) => reply.send(answers.next().unwrap_or(Reply::Cancel)).unwrap(),
                    // Everything else is what the test goes on to read.
                    other => kept.push(other),
                }
                if worker.is_finished() {
                    break;
                }
            }
            out = Some(worker.join().unwrap());
        });
        kept.extend(rx.try_iter());
        let (back, seen) = channel();
        for msg in kept {
            let _ = back.send(msg);
        }
        (out.unwrap(), seen)
    }

    fn in_new_playlist(name: &str) -> Vec<Reply> {
        vec![Reply::Choice(0), Reply::Text(name.into())]
    }

    #[test]
    fn an_edit_takes_unmarked_tracks_off_the_list_and_never_off_disk() {
        let dir = root("edit");
        let a = folder(&dir, "A", &[("a1", "01 - One.opus")]);
        let b = folder(&dir, "B", &[("b1", "01 - Two.opus"), ("b2", "02 - Three.opus")]);
        playlists::save(&dir, &Playlist { name: "Mix".into(), entries: Vec::new() }).unwrap();
        let first = vec![(a.clone(), "01 - One.opus".to_string()), (b.clone(), "01 - Two.opus".to_string())];
        let (result, _rx) = answering(Vec::new(), |tx, asker| add(&dir, tx, asker, &first, Some("Mix")));
        result.unwrap();
        let remove = vec![(a.clone(), "01 - One.opus".to_string())];
        let added = vec![(b.clone(), "02 - Three.opus".to_string())];
        let (result, rx) = answering(Vec::new(), |tx, _| edit(&dir, tx, "Mix", &added, &remove));
        result.unwrap();
        let ids: Vec<String> = playlists::load(&dir, "Mix").unwrap().entries.into_iter().map(|e| e.id).collect();
        assert_eq!(ids, ["b1", "b2"]);
        assert!(a.join("01 - One.opus").is_file(), "the audio was touched");
        assert!(rx.try_iter().any(|m| matches!(m, Msg::Flash(t) if t.contains("added 1") && t.contains("removed 1"))));
    }

    #[test]
    fn a_new_list_is_saved_empty_and_opens_the_picker() {
        let dir = root("newpick");
        let (result, rx) = answering(vec![Reply::Text("Mix".into())], |tx, asker| new(&dir, tx, asker));
        result.unwrap();
        assert!(playlists::load(&dir, "Mix").unwrap().entries.is_empty());
        assert!(rx.try_iter().any(|m| matches!(m, Msg::PickTracks(name) if name == "Mix")));
    }

    #[test]
    fn an_add_with_a_target_writes_to_that_list_with_no_question_asked() {
        let dir = root("target");
        let a = folder(&dir, "A", &[("a1", "01 - One.opus")]);
        playlists::save(&dir, &Playlist { name: "Mix".into(), entries: Vec::new() }).unwrap();
        let picks = vec![(a, "01 - One.opus".to_string())];
        let (result, rx) = answering(Vec::new(), |tx, asker| add(&dir, tx, asker, &picks, Some("Mix")));
        result.unwrap();
        assert_eq!(playlists::load(&dir, "Mix").unwrap().entries.len(), 1);
        let msgs: Vec<Msg> = rx.try_iter().collect();
        assert!(!msgs.iter().any(|m| matches!(m, Msg::Ask(..))));
        assert!(msgs.iter().any(|m| matches!(m, Msg::Custom { show: true, .. })), "the list should open");
    }

    // The plan's exit test: two folders, a source edit that renames its file, and every entry still resolves.
    #[test]
    fn a_playlist_built_from_two_folders_survives_an_edit_that_renames_a_source() {
        let dir = root("two");
        let a = folder(&dir, "A", &[("a1", "01 - One.opus")]);
        let b = folder(&dir, "B", &[("b1", "01 - Two.opus")]);
        let picks = vec![(a.clone(), "01 - One.opus".to_string()), (b.clone(), "01 - Two.opus".to_string())];
        let (result, _rx) = answering(in_new_playlist("Mix"), |tx, asker| add(&dir, tx, asker, &picks, None));
        result.unwrap();
        let list = playlists::load(&dir, "Mix").unwrap();
        assert_eq!(list.entries.iter().map(|e| e.id.as_str()).collect::<Vec<_>>(), ["a1", "b1"]);

        // What `write_track` does to a source: the file is renamed and the sidecar follows it.
        std::fs::rename(b.join("01 - Two.opus"), b.join("01 - Kraftwerk - Two.opus")).unwrap();
        manifest::write(&b, "u", [("b1".to_string(), b.join("01 - Kraftwerk - Two.opus"))].into_iter()).unwrap();

        let rows = rows_of(&dir, &list);
        assert_eq!(rows.len(), 2);
        assert!(rows.iter().all(|r| r.path.as_ref().is_some_and(|p| p.is_file())), "{rows:?}");
        assert_eq!(rows[1].file, "01 - Kraftwerk - Two.opus");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    // A file in a folder earworm never edited has no id, so the add has to give it one that outlives a rename.
    #[test]
    fn a_file_with_no_sidecar_entry_is_adopted_so_the_playlist_can_follow_its_rename() {
        let dir = root("adopt");
        let loose = dir.join("Loose");
        std::fs::create_dir_all(&loose).unwrap();
        std::fs::write(loose.join("Song.opus"), "audio").unwrap();
        assert!(!manifest::path(&loose).exists());

        let picks = vec![(loose.clone(), "Song.opus".to_string())];
        let (result, rx) = answering(in_new_playlist("Mix"), |tx, asker| add(&dir, tx, asker, &picks, None));
        result.unwrap();
        assert!(manifest::path(&loose).is_file(), "nothing recorded the file, so a rename would lose it");
        let list = playlists::load(&dir, "Mix").unwrap();
        assert_eq!(list.entries[0].id, "~Song.opus");
        let said: Vec<String> = rx.try_iter().filter_map(|m| if let Msg::Log(l) = m { Some(l) } else { None }).collect();
        assert!(said.iter().any(|l| l.contains("Loose") && l.contains(".earworm")), "adoption went unsaid: {said:?}");

        // Renamed the way an edit would, with the sidecar following.
        std::fs::rename(loose.join("Song.opus"), loose.join("01 - Song.opus")).unwrap();
        manifest::write(&loose, "", [("~Song.opus".to_string(), loose.join("01 - Song.opus"))].into_iter()).unwrap();
        assert_eq!(rows_of(&dir, &list)[0].path, Some(loose.join("01 - Song.opus")));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    // Hiding takes a row off the library and nothing else, and it is documented to keep playlists playing.
    #[test]
    fn a_hidden_folders_track_still_resolves_after_it_is_renamed() {
        let dir = root("hidden");
        let a = folder(&dir, "A", &[("a1", "01 - One.opus")]);
        let picks = vec![(a.clone(), "01 - One.opus".to_string())];
        answering(in_new_playlist("Mix"), |tx, asker| add(&dir, tx, asker, &picks, None)).0.unwrap();
        manifest::hide(&a).unwrap();
        std::fs::rename(a.join("01 - One.opus"), a.join("01 - Renamed.opus")).unwrap();
        manifest::write(&a, "u", [("a1".to_string(), a.join("01 - Renamed.opus"))].into_iter()).unwrap();
        manifest::hide(&a).unwrap();

        let list = playlists::load(&dir, "Mix").unwrap();
        assert_eq!(rows_of(&dir, &list)[0].path, Some(a.join("01 - Renamed.opus")), "a hidden folder lost its entry");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    // A sync that matches an adopted file to a video drops its `~` id, and the hint then went stale on the next edit.
    #[test]
    fn opening_a_list_moves_a_dropped_local_id_and_a_stale_hint_onto_what_the_sidecar_now_says() {
        let dir = root("promote");
        let loose = dir.join("Loose");
        std::fs::create_dir_all(&loose).unwrap();
        std::fs::write(loose.join("Song.opus"), "audio").unwrap();
        let picks = vec![(loose.clone(), "Song.opus".to_string())];
        answering(in_new_playlist("Mix"), |tx, asker| add(&dir, tx, asker, &picks, None)).0.unwrap();
        // The URL is attached and the sync matches the file: the `~` line becomes a video id.
        manifest::write(&loose, "u", [("vid1".to_string(), loose.join("Song.opus"))].into_iter()).unwrap();
        let (tx, _rx) = channel();
        open(&dir, &tx, "Mix").unwrap();
        assert_eq!(playlists::load(&dir, "Mix").unwrap().entries[0].id, "vid1");

        // Now the edit renames it, and the entry still follows.
        std::fs::rename(loose.join("Song.opus"), loose.join("01 - Song.opus")).unwrap();
        manifest::write(&loose, "u", [("vid1".to_string(), loose.join("01 - Song.opus"))].into_iter()).unwrap();
        let list = playlists::load(&dir, "Mix").unwrap();
        assert_eq!(rows_of(&dir, &list)[0].path, Some(loose.join("01 - Song.opus")));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    // A renamed adopted file keeps `~Song.opus`, so a new file called Song.opus used to get the same id.
    #[test]
    fn a_new_file_named_like_a_renamed_one_gets_an_id_of_its_own() {
        let dir = root("repeat");
        let loose = dir.join("Loose");
        std::fs::create_dir_all(&loose).unwrap();
        std::fs::write(loose.join("Song.opus"), "first").unwrap();
        let (first, fresh) = id_for(&loose, "Song.opus").unwrap();
        assert!((first.as_str(), fresh) == ("~Song.opus", true));
        std::fs::rename(loose.join("Song.opus"), loose.join("01 - Song.opus")).unwrap();
        manifest::write(&loose, "", [(first.clone(), loose.join("01 - Song.opus"))].into_iter()).unwrap();

        std::fs::write(loose.join("Song.opus"), "second").unwrap();
        let (second, _) = id_for(&loose, "Song.opus").unwrap();
        assert_ne!(second, first, "two files share one id");
        assert!(manifest::is_local(&second));
        let held = manifest::entries(&loose);
        assert!(held.iter().any(|(id, p)| *id == first && p.ends_with("01 - Song.opus")), "{held:?}");
        assert!(held.iter().any(|(id, p)| *id == second && p.ends_with("Song.opus")), "{held:?}");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_name_a_list_cannot_hold_is_not_counted_as_added() {
        let dir = root("tab");
        let a = folder(&dir, "A", &[]);
        std::fs::write(a.join("Odd\tName.opus"), "audio").unwrap();
        let picks = vec![(a.clone(), "Odd\tName.opus".to_string())];
        let (result, rx) = answering(in_new_playlist("Mix"), |tx, asker| add(&dir, tx, asker, &picks, None));
        result.unwrap();
        let flashes: Vec<String> = rx.try_iter().filter_map(|m| if let Msg::Flash(f) = m { Some(f) } else { None }).collect();
        assert!(flashes.iter().any(|f| f.starts_with("added 0 to Mix")), "{flashes:?}");
        assert!(playlists::load(&dir, "Mix").unwrap().entries.is_empty());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn adding_a_track_twice_says_it_was_already_there_and_adds_nothing() {
        let dir = root("twice");
        let a = folder(&dir, "A", &[("a1", "01 - One.opus"), ("a2", "02 - Two.opus")]);
        let first = vec![(a.clone(), "01 - One.opus".to_string())];
        answering(in_new_playlist("Mix"), |tx, asker| add(&dir, tx, asker, &first, None)).0.unwrap();

        // Now `Mix` exists, so it is the first choice.
        let again = vec![(a.clone(), "01 - One.opus".to_string()), (a.clone(), "02 - Two.opus".to_string())];
        let (result, rx) = answering(vec![Reply::Choice(0)], |tx, asker| add(&dir, tx, asker, &again, None));
        result.unwrap();
        assert_eq!(playlists::load(&dir, "Mix").unwrap().entries.len(), 2);
        let flashes: Vec<String> = rx.try_iter().filter_map(|m| if let Msg::Flash(f) = m { Some(f) } else { None }).collect();
        assert!(flashes.iter().any(|f| f.contains("added 1") && f.contains("1 already there")), "{flashes:?}");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_cancelled_chooser_changes_nothing() {
        let dir = root("cancel");
        let a = folder(&dir, "A", &[("a1", "01 - One.opus")]);
        let picks = vec![(a.clone(), "01 - One.opus".to_string())];
        let (result, _rx) = answering(vec![Reply::Cancel], |tx, asker| add(&dir, tx, asker, &picks, None));
        result.unwrap();
        assert!(playlists::list(&dir).is_empty());
        assert!(!dir.join(".playlists").exists());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_pick_outside_the_library_is_skipped_and_never_written_into_a_list() {
        let dir = root("outside");
        let elsewhere = std::env::temp_dir().join(format!("earworm-elsewhere-{}", std::process::id()));
        std::fs::create_dir_all(&elsewhere).unwrap();
        std::fs::write(elsewhere.join("x.opus"), "audio").unwrap();
        let picks = vec![(elsewhere.clone(), "x.opus".to_string())];
        let (result, _rx) = answering(in_new_playlist("Mix"), |tx, asker| add(&dir, tx, asker, &picks, None));
        result.unwrap();
        assert!(playlists::load(&dir, "Mix").unwrap().entries.is_empty());
        assert!(!manifest::path(&elsewhere).exists(), "adopted a folder outside the library");
        std::fs::remove_dir_all(&elsewhere).unwrap();
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_move_and_a_remove_name_the_entry_by_id_and_keep_the_cursor_on_it() {
        let dir = root("order");
        let a = folder(&dir, "A", &[("a1", "01.opus"), ("a2", "02.opus"), ("a3", "03.opus")]);
        let picks: Vec<_> = ["01.opus", "02.opus", "03.opus"].iter().map(|f| (a.clone(), f.to_string())).collect();
        answering(in_new_playlist("Mix"), |tx, asker| add(&dir, tx, asker, &picks, None)).0.unwrap();
        let ids = |dir: &Path| playlists::load(dir, "Mix").unwrap().entries.into_iter().map(|e| e.id).collect::<Vec<_>>();

        let (tx, rx) = channel();
        move_entry(&dir, &tx, "Mix", "a3", true).unwrap();
        assert_eq!(ids(&dir), ["a1", "a3", "a2"]);
        let focus = rx.try_iter().find_map(|m| if let Msg::Order { focus, .. } = m { focus } else { None });
        assert_eq!(focus.as_deref(), Some("a3"), "the cursor would stay on whatever moved into the row");

        // At the edge it is a no-op and not an error or a wrap.
        move_entry(&dir, &tx, "Mix", "a1", true).unwrap();
        assert_eq!(ids(&dir), ["a1", "a3", "a2"]);
        remove_entry(&dir, &tx, "Mix", "a3").unwrap();
        assert_eq!(ids(&dir), ["a1", "a2"]);
        assert!(a.join("03.opus").is_file(), "removing an entry deleted the audio");
        assert!(move_entry(&dir, &tx, "Mix", "gone", false).is_err());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    // The plan's second exit test: `D` on a custom playlist removes the list and never audio.
    #[test]
    fn deleting_a_custom_playlist_removes_the_list_and_none_of_the_audio() {
        let dir = root("delete");
        let a = folder(&dir, "A", &[("a1", "01 - One.opus")]);
        let b = folder(&dir, "B", &[("b1", "01 - Two.opus")]);
        let picks = vec![(a.clone(), "01 - One.opus".to_string()), (b.clone(), "01 - Two.opus".to_string())];
        answering(in_new_playlist("Mix"), |tx, asker| add(&dir, tx, asker, &picks, None)).0.unwrap();

        // Keeping it, or backing out, changes nothing.
        for no in [Reply::Choice(1), Reply::Cancel] {
            let (result, _rx) = answering(vec![no], |tx, asker| delete(&dir, tx, asker, "Mix"));
            result.unwrap();
            assert!(playlists::load(&dir, "Mix").is_some(), "the list went without a yes");
        }

        let (result, rx) = answering(vec![Reply::Choice(0)], |tx, asker| delete(&dir, tx, asker, "Mix"));
        result.unwrap();
        assert!(playlists::load(&dir, "Mix").is_none());
        for audio in [a.join("01 - One.opus"), b.join("01 - Two.opus")] {
            assert!(audio.is_file(), "{} was deleted with the list", audio.display());
        }
        assert!(manifest::path(&a).is_file() && manifest::path(&b).is_file(), "a folder's sidecar went");
        let library = rx.try_iter().filter_map(|m| if let Msg::Library { shelves, .. } = m { Some(shelves) } else { None }).last().unwrap();
        assert!(library.iter().all(|s| s.name != "Mix"), "the row survived its list");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_list_that_cannot_be_read_is_named_in_the_log_when_you_work_with_lists() {
        let dir = root("broken");
        std::fs::create_dir_all(playlists::folder(&dir)).unwrap();
        std::fs::write(playlists::folder(&dir).join("Torn.playlist"), "no header\n").unwrap();
        let (tx, rx) = channel();
        let _ = open(&dir, &tx, "Torn");
        let said: Vec<String> = rx.try_iter().filter_map(|m| if let Msg::Log(l) = m { Some(l) } else { None }).collect();
        assert!(said.iter().any(|l| l.contains("Torn.playlist") && l.contains("cannot be read")), "{said:?}");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn renaming_a_custom_playlist_keeps_its_tracks() {
        let dir = root("rename");
        let a = folder(&dir, "A", &[("a1", "01.opus")]);
        let picks = vec![(a, "01.opus".to_string())];
        answering(in_new_playlist("Old"), |tx, asker| add(&dir, tx, asker, &picks, None)).0.unwrap();
        let (result, _rx) = answering(vec![Reply::Text("New".into())], |tx, asker| rename(&dir, tx, asker, "Old"));
        result.unwrap();
        assert!(playlists::load(&dir, "Old").is_none());
        assert_eq!(playlists::load(&dir, "New").unwrap().entries.len(), 1);
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
