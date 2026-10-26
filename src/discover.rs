use std::collections::HashMap;
use std::path::Path;
use std::sync::mpsc::Sender;
use std::sync::{Arc, Mutex};

use serde_json::Value;

use crate::app::{AlbumRow, Msg};
use crate::artists::{self, Artist};
use crate::lookup::{self, normalise};

// A whole word in a trailing bracket or dash part that makes it the same record: "Tago Mago (40th
// Anniversary Edition)" is Tago Mago.
const EDITION: [&str; 11] =
    ["deluxe", "remaster", "remastered", "edition", "expanded", "anniversary", "bonus", "reissue", "version", "special", "mono"];
// And one that makes it another record whatever else the part says: "(Live Version)" is not the studio album.
const OTHER: [&str; 4] = ["live", "acoustic", "instrumental", "demo"];

/// The Deezer artist a name resolved to.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Found {
    pub id: u64,
    pub name: String,
    pub fans: u64,
}

/// One release in an artist's Deezer discography.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Release {
    pub id: u64,
    pub title: String,
    pub year: Option<u16>,
    /// Deezer's `record_type`: album, ep, single or compile.
    pub kind: String,
}

pub fn artist(name: &str) -> Option<Found> {
    let data = lookup::deezer_get(&format!("search/artist?q={}&limit=10", lookup::encode(name)))?;
    pick(data.get("data")?.as_array()?, name)
}

/// The artist behind an id already resolved, for a starred artist whose id was stored.
pub fn artist_by_id(id: u64) -> Option<Found> {
    found(&lookup::deezer_get(&format!("artist/{id}"))?)
}

// Search returns every act with a similar name, and three bands are called Can. Only an exact name counts
// (on `normalise`), and the most followed of those is the one a library is most likely to hold.
fn pick(hits: &[Value], name: &str) -> Option<Found> {
    let want = normalise(name);
    hits.iter().filter_map(found).filter(|f| normalise(&f.name) == want).max_by_key(|f| f.fans)
}

fn found(hit: &Value) -> Option<Found> {
    Some(Found {
        id: hit.get("id")?.as_u64()?,
        name: hit.get("name")?.as_str()?.to_string(),
        fans: hit.get("nb_fan").and_then(Value::as_u64).unwrap_or(0),
    })
}

pub fn releases(id: u64) -> Option<Vec<Release>> {
    Some(parse_releases(&lookup::deezer_get(&format!("artist/{id}/albums?limit=200"))?))
}

fn parse_releases(data: &Value) -> Vec<Release> {
    let Some(list) = data.get("data").and_then(Value::as_array) else {
        return Vec::new();
    };
    list.iter()
        .filter_map(|r| {
            Some(Release {
                id: r.get("id")?.as_u64()?,
                title: r.get("title")?.as_str()?.to_string(),
                // Deezer dates an unknown release `0000-00-00`.
                year: r.get("release_date").and_then(Value::as_str).and_then(|d| d.get(..4)?.parse().ok()).filter(|y| *y > 0),
                kind: r.get("record_type").and_then(Value::as_str).unwrap_or("album").to_string(),
            })
        })
        .collect()
}

/// How many tracks a release has. Asked only of the ones a library holds, to tell whole from partial.
pub fn track_count(album: u64) -> Option<usize> {
    lookup::deezer_get(&format!("album/{album}"))?.get("nb_tracks")?.as_u64().map(|n| n as usize)
}

/// A title with its edition suffixes taken off, normalised, so reissues of one record compare equal.
pub fn album_key(title: &str) -> String {
    let mut text = title.trim().to_string();
    loop {
        let before = text.len();
        if let Some(open) = trailing_group(&text)
            && is_edition(&text[open..])
        {
            text.truncate(open);
        }
        if let Some(dash) = text.rfind(" - ")
            && is_edition(&text[dash..])
        {
            text.truncate(dash);
        }
        text = text.trim_end().to_string();
        if text.len() == before {
            break;
        }
    }
    normalise(&text)
}

// Where a final `(...)` or `[...]` starts, if the title ends with one.
fn trailing_group(text: &str) -> Option<usize> {
    let open = match text.chars().last()? {
        ')' => '(',
        ']' => '[',
        _ => return None,
    };
    text.rfind(open)
}

fn is_edition(part: &str) -> bool {
    let part = part.to_lowercase();
    let words: Vec<&str> = part.split(|c: char| !c.is_alphanumeric()).collect();
    words.iter().any(|w| EDITION.contains(w)) && !words.iter().any(|w| OTHER.contains(w))
}

/// Marks each release with how many of its tracks the library holds, matched on `album_key` alone.
// Equality and not `lookup::similar`: containment makes a self-titled album "Can" own "Can Delay". `of`
// is left for the caller, which asks Deezer only about the releases held.
pub fn owned(releases: &[Release], held: &[(String, usize)]) -> Vec<AlbumRow> {
    let mut by_key: HashMap<String, usize> = HashMap::new();
    for (title, n) in held {
        *by_key.entry(album_key(title)).or_default() += n;
    }
    releases
        .iter()
        .map(|r| {
            let have = by_key.get(&album_key(&r.title)).copied().unwrap_or(0);
            AlbumRow { id: r.id, title: r.title.clone(), year: r.year, kind: r.kind.clone(), have, of: None }
        })
        .collect()
}

/// Deezer's answers for the session, so going back and forth between pages costs nothing.
// The artist and release lists keep answers only, since a failure there is usually the connection and the
// page is useless without them. Track counts keep failures too: each is one more request on every open.
#[derive(Default)]
pub struct Cache {
    found: HashMap<String, Found>,
    releases: HashMap<u64, Vec<Release>>,
    // Shared with the thread `count_later` starts.
    tracks: Arc<Mutex<HashMap<u64, Option<usize>>>>,
}

/// An artist page: who Deezer took them to be, and their releases against the library. Track counts come
/// from the cache where known; the rest are the releases still to ask about, for `count_later`.
pub fn page(root: &Path, index: &[Artist], name: &str, cache: &mut Cache) -> (Option<(String, u64)>, Result<Vec<AlbumRow>, String>, Vec<u64>) {
    let key = artists::key(name);
    let Some(found) = resolve(root, name, &key, cache) else {
        return (None, Err(format!("Deezer has no artist called {name}, or did not answer  ·  esc goes back")), Vec::new());
    };
    let shown = Some((found.name.clone(), found.fans));
    if !cache.releases.contains_key(&found.id) {
        match releases(found.id) {
            Some(list) => {
                cache.releases.insert(found.id, list);
            }
            None => return (shown, Err(format!("Deezer did not send {name}'s releases  ·  esc goes back")), Vec::new()),
        }
    }
    let held = index.iter().find(|a| artists::key(&a.name) == key).map_or(&[][..], |a| a.held.as_slice());
    let mut rows = owned(&cache.releases[&found.id], held);
    let known = cache.tracks.lock().unwrap();
    let mut pending = Vec::new();
    for row in rows.iter_mut().filter(|r| r.have > 0) {
        match known.get(&row.id) {
            Some(of) => row.of = *of,
            None => pending.push((row.id, !matches!(row.kind.as_str(), "album" | "ep"))),
        }
    }
    // Albums and EPs first: they are what the page opens on.
    pending.sort_by_key(|(_, hidden)| *hidden);
    (shown, Ok(rows), pending.into_iter().map(|(id, _)| id).collect())
}

/// Asks Deezer for each release's length off the worker thread, sending each as it lands.
// A page with a dozen held releases is a dozen paced requests, and holding the worker for them left every
// key dead after Esc. An answer for a page already left is dropped by the UI.
pub fn count_later(name: String, ids: Vec<u64>, cache: &Cache, tx: Sender<Msg>) {
    if ids.is_empty() {
        return;
    }
    let tracks = Arc::clone(&cache.tracks);
    std::thread::spawn(move || {
        for id in ids {
            let of = track_count(id);
            tracks.lock().unwrap().insert(id, of);
            if let Some(of) = of
                && tx.send(Msg::Counted { name: name.clone(), id, of }).is_err()
            {
                return;
            }
        }
    });
}

// A starred artist's stored id wins over a search, and a starred one without an id gets the one found,
// on a cache hit too: they may have been starred after the page was first opened.
fn resolve(root: &Path, name: &str, key: &str, cache: &mut Cache) -> Option<Found> {
    let starred = artists::favourites(root).into_iter().find(|f| artists::key(&f.name) == key);
    let found = match (cache.found.get(key), starred.as_ref().and_then(|f| f.id)) {
        (Some(found), _) => found.clone(),
        (None, Some(id)) => artist_by_id(id)?,
        (None, None) => artist(name)?,
    };
    if let Some(f) = starred.filter(|f| f.id.is_none()) {
        let _ = artists::set_id(root, &f.name, found.id);
    }
    cache.found.insert(key.to_string(), found.clone());
    Some(found)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn release(id: u64, title: &str, kind: &str, date: &str) -> Value {
        json!({ "id": id, "title": title, "record_type": kind, "release_date": date })
    }

    fn held(albums: &[(&str, usize)]) -> Vec<(String, usize)> {
        albums.iter().map(|(t, n)| ((*t).to_string(), *n)).collect()
    }

    fn marks(rows: &[AlbumRow]) -> Vec<(&str, usize)> {
        rows.iter().map(|r| (r.title.as_str(), r.have)).collect()
    }

    #[test]
    fn a_release_list_parses_its_year_and_kind_and_skips_a_broken_entry() {
        let data = json!({ "data": [
            release(1, "Tago Mago", "album", "1971-02-01"),
            release(2, "Spoon", "single", ""),
            { "title": "no id" },
        ]});
        let found = parse_releases(&data);
        assert_eq!(found.len(), 2);
        assert_eq!((found[0].year, found[0].kind.as_str()), (Some(1971), "album"));
        assert_eq!((found[1].year, found[1].kind.as_str()), (None, "single"));
    }

    #[test]
    fn owned_partial_and_missing_are_told_apart_by_the_tracks_held() {
        let data = json!({ "data": [
            release(1, "Tago Mago", "album", "1971-02-01"),
            release(2, "Ege Bamyasi", "album", "1972-11-01"),
            release(3, "Future Days", "album", "1973-08-01"),
        ]});
        let rows = owned(&parse_releases(&data), &held(&[("Tago Mago", 7), ("Ege Bamyasi", 2)]));
        assert_eq!(marks(&rows), [("Tago Mago", 7), ("Ege Bamyasi", 2), ("Future Days", 0)]);
    }

    #[test]
    fn deluxe_and_remaster_suffixes_on_either_side_still_match() {
        let data = json!({ "data": [
            release(1, "Tago Mago (40th Anniversary Edition)", "album", "2011-01-01"),
            release(2, "Ege Bamyasi - 2004 Remaster", "album", "2004-01-01"),
            release(3, "Future Days [Deluxe]", "album", "2020-01-01"),
        ]});
        let rows = owned(
            &parse_releases(&data),
            &held(&[("Tago Mago", 7), ("Ege Bamyasi (Remastered)", 7), ("Future Days (Expanded Edition) [Bonus Tracks]", 4)]),
        );
        assert_eq!(marks(&rows), [("Tago Mago (40th Anniversary Edition)", 7), ("Ege Bamyasi - 2004 Remaster", 7), ("Future Days [Deluxe]", 4)]);
    }

    // A live or acoustic record is a different one, even when its bracket also names an edition.
    #[test]
    fn a_live_album_is_not_the_studio_album() {
        let data = json!({ "data": [
            release(1, "Tago Mago (Live)", "album", "1999-01-01"),
            release(2, "Tago Mago (Live Version)", "album", "1999-01-01"),
            release(3, "Tago Mago - Live Remastered", "album", "1999-01-01"),
            release(4, "Tago Mago (Acoustic Edition)", "album", "1999-01-01"),
        ]});
        let rows = owned(&parse_releases(&data), &held(&[("Tago Mago", 7)]));
        assert!(rows.iter().all(|r| r.have == 0), "{:?}", marks(&rows));
    }

    // Whole words: "mono" is inside "Monolith", which is a title and not an edition.
    #[test]
    fn an_edition_word_inside_another_word_is_not_an_edition() {
        assert_ne!(album_key("Tago Mago (Monolith)"), album_key("Tago Mago"));
        assert_eq!(album_key("Tago Mago (Mono)"), album_key("Tago Mago"));
    }

    #[test]
    fn an_unknown_release_date_is_no_year() {
        let data = json!({ "data": [release(1, "Lost Tapes", "album", "0000-00-00")] });
        assert_eq!(parse_releases(&data)[0].year, None);
    }

    // A starred artist's id is what keeps a later session on the same act, so a cached search must store it too.
    #[test]
    fn an_artist_starred_after_their_page_was_opened_still_gets_their_id() {
        let dir = std::env::temp_dir().join(format!("earworm-discover-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let mut cache = Cache::default();
        cache.found.insert(artists::key("Can"), Found { id: 3184, name: "Can".into(), fans: 1 });
        artists::toggle(&dir, "Can").unwrap();
        assert_eq!(resolve(&dir, "Can", &artists::key("Can"), &mut cache).map(|f| f.id), Some(3184));
        assert_eq!(artists::favourites(&dir)[0].id, Some(3184));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    // Containment would have the self-titled album own every title that starts with the band's name.
    #[test]
    fn a_self_titled_album_matches_only_itself() {
        let data = json!({ "data": [
            release(1, "Can", "album", "1979-01-01"),
            release(2, "Can Delay", "album", "1982-01-01"),
        ]});
        let rows = owned(&parse_releases(&data), &held(&[("Can", 8)]));
        assert_eq!(marks(&rows), [("Can", 8), ("Can Delay", 0)]);
    }

    #[test]
    fn the_artist_is_the_most_followed_exact_name_and_never_a_lookalike() {
        let hits = json!([
            { "id": 1, "name": "Can", "nb_fan": 13 },
            { "id": 2, "name": "CAN", "nb_fan": 57000 },
            { "id": 3, "name": "Cannonball", "nb_fan": 900000 },
        ]);
        assert_eq!(pick(hits.as_array().unwrap(), "can").map(|f| f.id), Some(2));
        assert_eq!(pick(hits.as_array().unwrap(), "Faust"), None);
    }
}
