use std::collections::HashSet;
use std::path::{Path, PathBuf};

use nucleo_matcher::pattern::{AtomKind, CaseMatching, Normalization, Pattern};
use nucleo_matcher::{Config, Matcher, Utf32Str};
use unicode_segmentation::UnicodeSegmentation;

use crate::app::Shelf;
use crate::config::composed;
use crate::manifest::Kind;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Hit {
    pub shelf: usize,
    pub file: usize,
    pub score: u32,
    // Char positions in `label` and in the folder's name, sorted and unique.
    pub at: Vec<usize>,
    pub folder_at: Vec<usize>,
    pub label: String,
}

fn stem(name: &str) -> &str {
    name.rsplit_once('.').map_or(name, |(stem, _)| stem)
}

// The label is the filename without its extension or its playlist number, which means nothing in a list
// ranked by relevance. The folder is matched too, so `focus creep` finds the track by artist folder.
fn label_of(name: &str) -> String {
    let stem = composed(stem(name));
    let bare = crate::worker::strip_number(&stem);
    if bare.is_empty() { stem } else { bare.to_string() }
}

fn grapheme_count(text: &str) -> usize {
    if text.is_ascii() { text.len() } else { text.graphemes(true).count() }
}

// Custom rows are skipped (their tracks are in the folders they came from) and so are missing files.
// The library's `a` kind filter does not apply here.
pub fn rows(library: &[Shelf], query: &str) -> Vec<Hit> {
    find(library, query, |_| true, Files::OnDisk)
}

// Whether files that are gone from disk are listed: the search shows them with a `!`, and the picker
// cannot add them.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Files {
    OnDisk,
    All,
}

// The matcher behind the picker and the `t` search screen. `keep` narrows the folders.
pub fn find(library: &[Shelf], query: &str, keep: impl Fn(&Shelf) -> bool, files: Files) -> Vec<Hit> {
    let query = composed(query.trim());
    // One or two letters match almost any title as a fuzzy query, so they must appear as typed.
    let short = query.chars().count() <= 2 && !query.contains(char::is_whitespace);
    let pattern = if short {
        Pattern::new(&query, CaseMatching::Ignore, Normalization::Smart, AtomKind::Substring)
    } else {
        Pattern::parse(&query, CaseMatching::Ignore, Normalization::Smart)
    };
    let mut matcher = Matcher::new(Config::DEFAULT);
    let (mut buf, mut found) = (Vec::new(), Vec::new());
    let mut hits = Vec::new();
    for (shelf, s) in library.iter().enumerate().filter(|(_, s)| s.kind != Some(Kind::Custom) && keep(s)) {
        let folder = composed(&s.name);
        for (file, (name, _)) in s.files.iter().enumerate().filter(|(_, (_, here))| *here || files == Files::All) {
            let label = label_of(name);
            if query.is_empty() {
                hits.push(Hit { shelf, file, score: 0, at: Vec::new(), folder_at: Vec::new(), label });
                continue;
            }
            found.clear();
            let both = format!("{label} {folder}");
            let Some(score) = pattern.indices(Utf32Str::new(&both, &mut buf), &mut matcher, &mut found) else {
                continue;
            };
            // Indices before the joining space are the label's and after it the folder's.
            let split = grapheme_count(&label) as u32;
            let in_label: Vec<u32> = found.iter().copied().filter(|i| *i < split).collect();
            let in_folder: Vec<u32> = found.iter().filter(|i| **i > split).map(|i| i - split - 1).collect();
            let mut at = char_positions(&label, &in_label);
            let mut folder_at = char_positions(&folder, &in_folder);
            for v in [&mut at, &mut folder_at] {
                v.sort_unstable();
                v.dedup();
            }
            hits.push(Hit { shelf, file, score, at, folder_at, label });
        }
    }
    // Stable, so equal scores keep library order and the list does not shuffle between keystrokes.
    hits.sort_by(|a, b| b.score.cmp(&a.score));
    hits
}

// nucleo counts graphemes in a non-ASCII label and the highlight walks chars, so each matched grapheme
// becomes the chars it spans.
fn char_positions(label: &str, found: &[u32]) -> Vec<usize> {
    if label.is_ascii() {
        return found.iter().map(|i| *i as usize).collect();
    }
    let mut spans = Vec::new();
    let mut start = 0;
    for g in label.graphemes(true) {
        let n = g.chars().count();
        spans.push(start..start + n);
        start += n;
    }
    found.iter().filter_map(|i| spans.get(*i as usize)).flat_map(|r| r.clone()).collect()
}

// Keyed by folder and filename, never row position: ranking reorders rows on every keystroke.
// `keys` holds the composed names so a lookup costs one composition, not one per mark.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Marks {
    order: Vec<(PathBuf, String)>,
    keys: HashSet<(PathBuf, String)>,
}

fn key(folder: &Path, file: &str) -> (PathBuf, String) {
    (folder.to_path_buf(), composed(file))
}

impl Marks {
    pub fn has(&self, folder: &Path, file: &str) -> bool {
        self.keys.contains(&key(folder, file))
    }

    pub fn toggle(&mut self, folder: &Path, file: &str) {
        let k = key(folder, file);
        if self.keys.remove(&k) {
            self.order.retain(|(f, n)| key(f, n) != k);
        } else {
            self.keys.insert(k);
            self.order.push((folder.to_path_buf(), file.to_string()));
        }
    }

    // Marks everything given, or unmarks all of it when it is all marked already.
    pub fn toggle_all<'a>(&mut self, items: impl Iterator<Item = (&'a Path, &'a str)>) {
        let items: Vec<(&Path, &str, (PathBuf, String))> = items.map(|(f, n)| (f, n, key(f, n))).collect();
        if items.iter().all(|(_, _, k)| self.keys.contains(k)) {
            let gone: HashSet<&(PathBuf, String)> = items.iter().map(|(_, _, k)| k).collect();
            self.order.retain(|(f, n)| !gone.contains(&key(f, n)));
            self.keys.retain(|k| !gone.contains(k));
        } else {
            for (f, n, k) in items {
                if self.keys.insert(k) {
                    self.order.push((f.to_path_buf(), n.to_string()));
                }
            }
        }
    }

    pub fn len(&self) -> usize {
        self.order.len()
    }

    pub fn clear(&mut self) {
        self.order.clear();
        self.keys.clear();
    }

    pub fn picks(&self) -> Vec<(PathBuf, String)> {
        self.order.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn shelf(name: &str, files: &[&str]) -> Shelf {
        Shelf {
            path: PathBuf::from("/music").join(name),
            name: name.to_string(),
            url: None,
            tracks: files.len(),
            missing: 0,
            synced: None,
            files: files.iter().map(|f| (f.to_string(), true)).collect(),
            playlist: None,
            kind: None,
        }
    }

    fn labels(hits: &[Hit]) -> Vec<&str> {
        hits.iter().map(|h| h.label.as_str()).collect()
    }

    #[test]
    fn an_empty_query_lists_every_addable_track_in_order() {
        let mut gone = shelf("C", &["01 - Gone.opus"]);
        gone.files[0].1 = false;
        let mut custom = shelf("L", &["01 - Dup.opus"]);
        custom.kind = Some(Kind::Custom);
        let library = vec![shelf("A", &["01 - One.opus", "02 - Two.opus"]), custom, gone, shelf("B", &["01 - Three.mp3"])];
        let hits = rows(&library, "");
        assert_eq!(labels(&hits), ["One", "Two", "Three"]);
        assert_eq!((hits[2].shelf, hits[2].file), (3, 0));
    }

    #[test]
    fn fuzzy_ranking_takes_gaps_and_out_of_order_characters_and_ignores_case() {
        let library = vec![shelf("A", &["01 - Autobahn.opus", "02 - Trans Europe Express.opus", "03 - The Model.opus"])];
        assert_eq!(labels(&rows(&library, "AUTBN")), ["Autobahn"]);
        assert_eq!(labels(&rows(&library, "europe trans")), ["Trans Europe Express"]);
        assert!(rows(&library, "zzz").is_empty());
    }

    #[test]
    fn an_accent_spelled_two_ways_matches_both_ways() {
        let composed_name = "01 - Caf\u{e9}.opus";
        let decomposed = "02 - Cafe\u{301}.opus";
        let library = vec![shelf("A", &[composed_name, decomposed])];
        assert_eq!(rows(&library, "caf\u{e9}").len(), 2);
        assert_eq!(rows(&library, "cafe\u{301}").len(), 2);
    }

    #[test]
    fn highlight_positions_name_the_matched_characters() {
        let library = vec![shelf("A", &["01 - Autobahn.opus"])];
        let hit = &rows(&library, "bahn")[0];
        let marked: String = hit.label.chars().enumerate().filter(|(i, _)| hit.at.contains(i)).map(|(_, c)| c).collect();
        assert_eq!(marked.to_lowercase(), "bahn");
        assert!(hit.at.windows(2).all(|w| w[0] < w[1]));
    }

    #[test]
    fn a_multi_codepoint_grapheme_does_not_shift_the_highlight() {
        // The flag is one grapheme and four chars (two regional indicators), ahead of the match.
        let library = vec![shelf("A", &["01 - \u{1F1E6}\u{1F1FA}\u{1F1E6}\u{1F1FA} Autobahn.opus"])];
        let hit = &rows(&library, "bahn")[0];
        let lit: String = hit.label.chars().enumerate().filter(|(i, _)| hit.at.contains(i)).map(|(_, c)| c).collect();
        assert_eq!(lit.to_lowercase(), "bahn");
    }

    #[test]
    fn the_playlist_number_is_dropped_but_a_title_that_opens_with_one_keeps_it() {
        let library = vec![shelf("A", &["01 - Autobahn.opus", "1979.opus", "99 Luftballons.opus"])];
        assert_eq!(labels(&rows(&library, "")), ["Autobahn", "1979", "99 Luftballons"]);
    }

    #[test]
    fn the_folder_is_matched_too_alone_or_with_the_title() {
        let library = vec![shelf("Focus", &["01 - Creep.opus", "02 - Karma.opus"]), shelf("Other", &["01 - Creep.opus"])];
        let by_folder = rows(&library, "focus");
        assert_eq!(by_folder.len(), 2);
        assert!(by_folder.iter().all(|h| h.shelf == 0 && h.at.is_empty() && !h.folder_at.is_empty()));
        let both = rows(&library, "focus creep");
        assert_eq!((both.len(), both[0].shelf, both[0].label.as_str()), (1, 0, "Creep"));
    }

    #[test]
    fn one_or_two_letters_must_appear_as_typed_where_three_may_have_gaps() {
        let library = vec![shelf("A", &["01 - Autobahn.opus", "02 - Radioactivity.opus", "03 - The Model.opus"])];
        // `ai` has a gap in Radioactivity and appears in no title as typed.
        assert!(rows(&library, "ai").is_empty());
        assert_eq!(labels(&rows(&library, "ad")), ["Radioactivity"]);
        assert_eq!(labels(&rows(&library, "rdc")), ["Radioactivity"]);
    }

    #[test]
    fn equal_scores_keep_library_order() {
        let library = vec![shelf("A", &["01 - Same.opus"]), shelf("B", &["01 - Same.opus"])];
        let hits = rows(&library, "same");
        assert_eq!(hits.iter().map(|h| h.shelf).collect::<Vec<_>>(), [0, 1]);
    }

    #[test]
    fn five_thousand_files_match_well_inside_a_keystroke() {
        let files: Vec<String> = (0..5000).map(|i| format!("{i:04} - Artist {i} - Some Song Title {}.opus", i * 7)).collect();
        let names: Vec<&str> = files.iter().map(String::as_str).collect();
        let library = vec![shelf("Big", &names)];
        let start = std::time::Instant::now();
        let hits = rows(&library, "sng ttl 42");
        assert!(!hits.is_empty());
        assert!(start.elapsed() < std::time::Duration::from_millis(500), "took {:?}", start.elapsed());
    }

    #[test]
    fn marks_keep_their_order_and_survive_a_new_query_and_a_refresh() {
        let mut library = vec![shelf("A", &["01 - One.opus", "02 - Two.opus"]), shelf("B", &["01 - Three.opus"])];
        let mut marks = Marks::default();
        let first = rows(&library, "three");
        marks.toggle(&library[first[0].shelf].path, &library[first[0].shelf].files[first[0].file].0);
        let second = rows(&library, "one");
        marks.toggle(&library[second[0].shelf].path, &library[second[0].shelf].files[second[0].file].0);
        // A refresh that moves every row.
        library.reverse();
        let hits = rows(&library, "");
        assert_eq!(hits.len(), 3);
        assert_eq!(
            marks.picks(),
            [(PathBuf::from("/music/B"), "01 - Three.opus".to_string()), (PathBuf::from("/music/A"), "01 - One.opus".to_string())]
        );
        assert!(marks.has(Path::new("/music/A"), "01 - One.opus"));
        marks.toggle(Path::new("/music/B"), "01 - Three.opus");
        assert_eq!(marks.len(), 1);
    }

    #[test]
    fn toggle_all_marks_everything_then_unmarks_it() {
        let mut marks = Marks::default();
        let a = Path::new("/music/A");
        marks.toggle(a, "x.opus");
        let items = [(a, "x.opus"), (a, "y.opus")];
        marks.toggle_all(items.iter().copied());
        assert_eq!(marks.len(), 2);
        marks.toggle_all(items.iter().copied());
        assert_eq!(marks.len(), 0);
    }

    #[test]
    fn marking_five_thousand_tracks_is_not_quadratic() {
        let names: Vec<String> = (0..5000).map(|i| format!("{i:04} - Song.opus")).collect();
        let folder = Path::new("/music/A");
        let items: Vec<(&Path, &str)> = names.iter().map(|n| (folder, n.as_str())).collect();
        let mut marks = Marks::default();
        let start = std::time::Instant::now();
        marks.toggle_all(items.iter().copied());
        assert_eq!(marks.len(), 5000);
        marks.toggle_all(items.iter().copied());
        assert_eq!(marks.len(), 0);
        assert!(start.elapsed() < std::time::Duration::from_millis(500), "took {:?}", start.elapsed());
    }
}
