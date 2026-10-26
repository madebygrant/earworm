// The artists screen (wave 2 of docs/plans/2026-10-26-artist-discovery-plan.md) is the first caller.
#![cfg_attr(not(test), allow(dead_code))]

use std::collections::{HashMap, HashSet};
use std::path::Path;

use anyhow::{Result, bail};

use crate::lookup::{lead_artist, normalise};
use crate::manifest::{self, Kind};
use crate::{tag, worker};

// Under `--dir` beside `.playlists`, not in the config: the config is written only by `^t` and the format keys.
const FILE: &str = ".artists";

/// One artist in the library, as the tags on disk name them.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Artist {
    pub name: String,
    pub tracks: usize,
    pub albums: usize,
}

/// What two spellings of one artist share, so "Björk" and "bjork" are one row and one star.
// A name of punctuation alone ("!!!") normalises to nothing, and would otherwise match every other one.
pub fn key(name: &str) -> String {
    let key = normalise(name);
    if key.is_empty() { name.trim().to_lowercase() } else { key }
}

/// Groups (artist, album) pairs by lead artist, most tracks first, shown under the commonest spelling.
// "A feat. B" counts towards A only: a guest verse is not one of B's albums.
pub fn group<'a>(tracks: impl IntoIterator<Item = (&'a str, &'a str)>) -> Vec<Artist> {
    struct Seen {
        key: String,
        spellings: Vec<(String, usize)>,
        tracks: usize,
        albums: HashSet<String>,
    }
    let mut seen: Vec<Seen> = Vec::new();
    let mut at: HashMap<String, usize> = HashMap::new();
    for (artist, album) in tracks {
        let lead = lead_artist(artist);
        if lead.is_empty() {
            continue;
        }
        let k = key(&lead);
        let i = *at.entry(k.clone()).or_insert_with(|| {
            seen.push(Seen { key: k, spellings: Vec::new(), tracks: 0, albums: HashSet::new() });
            seen.len() - 1
        });
        let one = &mut seen[i];
        one.tracks += 1;
        match one.spellings.iter_mut().find(|(s, _)| *s == lead) {
            Some((_, n)) => *n += 1,
            None => one.spellings.push((lead, 1)),
        }
        let album = normalise(album);
        if !album.is_empty() {
            one.albums.insert(album);
        }
    }
    seen.sort_by(|a, b| b.tracks.cmp(&a.tracks).then_with(|| a.key.cmp(&b.key)));
    seen.into_iter()
        .map(|one| {
            // `rev` so a tie goes to the spelling seen first: `max_by_key` keeps the last of equals.
            let name = one.spellings.iter().rev().max_by_key(|(_, n)| *n).map(|(s, _)| s.clone()).unwrap_or_default();
            Artist { name, tracks: one.tracks, albums: one.albums.len() }
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
            if let Some(id) = ids.get(path.as_path()).filter(|id| !id.starts_with('~')) {
                let recording = [Some(id.to_string()), isrcs.get(id).map(|i| format!("isrc {i}"))];
                if recording.iter().flatten().any(|r| counted.contains(r)) {
                    continue;
                }
                counted.extend(recording.into_iter().flatten());
            }
            if let Ok((artist, mut album)) = tag::artist_album(&path) {
                if named_album.as_deref() == Some(normalise(&album).as_str()) {
                    album.clear();
                }
                read.push((artist, album));
            }
        }
    }
    group(read.iter().map(|(artist, album)| (artist.as_str(), album.as_str())))
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

/// Stars an artist. False when they already were, under any spelling `group` would merge.
pub fn star(root: &Path, name: &str) -> Result<bool> {
    let name = name.trim();
    if name.is_empty() || name.contains(['\t', '\n', '\r']) {
        bail!("cannot star an artist called {name:?}");
    }
    let mut store = load(root);
    if position(&store.list, name).is_some() {
        return Ok(false);
    }
    store.list.push(Favourite { name: name.to_string(), id: None });
    save(root, &store)?;
    Ok(true)
}

/// Unstars an artist. False when they were not starred.
pub fn unstar(root: &Path, name: &str) -> Result<bool> {
    let mut store = load(root);
    let Some(at) = position(&store.list, name) else {
        return Ok(false);
    };
    store.list.remove(at);
    save(root, &store)?;
    Ok(true)
}

/// Records the Deezer id a starred artist resolved to. Nothing happens for one not starred.
pub fn set_id(root: &Path, name: &str, id: u64) -> Result<()> {
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

    fn counts(found: &[Artist]) -> Vec<(&str, usize, usize)> {
        found.iter().map(|a| (a.name.as_str(), a.tracks, a.albums)).collect()
    }

    #[test]
    fn a_featured_credit_counts_towards_the_lead_artist_only() {
        let found = group([
            ("Massive Attack", "Mezzanine"),
            ("Massive Attack feat. Elizabeth Fraser", "Mezzanine"),
            ("Massive Attack (ft. Tracey Thorn)", "Protection"),
            ("Elizabeth Fraser", "Moses"),
        ]);
        assert_eq!(counts(&found), [("Massive Attack", 3, 2), ("Elizabeth Fraser", 1, 1)]);
    }

    #[test]
    fn a_band_name_with_an_ampersand_slash_comma_or_with_is_kept_whole() {
        let found = group([
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
        let found = group([("!!!", "Myth Takes"), ("+/-", "Xs on Your Eyes")]);
        assert_eq!(found.len(), 2);
        let dir = root("punct");
        assert!(star(&dir, "!!!").unwrap());
        assert!(star(&dir, "+/-").unwrap());
        assert!(!unstar(&dir, "???").unwrap());
        assert_eq!(favourites(&dir).len(), 2);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn case_and_accents_do_not_split_an_artist_and_the_usual_spelling_is_shown() {
        let found = group([("bjork", "Post"), ("Björk", "Post"), ("BJÖRK", "Homogenic"), ("Björk", "Debut")]);
        assert_eq!(counts(&found), [("Björk", 4, 3)]);
    }

    #[test]
    fn a_tie_between_spellings_goes_to_the_first_seen() {
        let found = group([("Sigur Rós", "()"), ("Sigur Ros", "()")]);
        assert_eq!(found[0].name, "Sigur Rós");
    }

    #[test]
    fn an_untagged_track_has_no_artist_and_a_missing_album_is_not_an_album() {
        let found = group([("", "Loose"), ("Low", ""), ("Low", "Things We Lost in the Fire")]);
        assert_eq!(counts(&found), [("Low", 2, 1)]);
    }

    #[test]
    fn most_tracks_first_then_by_name() {
        let found = group([("Can", "Tago Mago"), ("Neu!", "Neu!"), ("Can", "Ege Bamyasi"), ("Faust", "IV")]);
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
    fn a_star_is_kept_once_under_any_spelling_and_an_unstar_removes_it() {
        let dir = root("star");
        assert!(star(&dir, "Björk").unwrap());
        assert!(!star(&dir, "bjork").unwrap());
        assert_eq!(favourites(&dir), [Favourite { name: "Björk".into(), id: None }]);
        assert!(unstar(&dir, "BJORK").unwrap());
        assert!(!unstar(&dir, "Björk").unwrap());
        assert!(favourites(&dir).is_empty());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_resolved_id_survives_a_reload_and_a_later_star() {
        let dir = root("id");
        star(&dir, "Can").unwrap();
        set_id(&dir, "can", 3184).unwrap();
        star(&dir, "Neu!").unwrap();
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
        assert!(star(&dir, "A\tB").is_err());
        assert!(star(&dir, "A\nB").is_err());
        assert!(star(&dir, "  ").is_err());
        assert!(!dir.join(FILE).exists());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_name_starting_with_a_hash_or_backslash_round_trips_and_is_not_a_header() {
        let dir = root("hash");
        star(&dir, "#1 Dads").unwrap();
        star(&dir, "\\Backslash").unwrap();
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
        star(&dir, "Faust").unwrap();
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
        assert_eq!(counts(&index(&dir)), [("Can", 2, 2)]);
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
}
