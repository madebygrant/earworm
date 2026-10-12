use crate::manifest::Kind;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Icons {
    // The words and pills the library drew before icons existed.
    #[default]
    Text,
    // Plain Unicode that most fonts carry, one cell wide.
    Symbols,
    // Nerd Font glyphs, drawn full-cell. A box in a terminal font without them.
    Nerd,
}

pub const NAMES: &str = "text, symbols, nerd";

const NF_ALBUM: &str = "\u{f0025}";
const NF_PLAYLIST: &str = "\u{f0cb8}";
const NF_LOCAL: &str = "\u{f1359}";
const NF_CUSTOM: &str = "\u{f0900}";

impl Icons {
    pub fn parse(name: &str) -> Option<Icons> {
        match name {
            "text" => Some(Icons::Text),
            "symbols" => Some(Icons::Symbols),
            "nerd" => Some(Icons::Nerd),
            _ => None,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Icons::Text => "text",
            Icons::Symbols => "symbols",
            Icons::Nerd => "nerd",
        }
    }

    // Cells the kind slot takes on every row: a space, the glyph, and for Nerd
    // a trailing space, since those glyphs are drawn wider than one cell.
    pub fn slot(self) -> usize {
        match self {
            Icons::Text => 0,
            Icons::Symbols => 2,
            Icons::Nerd => 3,
        }
    }

    // A folder with no kind is a playlist, the same reading `kind_shows` takes.
    pub fn kind(self, kind: Option<Kind>) -> Option<&'static str> {
        match (self, kind) {
            (Icons::Text, _) => None,
            (Icons::Symbols, Some(Kind::Album)) => Some("●"),
            (Icons::Symbols, Some(Kind::Custom)) => Some("◈"),
            (Icons::Symbols, _) => Some("▤"),
            (Icons::Nerd, Some(Kind::Album)) => Some(NF_ALBUM),
            (Icons::Nerd, Some(Kind::Custom)) => Some(NF_CUSTOM),
            (Icons::Nerd, _) => Some(NF_PLAYLIST),
        }
    }

    pub fn local(self) -> Option<&'static str> {
        match self {
            Icons::Text => None,
            Icons::Symbols => Some("◆"),
            Icons::Nerd => Some(NF_LOCAL),
        }
    }

    // The legend rows, in the order the library draws them.
    pub fn legend(self) -> Vec<(&'static str, &'static str)> {
        let local = "local folder, no URL to sync from";
        match self {
            Icons::Text => Vec::new(),
            Icons::Symbols => vec![("●", "album"), ("▤", "playlist"), ("◈", "custom playlist"), ("◆", local)],
            Icons::Nerd => vec![
                (NF_ALBUM, "album"),
                (NF_PLAYLIST, "playlist"),
                (NF_CUSTOM, "custom playlist"),
                (NF_LOCAL, local),
            ],
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use unicode_width::UnicodeWidthStr;

    #[test]
    fn every_glyph_is_one_cell_wide() {
        for (glyph, _) in Icons::Symbols.legend().into_iter().chain(Icons::Nerd.legend()) {
            assert_eq!(glyph.width(), 1, "{glyph} would spill into the next column");
        }
    }

    #[test]
    fn names_round_trip() {
        for icons in [Icons::Text, Icons::Symbols, Icons::Nerd] {
            assert_eq!(Icons::parse(icons.name()), Some(icons));
        }
        assert_eq!(Icons::parse("emoji"), None);
    }

    #[test]
    fn text_draws_no_glyphs_and_a_kindless_folder_is_a_playlist() {
        assert_eq!(Icons::Text.kind(Some(Kind::Album)), None);
        assert_eq!(Icons::Text.local(), None);
        assert_eq!(Icons::Symbols.kind(None), Icons::Symbols.kind(Some(Kind::Playlist)));
        assert_ne!(Icons::Symbols.kind(None), Icons::Symbols.kind(Some(Kind::Album)));
    }
}
