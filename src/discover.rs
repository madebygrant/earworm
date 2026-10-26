use std::collections::HashMap;
use std::path::Path;
use std::sync::mpsc::Sender;
use std::sync::{Arc, Mutex};

use serde_json::Value;

use crate::app::{AlbumRow, Msg, RelatedRow, TopRow};
use crate::artists::{self, Artist};
use crate::lookup::{self, normalise};

// A whole word in a trailing bracket or dash part that makes it the same record: "Tago Mago (40th
// Anniversary Edition)" is Tago Mago.
const EDITION: [&str; 11] =
    ["deluxe", "remaster", "remastered", "edition", "expanded", "anniversary", "bonus", "reissue", "version", "special", "mono"];
const MASTERING: [&str; 4] = ["remaster", "remastered", "mono", "stereo"];
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
    /// `YYYY-MM-DD` as Deezer sends it, which compares as a string. Empty when unknown.
    pub date: String,
    /// Deezer's `record_type`: album, ep, single or compile.
    pub kind: String,
}

/// One of an artist's most played tracks on Deezer.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Top {
    pub title: String,
    pub album: String,
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
    let want = artists::key(name);
    hits.iter().filter_map(found).filter(|f| artists::key(&f.name) == want).max_by_key(|f| f.fans)
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
                date: r.get("release_date").and_then(Value::as_str).filter(|d| !d.starts_with("0000")).unwrap_or("").to_string(),
                kind: r.get("record_type").and_then(Value::as_str).unwrap_or("album").to_string(),
            })
        })
        .collect()
}

/// Deezer's artists like this one.
pub fn related(id: u64) -> Option<Vec<Found>> {
    let data = lookup::deezer_get(&format!("artist/{id}/related?limit=20"))?;
    Some(data.get("data")?.as_array()?.iter().filter_map(found).collect())
}

pub fn top(id: u64) -> Option<Vec<Top>> {
    let data = lookup::deezer_get(&format!("artist/{id}/top?limit=10"))?;
    Some(parse_top(&data))
}

fn parse_top(data: &Value) -> Vec<Top> {
    let Some(list) = data.get("data").and_then(Value::as_array) else {
        return Vec::new();
    };
    list.iter()
        .filter_map(|t| {
            Some(Top {
                title: t.get("title")?.as_str()?.to_string(),
                album: t.get("album").and_then(|a| a.get("title")).and_then(Value::as_str).unwrap_or("").to_string(),
            })
        })
        .collect()
}

/// Releases not seen before and already out, newest first. `None` is an artist never looked at, whose whole
/// discography is not news.
// By id and not by date: Deezer adds a release days after its date, which a "since" day would never see,
// and a pre-order dated next month would be new on every launch until then.
pub fn fresh<'a>(releases: &'a [Release], known: Option<&std::collections::HashSet<u64>>, today: &str) -> Vec<&'a Release> {
    let Some(known) = known else {
        return Vec::new();
    };
    let mut found: Vec<&Release> = releases.iter().filter(|r| !known.contains(&r.id) && out(r, today)).collect();
    found.sort_by(|a, b| b.date.cmp(&a.date));
    found
}

// Undated counts as out: there is no date to wait for.
fn out(release: &Release, today: &str) -> bool {
    release.date.is_empty() || release.date.as_str() <= today
}

/// Today as `YYYY-MM-DD` in UTC, the form Deezer dates releases in.
pub fn today() -> String {
    let secs = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs());
    day_of(secs / 86_400)
}

// Days since 1970-01-01 to a civil date, by Howard Hinnant's `civil_from_days`.
fn day_of(days: u64) -> String {
    let z = days as i64 + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    format!("{y:04}-{m:02}-{d:02}")
}

/// How many tracks a release has. Asked only of the ones a library holds, to tell whole from partial.
pub fn track_count(album: u64) -> Option<usize> {
    lookup::deezer_get(&format!("album/{album}"))?.get("nb_tracks")?.as_u64().map(|n| n as usize)
}

/// A title with its edition suffixes taken off, normalised, so reissues of one record compare equal.
pub fn album_key(title: &str) -> String {
    stripped(title, &EDITION)
}

/// A song title with only its mastering suffixes off: "Paint It, Black (Mono)" is the song, where
/// "(Extended Version)" is another recording and keeps its words.
pub fn song_key(title: &str) -> String {
    stripped(crate::worker::strip_number(title.trim()), &MASTERING)
}

fn stripped(title: &str, words: &[&str]) -> String {
    let mut text = title.trim().to_string();
    loop {
        let before = text.len();
        if let Some(open) = trailing_group(&text)
            && is_edition(&text[open..], words)
        {
            text.truncate(open);
        }
        if let Some(dash) = text.rfind(" - ")
            && is_edition(&text[dash..], words)
        {
            text.truncate(dash);
        }
        text = text.trim_end().to_string();
        if text.len() == before {
            break;
        }
    }
    // A title of punctuation alone ("()") keeps one, as an artist's name does in `artists::key`.
    let key = normalise(&text);
    if key.is_empty() { text.trim().to_lowercase() } else { key }
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

fn is_edition(part: &str, edition: &[&str]) -> bool {
    let part = part.to_lowercase();
    let words: Vec<&str> = part.split(|c: char| !c.is_alphanumeric()).collect();
    words.iter().any(|w| edition.contains(w)) && !words.iter().any(|w| OTHER.contains(w))
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
            AlbumRow { id: r.id, title: r.title.clone(), year: r.year, kind: r.kind.clone(), have, of: None, new: false }
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
    top: HashMap<u64, Vec<Top>>,
    related: HashMap<u64, Vec<Found>>,
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
    // Opening a starred artist's page is reading what is new there, so it stops being new.
    if artists::favourites(root).iter().any(|f| artists::key(&f.name) == key) {
        let today = today();
        let ids = cache.releases[&found.id].iter().filter(|r| out(r, &today)).map(|r| r.id);
        let _ = artists::mark_seen(root, name, ids.collect::<Vec<_>>());
    }
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

/// The artist's top tracks, each marked by whether a track of that title by them is here.
pub fn top_rows(root: &Path, index: &[Artist], name: &str, cache: &mut Cache) -> Result<Vec<TopRow>, String> {
    let found = resolve(root, name, &artists::key(name), cache).ok_or_else(|| format!("Deezer has no artist called {name}, or did not answer"))?;
    if !cache.top.contains_key(&found.id) {
        let list = top(found.id).ok_or("Deezer did not send the top tracks  ·  tab again tries again")?;
        cache.top.insert(found.id, list);
    }
    let here: std::collections::HashSet<String> = artists::titles_of(index, name).iter().map(|t| song_key(t)).collect();
    Ok(cache.top[&found.id]
        .iter()
        .map(|t| TopRow { title: t.title.clone(), album: t.album.clone(), have: here.contains(&song_key(&t.title)) })
        .collect())
}

/// Deezer's related artists, with how many of their tracks the library holds.
pub fn related_rows(root: &Path, index: &[Artist], name: &str, cache: &mut Cache) -> Result<Vec<RelatedRow>, String> {
    let found = resolve(root, name, &artists::key(name), cache).ok_or_else(|| format!("Deezer has no artist called {name}, or did not answer"))?;
    if !cache.related.contains_key(&found.id) {
        let list = related(found.id).ok_or("Deezer did not send related artists  ·  tab again tries again")?;
        cache.related.insert(found.id, list);
    }
    // Opened by name, so the id Deezer already gave is what that name resolves to: a search for it could
    // land on a bigger act of the same name.
    for f in &cache.related[&found.id] {
        cache.found.entry(artists::key(&f.name)).or_insert_with(|| f.clone());
    }
    Ok(cache.related[&found.id]
        .iter()
        .map(|f| {
            let tracks = index.iter().find(|a| artists::key(&a.name) == artists::key(&f.name)).map_or(0, |a| a.tracks);
            RelatedRow { name: f.name.clone(), fans: f.fans, tracks }
        })
        .collect())
}

// Between artists, so a page opened during the check gets its requests in rather than queueing behind all of it.
const CHECK_PAUSE: std::time::Duration = std::time::Duration::from_millis(500);

/// Looks for starred artists' releases not seen before, off the worker thread, and says what it found.
// Every starred artist, however many. An artist's first look only records what exists. One that does not
// answer keeps what it had and is looked at again next launch.
pub fn check_new(root: std::path::PathBuf, tx: Sender<Msg>) {
    std::thread::spawn(move || {
        let favourites = artists::favourites(&root);
        if favourites.is_empty() {
            return;
        }
        let today = today();
        let known = artists::seen(&root);
        let mut found = Vec::new();
        for f in favourites {
            let id = match f.id {
                Some(id) => Some(id),
                None => artist(&f.name).inspect(|a| {
                    let _ = artists::set_id(&root, &f.name, a.id);
                }).map(|a| a.id),
            };
            let Some(list) = id.and_then(releases) else {
                std::thread::sleep(CHECK_PAUSE);
                continue;
            };
            let had = known.get(&artists::key(&f.name));
            found.extend(fresh(&list, had, &today).into_iter().map(|r| (f.name.clone(), r.id, r.title.clone())));
            if had.is_none() {
                let _ = artists::mark_seen(&root, &f.name, list.iter().filter(|r| out(r, &today)).map(|r| r.id).collect::<Vec<_>>());
            }
            std::thread::sleep(CHECK_PAUSE);
        }
        if !found.is_empty() {
            let _ = tx.send(Msg::NewReleases(found));
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

    // Late additions are found by id whatever their date, and a pre-order waits for its day.
    #[test]
    fn a_new_release_is_one_not_seen_and_already_out() {
        let data = json!({ "data": [
            release(1, "Old", "album", "2026-01-01"),
            release(2, "Added Late", "single", "2026-09-01"),
            release(3, "Newer", "album", "2026-10-20"),
            release(4, "Pre-order", "album", "2026-12-01"),
            release(5, "Undated", "ep", "0000-00-00"),
        ]});
        let releases = parse_releases(&data);
        let known = std::collections::HashSet::from([1]);
        let found: Vec<&str> = fresh(&releases, Some(&known), "2026-10-26").iter().map(|r| r.title.as_str()).collect();
        assert_eq!(found, ["Newer", "Added Late", "Undated"]);
        assert!(fresh(&releases, None, "2026-10-26").is_empty(), "a first look reported the whole discography");
    }

    #[test]
    fn a_song_key_drops_mastering_and_keeps_other_versions() {
        assert_eq!(song_key("Paint It, Black (Mono)"), song_key("Paint It, Black"));
        assert_eq!(song_key("03 - Paint It, Black - 2002 Remaster"), song_key("Paint It, Black"));
        assert_ne!(song_key("Paint It, Black (Extended Version)"), song_key("Paint It, Black"));
        assert_ne!(song_key("Paint It, Black (Live)"), song_key("Paint It, Black"));
    }

    #[test]
    fn days_since_the_epoch_become_the_date_deezer_writes() {
        assert_eq!(day_of(0), "1970-01-01");
        assert_eq!(day_of(19_782), "2024-02-29");
        assert_eq!(day_of(20_752), "2026-10-26");
    }

    #[test]
    fn top_tracks_keep_their_album_and_skip_a_broken_entry() {
        let data = json!({ "data": [
            { "title": "Yellow", "album": { "title": "Parachutes" } },
            { "title": "Clocks" },
            { "album": { "title": "no title" } },
        ]});
        let found = parse_top(&data);
        assert_eq!(found, [Top { title: "Yellow".into(), album: "Parachutes".into() }, Top { title: "Clocks".into(), album: String::new() }]);
    }

    fn wait_for(done: impl Fn() -> bool) -> bool {
        (0..40).any(|_| {
            std::thread::sleep(std::time::Duration::from_millis(50));
            done()
        })
    }

    // Nothing starred is nothing to look for, and no file appears in a folder that never starred anyone.
    #[test]
    fn the_startup_check_writes_nothing_with_nobody_starred() {
        let dir = std::env::temp_dir().join(format!("earworm-checknew-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let (tx, _rx) = std::sync::mpsc::channel();
        check_new(dir.clone(), tx);
        assert!(!wait_for(|| std::fs::read_dir(&dir).unwrap().next().is_some()), "a check with nobody starred wrote a file");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    fn cached(dir: &Path) -> Cache {
        let mut cache = Cache::default();
        cache.found.insert(artists::key("Can"), Found { id: 3184, name: "Can".into(), fans: 9 });
        let out = Release { id: 1, title: "Tago Mago".into(), year: Some(1971), date: "1971-02-01".into(), kind: "album".into() };
        let coming = Release { id: 2, title: "Next".into(), year: Some(2999), date: "2999-01-01".into(), kind: "album".into() };
        cache.releases.insert(3184, vec![out, coming]);
        cache.related.insert(3184, vec![Found { id: 77, name: "Nirvana".into(), fans: 3 }]);
        let _ = dir;
        cache
    }

    // Reading a starred artist's page is what stops their releases being new; a pre-order stays to come.
    #[test]
    fn opening_a_starred_artists_page_marks_what_is_out_as_seen() {
        let dir = std::env::temp_dir().join(format!("earworm-pageseen-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let mut cache = cached(&dir);
        let (_, rows, _) = page(&dir, &[], "Can", &mut cache);
        assert_eq!(rows.unwrap().len(), 2);
        assert!(artists::seen(&dir).is_empty(), "an artist nobody starred was recorded");
        artists::toggle(&dir, "Can").unwrap();
        let _ = page(&dir, &[], "Can", &mut cache);
        assert_eq!(artists::seen(&dir)[&artists::key("Can")], std::collections::HashSet::from([1]));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    // A related artist opens on the id Deezer gave, not on whoever a search for the name finds first.
    #[test]
    fn a_related_artist_resolves_to_the_id_it_came_with() {
        let dir = std::env::temp_dir().join(format!("earworm-relatedid-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let mut cache = cached(&dir);
        let rows = related_rows(&dir, &[], "Can", &mut cache).unwrap();
        assert_eq!(rows[0].name, "Nirvana");
        assert_eq!(resolve(&dir, "Nirvana", &artists::key("Nirvana"), &mut cache).map(|f| f.id), Some(77));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    // Sigur Rós's "()" normalised to nothing, so it was never held and matched every untagged album.
    #[test]
    fn an_album_titled_in_punctuation_keeps_a_key_of_its_own() {
        assert_eq!(album_key("()"), "()");
        assert_ne!(album_key("()"), album_key(""));
        let data = json!({ "data": [release(1, "()", "album", "2002-10-28")] });
        assert_eq!(marks(&owned(&parse_releases(&data), &held(&[("()", 8)]))), [("()", 8)]);
    }
}
