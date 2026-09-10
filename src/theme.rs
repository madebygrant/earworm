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

/* Vertical gradient behind everything: warm charcoal at the header fading to
   near-black at the status bar. Dark enough at every row that cream still
   reads as the brightest thing on screen. */
const TOP: (u8, u8, u8) = (34, 30, 24);
const BOTTOM: (u8, u8, u8) = (16, 15, 12);

/// Popups sit above the gradient, so they need one flat tone of their own or
/// they read as a hole rather than a raised surface.
pub const SURFACE: Color = Color::Rgb(46, 41, 33);

pub fn background(row: u16, height: u16) -> Color {
    // A one-row frame has nowhere to fade, and 0/0 is not a ratio.
    let t = if height <= 1 {
        0.0
    } else {
        f32::from(row.min(height - 1)) / f32::from(height - 1)
    };
    let mix = |a: u8, b: u8| (f32::from(a) + (f32::from(b) - f32::from(a)) * t).round() as u8;
    Color::Rgb(
        mix(TOP.0, BOTTOM.0),
        mix(TOP.1, BOTTOM.1),
        mix(TOP.2, BOTTOM.2),
    )
}
