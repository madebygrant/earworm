//! The list rules both front ends share: what each narrowing keeps, the library's order, and which tracks
//! a marking key takes. Pure functions over the state a front end holds, so the terminal and the desktop
//! app answer every list question the same way. Cursors, scrolling and drawing stay in each front end.

use std::borrow::Cow;
use std::collections::HashSet;
use std::path::PathBuf;

use crate::manifest::Kind;
use crate::picker::{self, Files, Hit};
use crate::protocol::{AlbumRow, ArtistRow, Shelf, Status, Track};

/// The track list's two narrowings: the filter box and the review. Review is a predicate rather than a
/// filter string, because the four statuses it keeps share no word for the filter to match on.
pub struct Narrow<'a> {
    // Lowercased once here, not once per row, and borrowed when there is nothing to lowercase: a front end
    // builds one for every row it asks about.
    needle: Cow<'a, str>,
    review: bool,
}

impl<'a> Narrow<'a> {
    pub fn new(filter: &'a str, review: bool) -> Narrow<'a> {
        let needle = if filter.chars().any(char::is_uppercase) { Cow::Owned(filter.to_lowercase()) } else { Cow::Borrowed(filter) };
        Narrow { needle, review }
    }

    pub fn narrows(&self) -> bool {
        !self.needle.is_empty() || self.review
    }

    /// Whether a row survives, matched against the name and the status word: the artist or title you are
    /// looking for, or `/failed`.
    pub fn keeps(&self, track: &Track) -> bool {
        (!self.review || track.status.wants_a_look())
            && (self.needle.is_empty()
                || track.name.to_lowercase().contains(&*self.needle)
                || track.status.label().contains(&*self.needle))
    }
}

/// Positions in `tracks` the list is showing, in order.
pub fn rows(tracks: &[Track], narrow: &Narrow) -> Vec<usize> {
    if !narrow.narrows() {
        return (0..tracks.len()).collect();
    }
    tracks.iter().enumerate().filter(|(_, t)| narrow.keeps(t)).map(|(pos, _)| pos).collect()
}

/// How many rows are showing, counted rather than collected.
pub fn shown(tracks: &[Track], narrow: &Narrow) -> usize {
    if !narrow.narrows() {
        return tracks.len();
    }
    tracks.iter().filter(|t| narrow.keeps(t)).count()
}

/// Library order. Name is what the worker hands over, so it is the one that
/// costs nothing; the other two answer "which of these needs a sync".
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum Sort {
    #[default]
    Name,
    Synced,
    Missing,
}

impl Sort {
    pub fn next(self) -> Sort {
        match self {
            Sort::Name => Sort::Synced,
            Sort::Synced => Sort::Missing,
            Sort::Missing => Sort::Name,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Sort::Name => "name",
            Sort::Synced => "last synced",
            Sort::Missing => "most missing",
        }
    }
}

// A folder with no kind is a playlist, since silence is the playlist answer everywhere else.
pub fn kind_shows(only: Option<Kind>, shelf: &Shelf) -> bool {
    match only {
        None => true,
        Some(Kind::Album) => shelf.kind == Some(Kind::Album),
        Some(Kind::Playlist) => !matches!(shelf.kind, Some(Kind::Album | Kind::Custom)),
        Some(Kind::Custom) => shelf.kind == Some(Kind::Custom),
    }
}

/// Every folder, then albums, then playlists, then custom playlists. Albums come first because they are
/// the ones the pill already points at.
pub fn next_kind(only: Option<Kind>) -> Option<Kind> {
    match only {
        None => Some(Kind::Album),
        Some(Kind::Album) => Some(Kind::Playlist),
        Some(Kind::Playlist) => Some(Kind::Custom),
        Some(Kind::Custom) => None,
    }
}

pub fn kind_label(only: Option<Kind>) -> &'static str {
    match only {
        None => "every folder",
        Some(Kind::Album) => "albums",
        Some(Kind::Playlist) => "playlists",
        Some(Kind::Custom) => "custom playlists",
    }
}

// A folder matches on its name or a track inside it; `Shelf.files` is already in memory, so this costs
// no disk. The filenames are lowercased per call rather than kept twice, the cheaper of the two.
pub fn shelf_matches(shelf: &Shelf, needle: &str) -> bool {
    shelf.name.to_lowercase().contains(needle)
        || shelf.files.iter().any(|(name, _)| name.to_lowercase().contains(needle))
}

/// The library's narrowings: the filter box, the `a` kind and the artist the artists screen opened.
#[derive(Clone, Copy, Default)]
pub struct ShelfNarrow<'a> {
    pub filter: &'a str,
    pub only: Option<Kind>,
    /// Folders rather than a query, because filenames are rewritten by `clean` and a substring of a name
    /// is in other names.
    pub folders: Option<&'a [PathBuf]>,
}

impl ShelfNarrow<'_> {
    pub fn narrows(&self) -> bool {
        !self.filter.is_empty() || self.only.is_some() || self.folders.is_some()
    }
}

/// Library rows in the order and the narrowing the screen is showing. Positions in `library`, never a
/// sort of it, so a front end's cursor keeps meaning one folder however the order changes under it.
pub fn shelf_rows(library: &[Shelf], narrow: &ShelfNarrow, sort: Sort) -> Vec<usize> {
    let needle = narrow.filter.to_lowercase();
    let mut rows: Vec<usize> = library
        .iter()
        .enumerate()
        .filter(|(_, s)| {
            kind_shows(narrow.only, s)
                && narrow.folders.is_none_or(|folders| folders.contains(&s.path))
                && (needle.is_empty() || shelf_matches(s, &needle))
        })
        .map(|(pos, _)| pos)
        .collect();
    match sort {
        // Already what the worker built, which is name order.
        Sort::Name => {}
        // Never synced sorts first: it is the folder furthest from matching its playlist upstream.
        // A list is built here and syncs from nothing, so it takes no part in either: it goes last by
        // time, and its dead entries are not something a sync would fetch.
        Sort::Synced => rows.sort_by_key(|pos| {
            let s = &library[*pos];
            (s.kind == Some(Kind::Custom), s.synced.unwrap_or(0))
        }),
        Sort::Missing => rows.sort_by_key(|pos| {
            let s = &library[*pos];
            std::cmp::Reverse(if s.kind == Some(Kind::Custom) { 0 } else { s.missing })
        }),
    }
    rows
}

// Ranked by the picker's matcher and narrowed by the kind, so `t` answers the query the picker and the
// rows do. An empty query finds nothing: a search with nothing typed would be the library screen again.
pub fn found_hits(library: &[Shelf], query: &str, only: Option<Kind>) -> Vec<Hit> {
    if query.trim().is_empty() {
        return Vec::new();
    }
    picker::find(library, query, |s| kind_shows(only, s), Files::All)
}

/// How many tracks a search under this kind looks through, for the band's "of". The folders `found_hits`
/// searches, so a custom playlist's tracks are not counted a second time.
pub fn found_total(library: &[Shelf], only: Option<Kind>) -> usize {
    library.iter().filter(|s| s.kind != Some(Kind::Custom) && kind_shows(only, s)).map(|s| s.files.len()).sum()
}

// On `artists::key`, so the query merges spellings the way the rows were merged: `bjork` finds "Björk".
pub fn artist_keeps(query: &str, row: &ArtistRow) -> bool {
    keeps_artist(&crate::artists::key(query), row)
}

/// Positions in `rows` the artist query keeps, in order.
pub fn artist_rows(rows: &[ArtistRow], query: &str) -> Vec<usize> {
    let key = crate::artists::key(query);
    (0..rows.len()).filter(|at| keeps_artist(&key, &rows[*at])).collect()
}

fn keeps_artist(key: &str, row: &ArtistRow) -> bool {
    key.is_empty() || crate::artists::key(&row.name).contains(key)
}

// Albums and EPs unless `all_kinds`: a band's singles and compilations outnumber its records.
pub fn release_shows(all_kinds: bool, row: &AlbumRow) -> bool {
    all_kinds || matches!(row.kind.as_str(), "album" | "ep")
}

/// Track indices of every row the narrowing shows, and only those: what `a` and `i` act on, so the two
/// keys mean the same amount of list.
pub fn showing(tracks: &[Track], narrow: &Narrow) -> Vec<usize> {
    tracks.iter().filter(|t| narrow.keeps(t)).map(|t| t.index).collect()
}

// The shown tracks sharing a status: a playlist gets `kept` or `weak` wrong in batches. Only what is
// on screen, because `/` then `m` is how you mark one artist's bad rows.
pub fn like_status(tracks: &[Track], narrow: &Narrow, status: Status) -> Vec<usize> {
    tracks.iter().filter(|t| t.status == status && narrow.keeps(t)).map(|t| t.index).collect()
}

/// The artist a track is grouped under, or `None` before the tag pass has given it one.
// Case is a tagging accident, not a different artist.
pub fn artist_of(track: &Track) -> Option<String> {
    Some(track.artist.trim().to_lowercase()).filter(|a| !a.is_empty())
}

/// The shown tracks by the artist `artist_of` named.
pub fn like_artist(tracks: &[Track], narrow: &Narrow, artist: &str) -> Vec<usize> {
    tracks.iter().filter(|t| artist_of(t).as_deref() == Some(artist) && narrow.keeps(t)).map(|t| t.index).collect()
}

/// Marks the group, or takes it back off when it is all marked already, so the key that grabbed a batch
/// can give it back.
pub fn flip(marked: &mut HashSet<usize>, group: &[usize]) {
    if group.iter().all(|i| marked.contains(i)) {
        marked.retain(|i| !group.contains(i));
    } else {
        marked.extend(group);
    }
}

/// Marked becomes unmarked and the other way round, over the tracks given.
pub fn invert(marked: &mut HashSet<usize>, tracks: &[usize]) {
    for index in tracks {
        if !marked.remove(index) {
            marked.insert(*index);
        }
    }
}

/// What the pick gate answers: the marks and nothing else, in track order. Never a fallback to the
/// cursor, which would turn "I unmarked everything" into "download the row I happen to be on".
pub fn picked(marked: &HashSet<usize>) -> Vec<usize> {
    let mut picked: Vec<usize> = marked.iter().copied().collect();
    picked.sort_unstable();
    picked
}

/// What the pick gate opens with: everything it could fetch, so Enter is the whole playlist and the work
/// is unmarking. `held` is what the run asked to leave unmarked.
// Nothing on disk, departed, or a file the playlist never had: the gate has nothing to ask about those.
pub fn gate_marks(tracks: &[Track], held: &[usize]) -> HashSet<usize> {
    tracks
        .iter()
        .filter(|t| !matches!(t.status, Status::Have | Status::Gone | Status::Local))
        .filter(|t| !held.contains(&t.index))
        .map(|t| t.index)
        .collect()
}

/// Positions in an artist page's releases that `release_shows` keeps, in order.
pub fn release_rows(rows: &[AlbumRow], all_kinds: bool) -> Vec<usize> {
    (0..rows.len()).filter(|at| release_shows(all_kinds, &rows[*at])).collect()
}

/// The tally beside the header, in `Status::tally` order. Built from the tracks rather than a hand-kept
/// list of statuses, so a new status is given a column or left out by `tally`'s exhaustive match.
pub fn counts(tracks: &[Track]) -> Vec<(Status, usize)> {
    let mut seen: Vec<(u8, Status, usize)> = Vec::new();
    for track in tracks {
        let Some(rank) = track.status.tally() else {
            continue;
        };
        match seen.iter_mut().find(|(_, s, _)| *s == track.status) {
            Some((_, _, n)) => *n += 1,
            None => seen.push((rank, track.status, 1)),
        }
    }
    seen.sort_unstable_by_key(|(rank, _, _)| *rank);
    seen.into_iter().map(|(_, s, n)| (s, n)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn named(names: &[&str]) -> Vec<Track> {
        names
            .iter()
            .enumerate()
            .map(|(i, name)| {
                let mut track = Track::new(i + 1, "id".into(), (*name).into(), PathBuf::from("/tmp/x.opus"));
                track.status = if i % 2 == 0 { Status::Ok } else { Status::Failed };
                track
            })
            .collect()
    }

    fn with(statuses: &[Status]) -> Vec<Track> {
        let mut tracks = named(&vec!["name"; statuses.len()]);
        for (track, status) in tracks.iter_mut().zip(statuses) {
            track.status = *status;
        }
        tracks
    }

    fn folder(name: &str, missing: usize, synced: Option<u64>) -> Shelf {
        Shelf {
            path: PathBuf::from("/tmp").join(name),
            name: name.into(),
            url: Some("u".into()),
            tracks: 10,
            missing,
            synced,
            ..Shelf::default()
        }
    }

    fn stocked(name: &str, files: &[&str]) -> Shelf {
        Shelf {
            files: files.iter().map(|f| ((*f).to_string(), true)).collect(),
            tracks: files.len(),
            ..folder(name, 0, None)
        }
    }

    fn found(library: &[Shelf], query: &str, only: Option<Kind>) -> Vec<(usize, usize)> {
        found_hits(library, query, only).iter().map(|h| (h.shelf, h.file)).collect()
    }

    fn by_name(library: &[Shelf], query: &str) -> Vec<usize> {
        shelf_rows(library, &ShelfNarrow { filter: query, ..ShelfNarrow::default() }, Sort::Name)
    }

    #[test]
    fn a_filter_matches_the_status_word_as_well_as_the_name() {
        let tracks = named(&["One", "Two", "Three"]);
        assert_eq!(rows(&tracks, &Narrow::new("failed", false)), [1]);
        assert!(rows(&tracks, &Narrow::new("APHEX", false)).is_empty());
    }

    // The review statuses share no word to filter on, so review is a predicate beside the filter.
    #[test]
    fn review_keeps_only_the_unconfirmed_and_composes_with_the_filter() {
        let mut tracks = with(&[Status::Ok, Status::Kept, Status::Failed, Status::Have]);
        tracks[1].name = "Neu! - Hallogallo".into();
        tracks[2].name = "Can - Vitamin C".into();
        assert_eq!(rows(&tracks, &Narrow::new("", true)), vec![1, 2]);
        assert_eq!(shown(&tracks, &Narrow::new("", true)), 2);
        assert_eq!(rows(&tracks, &Narrow::new("neu", true)), vec![1], "the filter stopped at the review");
        assert_eq!(rows(&tracks, &Narrow::new("", false)).len(), 4);
    }

    #[test]
    fn the_library_order_is_a_view_and_never_synced_comes_first() {
        let library = vec![folder("Focus", 0, Some(500)), folder("Road trip", 4, None), folder("Sleep", 1, Some(100))];
        let order = |sort| shelf_rows(&library, &ShelfNarrow::default(), sort);
        assert_eq!(order(Sort::Name), vec![0, 1, 2]);
        // Never synced first: it is the furthest from matching upstream.
        assert_eq!(order(Sort::Synced), vec![1, 2, 0]);
        assert_eq!(order(Sort::Missing), vec![1, 2, 0]);
    }

    // Lists have no sync time and no missing downloads, so neither sort has anything to say about them.
    #[test]
    fn a_list_takes_no_part_in_the_sync_sorts() {
        let dead = Shelf { kind: Some(Kind::Custom), missing: 9, ..folder("Dead", 0, None) };
        let library = vec![dead, folder("Gappy", 2, None), folder("Old", 0, Some(5))];
        let order = |sort| {
            shelf_rows(&library, &ShelfNarrow::default(), sort).into_iter().map(|p| library[p].name.clone()).collect::<Vec<_>>()
        };
        assert_eq!(order(Sort::Synced), ["Gappy", "Old", "Dead"], "a list ranked as never synced");
        assert_eq!(order(Sort::Missing)[0], "Gappy", "a list's dead entries outranked a folder's");
    }

    // The kind keeps a folder that answered nothing, or no setting of the key could reach it.
    #[test]
    fn the_kind_walks_albums_then_playlists_and_keeps_the_unmarked() {
        let kinded = |name: &str, kind: Option<Kind>| Shelf { kind, ..folder(name, 0, None) };
        let library = vec![
            kinded("Autobahn", Some(Kind::Album)),
            kinded("Road trip", Some(Kind::Playlist)),
            kinded("Sleep", None),
            kinded("Mix", Some(Kind::Custom)),
        ];
        let mut only = None;
        let mut walk = Vec::new();
        for _ in 0..4 {
            only = next_kind(only);
            walk.push((kind_label(only), shelf_rows(&library, &ShelfNarrow { only, ..ShelfNarrow::default() }, Sort::Name)));
        }
        assert_eq!(
            walk,
            [
                ("albums", vec![0]),
                // A list built here is not a downloaded playlist, so it has its own stop.
                ("playlists", vec![1, 2]),
                ("custom playlists", vec![3]),
                ("every folder", vec![0, 1, 2, 3]),
            ]
        );
    }

    // The filenames are in memory from the stat pass, so the box finds a folder by a track inside it.
    #[test]
    fn the_library_filter_finds_a_folder_by_a_track_inside_it() {
        let library = vec![
            stocked("Focus", &["01 Steve Reich - Music for 18.opus"]),
            stocked("Road trip", &["01 Neu! - Hallogallo.opus", "02 Can - Spoon.opus"]),
            stocked("Sleep", &["01 Eno - Ascent.opus"]),
        ];
        // A track nobody would guess the folder of from its name.
        assert_eq!(by_name(&library, "hallogallo"), vec![1]);
        // Case is a tagging accident here as much as anywhere else.
        assert_eq!(by_name(&library, "NEU!"), vec![1]);
        // The folder's own name still matches, and matching both is one row.
        assert_eq!(by_name(&library, "o"), vec![0, 1, 2]);
        assert_eq!(by_name(&library, "focus"), vec![0]);
        // And something in neither place narrows to nothing rather than all.
        assert!(by_name(&library, "zzzz").is_empty());
    }

    #[test]
    fn the_kind_and_the_artist_compose_with_the_filter() {
        let library = vec![
            Shelf { kind: Some(Kind::Album), ..folder("Autobahn", 0, None) },
            Shelf { kind: Some(Kind::Album), ..folder("Radio-Activity", 0, None) },
            folder("Road trip", 0, None),
        ];
        let narrow = ShelfNarrow { filter: "ra", only: Some(Kind::Album), folders: None };
        assert_eq!(shelf_rows(&library, &narrow, Sort::Name), vec![1]);
        // All three hold an "o", so only the kind can leave the playlist out.
        let narrow = ShelfNarrow { filter: "o", only: Some(Kind::Album), folders: None };
        assert_eq!(shelf_rows(&library, &narrow, Sort::Name), vec![0, 1]);
        let folders = [library[0].path.clone(), library[2].path.clone()];
        let narrow = ShelfNarrow { filter: "", only: None, folders: Some(&folders) };
        assert_eq!(shelf_rows(&library, &narrow, Sort::Name), vec![0, 2]);
        assert!(narrow.narrows(), "Esc would have nothing to clear");
        assert!(!ShelfNarrow::default().narrows());
    }

    // Its tracks are found in the folders they came from, so listing them again would show each twice.
    #[test]
    fn the_search_never_lists_a_custom_playlists_tracks_a_second_time() {
        let library = vec![
            Shelf { kind: None, ..stocked("Folder", &["01 - Creep.opus"]) },
            Shelf { kind: Some(Kind::Custom), ..stocked("Mix", &["01 - Creep.opus"]) },
        ];
        assert_eq!(found(&library, "creep", None), vec![(0, 0)]);
        assert_eq!(found_total(&library, None), 1, "the band counted the list's tracks again");
    }

    // The picker and `t` are one matcher, so one query answers the same way on both.
    #[test]
    fn the_search_screen_is_fuzzy_ranked_and_matches_the_folder_like_the_picker() {
        let library = vec![stocked("Focus", &["01 - Creep.opus", "02 - Karma.opus"]), stocked("Other", &["01 - Autobahn.opus"])];
        // Nothing typed is no results, not the whole library.
        assert!(found(&library, "  ", None).is_empty());
        assert_eq!(found(&library, "autbn", None), vec![(1, 0)], "gaps in the query match");
        assert_eq!(found(&library, "focus", None), vec![(0, 0), (0, 1)], "the folder is matched too");
        assert_eq!(found_hits(&library, "krma", None)[0].label, "Karma", "the number is not part of the label");
        // The same query on the picker lists the same tracks.
        let picker: Vec<(usize, usize)> = picker::rows(&library, "creep").iter().map(|h| (h.shelf, h.file)).collect();
        assert_eq!(found(&library, "creep", None), picker);
    }

    /// Same box, same answer: with the kind on, `t` must not turn up tracks
    /// from the folders the rows just hid.
    #[test]
    fn the_search_screen_honours_the_kind_the_rows_are_narrowed_to() {
        let library = vec![
            Shelf { kind: Some(Kind::Album), ..stocked("Autobahn", &["01 Kraftwerk - Autobahn.opus"]) },
            stocked("Road trip", &["01 Kraftwerk - Neonlicht.opus"]),
        ];
        assert_eq!(found(&library, "kraftwerk", Some(Kind::Album)), vec![(0, 0)]);
        assert_eq!(found(&library, "kraftwerk", Some(Kind::Playlist)), vec![(1, 0)]);
    }

    #[test]
    fn an_artist_query_merges_spellings_like_the_rows_do() {
        let row = |name: &str| ArtistRow { name: name.into(), tracks: 1, albums: 1, starred: false, folders: Vec::new() };
        let rows = vec![row("Björk"), row("Can"), row("Neu!")];
        assert_eq!(artist_rows(&rows, "bjork"), vec![0]);
        assert_eq!(artist_rows(&rows, ""), vec![0, 1, 2]);
        assert!(artist_keeps("BJÖRK", &rows[0]));
        assert!(!artist_keeps("zzz", &rows[1]));
    }

    #[test]
    fn an_artist_page_shows_albums_and_eps_until_asked_for_the_rest() {
        let release =
            |kind: &str| AlbumRow { id: 1, title: "t".into(), year: None, kind: kind.into(), have: 0, of: None, new: false };
        assert!(release_shows(false, &release("album")) && release_shows(false, &release("ep")));
        assert!(!release_shows(false, &release("single")));
        assert!(release_shows(true, &release("compile")));
    }

    // "Everything except these three" means the rows the filter shows, as `a` does.
    #[test]
    fn inverting_marks_stops_at_the_filter_like_marking_all_does() {
        let mut tracks = with(&[Status::Ok, Status::Failed, Status::Ok]);
        tracks[0].name = "Neu! - Hallogallo".into();
        tracks[1].name = "Can - Vitamin C".into();
        tracks[2].name = "Neu! - Negativland".into();
        let mut marked: HashSet<usize> = [1].into_iter().collect();
        invert(&mut marked, &showing(&tracks, &Narrow::new("", false)));
        // The one that was marked comes off, the two that were not go on.
        assert_eq!(marked, [2, 3].into_iter().collect());

        marked.clear();
        invert(&mut marked, &showing(&tracks, &Narrow::new("neu", false)));
        assert_eq!(marked, [1, 3].into_iter().collect(), "it reached past the filter");
    }

    // The key that grabbed a class of bad rows has to give them back, or a misfire cannot be undone.
    #[test]
    fn marking_by_status_covers_that_status_and_then_clears_it() {
        let tracks = with(&[Status::Kept, Status::Ok, Status::Kept, Status::Weak]);
        let all = Narrow::new("", false);
        let mut marked = HashSet::new();
        flip(&mut marked, &like_status(&tracks, &all, Status::Kept));
        assert_eq!(picked(&marked), [1, 3]);
        flip(&mut marked, &like_status(&tracks, &all, Status::Kept));
        assert!(marked.is_empty(), "the second press unmarked them");
    }

    #[test]
    fn marking_by_status_keeps_marks_made_by_hand() {
        let tracks = with(&[Status::Kept, Status::Ok, Status::Kept]);
        let mut marked: HashSet<usize> = [2].into_iter().collect();
        flip(&mut marked, &like_status(&tracks, &Narrow::new("", false), Status::Kept));
        assert_eq!(picked(&marked), [1, 2, 3]);
    }

    // `/` then `m` marks one artist's bad rows; reaching past the filter would mark the whole run.
    #[test]
    fn marking_by_status_stops_at_the_filter() {
        let mut tracks = named(&["Aphex Twin - Xtal", "Aphex Twin - Avril", "Boards - Olson"]);
        tracks[2].status = Status::Ok;
        assert_eq!(like_status(&tracks, &Narrow::new("aphex", false), Status::Ok), [1], "the hidden Ok track was marked too");
    }

    // A compilation the lookup split across twenty artists: the question `m` cannot answer.
    #[test]
    fn marking_by_artist_takes_one_artist_under_any_case() {
        let mut tracks = with(&[Status::Ok, Status::Ok, Status::Ok]);
        for (track, artist) in tracks.iter_mut().zip(["Neu!", "Can", " neu!"]) {
            track.artist = artist.into();
        }
        let artist = artist_of(&tracks[0]).unwrap();
        assert_eq!(like_artist(&tracks, &Narrow::new("", false), &artist), [1, 3]);
        // Before the tag pass there is no artist to match on.
        assert_eq!(artist_of(&with(&[Status::Pending])[0]), None);
    }

    // Tracks already on disk, departed or never on the playlist have nothing for the gate to fetch.
    #[test]
    fn the_pick_gate_opens_with_everything_it_could_fetch_marked() {
        let tracks = with(&[Status::Pending, Status::Have, Status::Gone, Status::Local, Status::Pending, Status::Pending]);
        assert_eq!(picked(&gate_marks(&tracks, &[6])), [1, 5]);
    }

    #[test]
    fn the_tally_counts_each_status_in_its_column_order() {
        let tracks = with(&[Status::Failed, Status::Ok, Status::Failed, Status::Manual]);
        assert_eq!(counts(&tracks), [(Status::Ok, 1), (Status::Manual, 1), (Status::Failed, 2)]);
    }
}
