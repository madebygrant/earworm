use std::sync::OnceLock;

use ratatui::style::Color;

/* A palette is a set of roles, not a set of pigments. The names used to be
   the colours themselves — CREAM, GOLD — which reads fine on one warm dark
   theme and lies on every other: a light theme's "cream" is near-black ink.
   What each slot is *for* is the part that survives a theme change, so that
   is what they are called. */
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Palette {
    /// Track titles and anything the eye should land on first.
    pub text: Color,
    /// Chrome that should read as chrome: borders, bars, the spinner, and the
    /// fill behind a mode band.
    pub accent: Color,
    /// Where the cursor is, and work that came out right.
    pub cursor: Color,
    /// Something to look at but not an error.
    pub warn: Color,
    /// A real failure.
    pub error: Color,
    /// Secondary text: sources, notes, hints.
    pub muted: Color,
    /// Structure that must not compete with the list.
    pub rule: Color,
    /// Popups sit above the gradient, so they need one flat tone of their own
    /// or they read as a hole rather than a raised surface.
    pub surface: Color,
    /// Identified, but nothing was found to identify it as. Its own slot
    /// rather than `muted`, because "we looked and found nothing" is a
    /// different claim from "this is secondary".
    pub unsure: Color,
    /// Text on a filled band, where the gradient is covered. Read against
    /// `accent` and nothing else: the band is the only thing that fills.
    pub ink: Color,
    /* The diagonal gradient behind everything, as its two ends. Part of the
       palette because a light theme is not a light palette over a dark
       ground: they move together or nothing is legible. */
    pub near: (u8, u8, u8),
    pub far: (u8, u8, u8),
    /// Where the gradient stop sits: past it the whole corner is flat `far`.
    pub stop: f32,
}

impl Palette {
    /* Every slot with its name, so a check that must cover the palette cannot
       quietly miss one: adding a field to the struct without adding it here
       is the mistake this exists to make hard. */
    pub fn slots(&self) -> [(&'static str, Color); 10] {
        [
            ("text", self.text),
            ("accent", self.accent),
            ("cursor", self.cursor),
            ("warn", self.warn),
            ("error", self.error),
            ("muted", self.muted),
            ("rule", self.rule),
            ("surface", self.surface),
            ("unsure", self.unsure),
            ("ink", self.ink),
        ]
    }
}

/* Warm cream-on-dark: gold carries structure, teal marks where you are, and
   everything secondary is a muted sand rather than a grey. Text and ground
   are both warm, so the one cool colour is what the eye lands on.

   Measured, not asserted: `every_palette_is_legible_on_its_own_ground` runs
   the numbers over every built-in, against both ends of the gradient and the
   tone popups raise themselves with.

   The gradient is the hues of
   linear-gradient(45deg, hsla(10,19%,36%,1) 0%, hsla(290,95%,9%,1) 76%) at
   half brightness, so text stays the brightest thing on screen. */
pub const WARM: Palette = Palette {
    text: Color::Rgb(236, 223, 192),
    /* Darker than the text on purpose: the two were 1.13:1 apart before and
       read as one. */
    accent: Color::Rgb(214, 180, 96),
    // Cool against the warm ground, so it separates from warn by hue.
    cursor: Color::Rgb(96, 178, 158),
    warn: Color::Rgb(214, 154, 78),
    /* Lifted from (196,106,92), which measured 3.73:1 over `near` — under the
       4.5:1 this file has always claimed, on the one colour a failed track is
       read in. The same red, two shades up: 4.74:1. */
    error: Color::Rgb(214, 126, 112),
    muted: Color::Rgb(158, 148, 128),
    rule: Color::Rgb(92, 84, 66),
    surface: Color::Rgb(46, 41, 33),
    unsure: Color::Rgb(168, 155, 126),
    ink: Color::Rgb(24, 18, 16),
    near: (55, 40, 37),
    far: (18, 1, 22),
    stop: 0.76,
};

/* Ink on parchment, for a terminal with a light background — where every
   other palette here is unreadable. The inversion is the whole palette, not
   the text: the gradient's ends go light too, and `surface` is lighter than
   the page rather than darker, so a popup still reads as raised. */
pub const LIGHT: Palette = Palette {
    text: Color::Rgb(38, 34, 30),
    accent: Color::Rgb(124, 84, 14),
    cursor: Color::Rgb(17, 110, 98),
    warn: Color::Rgb(132, 82, 6),
    error: Color::Rgb(170, 40, 40),
    muted: Color::Rgb(105, 98, 88),
    rule: Color::Rgb(172, 164, 150),
    surface: Color::Rgb(253, 251, 247),
    /* Warm where `muted` is grey, which is the whole difference between "this
       is secondary" and "we looked and found nothing". A first attempt at
       (118,110,98) measured 3.86:1 on the pale end of the gradient. */
    unsure: Color::Rgb(116, 96, 56),
    // A filled band on a light theme is dark, so its text is light.
    ink: Color::Rgb(250, 248, 244),
    near: (247, 243, 236),
    far: (231, 225, 214),
    stop: 0.76,
};

/* Slate and steel: the same structure as warm with the warmth taken out, for
   anyone who finds a gold-and-cream terminal too much furniture. */
pub const COOL: Palette = Palette {
    text: Color::Rgb(226, 232, 240),
    accent: Color::Rgb(126, 176, 222),
    cursor: Color::Rgb(94, 206, 196),
    warn: Color::Rgb(222, 170, 90),
    error: Color::Rgb(236, 132, 128),
    muted: Color::Rgb(150, 162, 176),
    rule: Color::Rgb(84, 98, 114),
    surface: Color::Rgb(34, 42, 52),
    unsure: Color::Rgb(160, 170, 184),
    ink: Color::Rgb(16, 20, 26),
    near: (44, 52, 62),
    far: (14, 18, 24),
    stop: 0.76,
};

/* Magenta and cyan over a violet-to-black ground. The hazard in a neon
   palette is always `muted` — every source, note and hint draws in it, and
   the temptation is a dim purple that measures 2:1. This one is light enough
   to read and cool enough to stay behind the text. */
pub const NEON: Palette = Palette {
    text: Color::Rgb(226, 232, 255),
    accent: Color::Rgb(255, 92, 213),
    cursor: Color::Rgb(86, 240, 255),
    warn: Color::Rgb(246, 226, 90),
    error: Color::Rgb(255, 94, 120),
    muted: Color::Rgb(166, 150, 205),
    rule: Color::Rgb(86, 48, 120),
    surface: Color::Rgb(24, 14, 38),
    unsure: Color::Rgb(180, 168, 215),
    ink: Color::Rgb(10, 6, 16),
    near: (26, 10, 44),
    far: (8, 4, 14),
    stop: 0.76,
};

/* Every palette earworm ships. The list is what the contrast, quantise and
   NO_COLOR checks iterate, so a new theme is covered the moment it is added
   here and cannot be shipped unmeasured. */
pub const BUILT_INS: [(&str, Palette); 4] = [
    ("warm", WARM),
    ("light", LIGHT),
    ("cool", COOL),
    ("neon", NEON),
];

/* WCAG 2.1 relative luminance and contrast ratio. A palette is a claim about
   legibility — theme.rs has carried one in a comment since the first commit —
   and a claim nothing measures is a claim that quietly stops being true. */
pub fn contrast(a: (u8, u8, u8), b: (u8, u8, u8)) -> f64 {
    let luminance = |c: (u8, u8, u8)| {
        let channel = |v: u8| {
            let v = f64::from(v) / 255.0;
            if v <= 0.03928 {
                v / 12.92
            } else {
                ((v + 0.055) / 1.055).powf(2.4)
            }
        };
        0.2126 * channel(c.0) + 0.7152 * channel(c.1) + 0.0722 * channel(c.2)
    };
    let (x, y) = (luminance(a), luminance(b));
    (x.max(y) + 0.05) / (x.min(y) + 0.05)
}

impl Palette {
    /// The palette a config or a flag names, or `None` for a name nobody
    /// ships — which the caller turns into a startup error rather than a
    /// silent fallback to the default.
    pub fn named(name: &str) -> Option<Palette> {
        BUILT_INS
            .iter()
            .find(|(known, _)| *known == name)
            .map(|(_, palette)| *palette)
    }

    /// What this palette is called, for `--check`, the help overlay and the
    /// flash that names a switch. A palette with `[colors]` on it matches no
    /// built-in and is "custom".
    pub fn name(&self) -> &'static str {
        BUILT_INS
            .iter()
            .find(|(_, known)| known == self)
            .map_or("custom", |(name, _)| *name)
    }

    /// The next palette in the shipped order, wrapping. What `^t` walks.
    pub fn next(&self) -> Palette {
        match BUILT_INS.iter().position(|(_, known)| known == self) {
            Some(at) => BUILT_INS[(at + 1) % BUILT_INS.len()].1,
            /* A palette with [colors] on it is in no walk, so the walk starts
               at its own beginning. Stepping off it to the *second* built-in
               would make `warm` the one theme `^t` could never reach. */
            None => BUILT_INS[0].1,
        }
    }

    /// Every name earworm knows, for the message that lists them.
    pub fn names() -> String {
        BUILT_INS
            .iter()
            .map(|(name, _)| *name)
            .collect::<Vec<_>>()
            .join(", ")
    }
}

impl Palette {
    /* One slot by name, for a config that repaints a colour or two on top of
       a built-in. Returning the field to write into keeps the name list in
       one place — `slots()` and this have to agree, and a mismatch is a key
       that parses and paints nothing. Every slot earworm holds is drawn in
       something, so every one of them can be set. */
    pub fn slot_mut(&mut self, slot: &str) -> Option<&mut Color> {
        Some(match slot {
            "text" => &mut self.text,
            "accent" => &mut self.accent,
            "cursor" => &mut self.cursor,
            "warn" => &mut self.warn,
            "error" => &mut self.error,
            "muted" => &mut self.muted,
            "rule" => &mut self.rule,
            "surface" => &mut self.surface,
            "unsure" => &mut self.unsure,
            "ink" => &mut self.ink,
            _ => return None,
        })
    }

    /// Every slot name a config may repaint, for the error that lists them.
    /* Asked of `slot_mut` rather than listed again, so the message cannot
       offer a slot that does nothing — or omit one that works. */
    pub fn slot_names() -> String {
        let mut probe = WARM;
        WARM.slots()
            .iter()
            .map(|(name, _)| *name)
            .filter(|name| probe.slot_mut(name).is_some())
            .collect::<Vec<_>>()
            .join(", ")
    }

    /* Which repainted colours are hard to read on this palette's own ground.
       A warning, never a refusal: it is the user's screen and their eyes, and
       a warning that blocks is one people learn to route around. */
    pub fn unreadable(&self) -> Vec<String> {
        let mut out = Vec::new();
        for (slot, color) in self.slots() {
            let Some(want) = wants(slot) else { continue };
            let Color::Rgb(r, g, b) = color else {
                continue;
            };
            for (where_, ground) in self.grounds(slot) {
                let ratio = contrast((r, g, b), ground);
                if ratio < want {
                    out.push(format!("{slot} on {where_} is {ratio:.1}:1, wants {want}:1"));
                    break;
                }
            }
        }
        out
    }

    /* What a slot is read against. Everything is read over the gradient and
       over the tone popups are raised with, except the band's ink, which only
       ever sits on `accent`. A ground nobody can measure drops out on its
       own: a surface that is not RGB used to return from this function, which
       silently switched off the gradient checks as well. */
    fn grounds(&self, slot: &str) -> Vec<(&'static str, (u8, u8, u8))> {
        let rgb = |color: Color| match color {
            Color::Rgb(r, g, b) => Some((r, g, b)),
            _ => None,
        };
        if slot == "ink" {
            return rgb(self.accent).map_or_else(Vec::new, |a| vec![("the band", a)]);
        }
        let mut out = vec![("the gradient", self.near), ("the gradient", self.far)];
        out.extend(rgb(self.surface).map(|s| ("popups", s)));
        out
    }
}

/* The ratio a slot has to clear, or `None` for one that is not read as text.
   4.5:1 is WCAG's bar for body text, which is what all of these are.

   Two are named rather than silently skipped. `rule` is structure, meant to
   be barely there. `surface` is a background and is read against nothing. */
fn wants(slot: &str) -> Option<f64> {
    match slot {
        "surface" => None,
        "rule" => Some(1.8),
        _ => Some(4.5),
    }
}

impl Default for Palette {
    fn default() -> Self {
        WARM
    }
}

impl Palette {
    pub fn background(&self, col: u16, row: u16, width: u16, height: u16) -> Color {
        // A single-cell span has nowhere to fade, and 0/0 is not a ratio.
        let frac = |v: u16, span: u16| {
            if span <= 1 {
                0.0
            } else {
                f32::from(v.min(span - 1)) / f32::from(span - 1)
            }
        };
        /* Normalised in cell space rather than pixels: terminal cells are
           about twice as tall as they are wide, so a true 45° would barely
           tilt. */
        let axis = (frac(col, width) + (1.0 - frac(row, height))) / 2.0;
        let t = (axis / self.stop).min(1.0);
        Color::Rgb(
            mix(self.near.0, self.far.0, t),
            mix(self.near.1, self.far.1, t),
            mix(self.near.2, self.far.2, t),
        )
    }

    /* Intro letters land rather than blink on, fading from the rule colour up
       to the accent over their own short window. Off the palette rather than
       hardcoded: the old pair was the warm theme's rule and a gold brighter
       than any slot, which on a light theme would fade dark-to-dark. */
    pub fn glow(&self, t: f32) -> Color {
        let t = t.clamp(0.0, 1.0);
        let (Color::Rgb(ar, ag, ab), Color::Rgb(br, bg, bb)) = (self.rule, self.accent) else {
            // A repainted slot that is not RGB has nothing to fade between.
            return self.accent;
        };
        Color::Rgb(mix(ar, br, t), mix(ag, bg, t), mix(ab, bb, t))
    }
}

fn mix(a: u8, b: u8, t: f32) -> u8 {
    (f32::from(a) + (f32::from(b) - f32::from(a)) * t).round() as u8
}

/* Every colour here is 24-bit, which a terminal that cannot do 24-bit renders
   by its own rules: the gradient turns to bands and cream can land on a grey
   that no longer reads as the brightest thing on screen. Resolved once and
   applied to the finished buffer, so the palette above stays one set of
   numbers rather than three. */
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Depth {
    Full,
    /// The xterm cube and its grey ramp, which is what `TERM=xterm-256color`
    /// promises without `COLORTERM`.
    Ansi256,
    /// NO_COLOR: the terminal's own two colours and nothing else.
    Plain,
}

/// Kept apart from the environment so the decision can be tested as the
/// decision it is, rather than as whatever the machine running the tests is.
pub fn depth_from(no_color: Option<&str>, colorterm: &str, term: &str) -> Depth {
    /* The convention is presence, not truth: any non-empty value means no
       colour, and setting it to "0" still means no. */
    if no_color.is_some_and(|v| !v.is_empty()) {
        return Depth::Plain;
    }
    let says = |v: &str, what: &str| v.to_lowercase().contains(what);
    if says(colorterm, "truecolor") || says(colorterm, "24bit") {
        return Depth::Full;
    }
    // Some terminals advertise it in TERM instead, and nowhere else.
    if says(term, "truecolor") || says(term, "direct") {
        return Depth::Full;
    }
    /* Assumed rather than detected, and deliberately the cautious way round:
       a truecolor terminal shown 256 colours loses some smoothness in the
       gradient, where the other way round loses the text. COLORTERM=truecolor
       is how to say otherwise. */
    Depth::Ansi256
}

pub fn depth() -> Depth {
    static DEPTH: OnceLock<Depth> = OnceLock::new();
    *DEPTH.get_or_init(|| {
        let var = |name: &str| std::env::var(name).unwrap_or_default();
        depth_from(
            std::env::var("NO_COLOR").ok().as_deref(),
            &var("COLORTERM"),
            &var("TERM"),
        )
    })
}

/// Whether colour is switched off altogether, which the widgets that paint a
/// background have to know: reversed video is what carries a filled band when
/// there is no colour to fill it with.
pub fn plain() -> bool {
    depth() == Depth::Plain
}

/// The xterm ramp, which is not linear: the first step is 95, not 51.
const RAMP: [u8; 6] = [0, 95, 135, 175, 215, 255];

fn nearest(value: u8, to: &[u8]) -> usize {
    to.iter()
        .enumerate()
        .min_by_key(|(_, v)| i16::from(**v).abs_diff(i16::from(value)))
        .map_or(0, |(at, _)| at)
}

/// The nearest of the 216-colour cube and the 24-step grey ramp, whichever is
/// closer: the greys are far finer than the cube, and most of this palette is
/// a near-grey.
pub fn quantise(r: u8, g: u8, b: u8) -> Color {
    let cube = [nearest(r, &RAMP), nearest(g, &RAMP), nearest(b, &RAMP)];
    let cube_rgb = [RAMP[cube[0]], RAMP[cube[1]], RAMP[cube[2]]];

    let grey_at = |n: usize| 8 + 10 * n as u8;
    let greys: Vec<u8> = (0..24).map(grey_at).collect();
    let grey = nearest(((u16::from(r) + u16::from(g) + u16::from(b)) / 3) as u8, &greys);
    let grey_rgb = [greys[grey]; 3];

    let apart = |a: [u8; 3]| {
        [(a[0], r), (a[1], g), (a[2], b)]
            .iter()
            .map(|(x, y)| u32::from(x.abs_diff(*y)).pow(2))
            .sum::<u32>()
    };
    if apart(grey_rgb) < apart(cube_rgb) {
        Color::Indexed(232 + grey as u8)
    } else {
        Color::Indexed(16 + 36 * cube[0] as u8 + 6 * cube[1] as u8 + cube[2] as u8)
    }
}

pub fn shade_at(depth: Depth, color: Color) -> Color {
    match (depth, color) {
        (Depth::Full, _) => color,
        // Reset, not black: the terminal's own two colours are the point.
        (Depth::Plain, _) => Color::Reset,
        (Depth::Ansi256, Color::Rgb(r, g, b)) => quantise(r, g, b),
        (Depth::Ansi256, _) => color,
    }
}

/// What a colour becomes on this terminal.
pub fn shade(color: Color) -> Color {
    shade_at(depth(), color)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The cube is coarse and the grey ramp is fine, and most of this palette
    /// sits nearer a grey than a cube corner.
    #[test]
    fn a_colour_lands_on_whichever_of_the_two_ramps_is_closer() {
        // Exactly a cube corner.
        assert_eq!(quantise(255, 0, 0), Color::Indexed(196));
        assert_eq!(quantise(0, 0, 0), Color::Indexed(16));
        // A near-grey: the cube would round it to 135,135,135 and lose it.
        assert_eq!(quantise(128, 128, 128), Color::Indexed(244));
        // Something saturated stays on the cube rather than going grey.
        let teal = quantise(96, 178, 158);
        assert!(
            matches!(teal, Color::Indexed(n) if (16..232).contains(&n)),
            "teal went grey: {teal:?}"
        );
    }

    /// Every index has to be one a terminal actually has.
    #[test]
    fn nothing_lands_outside_the_256_colours() {
        for r in (0..=255).step_by(17) {
            for g in (0..=255).step_by(17) {
                for b in (0..=255).step_by(17) {
                    match quantise(r, g, b) {
                        Color::Indexed(n) => assert!(n >= 16, "{r},{g},{b} hit a system colour"),
                        other => panic!("{r},{g},{b} gave {other:?}"),
                    }
                }
            }
        }
    }

    /* The cautious way round on purpose: a truecolor terminal shown 256
       colours loses smoothness, where the other way round loses the text. */
    #[test]
    fn the_depth_is_read_from_the_environment_the_way_round_that_is_safe() {
        assert_eq!(depth_from(None, "truecolor", "xterm"), Depth::Full);
        assert_eq!(depth_from(None, "24bit", "xterm"), Depth::Full);
        assert_eq!(depth_from(None, "", "xterm-direct"), Depth::Full);
        // 256color promises 256 and nothing more, whatever it can really do.
        assert_eq!(depth_from(None, "", "xterm-256color"), Depth::Ansi256);
        assert_eq!(depth_from(None, "", ""), Depth::Ansi256);

        // Presence, not truth: NO_COLOR=0 still means no colour.
        assert_eq!(depth_from(Some("1"), "truecolor", ""), Depth::Plain);
        assert_eq!(depth_from(Some("0"), "truecolor", ""), Depth::Plain);
        // An empty value is the one case that does not count.
        assert_eq!(depth_from(Some(""), "truecolor", ""), Depth::Full);
    }

    /// Back to RGB, for checking a quantised colour against the one it came
    /// from. The cube and the grey ramp have different formulas.
    fn rgb_of(color: Color) -> (u8, u8, u8) {
        let Color::Indexed(n) = color else {
            panic!("{color:?} is not an indexed colour");
        };
        if n >= 232 {
            let grey = 8 + 10 * (n - 232);
            return (grey, grey, grey);
        }
        let n = n - 16;
        (RAMP[n as usize / 36], RAMP[n as usize % 36 / 6], RAMP[n as usize % 6])
    }

    fn rgb(color: Color) -> (u8, u8, u8) {
        match color {
            Color::Rgb(r, g, b) => (r, g, b),
            other => panic!("{other:?} is not an RGB colour"),
        }
    }

    /* The palette is what has to survive the trip, not any colour: a cream
       that lands on a grey stops being the brightest thing on screen.

       The default alone, because this is a claim about hue and not about
       legibility. A saturated red or violet has no near neighbour in a cube
       whose steps are 40 apart — `light`'s error moves 40 and `neon`'s rule
       47, and there is no shade of either that does not — so holding every
       palette to this would mean shipping only near-greys. What must survive
       on all four is the contrast, which the test below measures after the
       same trip. */
    #[test]
    fn the_default_palette_survives_the_256_colour_cube() {
        for (slot, color) in Palette::default().slots() {
            let (r, g, b) = rgb(color);
            let (qr, qg, qb) = rgb_of(shade_at(Depth::Ansi256, color));
            let off = r.abs_diff(qr).max(g.abs_diff(qg)).max(b.abs_diff(qb));
            assert!(off <= 20, "{slot} moved {off} to ({qr},{qg},{qb})");
        }
    }

    /* The claim theme.rs has always made, now measured: every palette, every
       readable slot, against every ground it can land on. `grounds` is asked
       of the palette rather than listed here, so the band's ink is checked
       against the band and not against a gradient it never sits on. */
    #[test]
    fn every_palette_is_legible_on_its_own_ground() {
        for (theme, palette) in BUILT_INS {
            for (slot, color) in palette.slots() {
                let Some(want) = wants(slot) else { continue };
                for (ground, bg) in palette.grounds(slot) {
                    let ratio = contrast(rgb(color), bg);
                    assert!(
                        ratio >= want,
                        "{theme}: {slot} on {ground} is {ratio:.2}:1, wants {want}:1"
                    );
                }
            }
        }
    }

    /* A terminal without 24-bit colour gets the nearest cube or grey, and the
       nearest grey to a warm brown is a long way from it. 3:1 is WCAG's bar
       for large text and interface parts, which is the honest bar for a
       palette somebody else's terminal has already approximated. */
    #[test]
    fn every_palette_stays_legible_after_quantising() {
        for (theme, palette) in BUILT_INS {
            for (slot, color) in palette.slots() {
                if wants(slot).is_none() || slot == "rule" {
                    continue;
                }
                let ink = rgb_of(shade_at(Depth::Ansi256, color));
                for (ground, bg) in palette.grounds(slot) {
                    let ground_rgb =
                        rgb_of(shade_at(Depth::Ansi256, Color::Rgb(bg.0, bg.1, bg.2)));
                    let ratio = contrast(ink, ground_rgb);
                    assert!(
                        ratio >= 3.0,
                        "{theme}: {slot} on {ground} quantises to {ratio:.2}:1"
                    );
                }
            }
        }
    }

    /* The default palette is the one earworm has always drawn in: the move to
       role names was a rename, and a rename that changes a colour is not a
       rename. Pinned by value so a future edit has to mean it. */
    #[test]
    fn the_default_palette_is_the_warm_one_unchanged() {
        assert_eq!(Palette::default(), WARM);
        assert_eq!(WARM.text, Color::Rgb(236, 223, 192));
        assert_eq!(WARM.accent, Color::Rgb(214, 180, 96));
        assert_eq!(WARM.cursor, Color::Rgb(96, 178, 158));
        assert_eq!(WARM.warn, Color::Rgb(214, 154, 78));
        /* The one slot that moved, and the reason: 3.73:1 over `near` failed
           the bar this file claims, on the colour a failed track is read in. */
        assert_eq!(WARM.error, Color::Rgb(214, 126, 112));
        assert_eq!(WARM.muted, Color::Rgb(158, 148, 128));
        assert_eq!(WARM.rule, Color::Rgb(92, 84, 66));
        assert_eq!(WARM.surface, Color::Rgb(46, 41, 33));
        assert_eq!(WARM.unsure, Color::Rgb(168, 155, 126));
        assert_eq!(WARM.ink, Color::Rgb(24, 18, 16));
        assert_eq!((WARM.near, WARM.far, WARM.stop), ((55, 40, 37), (18, 1, 22), 0.76));
    }

    /* A palette with [colors] on it is in no walk. Stepping off it has to
       reach the first built-in, or `warm` becomes the one theme `^t` can
       never get back to. */
    #[test]
    fn a_repainted_palette_steps_onto_the_first_built_in() {
        let mut custom = NEON;
        custom.cursor = Color::Rgb(0, 255, 136);
        assert_eq!(custom.name(), "custom");
        assert_eq!(custom.next(), BUILT_INS[0].1);
        // And the walk itself still goes round the houses and back.
        let mut at = WARM;
        for _ in 0..BUILT_INS.len() {
            at = at.next();
        }
        assert_eq!(at, WARM);
    }

    /* Every slot earworm holds is drawn in something, so every one of them
       can be repainted and every one is offered. A slot the struct gains
       without a `slot_mut` arm is a key that parses and paints nothing. */
    #[test]
    fn every_slot_the_palette_holds_can_be_repainted() {
        let mut palette = WARM;
        for (slot, _) in WARM.slots() {
            assert!(palette.slot_mut(slot).is_some(), "{slot} cannot be set");
            assert!(Palette::slot_names().contains(slot), "{slot} is not offered");
        }
        assert!(palette.slot_mut("gradient").is_none(), "an unknown slot took a colour");
    }

    /* A ground nobody can measure drops out of the check on its own — it used
       to take the gradient checks with it. */
    #[test]
    fn an_unmeasurable_surface_does_not_switch_off_the_other_grounds() {
        let mut palette = WARM;
        palette.surface = Color::Indexed(4);
        palette.muted = Color::Rgb(58, 58, 58);
        let notes = palette.unreadable();
        assert!(notes.iter().any(|n| n.contains("gradient")), "{notes:?}");
    }

    /* The band's ink is read on the band and nowhere else. Measured against
       the gradient it would pass while being invisible on the thing it is
       actually drawn on, which is the whole point of asking the palette. */
    #[test]
    fn the_bands_ink_is_measured_against_the_band() {
        let grounds = WARM.grounds("ink");
        assert_eq!(grounds, vec![("the band", rgb(WARM.accent))]);

        let mut palette = WARM;
        // The band's own gold, which is unreadable on itself.
        palette.ink = WARM.accent;
        let notes = palette.unreadable();
        assert!(notes.iter().any(|n| n.contains("ink on the band")), "{notes:?}");
    }

    /* The gradient reads off the palette now, so its ends have to be the
       palette's ends: a theme whose ground does not move with its ink is the
       one way a light theme becomes unreadable. */
    #[test]
    fn the_gradient_runs_between_the_palettes_own_stops() {
        // Bottom-left is `near`; the far corner is flat past the stop.
        assert_eq!(WARM.background(0, 9, 10, 10), Color::Rgb(55, 40, 37));
        assert_eq!(WARM.background(9, 0, 10, 10), Color::Rgb(18, 1, 22));
        // And a light theme's ground moves with it, rather than staying dark.
        assert_eq!(LIGHT.background(0, 9, 10, 10), Color::Rgb(247, 243, 236));
    }

    /* The intro's fade runs between two of the palette's own slots. Hardcoded
       it was the warm theme's rule and a gold brighter than any slot in it,
       which on `light` would have faded dark into dark. */
    #[test]
    fn the_intro_fades_between_the_palettes_own_rule_and_accent() {
        for (theme, palette) in BUILT_INS {
            assert_eq!(palette.glow(0.0), palette.rule, "{theme} starts elsewhere");
            assert_eq!(palette.glow(1.0), palette.accent, "{theme} lands elsewhere");
            // Out of range is clamped rather than extrapolated off the ramp.
            assert_eq!(palette.glow(-1.0), palette.rule, "{theme} ran backwards");
            assert_eq!(palette.glow(9.0), palette.accent, "{theme} overshot");
        }
    }

    /// Every colour goes, so the words and the marks have to carry it.
    #[test]
    fn no_colour_leaves_nothing_for_a_terminal_to_render() {
        for (theme, palette) in BUILT_INS {
            for (slot, color) in palette.slots() {
                assert_eq!(
                    shade_at(Depth::Plain, color),
                    Color::Reset,
                    "{theme}: {slot} kept its colour"
                );
            }
        }
        assert_eq!(shade_at(Depth::Plain, Color::Indexed(4)), Color::Reset);
        assert_eq!(shade_at(Depth::Full, WARM.text), WARM.text);
    }
}
