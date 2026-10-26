use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use anyhow::{Result, bail};

use crate::app::ArtistRow;
use crate::lookup::{lead_artist, normalise};
use crate::manifest::{self, Kind};
use crate::{tag, worker};

// Under `--dir` beside `.playlists`, not in the config: the config is written only by `^t` and the format keys.
const FILE: &str = ".artists";
// Release ids already known per starred artist, one `key<TAB>id,id` line each. Its own file: it grows with
// every release of every starred artist, which the favourites file has no reason to carry.
const SEEN: &str = ".artists-seen";
// Every read-modify-write of either file holds this: the new-release check writes from its own thread.
static STORE: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// One artist in the library, as the tags on disk name them.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Artist {
    pub name: String,
    pub tracks: usize,
    pub albums: usize,
    /// The folders holding their tracks, in the order they were read.
    pub folders: Vec<PathBuf>,
    /// Each album as first spelt, with how many of its tracks are here. What the discography is matched to.
    pub held: Vec<(String, usize)>,
}

/// What two spellings of one artist share, so "Björk" and "bjork" are one row and one star.
// A name of punctuation alone ("!!!") normalises to nothing, and would otherwise match every other one.
pub fn key(name: &str) -> String {
    let key = normalise(name);
    if key.is_empty() { name.trim().to_lowercase() } else { key }
}

/// Groups (artist, album, folder, counted) by lead artist, most tracks first, under the commonest spelling.
// "A feat. B" counts towards A only: a guest verse is not one of B's albums. A track not counted is a copy
// of one already seen, which still says the folder holds the artist.
pub fn group<'a>(tracks: impl IntoIterator<Item = (&'a str, &'a str, &'a Path, bool)>) -> Vec<Artist> {
    struct Seen {
        key: String,
        spellings: Vec<(String, usize)>,
        tracks: usize,
        // Keyed on `normalise`, so two spellings of one album are one album.
        albums: Vec<(String, String, usize)>,
        folders: Vec<PathBuf>,
    }
    let mut seen: Vec<Seen> = Vec::new();
    let mut at: HashMap<String, usize> = HashMap::new();
    for (artist, album, folder, counted) in tracks {
        let lead = lead_artist(artist);
        if lead.is_empty() {
            continue;
        }
        let k = key(&lead);
        let i = *at.entry(k.clone()).or_insert_with(|| {
            seen.push(Seen { key: k, spellings: Vec::new(), tracks: 0, albums: Vec::new(), folders: Vec::new() });
            seen.len() - 1
        });
        let one = &mut seen[i];
        if !one.folders.iter().any(|f| f == folder) {
            one.folders.push(folder.to_path_buf());
        }
        if !counted {
            continue;
        }
        one.tracks += 1;
        match one.spellings.iter_mut().find(|(s, _)| *s == lead) {
            Some((_, n)) => *n += 1,
            None => one.spellings.push((lead, 1)),
        }
        let album_key = normalise(album);
        if !album_key.is_empty() {
            match one.albums.iter_mut().find(|(k, ..)| *k == album_key) {
                Some((.., n)) => *n += 1,
                None => one.albums.push((album_key, album.trim().to_string(), 1)),
            }
        }
    }
    seen.sort_by(|a, b| b.tracks.cmp(&a.tracks).then_with(|| a.key.cmp(&b.key)));
    seen.into_iter()
        .map(|one| {
            // `rev` so a tie goes to the spelling seen first: `max_by_key` keeps the last of equals.
            let name = one.spellings.iter().rev().max_by_key(|(_, n)| *n).map(|(s, _)| s.clone()).unwrap_or_default();
            let held: Vec<(String, usize)> = one.albums.into_iter().map(|(_, album, n)| (album, n)).collect();
            Artist { name, tracks: one.tracks, albums: held.len(), folders: one.folders, held }
        })
        .collect()
}

/// Every artist in the folders `library` lists, from the tags on disk.
// Custom playlists stay out because their tracks are already counted in the folders they live in.
pub fn index(dir: &Path) -> Vec<Artist> {
    let mut read = Vec::new();
    let mut counted: HashSet<String> = HashSet::new();
    for shelf in worker::library(dir) {
        let sidecar = manifest::load(&shelf.path);
        let ids: HashMap<&Path, &str> = sidecar.entries.iter().map(|(id, f)| (f.as_path(), id.as_str())).collect();
        let isrcs: HashMap<&str, &str> = sidecar.isrcs.iter().map(|(id, i)| (id.as_str(), i.as_str())).collect();
        // With `album` on, a track the lookup found no album for is tagged with the playlist's name.
        let named_album = (shelf.kind != Some(Kind::Album)).then(|| normalise(&shelf.name));
        for (file, here) in &shelf.files {
            if !here {
                continue;
            }
            let path = shelf.path.join(file);
            // A song on two playlists is a copy in each folder. A `~` id is only unique within its folder.
            let mut first = true;
            if let Some(id) = ids.get(path.as_path()).filter(|id| !id.starts_with('~')) {
                let recording = [Some(id.to_string()), isrcs.get(id).map(|i| format!("isrc {i}"))];
                first = !recording.iter().flatten().any(|r| counted.contains(r));
                counted.extend(recording.into_iter().flatten());
            }
            if let Ok((artist, _, mut album)) = tag::brief(&path) {
                if named_album.as_deref() == Some(normalise(&album).as_str()) {
                    album.clear();
                }
                read.push((artist, album, shelf.path.clone(), first));
            }
        }
    }
    group(read.iter().map(|(artist, album, folder, first)| (artist.as_str(), album.as_str(), folder.as_path(), *first)))
}

// The title and album of every track by one artist in their folders, read when a page or download needs them.
// Not kept in the index: every title of every track for the session, to serve a few keys on one page.
fn tracks_of(index: &[Artist], artist: &str) -> Vec<(String, String)> {
    let who = key(artist);
    let Some(found) = index.iter().find(|a| key(&a.name) == who) else {
        return Vec::new();
    };
    found
        .folders
        .iter()
        .flat_map(|folder| worker::audio_files(folder))
        .filter_map(|file| tag::brief(&file).ok())
        .filter(|(a, ..)| key(&lead_artist(a)) == who)
        .map(|(_, title, album)| (title, album))
        .collect()
}

/// Every title the library holds by one artist.
pub fn titles_of(index: &[Artist], artist: &str) -> Vec<String> {
    tracks_of(index, artist).into_iter().map(|(title, _)| title).collect()
}

/// The titles the library holds of one artist's album, matched as the discography matches it.
pub fn owned_titles(index: &[Artist], artist: &str, album: &str) -> Vec<String> {
    let want = crate::discover::album_key(album);
    tracks_of(index, artist).into_iter().filter(|(_, al)| crate::discover::album_key(al) == want).map(|(title, _)| title).collect()
}

/// The release ids already known for each starred artist, by `key`. An artist with no line has never been
/// looked at, which is different from one looked at with nothing out.
pub fn seen(root: &Path) -> HashMap<String, HashSet<u64>> {
    let text = std::fs::read_to_string(root.join(SEEN)).unwrap_or_default();
    text.lines()
        .filter_map(|line| line.split_once('\t'))
        .map(|(who, ids)| (who.to_string(), ids.split(',').filter_map(|id| id.trim().parse().ok()).collect()))
        .collect()
}

/// Adds release ids to what is known of an artist, which is what stops them being new.
pub fn mark_seen(root: &Path, artist: &str, ids: impl IntoIterator<Item = u64>) -> Result<()> {
    let _held = STORE.lock().unwrap_or_else(|e| e.into_inner());
    let mut all = seen(root);
    all.entry(key(artist)).or_default().extend(ids);
    let mut lines: Vec<String> = all
        .iter()
        .map(|(who, ids)| {
            let mut ids: Vec<u64> = ids.iter().copied().collect();
            ids.sort_unstable();
            format!("{who}\t{}", ids.iter().map(u64::to_string).collect::<Vec<_>>().join(","))
        })
        .collect();
    lines.sort();
    crate::config::write_atomically(&root.join(SEEN), &(lines.join("\n") + "\n"))
}

/// The artists screen's rows: starred first, then by track count.
// A starred artist with nothing left in the library still gets a row, so the star can be taken off.
pub fn rows(root: &Path, index: &[Artist]) -> Vec<ArtistRow> {
    let favourites = favourites(root);
    let starred: HashSet<String> = favourites.iter().map(|f| key(&f.name)).collect();
    let mut rows: Vec<ArtistRow> = index
        .iter()
        .map(|a| ArtistRow {
            name: a.name.clone(),
            tracks: a.tracks,
            albums: a.albums,
            starred: starred.contains(&key(&a.name)),
            folders: a.folders.clone(),
        })
        .collect();
    let listed: HashSet<String> = index.iter().map(|a| key(&a.name)).collect();
    for f in favourites {
        if !listed.contains(&key(&f.name)) {
            rows.push(ArtistRow { name: f.name, tracks: 0, albums: 0, starred: true, folders: Vec::new() });
        }
    }
    // Stable, so each half keeps the index's order.
    rows.sort_by_key(|r| !r.starred);
    rows
}

/// Stars the artist, or unstars one already starred. True when they end up starred.
pub fn toggle(root: &Path, name: &str) -> Result<bool> {
    let name = writable(name)?;
    let _held = STORE.lock().unwrap_or_else(|e| e.into_inner());
    let mut store = load(root);
    let on = match position(&store.list, name) {
        Some(at) => {
            store.list.remove(at);
            false
        }
        None => {
            store.list.push(Favourite { name: name.to_string(), id: None });
            true
        }
    };
    save(root, &store)?;
    Ok(on)
}

// A tab or newline would split the line.
fn writable(name: &str) -> Result<&str> {
    let name = name.trim();
    if name.is_empty() || name.contains(['\t', '\n', '\r']) {
        bail!("cannot star an artist called {name:?}");
    }
    Ok(name)
}

/// A starred artist. The Deezer id is filled in the first time their page is opened.
// Starring is offline, so the name comes first. The discography needs the id: two artists can share a name.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Favourite {
    pub name: String,
    pub id: Option<u64>,
}

// Headers this version does not know are kept and written back, or an older build would erase them.
#[derive(Default)]
struct Store {
    headers: Vec<String>,
    list: Vec<Favourite>,
}

fn path(root: &Path) -> std::path::PathBuf {
    root.join(FILE)
}

// One `name<TAB>id` line each, after any `#` headers. A name starting `#` or `\` is written behind a `\`.
fn parse(text: &str) -> Store {
    let mut store = Store::default();
    for line in text.lines() {
        if line.starts_with('#') {
            store.headers.push(line.to_string());
            continue;
        }
        let line = line.strip_prefix('\\').unwrap_or(line);
        let (name, id) = line.split_once('\t').unwrap_or((line, ""));
        let name = name.trim();
        if !name.is_empty() {
            store.list.push(Favourite { name: name.to_string(), id: id.trim().parse().ok() });
        }
    }
    store
}

fn load(root: &Path) -> Store {
    std::fs::read_to_string(path(root)).map(|text| parse(&text)).unwrap_or_default()
}

pub fn favourites(root: &Path) -> Vec<Favourite> {
    load(root).list
}

fn save(root: &Path, store: &Store) -> Result<()> {
    let mut text = String::new();
    for header in &store.headers {
        text.push_str(header);
        text.push('\n');
    }
    for f in &store.list {
        let escape = if f.name.starts_with(['#', '\\']) { "\\" } else { "" };
        let id = f.id.map(|id| id.to_string()).unwrap_or_default();
        text.push_str(&format!("{escape}{}\t{id}\n", f.name));
    }
    crate::config::write_atomically(&path(root), &text)
}

fn position(list: &[Favourite], name: &str) -> Option<usize> {
    let k = key(name);
    list.iter().position(|f| key(&f.name) == k)
}

/// Records the Deezer id a starred artist resolved to. Nothing happens for one not starred.
pub fn set_id(root: &Path, name: &str, id: u64) -> Result<()> {
    let _held = STORE.lock().unwrap_or_else(|e| e.into_inner());
    let mut store = load(root);
    let Some(at) = position(&store.list, name) else {
        return Ok(());
    };
    store.list[at].id = Some(id);
    save(root, &store)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn root(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("earworm-artists-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    // Every track in one folder, for the tests about grouping rather than about where tracks are.
    fn grouped<'a>(pairs: impl IntoIterator<Item = (&'a str, &'a str)>) -> Vec<Artist> {
        group(pairs.into_iter().map(|(artist, album)| (artist, album, Path::new("F"), true)))
    }

    fn counts(found: &[Artist]) -> Vec<(&str, usize, usize)> {
        found.iter().map(|a| (a.name.as_str(), a.tracks, a.albums)).collect()
    }

    #[test]
    fn a_featured_credit_counts_towards_the_lead_artist_only() {
        let found = grouped([
            ("Massive Attack", "Mezzanine"),
            ("Massive Attack feat. Elizabeth Fraser", "Mezzanine"),
            ("Massive Attack (ft. Tracey Thorn)", "Protection"),
            ("Elizabeth Fraser", "Moses"),
        ]);
        assert_eq!(counts(&found), [("Massive Attack", 3, 2), ("Elizabeth Fraser", 1, 1)]);
    }

    #[test]
    fn a_band_name_with_an_ampersand_slash_comma_or_with_is_kept_whole() {
        let found = grouped([
            ("AC/DC", "Back in Black"),
            ("Simon & Garfunkel", "Bookends"),
            ("Earth, Wind & Fire", "I Am"),
            ("Nurse With Wound", "Soliloquy"),
        ]);
        let names: Vec<&str> = found.iter().map(|a| a.name.as_str()).collect();
        assert_eq!(names, ["AC/DC", "Earth, Wind & Fire", "Nurse With Wound", "Simon & Garfunkel"]);
    }

    #[test]
    fn a_name_of_punctuation_alone_is_an_artist_and_not_every_such_artist() {
        let found = grouped([("!!!", "Myth Takes"), ("+/-", "Xs on Your Eyes")]);
        assert_eq!(found.len(), 2);
        let dir = root("punct");
        assert!(toggle(&dir, "!!!").unwrap());
        assert!(toggle(&dir, "+/-").unwrap());
        assert!(toggle(&dir, "???").unwrap(), "a third punctuation name unstarred one of the others");
        assert_eq!(favourites(&dir).len(), 3);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn case_and_accents_do_not_split_an_artist_and_the_usual_spelling_is_shown() {
        let found = grouped([("bjork", "Post"), ("Björk", "Post"), ("BJÖRK", "Homogenic"), ("Björk", "Debut")]);
        assert_eq!(counts(&found), [("Björk", 4, 3)]);
    }

    #[test]
    fn a_tie_between_spellings_goes_to_the_first_seen() {
        let found = grouped([("Sigur Rós", "()"), ("Sigur Ros", "()")]);
        assert_eq!(found[0].name, "Sigur Rós");
    }

    #[test]
    fn an_untagged_track_has_no_artist_and_a_missing_album_is_not_an_album() {
        let found = grouped([("", "Loose"), ("Low", ""), ("Low", "Things We Lost in the Fire")]);
        assert_eq!(counts(&found), [("Low", 2, 1)]);
    }

    #[test]
    fn most_tracks_first_then_by_name() {
        let found = grouped([("Can", "Tago Mago"), ("Neu!", "Neu!"), ("Can", "Ege Bamyasi"), ("Faust", "IV")]);
        let names: Vec<&str> = found.iter().map(|a| a.name.as_str()).collect();
        assert_eq!(names, ["Can", "Faust", "Neu!"]);
    }

    #[test]
    fn the_index_reads_tags_from_listed_folders_and_skips_hidden_ones() {
        let dir = root("index");
        let shown = dir.join("Shown");
        let hidden = dir.join("Hidden");
        std::fs::create_dir_all(&shown).unwrap();
        std::fs::create_dir_all(&hidden).unwrap();
        let one = shown.join("01 - A.opus");
        let two = hidden.join("01 - B.opus");
        if crate::worker::tests::tiny_opus(&one).is_none() || crate::worker::tests::tiny_opus(&two).is_none() {
            eprintln!("no ffmpeg, skipping");
            std::fs::remove_dir_all(&dir).unwrap();
            return;
        }
        let fields = |artist: &str| tag::Fields { artist: artist.into(), title: "T".into(), album: "Al".into(), year: None };
        tag::set_fields(&one, &fields("Stereolab feat. Nurse With Wound")).unwrap();
        tag::set_fields(&two, &fields("Broadcast")).unwrap();
        crate::manifest::hide(&hidden).unwrap();

        assert_eq!(counts(&index(&dir)), [("Stereolab", 1, 1)]);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_second_star_under_another_spelling_unstars_the_same_artist() {
        let dir = root("star");
        assert!(toggle(&dir, "Björk").unwrap());
        assert_eq!(favourites(&dir), [Favourite { name: "Björk".into(), id: None }]);
        assert!(!toggle(&dir, "BJORK").unwrap());
        assert!(favourites(&dir).is_empty());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_resolved_id_survives_a_reload_and_a_later_star() {
        let dir = root("id");
        toggle(&dir, "Can").unwrap();
        set_id(&dir, "can", 3184).unwrap();
        toggle(&dir, "Neu!").unwrap();
        set_id(&dir, "Faust", 1).unwrap();
        assert_eq!(
            favourites(&dir),
            [Favourite { name: "Can".into(), id: Some(3184) }, Favourite { name: "Neu!".into(), id: None }]
        );
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_name_that_would_break_the_file_is_refused() {
        let dir = root("refuse");
        assert!(toggle(&dir, "A\tB").is_err());
        assert!(toggle(&dir, "A\nB").is_err());
        assert!(toggle(&dir, "  ").is_err());
        assert!(!dir.join(FILE).exists());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_name_starting_with_a_hash_or_backslash_round_trips_and_is_not_a_header() {
        let dir = root("hash");
        toggle(&dir, "#1 Dads").unwrap();
        toggle(&dir, "\\Backslash").unwrap();
        assert_eq!(
            favourites(&dir),
            [Favourite { name: "#1 Dads".into(), id: None }, Favourite { name: "\\Backslash".into(), id: None }]
        );
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_header_from_a_later_version_survives_a_star() {
        let dir = root("header");
        std::fs::write(dir.join(FILE), "#checked\t2026-10-01\nCan\t3184\nNeu!\tnot-a-number\n").unwrap();
        assert_eq!(
            favourites(&dir),
            [Favourite { name: "Can".into(), id: Some(3184) }, Favourite { name: "Neu!".into(), id: None }]
        );
        toggle(&dir, "Faust").unwrap();
        let text = std::fs::read_to_string(dir.join(FILE)).unwrap();
        assert!(text.starts_with("#checked\t2026-10-01\n"), "{text}");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    // A playlist folder: two tracks under video ids, as a finished sync leaves them.
    fn synced(dir: &Path, name: &str, tracks: &[(&str, &str, &str)]) -> Option<()> {
        let folder = dir.join(name);
        std::fs::create_dir_all(&folder).unwrap();
        let mut entries = Vec::new();
        for (id, artist, album) in tracks {
            let file = folder.join(format!("{id}.opus"));
            crate::worker::tests::tiny_opus(&file)?;
            let fields = tag::Fields { artist: (*artist).into(), title: "T".into(), album: (*album).into(), year: None };
            tag::set_fields(&file, &fields).unwrap();
            entries.push((id.to_string(), file));
        }
        manifest::write(&folder, "https://www.youtube.com/playlist?list=PL", entries.into_iter()).unwrap();
        Some(())
    }

    #[test]
    fn a_song_copied_into_two_playlists_is_counted_once() {
        let dir = root("twice");
        if synced(&dir, "Gym", &[("aaa", "Can", "Tago Mago"), ("bbb", "Can", "Ege Bamyasi")]).is_none()
            || synced(&dir, "Road", &[("aaa", "Can", "Tago Mago")]).is_none()
        {
            eprintln!("no ffmpeg, skipping");
            std::fs::remove_dir_all(&dir).unwrap();
            return;
        }
        let found = index(&dir);
        assert_eq!(counts(&found), [("Can", 2, 2)]);
        // Counted once, but Road holds it too, and Enter has to find both folders.
        assert_eq!(found[0].folders, [dir.join("Gym"), dir.join("Road")]);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_playlists_own_name_in_the_album_tag_is_not_an_album() {
        let dir = root("named");
        if synced(&dir, "Road Trip", &[("aaa", "Can", "Road Trip"), ("bbb", "Can", "Tago Mago")]).is_none() {
            eprintln!("no ffmpeg, skipping");
            std::fs::remove_dir_all(&dir).unwrap();
            return;
        }
        assert_eq!(counts(&index(&dir)), [("Can", 2, 1)]);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn starred_artists_come_first_and_one_gone_from_the_library_keeps_a_row() {
        let dir = root("rows");
        toggle(&dir, "neu!").unwrap();
        toggle(&dir, "Faust").unwrap();
        let index = grouped([("Can", "Tago Mago"), ("Can", "Ege Bamyasi"), ("Neu!", "Neu!")]);
        let found: Vec<(String, usize, bool)> = rows(&dir, &index).into_iter().map(|r| (r.name, r.tracks, r.starred)).collect();
        assert_eq!(found, [("Neu!".to_string(), 1, true), ("Faust".to_string(), 0, true), ("Can".to_string(), 2, false)]);
        std::fs::remove_dir_all(&dir).unwrap();
    }


    // Read off disk at download time, and only the artist's own tracks on that album.
    #[test]
    fn owned_titles_are_the_artists_tracks_on_that_album() {
        let dir = root("owned");
        let folder = dir.join("Mix");
        std::fs::create_dir_all(&folder).unwrap();
        let tracks = [("01.opus", "Can feat. Damo", "Vitamin C", "Ege Bamyasi"), ("02.opus", "Can", "Spoon", "Spoon"), ("03.opus", "Neu!", "Isi", "Ege Bamyasi")];
        for (file, artist, title, album) in tracks {
            if crate::worker::tests::tiny_opus(&folder.join(file)).is_none() {
                eprintln!("no ffmpeg, skipping");
                std::fs::remove_dir_all(&dir).unwrap();
                return;
            }
            let fields = tag::Fields { artist: artist.into(), title: title.into(), album: album.into(), year: None };
            tag::set_fields(&folder.join(file), &fields).unwrap();
        }
        let index = vec![Artist { name: "Can".into(), tracks: 2, albums: 2, folders: vec![folder], held: Vec::new() }];
        assert_eq!(owned_titles(&index, "Can", "Ege Bamyasi (Remastered)"), ["Vitamin C"]);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn seen_releases_add_up_per_artist_under_any_spelling() {
        let dir = root("seen");
        assert!(seen(&dir).is_empty());
        mark_seen(&dir, "Björk", [3, 1]).unwrap();
        mark_seen(&dir, "bjork", [2]).unwrap();
        mark_seen(&dir, "Can", Vec::new()).unwrap();
        let all = seen(&dir);
        assert_eq!(all[&key("Björk")], HashSet::from([1, 2, 3]));
        assert!(all[&key("Can")].is_empty(), "an artist looked at with nothing out still has a line");
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
