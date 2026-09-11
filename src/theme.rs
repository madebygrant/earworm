use ratatui::style::Color;

/* Warm cream-on-dark: gold carries structure, green marks where you are, and
   everything secondary is a muted sand rather than a grey. Text and ground
   are both warm, so nothing on screen fights the gold. */

/// Track titles and anything the eye should land on first.
pub const CREAM: Color = Color::Rgb(236, 223, 192);
/// Chrome that should read as chrome: borders, bars, the spinner.
pub const GOLD: Color = Color::Rgb(230, 211, 138);
/// Where the cursor is, and work that came out right.
pub const GREEN: Color = Color::Rgb(79, 138, 91);
/// Something to look at but not an error.
pub const AMBER: Color = Color::Rgb(214, 154, 78);
/// A real failure.
pub const RED: Color = Color::Rgb(196, 106, 92);
/// Secondary text: sources, notes, hints.
pub const DIM: Color = Color::Rgb(128, 119, 102);
/// Structure that must not compete with the list.
pub const RULE: Color = Color::Rgb(92, 84, 66);
/// Identified, but nothing was found to identify it as.
pub const SAND: Color = Color::Rgb(168, 155, 126);

/* Diagonal gradient behind everything, running bottom-left to top-right:
   a warm clay fading to near-black plum. The hues of
   linear-gradient(45deg, hsla(10,19%,36%,1) 0%, hsla(290,95%,9%,1) 76%) at
   half brightness, so cream stays the brightest thing on screen. */
const NEAR: (u8, u8, u8) = (55, 40, 37);
const FAR: (u8, u8, u8) = (18, 1, 22);
/// Where the CSS stop sits: past it the whole corner is flat `FAR`.
const STOP: f32 = 0.76;

/// Intro letters land rather than blink on, fading from the rule colour up to
/// gold over their own short window.
pub fn glow(t: f32) -> Color {
    let t = t.clamp(0.0, 1.0);
    let mix = |a: u8, b: u8| (f32::from(a) + (f32::from(b) - f32::from(a)) * t).round() as u8;
    Color::Rgb(mix(92, 230), mix(84, 211), mix(66, 138))
}

/// Text on a filled band, where the gradient is covered and cream would glare.
pub const INK: Color = Color::Rgb(24, 18, 16);

/// Popups sit above the gradient, so they need one flat tone of their own or
/// they read as a hole rather than a raised surface.
pub const SURFACE: Color = Color::Rgb(46, 41, 33);

pub fn background(col: u16, row: u16, width: u16, height: u16) -> Color {
    // A single-cell span has nowhere to fade, and 0/0 is not a ratio.
    let frac = |v: u16, span: u16| {
        if span <= 1 {
            0.0
        } else {
            f32::from(v.min(span - 1)) / f32::from(span - 1)
        }
    };
    /* Normalised in cell space rather than pixels: terminal cells are about
       twice as tall as they are wide, so a true 45° would barely tilt. */
    let axis = (frac(col, width) + (1.0 - frac(row, height))) / 2.0;
    let t = (axis / STOP).min(1.0);
    let mix = |a: u8, b: u8| (f32::from(a) + (f32::from(b) - f32::from(a)) * t).round() as u8;
    Color::Rgb(
        mix(NEAR.0, FAR.0),
        mix(NEAR.1, FAR.1),
        mix(NEAR.2, FAR.2),
    )
}
