use ratatui::style::Color;

/* Warm cream-on-dark: gold carries structure, green marks where you are, and
   everything secondary is a muted sand rather than a grey. The background is
   deliberately never set, so the terminal's own stays underneath. */

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
