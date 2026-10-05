//! Codes typed into the `^z ^x` console, and what each one turns on.

use std::sync::atomic::{AtomicBool, Ordering};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Feature {
    /// `music.youtube.com` links at the URL prompt.
    Music,
    /// A secret palette from `theme::SECRETS`, each behind its own code.
    Vaporwave,
    GameBoy,
}

impl Feature {
    /// What the celebration says was unlocked.
    pub fn prize(self) -> &'static str {
        match self {
            Feature::Music => "YouTube Music links",
            Feature::Vaporwave => "the vaporwave theme",
            Feature::GameBoy => "the game boy theme",
        }
    }

    /// The `theme::SECRETS` palette this unlocks, if it is a theme.
    pub fn theme(self) -> Option<&'static str> {
        match self {
            Feature::Vaporwave => Some("vaporwave"),
            Feature::GameBoy => Some("gameboy"),
            Feature::Music => None,
        }
    }

    /// The celebration's second line: how to find what was unlocked.
    pub fn note(self) -> &'static str {
        match self {
            Feature::Music => "for this session",
            Feature::Vaporwave | Feature::GameBoy => "^t walks to it  ·  this session",
        }
    }
}

pub struct Cheat {
    /// `digest` of the code, so the word is in neither the source nor the binary.
    pub hash: u64,
    pub feature: Feature,
}

/// The URL prompt's header while a music link is refused. Here rather than in
/// the worker because the UI rewrites it once the code is found.
pub const LOCKED: &str = "YouTube Music links are locked";
pub const OPENED: &str = "YouTube Music links unlocked";

/* Nothing in the suite types a real code, so a wrong hash here passes every
   test: check a new one by hand in the console. */
pub const CHEATS: &[Cheat] = &[
    Cheat {
        hash: 0x6a0c_1f80_1823_83a7,
        feature: Feature::Music,
    },
    Cheat {
        hash: 0x7fa3_52d7_3529_40b9,
        feature: Feature::Vaporwave,
    },
    Cheat {
        hash: 0x21d4_b63f_4541_c1cb,
        feature: Feature::GameBoy,
    },
];

/// What the tests type instead of a real code, one per feature.
#[cfg(test)]
pub const TEST_CODE: &str = "opensesame";
#[cfg(test)]
pub const TEST_THEME_CODE: &str = "hocuspocus";
#[cfg(test)]
pub const TEST_GAMEBOY_CODE: &str = "shazam";
#[cfg(test)]
const TEST_CHEATS: &[Cheat] = &[
    Cheat {
        hash: 0x7a92_14a9_c2bf_1ce7,
        feature: Feature::Music,
    },
    Cheat {
        hash: 0xf7d6_a145_f4c9_6f1d,
        feature: Feature::Vaporwave,
    },
    Cheat {
        hash: 0xc3a5_b36e_c15d_9f83,
        feature: Feature::GameBoy,
    },
];

/// FNV-1a, 64-bit. Written out because `DefaultHasher` is not stable between
/// Rust releases, and a changed hash would silently kill every code.
fn digest(text: &str) -> u64 {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in text.bytes() {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    hash
}

/// Case and surrounding spaces ignored: nobody should miss a code on a shift key.
pub fn find(typed: &str) -> Option<&'static Cheat> {
    let hash = digest(&typed.trim().to_ascii_lowercase());
    let table = CHEATS.iter();
    #[cfg(test)]
    let table = table.chain(TEST_CHEATS);
    table.into_iter().find(|c| c.hash == hash)
}

/* Atomics rather than a `Cmd`: the console is used while the worker is
   blocked on the URL prompt, where nothing reads the command channel. */
#[derive(Debug, Default)]
pub struct Unlocked {
    music: AtomicBool,
    vaporwave: AtomicBool,
    gameboy: AtomicBool,
}

impl Unlocked {
    fn slot(&self, feature: Feature) -> &AtomicBool {
        match feature {
            Feature::Music => &self.music,
            Feature::Vaporwave => &self.vaporwave,
            Feature::GameBoy => &self.gameboy,
        }
    }

    pub fn has(&self, feature: Feature) -> bool {
        self.slot(feature).load(Ordering::SeqCst)
    }

    pub fn count(&self) -> usize {
        [&self.music, &self.vaporwave, &self.gameboy]
            .iter()
            .filter(|f| f.load(Ordering::SeqCst))
            .count()
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
        for typed in ["opensesame", "OPENSESAME", "  OpenSesame "] {
            assert_eq!(find(typed).map(|c| c.feature), Some(Feature::Music), "{typed:?}");
        }
        assert_eq!(find(TEST_THEME_CODE).map(|c| c.feature), Some(Feature::Vaporwave));
        assert_eq!(find(TEST_GAMEBOY_CODE).map(|c| c.feature), Some(Feature::GameBoy));
        assert!(find("opensesam").is_none());
        assert!(find("").is_none());
    }

    // Pinned to published FNV-1a vectors, since every stored hash rests on it.
    #[test]
    fn the_digest_is_fnv_1a() {
        assert_eq!(digest(""), 0xcbf2_9ce4_8422_2325);
        assert_eq!(digest("a"), 0xaf63_dc4c_8601_ec8c);
        assert_eq!(digest("foobar"), 0x85944171f73967e8);
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
