use std::collections::HashSet;

use crate::config::{composed, decomposed};

/// How a name's accents are spelled on the device it is going to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Form {
    Composed,
    Decomposed,
}

// FAT and exFAT refuse these, and so do Windows and most players that read them.
const FORBIDDEN: [char; 9] = ['<', '>', ':', '"', '/', '\\', '|', '?', '*'];
const RESERVED: [&str; 22] = [
    "CON", "PRN", "AUX", "NUL", "COM1", "COM2", "COM3", "COM4", "COM5", "COM6", "COM7", "COM8",
    "COM9", "LPT1", "LPT2", "LPT3", "LPT4", "LPT5", "LPT6", "LPT7", "LPT8", "LPT9",
];
// Android's filesystem counts bytes, and exFAT counts fewer, so bytes is the safe bound.
const MAX_BYTES: usize = 255;

/// A filename that is valid on any filesystem a device might use, for export only.
// The library's own `clean()` stays as it is: changing it renames files on their next edit.
pub fn name(raw: &str, form: Form) -> String {
    let spelled = match form {
        Form::Composed => composed(raw),
        Form::Decomposed => decomposed(raw),
    };
    let swapped: String = spelled
        .chars()
        .map(|c| if FORBIDDEN.contains(&c) || c.is_control() { '_' } else { c })
        .collect();
    let (stem, ext) = match swapped.rsplit_once('.') {
        Some((stem, ext)) if !stem.is_empty() && !ext.is_empty() && ext.len() <= 5 => (stem, Some(ext)),
        _ => (swapped.as_str(), None),
    };
    // A trailing dot or space is dropped by Windows, so two names would become one.
    let mut stem = stem.trim_end_matches(['.', ' ']).to_string();
    if stem.is_empty() {
        stem = "_".into();
    }
    if RESERVED.contains(&stem.to_ascii_uppercase().as_str()) {
        stem.insert(0, '_');
    }
    let tail = ext.map_or_else(String::new, |e| format!(".{e}"));
    let room = MAX_BYTES.saturating_sub(tail.len());
    while stem.len() > room {
        stem.pop();
    }
    format!("{stem}{tail}")
}

/// `name`, made different from every name already `taken` in the same folder.
// Two library names can map to one (`a?b` and `a*b` both become `a_b`), and a copy over the first loses a track.
// The suffix is counted in the length, so a name already at the limit is cut to make room.
pub fn unique(taken: &mut HashSet<String>, name: &str) -> String {
    let key = |n: &str| n.to_lowercase();
    if taken.insert(key(name)) {
        return name.to_string();
    }
    let (stem, tail) = match name.rsplit_once('.') {
        Some((stem, ext)) if !stem.is_empty() => (stem, format!(".{ext}")),
        _ => (name, String::new()),
    };
    (2..)
        .map(|n| {
            let suffix = format!(" ({n})");
            let mut stem = stem.to_string();
            while stem.len() + suffix.len() + tail.len() > MAX_BYTES {
                stem.pop();
            }
            format!("{stem}{suffix}{tail}")
        })
        .find(|candidate| taken.insert(key(candidate)))
        .expect("an unbounded range always finds a free name")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_character_fat_refuses_is_replaced() {
        let got = name("01 - What? A \"Song\" <live>: a|b*c.opus", Form::Composed);
        assert_eq!(got, "01 - What_ A _Song_ _live__ a_b_c.opus");
        assert!(!got.contains(FORBIDDEN));
    }

    #[test]
    fn a_trailing_dot_or_space_and_a_reserved_name_are_made_safe() {
        assert_eq!(name("Song. .opus", Form::Composed), "Song.opus");
        assert_eq!(name("CON.opus", Form::Composed), "_CON.opus");
        assert_eq!(name("lpt1", Form::Composed), "_lpt1");
        assert_eq!(name("...", Form::Composed), "_");
    }

    #[test]
    fn a_long_name_keeps_its_extension_and_fits() {
        let long = format!("{}.opus", "한".repeat(200));
        let got = name(&long, Form::Composed);
        assert!(got.len() <= MAX_BYTES, "{}", got.len());
        assert!(got.ends_with(".opus"), "{got}");
    }

    // The setting docs/history.md said export would become.
    #[test]
    fn the_form_decides_how_an_accent_is_spelled() {
        let composed = name("Café.opus", Form::Composed);
        let decomposed = name("Café.opus", Form::Decomposed);
        assert_ne!(composed, decomposed);
        assert_eq!(composed.chars().count() + 1, decomposed.chars().count());
    }

    #[test]
    fn a_suffix_never_pushes_a_name_over_the_limit() {
        let mut taken = HashSet::new();
        let base = name(&format!("{}.opus", "a".repeat(300)), Form::Composed);
        let names: Vec<String> = (0..12).map(|_| unique(&mut taken, &base)).collect();
        assert!(names.iter().all(|n| n.len() <= MAX_BYTES && n.ends_with(".opus")), "{:?}", names.iter().map(String::len).collect::<Vec<_>>());
        assert_eq!(taken.len(), 12, "two of them were the same name");
    }

    #[test]
    fn two_names_that_collapse_to_one_are_kept_apart() {
        let mut taken = HashSet::new();
        let first = unique(&mut taken, &name("a?b.opus", Form::Composed));
        let second = unique(&mut taken, &name("a*b.opus", Form::Composed));
        let third = unique(&mut taken, &name("A_B.opus", Form::Composed));
        assert_eq!((first.as_str(), second.as_str(), third.as_str()), ("a_b.opus", "a_b (2).opus", "A_B (3).opus"));
    }
}
