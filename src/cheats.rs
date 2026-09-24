//! Codes typed into the `^g` console, and what each one turns on.

use std::sync::atomic::{AtomicBool, Ordering};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Feature {
    /// `music.youtube.com` links at the URL prompt.
    Music,
}

pub struct Cheat {
    pub code: &'static str,
    pub feature: Feature,
    /// What the celebration says was unlocked.
    pub prize: &'static str,
}

/// The URL prompt's header while a music link is refused. Here rather than in
/// the worker because the UI rewrites it once the code is found.
pub const LOCKED: &str = "YouTube Music links are locked";
pub const OPENED: &str = "YouTube Music links unlocked";

pub const CHEATS: &[Cheat] = &[Cheat {
    code: "treasure",
    feature: Feature::Music,
    prize: "YouTube Music links",
}];

/// Case and surrounding spaces ignored: nobody should miss a code on a shift key.
pub fn find(typed: &str) -> Option<&'static Cheat> {
    let typed = typed.trim();
    CHEATS.iter().find(|c| c.code.eq_ignore_ascii_case(typed))
}

/* Atomics rather than a `Cmd`: the console is used while the worker is
   blocked on the URL prompt, where nothing reads the command channel. */
#[derive(Debug, Default)]
pub struct Unlocked {
    music: AtomicBool,
}

impl Unlocked {
    fn slot(&self, feature: Feature) -> &AtomicBool {
        match feature {
            Feature::Music => &self.music,
        }
    }

    pub fn has(&self, feature: Feature) -> bool {
        self.slot(feature).load(Ordering::SeqCst)
    }

    /// True only for the call that turned it on, so a repeat is not celebrated.
    pub fn grant(&self, feature: Feature) -> bool {
        !self.slot(feature).swap(true, Ordering::SeqCst)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_code_matches_whatever_case_it_was_typed_in() {
        for typed in ["treasure", "TREASURE", "  Treasure "] {
            assert_eq!(find(typed).map(|c| c.feature), Some(Feature::Music), "{typed:?}");
        }
        assert!(find("treasur").is_none());
        assert!(find("").is_none());
    }

    #[test]
    fn a_feature_is_granted_once() {
        let unlocked = Unlocked::default();
        assert!(!unlocked.has(Feature::Music));
        assert!(unlocked.grant(Feature::Music));
        assert!(unlocked.has(Feature::Music));
        assert!(!unlocked.grant(Feature::Music), "a repeat is not news");
    }
}
