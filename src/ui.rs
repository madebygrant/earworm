use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{
    Block, Borders, Clear, List, ListItem, ListState, Paragraph, Scrollbar, ScrollbarOrientation,
    ScrollbarState,
};
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

use crate::app::{self, App, Confirm, Player, Prompt, Shelf, Status, View};
use crate::manifest;

use crate::theme::{self, Palette};
use crate::update;

const SPINNER: [&str; 8] = ["⣷", "⣯", "⣟", "⡿", "⢿", "⣻", "⣽", "⣾"];
const STATUS_WIDTH: usize = 8;
/// Narrowest library screen that still has room for a preview beside the rows.
const PREVIEW_FROM: u16 = 100;
/// Width of `  NNN missing`, held open on rows with nothing missing.
const MISSING_WIDTH: usize = 13;
/// As much of cliamp's track title as the bar will spend on it. Long enough
/// to recognise a song, short enough that it is not what pushed the counts
/// off the row.
const NOW_PLAYING: usize = 28;

/* Columns, not characters. A Korean or Japanese glyph takes two cells and a
   combining mark takes none, so a column padded to a character count is as
   ragged as the title in it is wide: every CJK row in a list overshoots by
   its own length and the dim columns after it step out of line. */
fn cols(text: &str) -> usize {
    UnicodeWidthStr::width(text)
}

fn dim(text: impl Into<String>, p: Palette) -> Span<'static> {
    Span::styled(text.into(), Style::new().fg(p.muted))
}

/// The row under the cursor is what the next key acts on, so its name is bold
/// as well as marked. Bold is safe here because every colour is RGB: a
/// terminal cannot swap it for a bright ANSI variant.
fn row_style(selected: bool, p: Palette) -> Style {
    let style = Style::new().fg(p.text);
    if selected { style.add_modifier(Modifier::BOLD) } else { style }
}

pub fn draw(frame: &mut Frame, app: &mut App) {
    let p = app.theme;
    // Takes the whole frame: a header above it would be the thing you read.
    if app.intro() {
        draw_background(frame, p);
        draw_intro(frame, app.started.elapsed().as_millis() as u64, app.update.as_deref(), p);
        return;
    }

    let [header, rule, body, footrule, status] = Layout::vertical([
        Constraint::Length(1),
        Constraint::Length(1),
        Constraint::Min(1),
        Constraint::Length(1),
        Constraint::Length(1),
    ])
    .areas(frame.area());

    draw_background(frame, p);
    draw_header(frame, app, header);
    // The pick wins: it says what the keys do, and it carries the filter.
    if app.picking.is_some() {
        draw_pick_bar(frame, app, rule);
    } else if app.filtering() {
        draw_filter_bar(frame, app, rule);
    } else {
        draw_rule(frame, rule, p);
    }
    if app.show_logs && !app.logs.is_empty() {
        let rows = app.log_rows(body.height as usize) as u16;
        let [list, logs] =
            Layout::vertical([Constraint::Min(1), Constraint::Length(rows)]).areas(body);
        draw_body(frame, app, list);
        draw_logs(frame, app, logs);
    } else {
        draw_body(frame, app, body);
    }
    draw_rule(frame, footrule, p);
    draw_status(frame, app, status);

    if app.show_help {
        draw_help(frame, app);
    }
    if app.prompt.is_some() {
        draw_prompt(frame, app);
    }
    if let Some(what) = app.confirm {
        draw_confirm(frame, app, what);
    }
    recolour(frame);
}

/* One pass over the finished buffer rather than three palettes: every colour
   in `theme` stays one set of numbers, and a terminal that cannot render them
   gets the nearest thing it has. Costs a walk of the cells already drawn. */
fn recolour(frame: &mut Frame) {
    if theme::depth() == theme::Depth::Full {
        return;
    }
    let area = frame.area();
    let buffer = frame.buffer_mut();
    for y in area.top()..area.bottom() {
        for x in area.left()..area.right() {
            let cell = &mut buffer[(x, y)];
            let (fg, bg) = (theme::shade(cell.fg), theme::shade(cell.bg));
            cell.set_fg(fg);
            cell.set_bg(bg);
        }
    }
}

/* Its own popup rather than a `Prompt`: no worker is blocked on the answer,
   so it carries no reply channel and no cursor. The keys are spelled out
   instead of driven, which is one row and no state. */
fn draw_confirm(frame: &mut Frame, app: &App, what: Confirm) {
    let p = app.theme;
    let Confirm::Quit = what;
    let (done, total) = app.progress();
    let mut lines = vec![Line::from(Span::styled(
        format!(" {done} of {total} finished so far"),
        Style::new().fg(p.text),
    ))];
    // What it costs, not what it does: "quit?" is already in the title.
    lines.push(Line::from(dim(" leaving now stops the download", p)));
    lines.push(Line::default());
    lines.push(Line::from(vec![
        Span::styled(" q  quit", Style::new().fg(p.warn)),
        dim("     esc  keep going", p),
    ]));
    let width = lines.iter().map(|l| l.width() as u16).max().unwrap_or(0) + 3;
    popup(frame, "quit?", lines, width, p);
}

/* Painted before anything else: every other widget styles only its foreground,
   so the gradient survives underneath them. */
fn draw_background(frame: &mut Frame, p: Palette) {
    if theme::plain() {
        return;
    }
    let area = frame.area();
    let buffer = frame.buffer_mut();
    for y in area.top()..area.bottom() {
        for x in area.left()..area.right() {
            let bg = p.background(x - area.left(), y - area.top(), area.width, area.height);
            buffer[(x, y)].set_bg(bg);
        }
    }
}

/// Five rows per glyph, all five columns wide, for `EARWORM`.
const GLYPHS: [[&str; 5]; 7] = [
    ["█████", "█    ", "████ ", "█    ", "█████"],
    [" ███ ", "█   █", "█████", "█   █", "█   █"],
    ["████ ", "█   █", "████ ", "█  █ ", "█   █"],
    ["█   █", "█   █", "█ █ █", "██ ██", "█   █"],
    [" ███ ", "█   █", "█   █", "█   █", " ███ "],
    ["████ ", "█   █", "████ ", "█  █ ", "█   █"],
    ["█   █", "██ ██", "█ █ █", "█   █", "█   █"],
];
const MARK_WIDTH: u16 = 41;
const MARK_HEIGHT: u16 = 5;
/// One glyph lands every 90ms, each fading in over the 220ms after its turn.
const LETTER: u64 = 90;
const FADE: f32 = 220.0;
const WAVE: [char; 8] = ['▁', '▂', '▃', '▄', '▅', '▆', '▇', '█'];

/* Drawn from the elapsed milliseconds alone, so it runs at the same speed
   whether the event loop is idling at its poll timeout or spinning on
   messages, and skipping it costs nothing that was ever computed. */
/* What the sign-off says earworm is for, to somebody who has just run it for
   the first time and is watching the wordmark land. Kept short because the
   update notice is appended to it and `truncate` drops the tail: on a narrow
   terminal a long one eats the command that acts on the news. A constant
   rather than a literal, since the tests assert on it in three places. */
pub const TAGLINE: &str = "playlist in, library out";

fn draw_intro(frame: &mut Frame, ms: u64, update: Option<&str>, p: Palette) {
    let area = frame.area();
    // Below this the wordmark would wrap into nonsense, so only the wave runs.
    let big = area.width >= MARK_WIDTH + 2 && area.height >= MARK_HEIGHT + 6;
    let mark_rows = if big { MARK_HEIGHT } else { 1 };
    let block = mark_rows + 4;
    if area.height < block {
        return;
    }

    let top = area.y + (area.height - block) / 2;
    // Clamped, or a Rect wider than the frame reaches past the buffer.
    let width = if big { MARK_WIDTH } else { 7 }.min(area.width);
    let left = area.x + (area.width - width) / 2;

    if big {
        for row in 0..MARK_HEIGHT {
            let mut spans = Vec::new();
            for (n, glyph) in GLYPHS.iter().enumerate() {
                let due = n as u64 * LETTER;
                if ms < due {
                    break;
                }
                if n > 0 {
                    spans.push(Span::raw(" "));
                }
                let lit = p.glow((ms - due) as f32 / FADE);
                spans.push(Span::styled(glyph[row as usize], Style::new().fg(lit)));
            }
            frame.render_widget(
                Paragraph::new(Line::from(spans)),
                Rect::new(left, top + row, width, 1),
            );
        }
    } else {
        frame.render_widget(
            Paragraph::new(Line::from(Span::styled(
                "earworm",
                Style::new().fg(p.glow(ms as f32 / FADE)),
            ))),
            Rect::new(left, top, width, 1),
        );
    }

    /* Starts once the last letter has landed and grows in, so the eye follows
       the word first and the wave second rather than competing for both. */
    let wave_at = GLYPHS.len() as u64 * LETTER;
    if ms > wave_at && area.width >= 8 {
        let grown = ((ms - wave_at) as f32 / 400.0).min(1.0);
        let phase = ms as f32 / 90.0;
        let span = width as usize;
        let wave: String = (0..span)
            .map(|x| {
                let height = (phase - x as f32 / 2.6).sin() * 3.5 * grown + 3.5;
                WAVE[(height.round().max(0.0) as usize).min(7)]
            })
            .collect();
        frame.render_widget(
            Paragraph::new(Line::from(Span::styled(wave, Style::new().fg(p.warn)))),
            Rect::new(left, top + mark_rows + 1, width, 1),
        );
    }

    // Last, so it reads as a signature rather than part of the animation.
    if ms > wave_at + 250 {
        let mut sign = format!("{TAGLINE}  ·  v{}", update::current());
        if let Some(latest) = update {
            /* The numbers lead and the hint yields: a narrow terminal keeps
               the news and drops the way to act on it, not the other round. */
            sign = format!("{sign} → v{latest}  ·  {}", update::hint());
        }
        let sign = truncate(&sign, area.width as usize);
        let at = area.x + (area.width - cols(&sign) as u16) / 2;
        frame.render_widget(
            Paragraph::new(Line::from(dim(sign, p))),
            Rect::new(at, top + mark_rows + 3, area.right() - at, 1),
        );
    }
}

fn draw_rule(frame: &mut Frame, area: Rect, p: Palette) {
    frame.render_widget(
        Paragraph::new(Span::styled(
            "─".repeat(area.width as usize),
            Style::new().fg(p.rule),
        )),
        area,
    );
}

/* A mode where the list is not the whole playlist, or the keys do not mean
   what they usually mean, takes the rule's row as a filled band rather than a
   hint among hints. It deliberately covers the gradient, which is what makes
   it impossible to miss. */
fn draw_band(frame: &mut Frame, area: Rect, left: String, mut right: String, p: Palette) {
    let width = area.width as usize;
    /* Reversed rather than filled when there is no colour: the band is the
       one thing on screen that must not be missable, and a mode nobody can
       see is worse than a mode drawn in the wrong style. */
    let band = if theme::plain() {
        Style::new().add_modifier(Modifier::REVERSED)
    } else {
        Style::new().fg(p.ink).bg(p.accent)
    };
    let used = cols(&left);
    let mut spans = vec![Span::styled(left, band.add_modifier(Modifier::BOLD))];
    /* Paragraph styles only the cells it writes, so the padding is what keeps
       the gradient out. It runs to the full width even when that costs the
       hint: a torn band reads as a rendering fault. */
    let rest = width.saturating_sub(used);
    let gap = if used + cols(&right) <= width {
        rest - cols(&right)
    } else {
        right.clear();
        rest
    };
    spans.push(Span::styled(" ".repeat(gap), band));
    spans.push(Span::styled(right, band));
    frame.render_widget(Paragraph::new(Line::from(spans)), area);
}

/* One band for both narrowings and both screens: what it has to say is that
   the list is not all of it, which is the same news whichever narrowed it. */
fn draw_filter_bar(frame: &mut Frame, app: &App, area: Rect) {
    let p = app.theme;
    let (shown, total) = match app.view {
        View::Library => (app.shelf_rows().len(), app.library.len()),
        // Every track in the library is what a search was asked of, not the
        // tracks of whichever playlist happens to be open behind it.
        View::Found => (
            app.found_rows().len(),
            app.library.iter().map(|s| s.files.len()).sum(),
        ),
        View::Tracks => (app.shown(), app.tracks.len()),
    };
    let caret = if app.typing_filter { "\u{2588}" } else { "" };
    // The review has no text to show, so it names itself instead.
    let left = if app.review && app.filter.is_empty() && !app.typing_filter {
        " REVIEW  tracks nothing confirmed".to_string()
    } else {
        let word = match (app.view, app.review) {
            // Its own word, because the box is the list here rather than a
            // narrowing of one: an empty query is an empty screen.
            (View::Found, _) => "SEARCH",
            (_, true) => "REVIEW",
            (_, false) => "FILTER",
        };
        format!(" {word}  /{}{}", app.filter, caret)
    };
    let right = if app.typing_filter {
        "enter keeps  ·  esc clears ".to_string()
    } else {
        format!("{shown} of {total} shown  ·  esc clears ")
    };
    draw_band(frame, area, left, right, p);
}

/* Carries the filter too, since the pick band replaces it: a narrowed list
   with nothing saying so is what `a` would then mark the wrong amount of. */
fn draw_pick_bar(frame: &mut Frame, app: &App, area: Rect) {
    let p = app.theme;
    let mut left = format!(" PICK  {} of {} selected", app.marked.len(), app.tracks.len());
    if !app.filter.is_empty() || app.typing_filter {
        let caret = if app.typing_filter { "\u{2588}" } else { "" };
        left = format!("{left}  /{}{}", app.filter, caret);
    }
    draw_band(
        frame,
        area,
        left,
        "space  ·  a all  ·  i invert  ·  enter downloads  ·  esc none ".to_string(),
        p,
    );
}

fn draw_header(frame: &mut Frame, app: &App, area: Rect) {
    let p = app.theme;
    let mut spans = vec![Span::styled(" earworm", Style::new().fg(p.accent))];
    // The open playlist is not what is on screen while the library is.
    if !app.playlist.is_empty() && app.view == View::Tracks {
        spans.push(dim("  ·  ", p));
        spans.push(Span::styled(app.playlist.clone(), Style::new().fg(p.text)));
    }
    spans.push(dim("  ·  ", p));
    // The stage belongs to the last run, which the library screen is not.
    let stage = match app.view {
        View::Library if !app.busy => "library".to_string(),
        View::Found if !app.busy => "search".to_string(),
        _ => app.stage.clone(),
    };
    spans.push(Span::styled(stage, Style::new().fg(p.text)));

    if app.view == View::Found {
        /* How many folders the matches are spread over, which is the thing
           the flat list cannot say and the reason to look at it at all. */
        let rows = app.found_rows();
        let folders: std::collections::HashSet<usize> =
            rows.iter().map(|(shelf, _)| *shelf).collect();
        let tracks = if rows.len() == 1 { "track" } else { "tracks" };
        let plural = if folders.len() == 1 { "playlist" } else { "playlists" };
        spans.push(dim("  ·  ", p));
        spans.push(Span::styled(
            format!("{} {tracks} in {} {plural}", rows.len(), folders.len()),
            Style::new().fg(p.text),
        ));
    } else if app.view == View::Library {
        let count = app.library.len();
        let plural = if count == 1 { "playlist" } else { "playlists" };
        spans.push(dim("  ·  ", p));
        spans.push(Span::styled(
            format!("{count} {plural}"),
            Style::new().fg(p.text),
        ));
    } else if !app.tracks.is_empty() {
        spans.push(dim("  ·  ", p));
        // How much of the run is showing is the filter band's to say.
        spans.push(Span::styled(
            format!("{}/{}", app.settled(), app.tracks.len()),
            Style::new().fg(p.text),
        ));
    }
    /* Movement the track rows cannot show: a slow lookup still looks alive,
       and a bulk action over fifty tracks explains why the keys went quiet.
       The library has no run behind it, so an idle one must not appear busy.
       A pick is waiting on a keypress, so nothing is moving behind it. */
    if app.picking.is_none() && ((app.done.is_none() && app.view == View::Tracks) || app.busy) {
        spans.push(Span::styled(
            format!("  {}", SPINNER[(app.tick / 2) % SPINNER.len()]),
            Style::new().fg(p.accent),
        ));
    }
    /* The right half was empty on every screen, which is the widest unused
       space in the layout. It holds the two things the list cannot say: how
       far along the whole run is, and what this run is doing to the files.
       Both give way before the left half, which names what you are looking
       at. */
    let mut right = Vec::new();
    let (done, total) = app.progress();
    if app.view == View::Tracks && total > 0 && app.done.is_none() {
        let (filled, rest) = bar(((done * 100) / total) as u16, 10);
        right.push(Span::styled(filled, Style::new().fg(p.accent)));
        right.push(dim(rest, p));
        if let Some(left) = app.eta() {
            right.push(dim(format!(" {} left", brief(left)), p));
        }
    } else if app.view == View::Library || app.done.is_some() {
        right.push(dim(app.settings.clone(), p));
    }

    let used: usize = spans.iter().map(|s| cols(&s.content)).sum();
    let wanted: usize = right.iter().map(|s| cols(&s.content)).sum();
    let width = area.width as usize;
    // Dropped whole rather than truncated: half a progress bar is a lie.
    if used + wanted + 2 <= width {
        spans.push(Span::raw(" ".repeat(width - used - wanted - 1)));
        spans.extend(right);
    }
    frame.render_widget(Paragraph::new(Line::from(spans)), area);
}

/// Coarse on purpose: the estimate behind it is, and `4m` claims less than
/// `4m 12s` does.
fn brief(left: std::time::Duration) -> String {
    let secs = left.as_secs();
    match secs {
        s if s < 90 => format!("{s}s"),
        s if s < 3600 => format!("{}m", s / 60),
        s => format!("{}h{:02}m", s / 3600, (s % 3600) / 60),
    }
}

/// Eighth-block fill, so a bar eight cells wide still moves on every percent.
fn bar(percent: u16, width: usize) -> (String, String) {
    let filled = (percent.min(100) as f32 / 100.0) * width as f32;
    let full = filled.floor() as usize;
    let mut done = "━".repeat(full.min(width));
    if full < width {
        let part = ((filled - full as f32) * 8.0) as usize;
        if part > 0 {
            done.push(['─', '╴', '╴', '╌', '╌', '╍', '╍', '━'][part]);
        }
    }
    let rest = width.saturating_sub(cols(&done));
    (done, "─".repeat(rest))
}

fn draw_body(frame: &mut Frame, app: &mut App, area: Rect) {
    match app.view {
        View::Tracks => draw_tracks(frame, app, area),
        View::Library => draw_library(frame, app, area),
        View::Found => draw_found(frame, app, area),
    }
}

/* Every matching track in the library, flat, with the folder it sits in. The
   library screen answers "which folders hold a match" and its preview answers
   "which tracks" one folder at a time; this is the same question asked of the
   whole library at once, which is what a fifty-folder library needs. */
fn draw_found(frame: &mut Frame, app: &mut App, area: Rect) {
    let p = app.theme;
    let rows = app.found_rows();
    if rows.is_empty() {
        let said = match app.filter.is_empty() {
            true => " type to search every folder   esc goes back".to_string(),
            false => format!(" no track matches {}   esc goes back", app.filter),
        };
        frame.render_widget(Paragraph::new(Line::from(dim(said, p))), area);
        return;
    }
    let width = area.width as usize;
    app.viewport = area.height as usize;

    /* One folder column for the list, so the titles beside it line up: the
       folder is the answer the screen exists to give, and a ragged column
       makes it the thing that is hardest to read. */
    let longest = rows
        .iter()
        .map(|(shelf, _)| cols(&app.library[*shelf].name))
        .max()
        .unwrap_or(0);
    let name_width = longest.min(width.saturating_sub(20).max(10));

    let at = rows.iter().position(|row| Some(*row) == app.found);
    let items: Vec<ListItem> = rows
        .iter()
        .map(|row| {
            let (shelf, file) = *row;
            let shelf = &app.library[shelf];
            let (name, here) = &shelf.files[file];
            let folder = truncate(&shelf.name, name_width);
            let pad = name_width.saturating_sub(cols(&folder));
            // The number and extension are the filename's, not the track's.
            let title = name.rsplit_once('.').map_or(name.as_str(), |(stem, _)| stem);
            let selected = app.found == Some(*row);
            ListItem::new(Line::from(vec![
                Span::styled(if selected { "▌" } else { " " }, Style::new().fg(p.cursor)),
                // Same mark the library preview uses, for the same reason.
                Span::styled(
                    if *here { "  " } else { " !" },
                    Style::new().fg(if *here { p.muted } else { p.warn }),
                ),
                dim(format!("{folder}{:pad$}", ""), p),
                Span::styled(
                    format!("  {}", truncate(title, width.saturating_sub(name_width + 5))),
                    row_style(selected, p),
                ),
            ]))
        })
        .collect();

    app.found_scroll = app::scroll_to(
        app.found_scroll,
        at.unwrap_or(0),
        rows.len(),
        area.height as usize,
    );
    let mut state = ListState::default().with_offset(app.found_scroll);
    state.select(at);
    frame.render_stateful_widget(List::new(items), area, &mut state);

    if rows.len() > area.height as usize {
        let mut bar_state = ScrollbarState::new(rows.len()).position(at.unwrap_or(0));
        frame.render_stateful_widget(
            Scrollbar::new(ScrollbarOrientation::VerticalRight)
                .begin_symbol(None)
                .end_symbol(None)
                .thumb_style(Style::new().fg(p.rule)),
            area,
            &mut bar_state,
        );
    }
}

/* One row per playlist folder, all of it from the manifests: the counts are
   what is on disk, and the sync time is the only thing that says whether that
   still matches the playlist upstream. */
fn draw_library(frame: &mut Frame, app: &mut App, area: Rect) {
    let p = app.theme;
    /* Only reachable by emptying --dir while the screen is open: a bare start
       with no playlists asks for a URL instead of showing this. */
    if app.library.is_empty() {
        frame.render_widget(
            Paragraph::new(Line::from(dim(" no playlists here now   n syncs one", p))),
            area,
        );
        return;
    }

    let rows = app.shelf_rows();
    if rows.is_empty() {
        frame.render_widget(
            Paragraph::new(Line::from(dim(format!(
                " nothing matches {}   esc clears it",
                app.filter
            ), p))),
            area,
        );
        return;
    }
    /* Resolved once and shared with the pane below, so the row the marker is
       on and the folder the pane describes cannot become two questions. */
    let at = rows.iter().position(|pos| *pos == app.shelf).unwrap_or(0);
    let selected = rows[at];

    /* The shelf row already needs name plus 40 columns of counts and sync
       time, so the pane only appears where both fit without squeezing names
       down to nothing. Below that it is simply absent. */
    let area = if area.width >= PREVIEW_FROM {
        // u32: the product overflows u16 past 32767 columns.
        let pane = (u32::from(area.width) * 2 / 5).min(60) as u16;
        let [list, preview] =
            Layout::horizontal([Constraint::Min(1), Constraint::Length(pane)]).areas(area);
        draw_preview(frame, &app.library[selected], &app.filter, &app.format, preview, p);
        list
    } else {
        area
    };

    let width = area.width as usize;
    app.viewport = area.height as usize;
    let longest = rows
        .iter()
        .map(|pos| cols(&app.library[*pos].name))
        .max()
        .unwrap_or(0);
    let name_width = longest.min(width.saturating_sub(40).max(12));

    let items: Vec<ListItem> = rows
        .iter()
        .map(|row| {
            let (row, shelf) = (*row, &app.library[*row]);
            let name = truncate(&shelf.name, name_width);
            let pad = name_width.saturating_sub(cols(&name));
            let mut spans = vec![
                Span::styled(
                    if row == selected { "▌" } else { " " },
                    Style::new().fg(p.cursor),
                ),
                // Bold as well as the marker: the bar alone is a thin signal
                // for "this is the folder Enter opens".
                Span::styled(format!(" {name}{:pad$}", ""), row_style(row == selected, p)),
                /* Two cells whether or not anything is playing, so the columns
                   after it do not step sideways as cliamp starts and stops. */
                match (app.playback.on(&shelf.path), app.playback.playing) {
                    (true, true) => Span::styled(" ▶", Style::new().fg(p.cursor)),
                    (true, false) => dim(" ⏸", p),
                    (false, _) => Span::raw("  "),
                },
                dim(format!("  {:>4} tracks", shelf.tracks), p),
                /* Files the manifest lists that are no longer there, which a
                   sync would download again. Worth colour: it is the one thing
                   on this screen that is a problem. A fixed slot either way,
                   or the sync column steps sideways between rows. */
                if shelf.missing > 0 {
                    Span::styled(format!("  {:>3} missing", shelf.missing), Style::new().fg(p.warn))
                } else {
                    Span::raw(" ".repeat(MISSING_WIDTH))
                },
            ];
            spans.push(dim(match shelf.synced {
                Some(at) => format!("  synced {}", manifest::ago(at)),
                None => "  never synced".into(),
            }, p));
            ListItem::new(Line::from(spans))
        })
        .collect();

    app.shelf_scroll = app::scroll_to(app.shelf_scroll, at, rows.len(), area.height as usize);
    let mut state = ListState::default().with_offset(app.shelf_scroll);
    state.select(Some(at));
    frame.render_stateful_widget(List::new(items), area, &mut state);

    if rows.len() > area.height as usize {
        let mut bar_state = ScrollbarState::new(rows.len()).position(at);
        frame.render_stateful_widget(
            Scrollbar::new(ScrollbarOrientation::VerticalRight)
                .begin_symbol(None)
                .end_symbol(None)
                .thumb_style(Style::new().fg(p.muted))
                .track_symbol(None),
            area,
            &mut bar_state,
        );
    }
}

/* Read-only and always from the top: giving the pane a cursor would mean a
   third view mode and a second set of keys, and Enter already opens the folder
   properly. What it is for is the `n missing` count on the shelf row, which
   says a sync would re-download something but never which one. */
fn draw_preview(frame: &mut Frame, shelf: &Shelf, filter: &str, format: &str, area: Rect, p: Palette) {
    let block = Block::new()
        .borders(Borders::LEFT)
        .border_style(Style::new().fg(p.rule));
    let inner = block.inner(area);
    frame.render_widget(block, area);
    if inner.width == 0 || inner.height == 0 {
        return;
    }

    if shelf.files.is_empty() {
        frame.render_widget(Paragraph::new(Line::from(dim(" nothing recorded", p))), inner);
        return;
    }

    let width = inner.width as usize;
    /* A folder can be on the list for a track inside it, and the row says
       only its name. Under a filter the pane is what answers which track, so
       it shows the ones that matched rather than the first screenful. */
    let needle = filter.to_lowercase();
    let matched: Vec<&(String, bool)> = match needle.is_empty() {
        true => Vec::new(),
        false => shelf
            .files
            .iter()
            .filter(|(name, _)| name.to_lowercase().contains(&needle))
            .collect(),
    };
    // A folder that matched on its own name shows what it always did.
    let files: Vec<&(String, bool)> = match matched.is_empty() {
        true => shelf.files.iter().collect(),
        false => matched,
    };

    /* A title line, because a bordered column of filenames floats: it says
       nothing about which folder it belongs to or how many rows were left
       out. The missing count is repeated here for the same reason. */
    let head = match (files.len() < shelf.files.len(), shelf.missing) {
        (true, _) => format!(" {} of {} tracks match", files.len(), shelf.tracks),
        (false, 0) => format!(" {} tracks", shelf.tracks),
        (false, n) => format!(" {} tracks  ·  {n} missing", shelf.tracks),
    };
    let mut lines = vec![Line::from(Span::styled(
        truncate(&head, width),
        Style::new().fg(if shelf.missing > 0 { p.warn } else { p.muted }),
    ))];
    /* Here rather than on the shelf row, which reserves fixed columns for
       everything it prints: a count that is zero for anyone not converting
       would cost every folder name ten columns for good. Dim, because it is
       a statement about the folder and not a problem with it. */
    let off = app::off_format(shelf, format);
    if off > 0 {
        lines.push(Line::from(dim(truncate(
            &format!(" {off} not {format}"),
            width,
        ), p)));
    }
    // Every line above the list, or the tail count runs off the bottom.
    let height = (inner.height as usize).saturating_sub(lines.len());
    // The last line goes to the tail count, so no track is silently dropped.
    let room = if files.len() > height {
        height.saturating_sub(1)
    } else {
        files.len()
    };

    lines.extend(files[..room]
        .iter()
        .map(|(name, here)| {
            let title = name.rsplit_once('.').map_or(name.as_str(), |(stem, _)| stem);
            /* Missing files are the reason the pane exists, so they get a
               gutter mark as well as the colour. */
            let mark = if *here { ' ' } else { '!' };
            Line::from(Span::styled(
                format!("{mark}{}", truncate(title, width.saturating_sub(1))),
                Style::new().fg(if *here { p.text } else { p.warn }),
            ))
        }));
    if room < files.len() {
        lines.push(Line::from(dim(format!(" … {} more", files.len() - room), p)));
    }
    frame.render_widget(Paragraph::new(lines), inner);
}

/// Seconds as a clock, since a track length is read as minutes and nobody
/// converts 251 in their head.
fn clock(seconds: u64) -> String {
    format!("{}:{:02}", seconds / 60, seconds % 60)
}

/// One labelled field, wrapped, with the continuation lines under the value
/// rather than under the label.
fn detail_rows(label: &str, value: &str, style: Style, width: usize, p: Palette) -> Vec<Line<'static>> {
    const LABEL: usize = 8;
    let room = width.saturating_sub(LABEL + 1).max(1);
    wrap(value, room)
        .into_iter()
        .enumerate()
        .map(|(n, piece)| {
            Line::from(vec![
                dim(format!(" {:pad$}", if n == 0 { label } else { "" }, pad = LABEL), p),
                Span::styled(piece, style),
            ])
        })
        .collect()
}

/* The row truncates everything that does not fit its columns, and what it
   truncates first is `was` at 32 characters: the video title, which is the
   half of a wrong identification that says how it went wrong. The path is
   never on the row at all. A wide terminal has the columns, so it says it. */
/* The pane's border, and with it the one derivation of what sits inside it.
   `art_pane` and `draw_detail` both need the inner rectangle, and two hand-
   rolled copies of "the area less the left border" is how they come to
   disagree by a column the day the border changes. */
fn detail_block(p: Palette) -> Block<'static> {
    Block::new()
        .borders(Borders::LEFT)
        .border_style(Style::new().fg(p.rule))
}

/* What the metadata needs under a picture: the wrapped name, a blank, and the
   rows that are always there. Below this the cover is what gives way, the same
   way the help overlay's legend does — the pane exists to say which track this
   is, and the words are what say it. */
const ART_FLOOR: u16 = 8;
/// Below this a cover is not worth the read, whatever protocol draws it.
const ART_MIN: u16 = 8;
/// The most either edge may ask for, so a wide pane cannot demand megabytes.
const COVER_PIXELS: u32 = 1024;

/* Where a cover goes, or `None` when the words need the rows. Square in cells
   is not square on screen, so the height is half the width: `Resize::Fit`
   corrects for the terminal's real font size and would otherwise letterbox
   inside an area twice as tall as the picture. */
fn art_pane(inner: Rect) -> Option<Rect> {
    let cols = inner.width;
    let rows = cols / 2;
    (cols >= ART_MIN && inner.height >= rows + ART_FLOOR).then_some(Rect {
        height: rows,
        ..inner
    })
}

/* ffmpeg finds, decodes and scales the picture, `image` only carries it, and
   the crate picks the protocol. */
fn load_art(
    picker: Option<&ratatui_image::picker::Picker>,
    path: &std::path::Path,
    area: Rect,
) -> Option<ratatui_image::protocol::Protocol> {
    let picker = picker?;
    let (width, height) = cover_pixels(picker.font_size(), area);
    let raw = crate::tag::cover_rgb(path, width, height)?;
    let image = image::RgbImage::from_raw(width, height, raw)?;
    picker
        .new_protocol(
            image::DynamicImage::ImageRgb8(image),
            area.as_size(),
            /* Never the default, which is `Nearest`: any rescale at all then
               turns a photograph into squares. The size asked for above means
               there is normally none, but a terminal reporting a cell size
               the pane does not divide into still gets one. */
            ratatui_image::Resize::Fit(Some(ratatui_image::FilterType::Lanczos3)),
        )
        .ok()
}

/* Exactly the pixels the encoder will transmit, which is `cells × font_size`:
   `Resize::resize` normalises to that number whatever it is handed, so asking
   for precisely it means `needs_resize` finds nothing to do and ffmpeg's
   Lanczos output goes to the wire untouched. Anything else is resampled twice.

   `font` is the picker's, which `sharpen` has already doubled past the
   point-versus-pixel gap — this function does not know about that and should
   not, or there would be two places deciding how many pixels a cell is worth.

   Both dimensions rather than a square: the pane is half as many rows as
   columns precisely because a cell is about twice as tall as it is wide, so
   this comes out near square without assuming that ratio anywhere.

   Capped, because the cost is real: a pane on a wide terminal would otherwise
   ask for several megabytes of raw RGB per track. Past the cap the encoder
   resizes after all, which is why `Resize::Fit` still names a filter. */
fn cover_pixels(font: ratatui_image::FontSize, area: Rect) -> (u32, u32) {
    let scale = |cells: u16, cell: u16| {
        (u32::from(cells) * u32::from(cell)).clamp(1, COVER_PIXELS)
    };
    (scale(area.width, font.width), scale(area.height, font.height))
}

/* `art` is how many rows a cover has taken, which the words start under.
   `None` leaves the pane exactly as it was before any of this. */
fn draw_detail(
    frame: &mut Frame,
    track: &crate::app::Track,
    art: Option<u16>,
    area: Rect,
    p: Palette,
) {
    let block = detail_block(p);
    let mut inner = block.inner(area);
    frame.render_widget(block, area);
    if inner.width == 0 || inner.height == 0 {
        return;
    }
    /* The cover's rows, plus one of air, belong to the picture. A count and
       not a rectangle: the caller has already decided where those rows are,
       and a second opinion about their position is one that can disagree. */
    if let Some(rows) = art {
        let taken = (rows + 1).min(inner.height);
        inner.y += taken;
        inner.height -= taken;
    }
    let width = inner.width as usize;

    let mut lines: Vec<Line> = wrap(&format!(" {}", track.name), width)
        .into_iter()
        .map(|piece| Line::from(Span::styled(piece, row_style(true, p))))
        .collect();
    lines.push(Line::default());
    lines.extend(detail_rows(
        "status",
        track.status.label(),
        Style::new().fg(track.status.color(p)),
        width,
        p,
    ));
    let cream = Style::new().fg(p.text);
    let faint = Style::new().fg(p.muted);
    for (label, value) in [
        ("artist", &track.artist),
        ("title", &track.title),
        ("album", &track.album),
    ] {
        if !value.is_empty() {
            lines.extend(detail_rows(label, value, cream, width, p));
        }
    }
    if let Some(year) = track.year {
        lines.extend(detail_rows("year", &year.to_string(), faint, width, p));
    }
    if track.duration > 0 {
        lines.extend(detail_rows("length", &clock(track.duration), faint, width, p));
    }
    for (label, value) in [("source", &track.source), ("note", &track.note)] {
        if !value.is_empty() {
            lines.extend(detail_rows(label, value, faint, width, p));
        }
    }
    // The whole of it: truncating this is what the pane exists to undo.
    if track.was != track.name {
        lines.extend(detail_rows("was", &track.was, faint, width, p));
    }
    if let Some(path) = &track.path {
        lines.extend(detail_rows("file", &path.display().to_string(), faint, width, p));
    }
    lines.truncate(inner.height as usize);
    frame.render_widget(Paragraph::new(lines), inner);
}

fn draw_tracks(frame: &mut Frame, app: &mut App, area: Rect) {
    let p = app.theme;
    let rows = app.rows();
    if rows.is_empty() && !app.tracks.is_empty() {
        frame.render_widget(
            Paragraph::new(Line::from(dim(if app.filter.is_empty() {
                " nothing here needs a look   esc clears it".to_string()
            } else {
                format!(" nothing matches {}   esc clears it", app.filter)
            }, p))),
            area,
        );
        return;
    }

    /* Same bargain as the library's preview: the row has to keep its status
       and tail columns, so the pane only appears where both fit. */
    let area = if area.width >= PREVIEW_FROM {
        let pane = (u32::from(area.width) * 2 / 5).min(46) as u16;
        let [list, detail] =
            Layout::horizontal([Constraint::Min(1), Constraint::Length(pane)]).areas(area);
        /* SPIKE: encoded here, on the UI thread, the first frame the cursor
           rests on a track whose cover is not the one held. Everything about
           that is wrong — it is a subprocess and a zlib encode inside the
           render loop — and it is deliberate for now: the only question this
           version answers is whether Ghostty draws the thing at all. */
        let pane = art_pane(detail_block(p).inner(detail));
        app.art_area = pane;
        if let Some(area) = pane {
            let want = app.tracks.get(app.cursor).and_then(|t| t.path.clone());
            let held = app.art.as_ref().map(|(at, was, _)| (at.clone(), *was));
            match want {
                /* Keyed on the area as well as the file. A cover encoded for a
                   45-cell pane and drawn in a 20-cell one is not the picture
                   the protocol was handed, and the pane changes size whenever
                   the terminal does. */
                Some(path) if held.as_ref() != Some(&(path.clone(), area)) => {
                    app.art = load_art(app.picker.as_ref(), &path, area)
                        .map(|protocol| (path, area, protocol));
                }
                /* A row with no file has no cover, and the last row's is not
                   it. Without this the picture simply stays, so a `pending`
                   track wears its neighbour's sleeve. */
                None => app.art = None,
                _ => {}
            }
        }
        if let Some(track) = app.tracks.get(app.cursor) {
            draw_detail(frame, track, pane.map(|a| a.height), detail, p);
        }
        /* Drawn after the words, so the placeholder cells land on the rows
           already set aside for them rather than being overwritten by text. */
        if let (Some(area), Some((_, _, protocol))) = (pane, app.art.as_ref()) {
            frame.render_widget(ratatui_image::Image::new(protocol), area);
        }
        list
    } else {
        area
    };

    let width = area.width as usize;
    // What a page key moves by, which only the layout knows.
    app.viewport = area.height as usize;

    /* One name column for the whole list, so the dim source and note columns
       line up instead of stepping in and out with each title's length. */
    let longest = rows
        .iter()
        .map(|pos| cols(&app.tracks[*pos].name))
        .max()
        .unwrap_or(0);
    let name_width = longest.min(width.saturating_sub(STATUS_WIDTH + 9).max(12));

    let items: Vec<ListItem> = rows
        .iter()
        .map(|pos| {
            let track = &app.tracks[*pos];
            let selected = *pos == app.cursor;
            let mut spans = vec![
                Span::styled(
                    if selected { "▌" } else { " " },
                    Style::new().fg(p.cursor),
                ),
                // Its own column, so a marked track under the cursor shows both.
                Span::styled(
                    if app.marked.contains(&track.index) {
                        "•"
                    } else {
                        " "
                    },
                    Style::new().fg(p.accent),
                ),
                dim(format!("{:>3} ", track.index), p),
            ];

            if track.status == Status::Downloading {
                let (done, rest) = bar(track.percent, STATUS_WIDTH);
                spans.push(Span::styled(done, Style::new().fg(p.accent)));
                spans.push(dim(rest, p));
            } else {
                spans.push(Span::styled(
                    format!("{:<STATUS_WIDTH$}", track.status.label()),
                    Style::new().fg(track.status.color(p)),
                ));
            }

            let mut bits: Vec<String> = Vec::new();
            if !track.source.is_empty() {
                bits.push(track.source.clone());
            }
            if !track.note.is_empty() {
                bits.push(track.note.clone());
            }
            // The video title the tags were corrected away from, so a wrong
            // identification is visible without opening the file.
            if track.was != track.name {
                bits.push(format!("was {}", truncate(&track.was, 32)));
            }
            let tail = bits.join("  ");
            let name = truncate(&track.name, name_width);
            let pad = name_width.saturating_sub(cols(&name));
            spans.push(Span::styled(format!("  {name}"), row_style(selected, p)));
            if !tail.is_empty() {
                spans.push(dim(format!("{:pad$}  {tail}", ""), p));
            }
            ListItem::new(Line::from(spans))
        })
        .collect();

    /* Carried across frames, or ratatui recomputes the least scroll that
       makes the selection visible and pins the cursor to the last row. */
    app.scroll = app::scroll_to(
        app.scroll,
        app.row_of_cursor(&rows).unwrap_or(0),
        rows.len(),
        area.height as usize,
    );
    let at = app.row_of_cursor(&rows);
    let mut state = ListState::default().with_offset(app.scroll);
    state.select(at);
    frame.render_stateful_widget(List::new(items), area, &mut state);

    // Only worth the column when the list actually runs off the pane.
    if rows.len() > area.height as usize {
        let mut bar_state =
            ScrollbarState::new(rows.len()).position(at.unwrap_or(0));
        frame.render_stateful_widget(
            Scrollbar::new(ScrollbarOrientation::VerticalRight)
                .begin_symbol(None)
                .end_symbol(None)
                .thumb_style(Style::new().fg(p.muted))
                .track_symbol(None),
            area,
            &mut bar_state,
        );
    }
}

/* The tail is where a running job writes and where the eye goes, so that is
   the resting position. The line worth reading is often well above it, which
   is what `K` is for, and the head then says how far back it has gone. */
fn draw_logs(frame: &mut Frame, app: &App, area: Rect) {
    let p = app.theme;
    let height = area.height.saturating_sub(1) as usize;
    let end = app.logs.len().saturating_sub(app.log_scroll);
    let start = end.saturating_sub(height);
    let below = app.logs.len() - end;
    let mut lines = vec![Line::from(dim(match below {
        0 => " yt-dlp output".to_string(),
        n => format!(" yt-dlp output   {n} newer below   J K scroll"),
    }, p))];
    lines.extend(
        app.logs[start..end]
            .iter()
            .map(|l| Line::styled(format!(" {l}"), Style::new().fg(app::log_color(l, p)))),
    );
    frame.render_widget(Paragraph::new(lines), area);
}

fn draw_status(frame: &mut Frame, app: &App, area: Rect) {
    let p = app.theme;
    let width = area.width as usize;

    /* yt-dlp's stderr is otherwise invisible until you happen to press l, so
       say it is there. Dimmed once the pane is open: nothing left to find. */
    let mut hints = Vec::new();
    if !app.logs.is_empty() {
        let count = app.logs.len();
        let plural = if count == 1 { "line" } else { "lines" };
        /* Amber is the colour of a problem on every other screen, so it is
           spent here only on a real one: a run that warns about one video is
           the ordinary case and had the pane burning amber every time. */
        let urgent = app.log_errors() && !app.show_logs;
        /* The `!` and not the amber alone: urgency was the one thing on this
           bar that only colour said, so a terminal without it, or a reader
           who cannot separate amber from dim, saw an ordinary hint. It is
           the same mark a missing file carries in the library preview. */
        let mark = if urgent { "! " } else { "" };
        hints.push((
            format!("   {mark}l  {count} yt-dlp {plural}"),
            Style::new().fg(if urgent { p.warn } else { p.muted }),
        ));
    }
    /* `f` toggles silently, and the cursor being dragged to whatever is
       downloading reads as a bug until you know a mode is doing it. A word
       rather than a colour, so it survives a terminal without one. */
    if app.following() {
        hints.push(("   ⟳ following".to_string(), Style::new().fg(p.cursor)));
    }
    if !app.marked.is_empty() && app.view == View::Tracks && app.picking.is_none() {
        hints.push((
            format!("   {} marked", app.marked.len()),
            Style::new().fg(p.accent),
        ));
    }
    /* The library is the first screen a bare run shows, and a list of folders
       gives no clue that Enter opens one. The track view has a finished run
       behind it and a summary worth the same space. */
    /* Held apart from the rest: these are the hints worth dropping, since the
       tally beside them cannot be recovered by pressing anything, and `h`
       still lists every key either way. */
    let mut extra: Vec<(String, Style)> = Vec::new();
    /* In `extra` and not beside the ♪ itself: this is the one hint whose
       length is somebody else's song title, and the hints proper are the part
       that never gives way to the tally. Truncated for the same reason. */
    if let Some(track) = app.playback.track.as_ref().filter(|_| app.can_play()) {
        extra.push((
            format!("   {}", truncate(track, NOW_PLAYING)),
            Style::new().fg(p.cursor),
        ));
    }
    if app.view == View::Found {
        hints.push(("   enter open".to_string(), Style::new().fg(p.accent)));
        hints.push(("   / search".to_string(), Style::new().fg(p.muted)));
        hints.push(("   esc library".to_string(), Style::new().fg(p.muted)));
    } else if app.view == View::Library {
        hints.push(("   enter open".to_string(), Style::new().fg(p.accent)));
        hints.push(("   t find a track".to_string(), Style::new().fg(p.muted)));
        hints.push(("   R resync all".to_string(), Style::new().fg(p.muted)));
        hints.push(("   n new URL".to_string(), Style::new().fg(p.muted)));
    } else if !app.tracks.is_empty() && app.picking.is_none() {
        if app.can_command() {
            extra.push(("   e edit".to_string(), Style::new().fg(p.accent)));
            /* Named rather than a bare `u undo`: one level of undo is only
               usable if you can see which level it is holding. */
            if let Some(what) = &app.undoable {
                extra.push((format!("   u undo {what}"), Style::new().fg(p.muted)));
            }
            // Only when there is something to retry, so it reads as an
            // answer to the failures beside it rather than as decoration.
            if app.tracks.iter().any(|t| t.status == Status::Failed) {
                extra.push(("   r retry".to_string(), Style::new().fg(p.warn)));
            }
            /* Counted, because the key deletes files and the number is the
               first thing anyone would ask. Dim like the `gone` rows it
               answers: a departure is a fact about the playlist, not a fault. */
            if app.can_purge() {
                extra.push((format!("   D delete {} departed", app.purgeable()), Style::new().fg(p.muted)));
            }
        } else {
            extra.push(("   / filter".to_string(), Style::new().fg(p.muted)));
        }
    }
    /* Absent entirely when cliamp is not installed, since a player nobody has
       is not news. Installed but stopped is worth saying: it is the answer to
       "where did `p` go". */
    match app.playback.player {
        /* Which of the two it is doing, since the row in the library says so
           and the bar is the only thing on the track list that mentions
           cliamp at all. A glyph, so it is not teal doing the work. */
        Player::Running => hints.push((
            format!("   ♪ cliamp {}", if app.playback.playing { "▶" } else { "⏸" }),
            Style::new().fg(p.cursor),
        )),
        Player::Stopped => hints.push(("   ♪ cliamp off".to_string(), Style::new().fg(p.muted))),
        Player::Missing => {}
    }
    hints.push(("   h keys".to_string(), Style::new().fg(p.muted)));

    /* Room for the widest two statuses plus their counts, or the tally goes
       and the hint that displaced it is one `h` away anyway. */
    const TALLY_FLOOR: usize = 24;
    let taken: usize = hints.iter().map(|(t, _)| cols(t)).sum();
    let wanted: usize = extra.iter().map(|(t, _)| cols(t)).sum();
    if taken + wanted + TALLY_FLOOR <= width {
        // Ahead of `h keys`, which is the last word on every screen.
        let tail = hints.split_off(hints.len() - 1);
        hints.extend(extra);
        hints.extend(tail);
    }
    let reserved: usize = hints.iter().map(|(t, _)| cols(t)).sum();

    /* The hints are the part you cannot recover by looking elsewhere, so both
       the summary and the tally give way before they do. Every status in the
       tally is still readable in the list itself. */
    // Counted from the tracks, which is not what either other screen lists.
    let tally: Vec<(Status, String)> = if app.view != View::Tracks {
        Vec::new()
    } else {
        app.counts()
            .into_iter()
            .map(|(status, count)| (status, format!("{count} {}  ", status.label())))
            .collect()
    };
    let total: usize = tally.iter().map(|(_, t)| cols(t)).sum();
    let budget = width.saturating_sub(reserved);
    // One cell for the ellipsis, and only when something is actually dropped.
    let limit = budget.saturating_sub(if 1 + total > budget { 1 } else { 0 });

    let mut spans = vec![Span::raw(" ")];
    let mut used = 1;
    let mut dropped = false;
    for (status, text) in tally {
        if used + cols(&text) > limit {
            dropped = true;
            break;
        }
        used += cols(&text);
        spans.push(Span::styled(text, Style::new().fg(status.color(p))));
    }
    if dropped && used < budget {
        spans.push(dim("…", p));
        used += 1;
    }

    let room = width.saturating_sub(used + reserved);
    // Also the last run's, and it names a folder that is one row of many here.
    let (middle, style) = match app.done.as_ref().filter(|_| app.view == View::Tracks) {
        Some(Ok(summary)) => (truncate(summary, room), Style::new().fg(p.muted)),
        Some(Err(err)) => (
            truncate(&format!("failed: {err}"), room),
            Style::new().fg(p.error),
        ),
        /* The header counts the folders; this is what is inside them. The
           whole library's health in one line, where the track view puts the
           run's summary: missing files are what a sync would fetch again. */
        None if app.view == View::Library && !app.library.is_empty() => {
            let (tracks, missing, synced) = app.library_totals();
            let mut text = format!("{tracks} tracks");
            if missing > 0 {
                text.push_str(&format!("  ·  {missing} missing"));
            }
            if let Some(at) = synced {
                text.push_str(&format!("  ·  last sync {}", manifest::ago(at)));
            }
            (
                truncate(&text, room),
                Style::new().fg(if missing > 0 { p.warn } else { p.muted }),
            )
        }
        None => (String::new(), Style::new()),
    };
    let pad = room.saturating_sub(cols(&middle));
    spans.push(Span::styled(middle, style));
    spans.push(Span::raw(" ".repeat(pad)));
    for (text, style) in hints {
        spans.push(Span::styled(text, style));
    }
    frame.render_widget(Paragraph::new(Line::from(spans)), area);
}

/* One entry per line in three aligned columns, widths measured from the
   content: the old single-string-per-group layout could not line anything up,
   and a six-character group label ate its own separator. */
fn draw_help(frame: &mut Frame, app: &App) {
    let p = app.theme;
    let mut rows: Vec<(&str, &str, &str)> = vec![
        ("move", "j k  ↑ ↓", ""),
        ("", "^d ^u  PgDn PgUp", "by a screenful"),
        ("", "g G", "first, last"),
    ];
    if app.view == View::Found {
        rows.extend([
            ("open", "enter", "open this track in its playlist"),
            ("find", "/", "search every folder by track name"),
            ("view", "l", "yt-dlp output"),
            ("back", "Esc", "the library"),
            ("quit", "q  ^c", ""),
        ]);
    } else if app.view == View::Library {
        rows.extend([
            ("open", "enter", "read this playlist off disk"),
            ("", "O", "this folder in the file manager"),
            ("name", "e", "rename this playlist"),
            ("", "D", "remove this playlist"),
            ("find", "/", "filter by name or track"),
            ("", "t", "every matching track, across the library"),
            ("", "o", "order: name, last synced, most missing"),
        ]);
        if app.can_play() {
            rows.push(("play", "p", "hand this playlist to cliamp"));
            rows.push(("", "space", "play/pause cliamp"));
        }
        rows.extend([
            ("sync", "R", "sync every playlist"),
            ("", "n", "sync a new URL"),
            ("set", "f", "audio format for new downloads"),
            ("view", "l", "yt-dlp output"),
            ("quit", "q  Esc  ^c", ""),
        ]);
    } else if app.picking.is_some() {
        rows.extend([
            ("pick", "space", "this track"),
            ("", "m", "every track like it"),
            ("", "a", "everything the filter shows"),
            ("", "i", "invert what is selected"),
            ("", "/", "filter by name or status"),
            ("go", "enter", "download what is selected"),
            ("", "Esc", "download nothing"),
            ("view", "l", "yt-dlp output"),
            ("quit", "q  ^c", ""),
        ]);
    } else {
        rows.extend([
            ("view", "f", "follow the active track"),
            ("", "l", "yt-dlp output"),
            ("", "L", "taller log pane"),
            ("", "J K", "scroll the log"),
            ("", "/", "filter by name or status"),
            ("", "v", "only the tracks nothing confirmed"),
            ("find", "n N", "next, previous worth a look"),
            ("mark", "space", "this track"),
            ("", "m", "every track like it"),
            ("", "M", "every track by its artist"),
        ]);
        if app.can_command() {
            rows.extend([
                ("act", "e T", "edit this track / search again"),
                ("", "c", "choose cover art"),
                ("", "r", "retry failures"),
                ("", "S", "sync this playlist"),
            ]);
            if app.can_play() {
                rows.push(("", "p", "play it in cliamp"));
            }
            if app.undoable.is_some() {
                rows.push(("", "u", "undo the last tag change"));
            }
            if app.can_purge() {
                rows.push(("", "D", "delete the departed files"));
            }
            rows.extend([
                ("marked", "s", "swap artist and title"),
                ("", "A", "set one artist"),
            ]);
        } else {
            rows.push(("act", "e c T r s S A", "once the run finishes"));
        }
        /* Only when there is one, because Esc no longer quits: a row saying
           it does is the documentation for the behaviour that lost runs. */
        if !app.library.is_empty() {
            rows.push(("back", "Esc", "clear marks, then the library"));
            /* Not a key here, a route. `t` stays library-only, so without a
               row naming the two presses the search is invisible from the
               screen people spend the most time on. */
            rows.push(("", "Esc t", "find a track in any playlist"));
        }
        rows.push(("quit", "q  ^c", ""));
    }

    /* Two rows of margin and the two borders, so the popup never reaches the
       status bar: one that covers the tally is one that hides the very thing
       it is explaining. */
    let fits = |rows: usize| rows + 4 <= frame.area().height as usize;

    /* Every branch above ends with its own quit row, and `^t` is live on
       every screen, so it goes in once here rather than four times, above
       quit because quit is the last word on this table.

       First to give way, ahead of the legend and the settings tail: the key
       table on the track list is already 21 rows and a 24-row terminal has
       no room for a 22nd. Of everything here this is the one that only
       changes colour, and `--check` and the settings tail both still name
       the theme, so nothing about it goes unsaid — where a key that acts on
       the music has nowhere else to be found. */
    if fits(rows.len() + 1) {
        rows.insert(rows.len().saturating_sub(1), ("look", "^t", "next colour theme"));
    }

    let widest = |pick: fn(&(&str, &str, &str)) -> usize| {
        rows.iter().map(pick).max().unwrap_or(0)
    };
    let group = widest(|r| r.0.chars().count()) + 2;
    let key = widest(|r| r.1.chars().count()) + 2;

    let mut lines: Vec<Line> = rows
        .iter()
        .map(|(label, keys, what)| {
            Line::from(vec![
                Span::styled(format!(" {label:group$}"), Style::new().fg(p.muted)),
                Span::styled(format!("{keys:key$}"), Style::new().fg(p.accent)),
                Span::styled((*what).to_string(), Style::new().fg(p.text)),
            ])
        })
        .collect();

    /* Read off the palette being drawn in rather than out of `app.settings`,
       which is built once at startup: `^t` changes the theme from inside the
       tool and nothing re-sends that string, so a copy of the name in it is
       stale from the first press. Everything else on the row is fixed for the
       run, which is why the rest can be a string. */
    let described = format!("{} · theme {}", app.settings, app.theme_name);

    // Wrapped to the table it sits under, or one long line sets the width.
    let settings = widest(|r| r.2.chars().count()) + key;
    let mut tail: Vec<Line> = vec![Line::default()];
    for (n, piece) in wrap(&described, settings.max(20)).into_iter().enumerate() {
        tail.push(Line::from(vec![
            Span::styled(
                format!(" {:group$}", if n == 0 { "run" } else { "" }),
                Style::new().fg(p.muted),
            ),
            Span::styled(piece, Style::new().fg(p.text)),
        ]));
    }

    /* Only on the screen that has statuses, and only when the terminal has
       the rows to spare. The keys are what the overlay is for, so the legend
       is what gives way; and the whole popup stays two rows short of the
       frame, because one tall enough to cover the status bar is one that
       hides the very tally it is explaining. */
    /* Grouped by what to do about them rather than listed one per line: the
       question a status has to answer is "is this one I need to look at", and
       nine glossed rows would not fit under the keys anyway. */
    let legend: Vec<(&str, Vec<Status>, &str)> = vec![
        ("tags", vec![Status::Ok, Status::Manual], "confirmed"),
        ("", vec![Status::Kept, Status::Weak, Status::NoMatch], "a guess, worth a look"),
        ("", vec![Status::Have, Status::Skipped, Status::Gone], "not touched this run"),
        ("", vec![Status::Failed], "l has the reason"),
    ];
    let room = lines.len() + tail.len() + legend.len() + 1;
    if app.view == View::Tracks && fits(room) {
        lines.push(Line::default());
        // Three slots whether or not a row fills them, so the glosses line up.
        let slots = 3;
        for (label, statuses, gloss) in legend {
            let mut spans = vec![Span::styled(format!(" {label:group$}"), Style::new().fg(p.muted))];
            for status in &statuses {
                spans.push(Span::styled(
                    format!("{:<9}", status.label()),
                    Style::new().fg(status.color(p)),
                ));
            }
            /* Two columns of air whatever the row holds: `no match` is
               eight characters in a nine-wide cell, so a full row ran its
               gloss straight into the last status word. */
            spans.push(dim(format!(
                "{:pad$}{gloss}",
                "",
                pad = (slots - statuses.len()) * 9 + 2
            ), p));
            lines.push(Line::from(spans));
        }
    }
    /* Next to give way after the legend, and for the same reason: the keys
       are what the overlay is for, and `--check` prints these settings on a
       screen that is not fighting for rows. */
    if fits(lines.len() + tail.len()) {
        lines.extend(tail);
    }

    let content = lines
        .iter()
        .map(|l| l.width() as u16)
        .max()
        .unwrap_or(0);
    popup(frame, "keys", lines, content + 3, p);
}

fn draw_prompt(frame: &mut Frame, app: &App) {
    let p = app.theme;
    let Some((prompt, _)) = &app.prompt else {
        return;
    };
    let width = popup_width(frame.area());
    let inner = (width as usize).saturating_sub(4);

    let (header, mut rows) = match prompt {
        Prompt::Choice {
            header,
            note,
            options,
            ..
        } => {
            let mut rows = Vec::new();
            if !note.is_empty() {
                rows.push((note.clone(), Style::new().fg(p.muted)));
                rows.push((String::new(), Style::new()));
            }
            /* Numbered so the answer is one key rather than a walk with
               j and k. Past nine there is no digit to press, so those rows
               keep the space and stay reachable the long way. */
            rows.extend(options.iter().enumerate().map(|(i, opt)| {
                let selected = i == app.choice;
                let style = if selected {
                    Style::new().fg(p.cursor)
                } else {
                    Style::new()
                };
                let key = match i {
                    i if i < 9 => format!("{} ", i + 1),
                    _ => "  ".into(),
                };
                (
                    format!("{} {key}{opt}", if selected { "▌" } else { " " }),
                    style,
                )
            }));
            (header, rows)
        }
        Prompt::Input { header, note, .. } => {
            let mut rows = Vec::new();
            if !note.is_empty() {
                rows.push((note.clone(), Style::new().fg(p.muted)));
                rows.push((String::new(), Style::new()));
            }
            // Drawn between the halves, so the block is where the next
            // character lands rather than always at the end of the line.
            let (before, after) = app.input_parts();
            rows.push((format!("{before}\u{2588}{after}"), Style::new().fg(p.text)));
            (header, rows)
        }
        Prompt::Form {
            header,
            note,
            fields,
            ..
        } => {
            let mut rows = Vec::new();
            /* The title the tags came from, above the boxes rather than in
               the popup's own title, which clips instead of wrapping. It is
               what the answer is being corrected against, so it belongs
               inside the frame with the boxes. */
            if !note.is_empty() {
                rows.push((format!("was  {note}"), Style::new().fg(p.unsure)));
                rows.push((String::new(), Style::new()));
            }
            let label = fields
                .iter()
                .map(|(l, _)| l.chars().count())
                .max()
                .unwrap_or(0);
            for (n, (name, _)) in fields.iter().enumerate() {
                let value = app.fields.get(n).cloned().unwrap_or_default();
                // Only the focused box carries the block, or the popup shows
                // two cursors and neither of them is where typing lands.
                let text = if n == app.field {
                    let (before, after) = app.input_parts();
                    format!("{name:label$}  {before}\u{2588}{after}")
                } else {
                    format!("{name:label$}  {value}")
                };
                let style = if n == app.field {
                    Style::new().fg(p.text)
                } else {
                    Style::new().fg(p.muted)
                };
                rows.push((text, style));
            }
            (header, rows)
        }
    };
    rows.push((String::new(), Style::new()));
    rows.push((
        match prompt {
            Prompt::Choice { escape, options, .. } => {
                let digits = match options.len().min(9) {
                    0 | 1 => String::new(),
                    n => format!("1-{n} pick   "),
                };
                format!("{digits}j/k select   enter confirm   {}", escape.hint())
            }
            Prompt::Input { escape, .. } => {
                format!("enter confirm   ^u clear   ^w word   {}", escape.hint())
            }
            Prompt::Form { escape, fields, .. } => {
                // Named rather than counted, like the key itself: the edit
                // form has four boxes and only two of them swap.
                let swap = match app::swappable(fields) {
                    Some(_) => "^s swap   ",
                    None => "",
                };
                format!("tab field   {swap}enter save   {}", escape.hint())
            }
        },
        Style::new().fg(p.muted),
    ));

    /* Wrapped here rather than by Paragraph's own Wrap so the popup is sized
       from the lines that actually render; a wrapped option used to push the
       key hint outside the border. */
    let lines: Vec<Line> = rows
        .iter()
        .flat_map(|(text, style)| {
            wrap(text, inner)
                .into_iter()
                .map(move |piece| Line::styled(format!(" {piece}"), *style))
        })
        .collect();
    popup(frame, header, lines, width, p);
}

fn popup_width(area: Rect) -> u16 {
    // Widened to u32 first: 937 columns is enough to overflow the multiply.
    let seventy = (area.width as u32 * 70 / 100) as u16;
    seventy.clamp(20.min(area.width), area.width)
}

fn popup(frame: &mut Frame, title: &str, lines: Vec<Line>, width: u16, p: Palette) {
    let screen = frame.area();
    let height = (lines.len() as u16 + 2).min(screen.height);
    // Never narrower than the title it has to fit, never wider than the screen.
    let width = width.clamp((title.chars().count() as u16 + 4).min(screen.width), screen.width);
    let area = centred(screen, width, height);
    frame.render_widget(Clear, area);
    // Without colour the border is the whole separation, so it stops being
    // chrome and becomes the only thing saying where the popup ends.
    let (fill, edge) = if theme::plain() {
        (Style::new(), Style::new())
    } else {
        (Style::new().bg(p.surface), Style::new().fg(p.rule).bg(p.surface))
    };
    frame.render_widget(
        Paragraph::new(lines).block(
            Block::default()
                .borders(Borders::ALL)
                .style(fill)
                .border_style(edge)
                .title(Span::styled(format!(" {title} "), Style::new().fg(p.text))),
        ),
        area,
    );
}

fn centred(area: Rect, width: u16, height: u16) -> Rect {
    Rect {
        x: area.x + area.width.saturating_sub(width) / 2,
        y: area.y + area.height.saturating_sub(height) / 2,
        width: width.min(area.width),
        height: height.min(area.height),
    }
}

fn truncate(text: &str, width: usize) -> String {
    // Without this, no room at all still yields a stray ellipsis.
    if width == 0 {
        return String::new();
    }
    if cols(text) <= width {
        return text.to_string();
    }
    /* Stops before a glyph that would straddle the edge rather than spilling
       a column into the next field, so the result can come back one column
       short. Every caller pads to the column count, which absorbs that. */
    let room = width - 1;
    let mut out = String::new();
    let mut used = 0;
    for c in text.chars() {
        let w = UnicodeWidthChar::width(c).unwrap_or(0);
        if used + w > room {
            break;
        }
        out.push(c);
        used += w;
    }
    out.push('…');
    out
}

/// Greedy word wrap, hard-splitting any single word too long for the box.
/// Leading spaces are the caller's indentation and survive onto the first
/// line: splitting on spaces used to eat them, which left every unselected
/// option in a menu two columns left of the one with the marker.
fn wrap(text: &str, width: usize) -> Vec<String> {
    if width == 0 {
        return vec![text.to_string()];
    }
    let indent: String = text.chars().take_while(|c| *c == ' ').collect();
    if !indent.is_empty() {
        let room = width.saturating_sub(indent.len()).max(1);
        let mut lines = wrap(&text[indent.len()..], room);
        lines[0] = format!("{indent}{}", lines[0]);
        return lines;
    }
    let mut out = Vec::new();
    let mut line = String::new();
    for word in text.split(' ') {
        let mut word = word;
        // A word with no break in it, which a path or a CJK title often is.
        while cols(word) > width {
            if !line.is_empty() {
                out.push(std::mem::take(&mut line));
            }
            let head = take_cols(word, width);
            word = &word[head.len()..];
            out.push(head);
        }
        let joined = cols(&line) + 1 + cols(word);
        if !line.is_empty() && joined > width {
            out.push(std::mem::take(&mut line));
        }
        if !line.is_empty() {
            line.push(' ');
        }
        line.push_str(word);
    }
    /* A word that filled whole lines by itself leaves nothing here, and a
       blank row at the end of a popup is a row it did not need. An empty
       input still gets its one line, since callers expect one. */
    if !line.is_empty() || out.is_empty() {
        out.push(line);
    }
    out
}

/// As much of `text` as fits in `width` columns, never splitting a glyph
/// across the edge.
fn take_cols(text: &str, width: usize) -> String {
    let mut out = String::new();
    let mut used = 0;
    for c in text.chars() {
        let w = UnicodeWidthChar::width(c).unwrap_or(0);
        if used + w > width {
            break;
        }
        out.push(c);
        used += w;
    }
    /* A width of one against a two-column glyph would otherwise take nothing
       and loop forever; one glyph over the edge is the lesser fault. */
    if out.is_empty() {
        out.extend(text.chars().next());
    }
    out
}

#[cfg(test)]
mod tests {
    use super::{TAGLINE, bar, popup_width, wrap};
    use crate::app::{App, Msg, Shelf, Status, Track, View};
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;
    use ratatui::layout::Rect;
    use ratatui::style::Modifier;
    use std::time::Duration;

    /// The bottom row of a rendered frame, trailing spaces trimmed.
    fn status_row(width: u16, tracks: Vec<Track>, logs: Vec<String>) -> String {
        let (tx, _rx) = std::sync::mpsc::channel();
        let mut app = App::new(tx, "settings".into());
        app.tracks = tracks;
        app.logs = logs;
        app.done = Some(Ok("finished".into()));
        let mut terminal = Terminal::new(TestBackend::new(width, 24)).unwrap();
        terminal.draw(|f| super::draw(f, &mut app)).unwrap();
        let buffer = terminal.backend().buffer().clone();
        let row: String = (0..width)
            .map(|x| buffer[(x, 23)].symbol().to_string())
            .collect();
        row.trim_end().to_string()
    }

    /* The name on screen has to be the palette on screen. `App.settings` is
       built once at startup and nothing re-sends it, so the theme cannot live
       in there: it is the one setting the UI changes by itself, and a copy in
       that string reads `warm` on a cool screen from the first `^t` onwards.
       Driven through `cycle_theme` rather than by setting the field, because
       the key is the thing that used to leave the two disagreeing. */
    #[test]
    fn the_overlay_names_the_theme_it_is_drawn_in() {
        let (tx, _rx) = std::sync::mpsc::channel();
        let mut app = App::new(tx, "format opus · convert off".into());
        app.intro_done = true;
        app.show_help = true;

        let overlay = |app: &mut App| {
            let mut terminal = Terminal::new(TestBackend::new(100, 40)).unwrap();
            terminal.draw(|f| super::draw(f, app)).unwrap();
            let buffer = terminal.backend().buffer().clone();
            (0..40)
                .map(|y| {
                    (0..100)
                        .map(|x| buffer[(x, y)].symbol().to_string())
                        .collect::<String>()
                })
                .collect::<Vec<_>>()
                .join("\n")
        };

        for name in ["warm", "light", "cool", "neon"] {
            let screen = overlay(&mut app);
            assert_eq!(app.theme_name, name, "the walk went somewhere else");
            /* The run row alone, not the whole screen: the header carries the
               flash `^t` raised, and with no event loop here to expire it
               that is a message about the last press rather than a claim
               about what is being drawn. */
            let row = screen
                .lines()
                .find(|l| l.contains("format opus"))
                .unwrap_or_else(|| panic!("no run row: {screen}"));
            assert!(row.contains(&format!("theme {name}")), "{name} unnamed: {row}");
            for other in ["warm", "light", "cool", "neon"] {
                assert!(
                    other == name || !row.contains(&format!("theme {other}")),
                    "it also claimed {other} while drawing {name}: {row}"
                );
            }
            app.cycle_theme();
        }
    }

    /* Half as many rows as columns, because a cell is about twice as tall as
       it is wide and a cover is square. The floor is the part worth pinning:
       the pane exists to say which track this is, and on a short terminal the
       picture is what gives way rather than the words. */
    #[test]
    fn a_cover_only_gets_room_the_words_can_spare() {
        let inner = |w, h| Rect { x: 4, y: 2, width: w, height: h };
        let pane = |w, h| super::art_pane(inner(w, h));

        assert_eq!(pane(24, 40).map(|a| (a.width, a.height)), Some((24, 12)));
        // Exactly its own rows plus the floor, which is the tightest fit.
        assert_eq!(pane(20, 10 + super::ART_FLOOR).map(|a| a.height), Some(10));
        // One row less and the words win.
        assert_eq!(pane(20, 9 + super::ART_FLOOR), None);
        // Too narrow to be worth the read, however tall.
        assert_eq!(pane(super::ART_MIN - 1, 80), None);

        // It sits at the top of the pane it was given, not somewhere else.
        let at = pane(24, 40).unwrap();
        assert_eq!((at.x, at.y), (4, 2));

        // And the words always keep their rows, at every height.
        for h in 0..60u16 {
            if let Some(a) = pane(30, h) {
                assert!(a.height + super::ART_FLOOR <= h, "the words lost rows at {h}");
            }
        }
    }

    /* Both derive the pane's inside from one border definition. Two hand-
       rolled copies of "the area less the left border" disagree by a column
       the day the border changes, and the failure is a cover drawn one cell
       off with nothing pointing at the cause. */
    #[test]
    fn the_cover_and_the_words_measure_the_same_pane() {
        let outer = Rect { x: 10, y: 3, width: 40, height: 40 };
        let inner = super::detail_block(crate::theme::WARM).inner(outer);
        let art = super::art_pane(inner).expect("no room in a pane this size");

        assert_eq!(art.x, inner.x, "the cover starts in a different column");
        assert_eq!(art.width, inner.width, "the cover is a different width");
        assert_eq!(art.y, inner.y, "the cover starts on a different row");
        // And it is inside the border rather than on top of it.
        assert!(art.x > outer.x, "the cover was drawn over the border");
    }

    /* The pair above proves the two helpers agree; it says nothing about the
       call site, which is free to hand `art_pane` a rectangle of its own. This
       is the half that catches that, and it is the same lesson `scan_template`
       taught: assert on what actually ran.

       Halfblocks so no terminal is needed — the geometry is the protocol's
       business either way, and what is being checked is where earworm puts
       the widget, not what the widget draws. */
    #[test]
    fn the_cover_lands_where_the_words_left_room_for_it() {
        let picker = ratatui_image::picker::Picker::halfblocks();
        let (tx, _rx) = std::sync::mpsc::channel();
        let mut app = App::new(tx, "settings".into());
        app.intro_done = true;
        app.done = Some(Ok("finished".into()));
        let path = std::path::PathBuf::from("/music/Focus/01 - A.opus");
        let mut track = Track::new(1, "v1".into(), "01 - A.opus".into(), path.clone());
        /* Something that appears exactly once in the pane. The filename is no
           good: it is also the track list's row and the `file` detail line, so
           a search for it finds a row below the cover even when the words were
           never moved out from under it. */
        track.artist = "Unrepeatable".into();
        app.tracks = vec![track];

        let mut terminal = Terminal::new(TestBackend::new(120, 40)).unwrap();
        // One frame to find out what the pane offered, before anything is held.
        terminal.draw(|f| super::draw(f, &mut app)).unwrap();
        let pane = app.art_area.expect("the pane offered no room for a cover");

        /* Built at the size the pane actually offered, so the cover fills the
           rows it reserved. A protocol encoded for some smaller area would sit
           entirely above the words and the assertion below would pass whether
           or not the rows were ever given up. */
        let art = picker
            .new_protocol(
                image::DynamicImage::ImageRgb8(image::RgbImage::from_pixel(
                    1024,
                    1024,
                    image::Rgb([200, 40, 40]),
                )),
                pane.as_size(),
                ratatui_image::Resize::Fit(None),
            )
            .expect("halfblocks refused a plain square");
        app.art = Some((path, pane, art));
        terminal.draw(|f| super::draw(f, &mut app)).unwrap();
        let buffer = terminal.backend().buffer().clone();

        /* The red square, wherever it ended up. Halfblocks paints it as
           foreground and background colour, so the cells it covers are the
           ones carrying that red. */
        let red = ratatui::style::Color::Rgb(200, 40, 40);
        let painted: Vec<(u16, u16)> = (0..40)
            .flat_map(|y| (0..120).map(move |x| (x, y)))
            .filter(|(x, y)| buffer[(*x, *y)].fg == red || buffer[(*x, *y)].bg == red)
            .collect();
        assert!(!painted.is_empty(), "the cover was not drawn at all");

        let left = painted.iter().map(|(x, _)| *x).min().unwrap();
        let top = painted.iter().map(|(_, y)| *y).min().unwrap();
        assert_eq!((left, top), (pane.x, pane.y), "the cover missed the rows reserved for it");
        /* And the pane's own border is immediately to its left. This is the
           assertion a shifted call site fails: the two helpers above can agree
           perfectly while `draw_tracks` hands them a rectangle of its own. */
        assert_eq!(
            buffer[(left - 1, top)].symbol(),
            "│",
            "the cover is not sitting against the pane's border"
        );
        /* The words start below the cover. Searched inside the pane's own
           columns: the track list to the left carries the same filename, and
           a whole-row search finds that one every time. */
        let name_row = (0..40).find(|y| {
            (pane.x..120)
                .map(|x| buffer[(x, *y)].symbol().to_string())
                .collect::<String>()
                .contains("Unrepeatable")
        });
        let lowest = painted.iter().map(|(_, y)| *y).max().unwrap();
        assert!(
            name_row.is_some_and(|row| row > lowest),
            "the words were not pushed below the cover: artist on {name_row:?}, cover ends {lowest}"
        );
    }

    /* A row with no file has no cover, and the row before it does not lend
       one. Without the clear the picture simply stays where it is, so a
       `pending` track wears its neighbour's sleeve — which is worse than an
       empty pane, because the one job this pane has is saying which track you
       are looking at. */
    #[test]
    fn a_track_with_no_file_shows_no_cover_at_all() {
        let picker = ratatui_image::picker::Picker::halfblocks();
        let (tx, _rx) = std::sync::mpsc::channel();
        let mut app = App::new(tx, "settings".into());
        app.intro_done = true;
        app.done = Some(Ok("finished".into()));

        let path = std::path::PathBuf::from("/music/Focus/01 - A.opus");
        let downloaded = Track::new(1, "v1".into(), "01 - A.opus".into(), path.clone());
        let mut pending = Track::new(2, "v2".into(), "02 - B.opus".into(), path.clone());
        // What a track nothing has fetched yet looks like.
        pending.path = None;
        app.tracks = vec![downloaded, pending];

        let red = ratatui::style::Color::Rgb(200, 40, 40);
        let mut terminal = Terminal::new(TestBackend::new(120, 40)).unwrap();
        let mut drawn = |app: &mut App| {
            terminal.draw(|f| super::draw(f, app)).unwrap();
            let buffer = terminal.backend().buffer().clone();
            (0..40)
                .flat_map(|y| (0..120).map(move |x| (x, y)))
                .filter(|(x, y)| buffer[(*x, *y)].fg == red || buffer[(*x, *y)].bg == red)
                .count()
        };

        drawn(&mut app);
        let pane = app.art_area.expect("the pane offered no room for a cover");
        let art = picker
            .new_protocol(
                image::DynamicImage::ImageRgb8(image::RgbImage::from_pixel(
                    1024,
                    1024,
                    image::Rgb([200, 40, 40]),
                )),
                pane.as_size(),
                ratatui_image::Resize::Fit(None),
            )
            .unwrap();
        app.art = Some((path, pane, art));
        assert!(drawn(&mut app) > 0, "the cover was not drawn on the track that has one");

        // Down to the row with no file behind it.
        app.cursor = 1;
        assert_eq!(drawn(&mut app), 0, "the cover followed the cursor onto a track without one");
        assert!(app.art.is_none(), "the stale cover is still held");
    }

    /* A cover is encoded for a given number of cells, and the pane changes
       size with the terminal. Keyed on the file alone, a cover encoded for a
       46-column pane stays on screen in a 40-column one: not the picture the
       protocol was handed. This is the third shape this same bug has taken in
       this feature, which is why it has a test of its own now. */
    #[test]
    fn a_resize_asks_for_a_cover_the_new_pane_fits() {
        let picker = ratatui_image::picker::Picker::halfblocks();
        let (tx, _rx) = std::sync::mpsc::channel();
        let mut app = App::new(tx, "settings".into());
        app.intro_done = true;
        app.done = Some(Ok("finished".into()));
        let path = std::path::PathBuf::from("/music/Focus/01 - A.opus");
        app.tracks = vec![Track::new(1, "v1".into(), "01 - A.opus".into(), path.clone())];

        let draw_at = |app: &mut App, width: u16| {
            let mut terminal = Terminal::new(TestBackend::new(width, 40)).unwrap();
            terminal.draw(|f| super::draw(f, app)).unwrap();
        };

        draw_at(&mut app, 120);
        let wide = app.art_area.expect("no room at 120 columns");
        let art = picker
            .new_protocol(
                image::DynamicImage::ImageRgb8(image::RgbImage::from_pixel(
                    512,
                    512,
                    image::Rgb([200, 40, 40]),
                )),
                wide.as_size(),
                ratatui_image::Resize::Fit(None),
            )
            .unwrap();
        app.art = Some((path, wide, art));

        // Still held at the size it was made for.
        draw_at(&mut app, 120);
        assert!(app.art.is_some(), "it threw away a cover that still fits");

        /* Narrower, so the pane is a different shape. There is no picker on a
           test `App`, so the re-read finds nothing and the held cover is
           dropped — which is the observable half of "it asked again". */
        draw_at(&mut app, 100);
        let narrow = app.art_area.expect("no room at 100 columns");
        assert_ne!(narrow, wide, "the pane did not change size, so this proves nothing");
        assert!(app.art.is_none(), "it kept a cover encoded for the old pane");
    }

    /* Exactly what the encoder transmits, so nothing is resampled twice.
       `Resize::resize` normalises to `cells × font_size` whatever it is given:
       ask for more and it is thrown away, ask for less and the terminal
       upscales the difference, which is what turned a photograph into
       squares. */
    #[test]
    fn a_cover_is_asked_for_at_exactly_the_size_the_encoder_sends() {
        let font = ratatui_image::FontSize::new(16, 40);
        let area = Rect { x: 0, y: 0, width: 40, height: 20 };
        let (w, h) = super::cover_pixels(font, area);
        assert_eq!((w, h), (40 * 16, 20 * 40));

        /* Near square in pixels, because the pane is half as many rows as
           columns precisely to cancel the cell's own proportions. */
        let ratio = f64::from(w) / f64::from(h);
        assert!((0.8..1.25).contains(&ratio), "the request is {ratio:.2}:1, not near square");

        // Capped, or a wide pane asks for megabytes of raw RGB per track.
        let huge = super::cover_pixels(font, Rect { x: 0, y: 0, width: 400, height: 200 });
        assert_eq!(huge, (super::COVER_PIXELS, super::COVER_PIXELS));
        // And never zero, which ffmpeg would refuse.
        let none = super::cover_pixels(ratatui_image::FontSize::new(0, 0), area);
        assert_eq!(none, (1, 1));
    }

    /* A theme changes colour and nothing else. Every glyph in every cell is
       compared across all four, because the palette is threaded through a
       hundred call sites by hand and the way that goes wrong is a slot whose
       colour also decides a width — a status word styled from one palette and
       padded from another, say. Colour is asserted to actually differ in the
       same pass, or a bug that painted every theme warm would pass this. */
    #[test]
    fn a_theme_changes_the_colours_and_never_the_layout() {
        let glyphs = |palette| {
            let (tx, _rx) = std::sync::mpsc::channel();
            let mut app = App::new(tx, "settings".into());
            app.theme = palette;
            app.intro_done = true;
            app.tracks = spread();
            app.playlist = "Focus".into();
            app.done = Some(Ok("finished".into()));
            let mut terminal = Terminal::new(TestBackend::new(100, 30)).unwrap();
            terminal.draw(|f| super::draw(f, &mut app)).unwrap();
            let buffer = terminal.backend().buffer().clone();
            let cells: Vec<(String, ratatui::style::Color)> = (0..30)
                .flat_map(|y| (0..100).map(move |x| (x, y)))
                .map(|(x, y)| (buffer[(x, y)].symbol().to_string(), buffer[(x, y)].fg))
                .collect();
            cells
        };

        let warm = glyphs(crate::theme::WARM);
        for (name, palette) in crate::theme::BUILT_INS {
            let other = glyphs(palette);
            let shifted = warm
                .iter()
                .zip(&other)
                .position(|((a, _), (b, _))| a != b);
            assert!(shifted.is_none(), "{name} moved a glyph at cell {shifted:?}");
            if name != "warm" {
                assert!(
                    warm.iter().zip(&other).any(|((_, a), (_, b))| a != b),
                    "{name} drew in warm's colours"
                );
            }
        }
    }

    fn spread() -> Vec<Track> {
        let statuses = [
            (Status::Ok, 120),
            (Status::Manual, 14),
            (Status::Kept, 9),
            (Status::Weak, 7),
            (Status::NoMatch, 5),
            (Status::Failed, 3),
            (Status::Have, 203),
        ];
        let mut tracks = Vec::new();
        for (status, n) in statuses {
            for _ in 0..n {
                let mut track = Track::new(
                    tracks.len() + 1,
                    "id".into(),
                    "name".into(),
                    std::path::PathBuf::from("/tmp/x.opus"),
                );
                track.status = status;
                tracks.push(track);
            }
        }
        tracks
    }

    fn track_named(index: usize, name: &str) -> Track {
        let mut track = Track::new(
            index,
            "id".into(),
            name.into(),
            std::path::PathBuf::from("/tmp/x.opus"),
        );
        track.status = Status::Ok;
        track
    }

    fn epoch() -> u64 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs()
    }

    fn shelves() -> Vec<Shelf> {
        vec![
            Shelf {
                path: std::path::PathBuf::from("/music/Focus"),
                name: "Focus".into(),
                url: "u1".into(),
                tracks: 42,
                missing: 0,
                synced: Some(epoch() - 3 * 86_400),
                files: (1..=42)
                    .map(|n| (format!("{n:02} Artist - Track {n}.opus"), true))
                    .collect(),
            },
            Shelf {
                path: std::path::PathBuf::from("/music/Road trip"),
                name: "Road trip".into(),
                url: "u2".into(),
                tracks: 9,
                missing: 2,
                synced: None,
                files: vec![
                    ("01 Kraftwerk - Autobahn.opus".into(), true),
                    ("02 Neu! - Hallogallo.opus".into(), false),
                    ("03 Can - Vitamin C.opus".into(), true),
                ],
            },
        ]
    }

    /* The folder says which playlist is on air and never which song, which
       is the question people actually have of a player. */
    #[test]
    fn the_bar_names_the_track_cliamp_has_loaded() {
        let bar = |width: u16, title: Option<&str>| {
            let (tx, _rx) = std::sync::mpsc::channel();
            let mut app = App::new(tx, "settings".into());
            app.tracks = spread();
            app.done = Some(Ok(String::new()));
            app.intro_done = true;
            app.apply(Msg::Player(crate::app::Playback {
                player: crate::app::Player::Running,
                folder: None,
                track: title.map(str::to_string),
                playing: true,
            }));
            let mut terminal = Terminal::new(TestBackend::new(width, 12)).unwrap();
            terminal.draw(|f| super::draw(f, &mut app)).unwrap();
            let buffer = terminal.backend().buffer().clone();
            (0..width)
                .map(|x| buffer[(x, 11)].symbol().to_string())
                .collect::<String>()
        };

        let wide = bar(160, Some("Vogel im Käfig"));
        assert!(wide.contains("♪ cliamp ▶"), "{wide:?}");
        assert!(wide.contains("Vogel im Käfig"), "{wide:?}");
        // Nothing loaded is not the same as nothing playing.
        assert!(!bar(160, None).contains("Vogel"), "{:?}", bar(160, None));

        /* Somebody else's song title is the one hint whose length earworm
           does not choose, so it is capped and it gives way first. */
        let long = bar(160, Some("Prescription for Sleep: Attack on Titan Jazz Arrangement"));
        assert!(long.contains("Prescription for Sleep"), "{long:?}");
        assert!(!long.contains("Arrangement"), "the title ran the length it liked: {long:?}");

        /* A CJK title is twice the columns of its characters, and the bar
           budgets by columns: counted as characters it fitted on paper and
           pushed `h keys` off the end of the real row. */
        /* Matched on the ASCII half: a double-width glyph occupies one cell
           and leaves the next one blank, so the joined row reads "시 작". */
        let korean = bar(120, Some("시작 - Gaho"));
        assert!(korean.contains("Gaho"), "{korean:?}");
        assert!(korean.contains("h keys"), "the last hint was pushed off: {korean:?}");

        let tight = bar(64, Some("Vogel im Käfig"));
        assert!(!tight.contains("Vogel"), "the title pushed the counts off: {tight:?}");
        assert!(tight.contains("120 ok"), "{tight:?}");
        assert!(tight.contains("♪ cliamp"), "the player went with it: {tight:?}");
    }

    /* Urgency was the one thing on this bar that only colour said, so a
       terminal without it, or a reader who cannot separate amber from dim,
       saw an ordinary hint; and the bar named the player without ever saying
       which of the two things it was doing. */
    #[test]
    fn no_state_in_the_bar_is_told_by_colour_alone() {
        let bar = |logs: Vec<&str>, player: crate::app::Player, playing: bool| {
            let (tx, _rx) = std::sync::mpsc::channel();
            let mut app = App::new(tx, "settings".into());
            app.tracks = vec![track_named(1, "Autobahn")];
            app.done = Some(Ok(String::new()));
            app.intro_done = true;
            app.logs = logs.into_iter().map(str::to_string).collect();
            app.apply(Msg::Player(crate::app::Playback {
                player,
                playing,
                ..Default::default()
            }));
            let mut terminal = Terminal::new(TestBackend::new(120, 12)).unwrap();
            terminal.draw(|f| super::draw(f, &mut app)).unwrap();
            let buffer = terminal.backend().buffer().clone();
            (0..120)
                .map(|x| buffer[(x, 11)].symbol().to_string())
                .collect::<String>()
        };
        use crate::app::Player;

        let warned = bar(vec!["WARNING: one video"], Player::Missing, false);
        let broke = bar(vec!["ERROR: it went wrong"], Player::Missing, false);
        assert!(warned.contains("l  1 yt-dlp line"), "{warned:?}");
        assert!(!warned.contains('!'), "an ordinary run was marked urgent: {warned:?}");
        assert!(broke.contains("! l"), "an error reads as an ordinary hint: {broke:?}");

        // Which of the two cliamp is doing, not merely that it is up.
        assert!(bar(vec![], Player::Running, true).contains("♪ cliamp ▶"));
        assert!(bar(vec![], Player::Running, false).contains("♪ cliamp ⏸"));
        assert!(bar(vec![], Player::Stopped, false).contains("♪ cliamp off"));
    }

    /* `f` toggles silently, and a cursor that jumps to whatever is
       downloading reads as a bug until you know a mode is doing it. */
    #[test]
    fn the_bar_says_when_the_cursor_is_following_the_run() {
        let bar = |settled: bool, follow: bool| {
            let (tx, _rx) = std::sync::mpsc::channel();
            let mut app = App::new(tx, "settings".into());
            app.tracks = spread();
            app.follow = follow;
            app.intro_done = true;
            if settled {
                app.done = Some(Ok(String::new()));
            }
            let mut terminal = Terminal::new(TestBackend::new(120, 12)).unwrap();
            terminal.draw(|f| super::draw(f, &mut app)).unwrap();
            let buffer = terminal.backend().buffer().clone();
            (0..120)
                .map(|x| buffer[(x, 11)].symbol().to_string())
                .collect::<String>()
        };
        assert!(bar(false, true).contains("following"), "{:?}", bar(false, true));
        assert!(!bar(false, false).contains("following"), "{:?}", bar(false, false));
        /* Nothing moves the cursor once the run is over, so the word would
           describe a mode that is not doing anything. */
        assert!(!bar(true, true).contains("following"), "{:?}", bar(true, true));
    }

    /* The first question a new user is ever asked, on a screen with nothing
       else on it: both facts they need before typing anything. */
    #[test]
    fn the_url_prompt_says_what_an_answer_looks_like() {
        let (tx, _rx) = std::sync::mpsc::channel();
        let mut app = App::new(tx, "settings".into());
        let (reply, _answers) = std::sync::mpsc::channel();
        app.apply(Msg::Ask(
            crate::app::Prompt::Input {
                header: "YouTube playlist URL".into(),
                note: "a playlist or a single video  ·  esc quits".into(),
                value: String::new(),
                escape: crate::app::Escape::Quit,
            },
            reply,
        ));
        app.intro_done = true;
        let mut terminal = Terminal::new(TestBackend::new(80, 14)).unwrap();
        terminal.draw(|f| super::draw(f, &mut app)).unwrap();
        let buffer = terminal.backend().buffer().clone();
        let screen: String = (0..14)
            .map(|y| {
                (0..80)
                    .map(|x| buffer[(x, y)].symbol().to_string())
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join("\n");
        assert!(screen.contains("YouTube playlist URL"), "{screen}");
        assert!(screen.contains("a playlist or a single video"), "{screen}");
        assert!(screen.contains("esc quits"), "{screen}");
    }

    /* The widest status is `no match` at eight characters in a nine-wide
       cell, so a row holding three of them ran the gloss into the last word
       and read as "no match a guess, worth a look". */
    #[test]
    fn the_status_legend_keeps_its_glosses_clear_of_the_words() {
        let (tx, _rx) = std::sync::mpsc::channel();
        let mut app = App::new(tx, "settings".into());
        app.tracks = spread();
        app.done = Some(Ok(String::new()));
        app.intro_done = true;
        app.show_help = true;
        let mut terminal = Terminal::new(TestBackend::new(120, 40)).unwrap();
        terminal.draw(|f| super::draw(f, &mut app)).unwrap();
        let buffer = terminal.backend().buffer().clone();
        let screen: Vec<String> = (0..40)
            .map(|y| (0..120).map(|x| buffer[(x, y)].symbol().to_string()).collect())
            .collect();

        let row = screen
            .iter()
            .find(|r| r.contains("a guess, worth a look"))
            .expect("the legend did not draw");
        assert!(row.contains("no match  "), "the gloss is touching the word: {row:?}");
        /* The other three rows of the same block, taken by position: their
           glosses are words that also appear among the key descriptions. */
        let at = screen.iter().position(|r| r == row).unwrap();
        for (n, gloss) in [(-1i32, "confirmed"), (1, "not touched this run"), (2, "l has the reason")]
        {
            let line = &screen[(at as i32 + n) as usize];
            let found = line.find(gloss).unwrap_or_else(|| panic!("{gloss}: {line:?}"));
            assert!(line[..found].ends_with("  "), "{gloss}: {line:?}");
        }
    }

    /* A Korean or Japanese glyph takes two cells, so a column padded to a
       character count is as ragged as the titles in it are wide: the dim
       columns after the name stepped out of line by the length of every CJK
       title on the screen. */
    #[test]
    fn a_column_of_cjk_titles_lines_up_with_an_ascii_one() {
        let (tx, _rx) = std::sync::mpsc::channel();
        let mut app = App::new(tx, "settings".into());
        app.tracks = vec![
            track_named(1, "시작 - Gaho"),
            track_named(2, "Your Existence - Wonstein"),
            track_named(3, "우리가 헤어져야 했던 이유 - BIBI"),
            track_named(4, "Flower - Yoonmirae"),
        ];
        for track in &mut app.tracks {
            track.status = Status::Have;
            track.source = "on disk".into();
        }
        app.done = Some(Ok(String::new()));
        app.intro_done = true;
        let mut terminal = Terminal::new(TestBackend::new(90, 10)).unwrap();
        terminal.draw(|f| super::draw(f, &mut app)).unwrap();
        let buffer = terminal.backend().buffer().clone();

        /* Which cell the source column starts in. Measured per cell and not
           by a string offset: a double-width glyph occupies one cell and
           leaves the next one empty, so a position in the joined string is
           not a position on the screen, which is the bug itself. */
        let starts: Vec<usize> = (2..6)
            .map(|y| {
                let cells: Vec<String> =
                    (0..90).map(|x| buffer[(x, y)].symbol().to_string()).collect();
                let at = |from: usize| {
                    (from..cells.len() - 7)
                        .find(|x| cells[*x..*x + 7].concat() == "on disk")
                        .expect("no source column on this row")
                };
                // The second one: the first is the status word in its column.
                at(at(0) + 7)
            })
            .collect();
        assert_eq!(
            starts.iter().collect::<std::collections::HashSet<_>>().len(),
            1,
            "the source column is ragged: {starts:?}"
        );
    }

    /// Cutting a title by characters overshoots its column by every
    /// double-width glyph in it, and can leave half a glyph on the edge.
    #[test]
    fn truncation_and_wrapping_count_columns() {
        assert_eq!(super::cols("시작"), 4);
        assert_eq!(super::cols("Gaho"), 4);

        // Eight columns is four of these, and the ellipsis costs one.
        let cut = super::truncate("시작하는 우리", 8);
        assert!(super::cols(&cut) <= 8, "{cut} is {} columns", super::cols(&cut));
        assert!(cut.ends_with('…'), "{cut}");
        // A title that already fits is left exactly as it is.
        assert_eq!(super::truncate("시작", 4), "시작");

        for line in super::wrap("우리가 헤어져야 했던 이유 - BIBI", 12) {
            assert!(super::cols(&line) <= 12, "{line} is {} columns", super::cols(&line));
        }
        /* One column against a two-column glyph: one glyph over the edge is
           the lesser fault, and taking nothing would never terminate. */
        assert_eq!(super::wrap("시작", 1), vec!["시", "작"]);
        // A word that filled its lines exactly leaves no blank row behind.
        assert_eq!(super::wrap("abcdef", 3), vec!["abc", "def"]);
        assert_eq!(super::wrap("", 5), vec![""], "callers expect one line");
    }

    /* The row truncates `was` at 32 characters, and the part it cuts is
       usually the part that says how the identification went wrong. The path
       is never on the row at all. */
    #[test]
    fn a_wide_terminal_says_the_whole_of_what_the_row_truncates() {
        let screen = |width: u16| {
            let (tx, _rx) = std::sync::mpsc::channel();
            let mut app = App::new(tx, "settings".into());
            let mut track = track_named(1, "Neu! - Hallogallo");
            track.was = "Neu! - Hallogallo (1972 Remaster, Official Audio) [HQ]".into();
            track.artist = "Neu!".into();
            track.title = "Hallogallo".into();
            track.duration = 610;
            track.path = Some(std::path::PathBuf::from("/music/Krautrock/01 Neu! - Hallogallo.opus"));
            app.tracks = vec![track];
            app.done = Some(Ok("finished".into()));
            app.intro_done = true;
            let mut terminal = Terminal::new(TestBackend::new(width, 14)).unwrap();
            terminal.draw(|f| super::draw(f, &mut app)).unwrap();
            let buffer = terminal.backend().buffer().clone();
            (0..14)
                .map(|y| {
                    (0..width)
                        .map(|x| buffer[(x, y)].symbol().to_string())
                        .collect::<String>()
                })
                .collect::<Vec<_>>()
                .join("\n")
        };

        let wide = screen(110);
        assert!(wide.contains("Official Audio"), "the tail is still cut: {wide}");
        assert!(wide.contains("10:10"), "no length: {wide}");
        assert!(wide.contains("Krautrock"), "no path: {wide}");

        /* Below the threshold the row needs every column it has, so the pane
           is simply absent rather than squeezed. */
        let narrow = screen(80);
        assert!(!narrow.contains("Krautrock"), "the pane took a narrow screen: {narrow}");
        assert!(narrow.contains("Hallogallo"), "{narrow}");
    }

    /* The header counts the folders and the rows count their tracks, and
       neither says what the library as a whole is carrying. */
    #[test]
    fn the_library_bar_carries_the_whole_librarys_totals() {
        let rows = library_screen_at(120, 0);
        let bar = rows.last().unwrap();
        assert!(bar.contains("51 tracks"), "{bar:?}");
        assert!(bar.contains("2 missing"), "{bar:?}");
        assert!(bar.contains("last sync 3d ago"), "{bar:?}");
        // The hints are what nothing else can recover, so they keep their room.
        assert!(bar.contains("enter open"), "{bar:?}");
    }

    /* Review narrows the list with nothing in the filter box, so the band has
       to name itself or the list looks like it has lost most of the run. */
    #[test]
    fn the_band_says_the_list_is_narrowed_to_the_review() {
        let (tx, _rx) = std::sync::mpsc::channel();
        let mut app = App::new(tx, "settings".into());
        app.tracks = spread();
        app.done = Some(Ok("finished".into()));
        app.intro_done = true;
        app.apply(Msg::Review);
        let mut terminal = Terminal::new(TestBackend::new(90, 12)).unwrap();
        terminal.draw(|f| super::draw(f, &mut app)).unwrap();
        let buffer = terminal.backend().buffer().clone();
        let band: String = (0..90).map(|x| buffer[(x, 1)].symbol().to_string()).collect();
        assert!(band.contains("REVIEW"), "{band:?}");
        // 9 kept, 7 weak, 5 no match and 3 failed, out of 361.
        assert!(band.contains("24 of 361 shown"), "{band:?}");
        assert!(band.contains("esc clears"), "{band:?}");
    }

    fn library_screen(width: u16) -> Vec<String> {
        library_screen_at(width, 0)
    }

    /* A folder on the list for a track inside it says only its own name on
       the row, so the pane is what has to answer which track. Showing the
       first screenful instead would put the matching row off the bottom of a
       long folder and leave the filter looking broken. */
    #[test]
    fn the_preview_shows_what_the_filter_matched_inside_the_folder() {
        let (tx, _rx) = std::sync::mpsc::channel();
        let mut app = App::new(tx, "settings".into());
        app.apply(Msg::Library {
            shelves: shelves(),
            show: true,
        });
        app.intro_done = true;
        app.filter = "hallogallo".into();
        app.snap();

        let mut terminal = Terminal::new(TestBackend::new(120, 12)).unwrap();
        terminal.draw(|f| super::draw(f, &mut app)).unwrap();
        let buffer = terminal.backend().buffer().clone();
        let rows: Vec<String> = (0..12)
            .map(|y| {
                (0..120)
                    .map(|x| buffer[(x, y)].symbol().to_string())
                    .collect::<String>()
                    .trim_end()
                    .to_string()
            })
            .collect();
        let screen = rows.join("\n");

        // One folder holds it, and the band says the list is not all of it.
        assert!(rows[1].contains("1 of 2 shown"), "{:?}", rows[1]);
        assert!(screen.contains("Road trip"), "{screen}");
        assert!(!screen.contains("Focus"), "a folder with no match stayed: {screen}");
        // The pane names the track and says how much of the folder matched.
        assert!(rows[2].contains("1 of 9 tracks match"), "{:?}", rows[2]);
        assert!(screen.contains("Hallogallo"), "{screen}");
        assert!(
            !screen.contains("Autobahn"),
            "the pane showed the folder rather than the match: {screen}"
        );
    }

    /* The intro is where the update notice draws, and `intro = false` is a
       setting: the probe ran, wrote its cache and threw the answer away, so
       turning off an animation turned off update notifications. */
    #[test]
    fn the_update_notice_is_not_lost_with_the_intro_switched_off() {
        let intro = |on: bool| {
            let (tx, _rx) = std::sync::mpsc::channel();
            let mut app = App::new(tx, "settings".into());
            app.update = Some("99.0.0".into());
            app.intro_done = !on;
            /* Far enough in for the signature line, which is the last thing
               the intro draws and the only part that carries the notice. */
            app.started = std::time::Instant::now() - Duration::from_millis(1500);
            /* What `main` does with the probe's answer when the intro is not
               up to say it: the drawn surface is the bar rather than nothing. */
            if !app.intro() {
                app.say("v99.0.0 is out  ·  earworm --check has the command");
            }
            let mut terminal = Terminal::new(TestBackend::new(110, 14)).unwrap();
            terminal.draw(|f| super::draw(f, &mut app)).unwrap();
            let buffer = terminal.backend().buffer().clone();
            (0..14)
                .map(|y| {
                    (0..110)
                        .map(|x| buffer[(x, y)].symbol().to_string())
                        .collect::<String>()
                })
                .collect::<Vec<_>>()
                .join("\n")
        };

        // With the intro on it is the intro's to say, as it always was.
        let animated = intro(true);
        assert!(animated.contains("99.0.0"), "the intro lost the notice:\n{animated}");

        // And with it off the news still reaches the screen.
        let bare = intro(false);
        assert!(bare.contains("99.0.0"), "the notice went with the intro:\n{bare}");
        // Pointing at where the command to act on it lives.
        assert!(bare.contains("--check"), "{bare}");
    }

    /* `t` is library-only, like `space` and `f`: it is about the library
       rather than the playlist on screen. That makes it invisible from the
       track list, which is the screen people spend the most time on, so the
       route is what the help names. */
    #[test]
    fn the_track_help_names_the_way_to_the_search() {
        let (tx, _rx) = std::sync::mpsc::channel();
        let mut app = App::new(tx, "settings".into());
        app.tracks = vec![track_named(1, "Autobahn")];
        app.done = Some(Ok("finished".into()));
        app.intro_done = true;
        app.show_help = true;

        let help = |app: &mut App| {
            let mut terminal = Terminal::new(TestBackend::new(100, 40)).unwrap();
            terminal.draw(|f| super::draw(f, app)).unwrap();
            let buffer = terminal.backend().buffer().clone();
            (0..40)
                .map(|y| {
                    (0..100)
                        .map(|x| buffer[(x, y)].symbol().to_string())
                        .collect::<String>()
                })
                .collect::<Vec<_>>()
                .join("\n")
        };

        /* No library behind the run, so there is nowhere for Esc to go and
           nothing to search: the row would document a key that refuses. */
        assert!(!help(&mut app).contains("any playlist"));

        app.apply(Msg::Library {
            shelves: shelves(),
            show: false,
        });
        let with = help(&mut app);
        assert!(with.contains("Esc t"), "no route to the search:\n{with}");
        assert!(with.contains("any playlist"), "{with}");
    }

    /* The flat list across the whole library, which is the thing neither the
       rows nor the preview can show: the preview is one folder at a time and
       only where there is room for it at all. */
    #[test]
    fn the_search_screen_names_the_folder_beside_every_match() {
        let screen = |width: u16, filter: &str| {
            let (tx, _rx) = std::sync::mpsc::channel();
            let mut app = App::new(tx, "settings".into());
            app.apply(Msg::Library {
                shelves: shelves(),
                show: true,
            });
            app.intro_done = true;
            app.filter = filter.into();
            app.find_tracks();
            let mut terminal = Terminal::new(TestBackend::new(width, 12)).unwrap();
            terminal.draw(|f| super::draw(f, &mut app)).unwrap();
            let buffer = terminal.backend().buffer().clone();
            (0..12)
                .map(|y| {
                    (0..width)
                        .map(|x| buffer[(x, y)].symbol().to_string())
                        .collect::<String>()
                        .trim_end()
                        .to_string()
                })
                .collect::<Vec<String>>()
        };

        // "Track 1" is in Focus; "Autobahn" and the rest are in Road trip.
        let rows = screen(90, "track 1");
        let all = rows.join("\n");
        assert!(rows[0].contains("search"), "{:?}", rows[0]);
        /* The count the flat list cannot give itself: 1, 10 through 19, and
           the folder they are all in. */
        assert!(rows[0].contains("11 tracks in 1 playlist"), "{:?}", rows[0]);
        assert!(rows[1].contains("SEARCH"), "{:?}", rows[1]);
        assert!(all.contains("Focus"), "no folder column: {all}");
        assert!(all.contains("Artist - Track 1"), "{all}");
        // The number and extension belong to the filename, not the track.
        assert!(!all.contains(".opus"), "the row kept the extension: {all}");
        assert!(rows[11].contains("enter open"), "{:?}", rows[11]);

        /* A missing file carries the same mark it does in the preview, since
           it is the same fact about the same file. */
        let missing = screen(90, "hallogallo");
        let all = missing.join("\n");
        assert!(all.contains("Road trip"), "{all}");
        assert!(all.contains('!'), "a missing file lost its mark: {all}");

        // Nothing typed is a prompt to type, not a list of the whole library.
        let empty = screen(90, "");
        assert!(
            empty.join("\n").contains("type to search"),
            "{:?}",
            empty.join("\n")
        );
    }

    fn library_screen_at(width: u16, shelf: usize) -> Vec<String> {
        let (tx, _rx) = std::sync::mpsc::channel();
        let mut app = App::new(tx, "settings".into());
        app.apply(Msg::Library {
            shelves: shelves(),
            show: true,
        });
        app.shelf = shelf;
        // These test the library screen, which the intro now sits in front of.
        app.intro_done = true;
        let mut terminal = Terminal::new(TestBackend::new(width, 12)).unwrap();
        terminal.draw(|f| super::draw(f, &mut app)).unwrap();
        let buffer = terminal.backend().buffer().clone();
        (0..12)
            .map(|y| {
                (0..width)
                    .map(|x| buffer[(x, y)].symbol().to_string())
                    .collect::<String>()
                    .trim_end()
                    .to_string()
            })
            .collect()
    }

    /* cliamp reports a track and never a playlist, so the row is found by the
       folder that track sits in. Worth marking: the library is a list of
       folders that otherwise gives no clue which one is on air. */
    #[test]
    fn the_library_marks_the_folder_cliamp_is_playing() {
        let screen = |folder: Option<&str>, playing: bool| {
            let (tx, _rx) = std::sync::mpsc::channel();
            let mut app = App::new(tx, "settings".into());
            app.apply(Msg::Library { shelves: shelves(), show: true });
            app.intro_done = true;
            app.apply(Msg::Player(crate::app::Playback {
                player: crate::app::Player::Running,
                folder: folder.map(std::path::PathBuf::from),
                track: None,
                playing,
            }));
            let mut terminal = Terminal::new(TestBackend::new(80, 12)).unwrap();
            terminal.draw(|f| super::draw(f, &mut app)).unwrap();
            let buffer = terminal.backend().buffer().clone();
            (2..4)
                .map(|y| {
                    (0..80)
                        .map(|x| buffer[(x, y)].symbol().to_string())
                        .collect::<String>()
                })
                .collect::<Vec<_>>()
        };

        // shelves() is Focus then Road trip; only the second is playing.
        let rows = screen(Some("/music/Road trip"), true);
        assert!(!rows[0].contains('▶'), "the wrong row was marked: {rows:?}");
        assert!(rows[1].contains('▶'), "the playing row is unmarked: {rows:?}");

        let paused = screen(Some("/music/Road trip"), false);
        assert!(paused[1].contains('⏸'), "loaded but stopped went unmarked: {paused:?}");
        assert!(!paused[1].contains('▶'), "{paused:?}");

        // A radio stream sits in no folder and must claim no row.
        let stream = screen(None, true);
        assert!(!stream.join("").contains('▶'), "a stream marked a row: {stream:?}");

        /* The marker holds two cells whether or not it is there, or every
           column after it steps sideways as cliamp starts and stops. */
        let idle = screen(None, false);
        // Cells, not bytes: the marker is three bytes and one column.
        let col = |r: &str| r[..r.find("tracks").unwrap()].chars().count();
        assert_eq!(col(&rows[1]), col(&idle[1]), "the counts moved");
    }

    /* Both cliamp keys are library-only and both need it running, so the help
       has to stop offering them the moment it is not. */
    #[test]
    fn the_library_help_offers_the_cliamp_keys_only_while_it_runs() {
        let help = |player: crate::app::Player| {
            let (tx, _rx) = std::sync::mpsc::channel();
            let mut app = App::new(tx, "settings".into());
            app.apply(Msg::Library { shelves: shelves(), show: true });
            app.intro_done = true;
            app.show_help = true;
            app.apply(Msg::Player(crate::app::Playback {
                player,
                ..Default::default()
            }));
            let mut terminal = Terminal::new(TestBackend::new(80, 24)).unwrap();
            terminal.draw(|f| super::draw(f, &mut app)).unwrap();
            let buffer = terminal.backend().buffer().clone();
            (0..24)
                .map(|y| {
                    (0..80)
                        .map(|x| buffer[(x, y)].symbol().to_string())
                        .collect::<String>()
                })
                .collect::<Vec<_>>()
                .join("\n")
        };

        let up = help(crate::app::Player::Running);
        assert!(up.contains("play/pause cliamp"), "{up}");
        assert!(up.contains("hand this playlist to cliamp"), "{up}");

        let down = help(crate::app::Player::Stopped);
        assert!(!down.contains("play/pause cliamp"), "{down}");
        assert!(!down.contains("hand this playlist to cliamp"), "{down}");
        // The rest of the library keys are unaffected.
        assert!(down.contains("read this playlist off disk"), "{down}");
    }

    /* Everything on this screen comes from the manifests, and the sync time is
       the only thing that says whether a folder still matches its playlist. */
    #[test]
    fn the_library_lists_each_folder_with_what_is_in_it() {
        let rows = library_screen(80);
        assert!(rows[0].contains("2 playlists"), "{:?}", rows[0]);
        assert!(rows[2].contains("Focus"), "{:?}", rows[2]);
        assert!(rows[2].contains("42 tracks"), "{:?}", rows[2]);
        assert!(rows[2].contains("synced 3d ago"), "{:?}", rows[2]);
        assert!(rows[3].contains("2 missing"), "{:?}", rows[3]);
        assert!(rows[3].contains("never synced"), "{:?}", rows[3]);
        // A list of folders gives no clue that Enter opens one.
        assert!(rows[11].contains("enter open"), "{:?}", rows[11]);
    }

    /* A walk with j and k to answer a question whose options are right
       there, on a menu that fires at the end of every run. */
    #[test]
    fn a_choice_numbers_its_options() {
        let (tx, _rx) = std::sync::mpsc::channel();
        let mut app = App::new(tx, "settings".into());
        app.apply(Msg::Ask(
            crate::app::Prompt::Choice {
                header: "finished".into(),
                note: "12 tracks".into(),
                options: vec![
                    "keep this open".into(),
                    "play it in cliamp".into(),
                    "sync another playlist".into(),
                    "quit".into(),
                ],
                escape: crate::app::Escape::Keep,
            },
            std::sync::mpsc::channel().0,
        ));

        let mut terminal = Terminal::new(TestBackend::new(80, 16)).unwrap();
        terminal.draw(|f| super::draw(f, &mut app)).unwrap();
        let buffer = terminal.backend().buffer().clone();
        let screen: String = (0..16)
            .map(|y| (0..80).map(|x| buffer[(x, y)].symbol().to_string()).collect::<String>())
            .collect::<Vec<_>>()
            .join("\n");

        assert!(screen.contains("▌ 1 keep this open"), "{screen}");
        // Every option in the same column, marker or not.
        assert!(screen.contains("  2 play it in cliamp"), "{screen}");
        assert!(screen.contains("  4 quit"), "{screen}");
        // And the keys say so, or the numbers are decoration.
        assert!(screen.contains("1-4 pick"), "{screen}");
    }

    /* Both boxes and the title they are being corrected against, in one
       frame: the popup is the whole edit now, not the first half of it. */
    #[test]
    fn the_edit_form_shows_both_boxes_and_the_title_behind_them() {
        let (tx, _rx) = std::sync::mpsc::channel();
        let mut app = App::new(tx, "settings".into());
        app.apply(Msg::Ask(
            crate::app::Prompt::Form {
                header: "track 4".into(),
                note: "Roygbiv - Boards Of Canada (Official Audio)".into(),
                fields: vec![
                    ("artist".into(), "Roygbiv".into()),
                    ("title".into(), "Boards Of Canada".into()),
                ],
                escape: crate::app::Escape::Skip,
            },
            std::sync::mpsc::channel().0,
        ));

        let render = |app: &mut App| {
            let mut terminal = Terminal::new(TestBackend::new(90, 16)).unwrap();
            terminal.draw(|f| super::draw(f, app)).unwrap();
            let buffer = terminal.backend().buffer().clone();
            (0..16)
                .map(|y| {
                    (0..90).map(|x| buffer[(x, y)].symbol().to_string()).collect::<String>()
                })
                .collect::<Vec<_>>()
                .join("\n")
        };

        let first = render(&mut app);
        assert!(first.contains("track 4"), "{first}");
        assert!(first.contains("was  Roygbiv - Boards Of Canada (Official Audio)"), "{first}");
        assert!(first.contains("artist"), "{first}");
        assert!(first.contains("title"), "{first}");
        assert!(first.contains("^s swap"), "the swap key went unlisted:\n{first}");
        // One block, in the box that has the keys.
        assert_eq!(first.matches('\u{2588}').count(), 1, "{first}");
        assert!(first.contains("Roygbiv\u{2588}"), "{first}");

        app.next_field(true);
        let second = render(&mut app);
        assert_eq!(second.matches('\u{2588}').count(), 1, "{second}");
        assert!(second.contains("Boards Of Canada\u{2588}"), "the caret stayed put:\n{second}");
    }

    /* The right half of the header was empty on every screen while the one
       thing a long run cannot say from its rows is how far along it is. */
    #[test]
    fn the_header_carries_the_run_and_gives_it_up_when_narrow() {
        let head = |width: u16, running: bool| {
            let (tx, _rx) = std::sync::mpsc::channel();
            let mut app = App::new(tx, "opus · ~/Music".into());
            app.apply(Msg::Tracks(spread()));
            app.playlist = "Focus".into();
            if !running {
                app.done = Some(Ok("finished".into()));
            }
            let mut terminal = Terminal::new(TestBackend::new(width, 12)).unwrap();
            terminal.draw(|f| super::draw(f, &mut app)).unwrap();
            let buffer = terminal.backend().buffer().clone();
            (0..width)
                .map(|x| buffer[(x, 0)].symbol().to_string())
                .collect::<String>()
        };

        // 358 of 361 settle at once in this fixture, so the bar is nearly full.
        let wide = head(120, true);
        assert!(wide.contains("━━━━"), "no progress bar: {wide:?}");
        assert!(wide.contains("Focus"), "the left half lost its playlist");

        // Half a bar is a lie, so it goes whole or not at all.
        let narrow = head(46, true);
        assert!(!narrow.contains("━━"), "the bar squeezed the playlist out: {narrow:?}");
        assert!(narrow.contains("Focus"), "{narrow:?}");

        /* A finished run has nothing left to estimate, so the space goes to
           what the run was doing, which is otherwise only under `h`. */
        let over = head(120, false);
        assert!(over.contains("opus · ~/Music"), "{over:?}");
        assert!(!over.contains("━━━━"), "a finished run still drew a bar: {over:?}");
    }

    /* The cost of quitting is the whole point of asking, so the box has to
       carry the count. */
    #[test]
    fn the_quit_question_says_what_it_would_cost() {
        let (tx, _rx) = std::sync::mpsc::channel();
        let mut app = App::new(tx, "settings".into());
        app.tracks = vec![track_named(1, "Autobahn"), track_named(2, "Hallogallo")];
        app.tracks[1].status = Status::Downloading;
        app.confirm = Some(crate::app::Confirm::Quit);

        let mut terminal = Terminal::new(TestBackend::new(80, 14)).unwrap();
        terminal.draw(|f| super::draw(f, &mut app)).unwrap();
        let buffer = terminal.backend().buffer().clone();
        let screen: String = (0..14)
            .map(|y| (0..80).map(|x| buffer[(x, y)].symbol().to_string()).collect::<String>())
            .collect::<Vec<_>>()
            .join("\n");
        assert!(screen.contains("quit?"), "{screen}");
        assert!(screen.contains("1 of 2 finished so far"), "{screen}");
        assert!(screen.contains("stops the download"), "{screen}");
        assert!(screen.contains("esc  keep going"), "{screen}");
    }

    /* A key nobody can see is a key nobody presses: the library named its
       keys in the status bar and the track list named none of its own. */
    #[test]
    fn the_track_bar_names_the_key_for_where_the_run_is() {
        let bar = |failures: bool, running: bool| {
            let (tx, _rx) = std::sync::mpsc::channel();
            let mut app = App::new(tx, "settings".into());
            app.tracks = vec![track_named(1, "Autobahn"), track_named(2, "Hallogallo")];
            if failures {
                app.tracks[1].status = Status::Failed;
            }
            if !running {
                app.done = Some(Ok("finished".into()));
            }
            let mut terminal = Terminal::new(TestBackend::new(100, 12)).unwrap();
            terminal.draw(|f| super::draw(f, &mut app)).unwrap();
            let buffer = terminal.backend().buffer().clone();
            (0..100)
                .map(|x| buffer[(x, 11)].symbol().to_string())
                .collect::<String>()
        };

        // Mid-run the post-run keys are dead, so the one live key is offered.
        assert!(bar(false, true).contains("/ filter"), "{:?}", bar(false, true));
        assert!(!bar(false, true).contains("e edit"), "a dead key was offered");

        assert!(bar(false, false).contains("e edit"), "{:?}", bar(false, false));
        // Retry is only an answer when there is something to answer for.
        assert!(!bar(false, false).contains("r retry"), "{:?}", bar(false, false));
        assert!(bar(true, false).contains("r retry"), "{:?}", bar(true, false));
    }

    /* The tally cannot be recovered by pressing anything and the hints can,
       so a narrow bar keeps the counts and drops the hint. */
    #[test]
    fn a_narrow_bar_keeps_the_tally_and_drops_the_hint() {
        let bar = |width: u16| {
            let (tx, _rx) = std::sync::mpsc::channel();
            let mut app = App::new(tx, "settings".into());
            app.tracks = spread();
            app.done = Some(Ok(String::new()));
            let mut terminal = Terminal::new(TestBackend::new(width, 12)).unwrap();
            terminal.draw(|f| super::draw(f, &mut app)).unwrap();
            let buffer = terminal.backend().buffer().clone();
            (0..width)
                .map(|x| buffer[(x, 11)].symbol().to_string())
                .collect::<String>()
        };
        assert!(bar(100).contains("e edit"), "{:?}", bar(100));
        let tight = bar(46);
        assert!(!tight.contains("e edit"), "the hint pushed the tally out: {tight:?}");
        assert!(tight.contains("120 ok"), "{tight:?}");
    }

    /* yt-dlp warns about a video on most good runs, so red everywhere made
       every run look broken and hid the one line that mattered. */
    #[test]
    fn the_log_pane_separates_a_warning_from_an_error() {
        let (tx, _rx) = std::sync::mpsc::channel();
        let mut app = App::new(tx, "settings".into());
        app.tracks = vec![track_named(1, "Autobahn")];
        app.done = Some(Ok(String::new()));
        app.show_logs = true;
        app.apply(Msg::Log("WARNING: nothing to merge".into()));
        app.apply(Msg::Log("ERROR: unable to download".into()));
        app.apply(Msg::Log("[info] writing thumbnail".into()));

        let mut terminal = Terminal::new(TestBackend::new(80, 14)).unwrap();
        terminal.draw(|f| super::draw(f, &mut app)).unwrap();
        let buffer = terminal.backend().buffer().clone();
        let colour_of = |needle: &str| {
            (0..14)
                .find_map(|y| {
                    let row: String =
                        (0..80).map(|x| buffer[(x, y)].symbol().to_string()).collect();
                    row.contains(needle).then(|| buffer[(2, y)].fg)
                })
                .unwrap_or_else(|| panic!("{needle} never drew"))
        };
        assert_eq!(colour_of("ERROR"), crate::theme::WARM.error);
        assert_eq!(colour_of("WARNING"), crate::theme::WARM.warn);
        assert_eq!(colour_of("[info]"), crate::theme::WARM.muted);
    }

    /* The block used to sit at the end of the line whatever the caret was
       doing, so an edit in the middle drew the cursor somewhere it was not. */
    #[test]
    fn the_prompt_draws_its_caret_where_the_next_letter_lands() {
        let screen = |back: usize| {
            let (tx, _rx) = std::sync::mpsc::channel();
            let mut app = App::new(tx, "settings".into());
            app.apply(Msg::Ask(
                crate::app::Prompt::Input {
                    note: String::new(),
                    header: "Title".into(),
                    value: "Autobahn".into(),
                    escape: crate::app::Escape::Skip,
                },
                std::sync::mpsc::channel().0,
            ));
            for _ in 0..back {
                app.input_move(false);
            }
            let mut terminal = Terminal::new(TestBackend::new(60, 12)).unwrap();
            terminal.draw(|f| super::draw(f, &mut app)).unwrap();
            let buffer = terminal.backend().buffer().clone();
            (0..12)
                .map(|y| {
                    (0..60)
                        .map(|x| buffer[(x, y)].symbol().to_string())
                        .collect::<String>()
                })
                .find(|r| r.contains("Auto"))
                .expect("the prompt drew no text")
        };
        assert!(screen(0).contains("Autobahn\u{2588}"), "{:?}", screen(0));
        assert!(screen(4).contains("Auto\u{2588}bahn"), "{:?}", screen(4));
    }

    /* The status words are the app's own vocabulary, so the overlay is where
       they get explained. It gives way on a short terminal rather than being
       clipped by the border with nothing saying so. */
    #[test]
    fn the_help_glosses_the_statuses_when_there_is_room() {
        let help = |height: u16, view: View| {
            let (tx, _rx) = std::sync::mpsc::channel();
            let mut app = App::new(tx, "settings".into());
            app.tracks = vec![track_named(1, "Autobahn")];
            app.done = Some(Ok("finished".into()));
            app.show_help = true;
            if view == View::Library {
                app.apply(Msg::Library { shelves: shelves(), show: true });
            }
            app.intro_done = true;
            let mut terminal = Terminal::new(TestBackend::new(90, height)).unwrap();
            terminal.draw(|f| super::draw(f, &mut app)).unwrap();
            let buffer = terminal.backend().buffer().clone();
            (0..height)
                .map(|y| {
                    (0..90)
                        .map(|x| buffer[(x, y)].symbol().to_string())
                        .collect::<String>()
                })
                .collect::<Vec<String>>()
                .join("\n")
        };
        let tall = help(40, View::Tracks);
        assert!(tall.contains("guessed"), "no legend on a tall terminal:\n{tall}");
        assert!(tall.contains("no match"), "{tall}");
        // The words alone say nothing without what to do about them.
        assert!(tall.contains("a guess, worth a look"), "{tall}");
        assert!(tall.contains("l has the reason"), "{tall}");

        // The keys are what the overlay is for, so they are what stays.
        let short = help(20, View::Tracks);
        assert!(!short.contains("guessed"), "the legend crowded out the keys:\n{short}");
        assert!(short.contains("edit this track"), "{short}");

        // The library has no statuses on it, so it has nothing to explain.
        assert!(!help(40, View::Library).contains("guessed"));
    }

    /* `missing` is only on some rows, so without a fixed slot the sync column
       stepped sideways between rows and the eye had to re-find it. */
    #[test]
    fn the_sync_column_holds_still_whether_or_not_files_are_missing() {
        let rows = library_screen(80);
        let col = |r: &String| r.chars().position(|c| c == 's').map(|_| {
            let text: Vec<char> = r.chars().collect();
            let want: Vec<char> = "synced".chars().collect();
            let never: Vec<char> = "never synced".chars().collect();
            (0..text.len())
                .find(|&i| text[i..].starts_with(&want) || text[i..].starts_with(&never))
                .unwrap()
        });
        let focus = col(&rows[2]).expect("no sync time on the Focus row");
        let road = col(&rows[3]).expect("no sync time on the Road trip row");
        assert_eq!(focus, road, "{:?}\n{:?}", rows[2], rows[3]);
    }

    /* The bar alone is a thin signal for "this is what the next key acts on",
       so the name on the cursor row is bold and no other name is. */
    #[test]
    fn only_the_cursor_rows_name_is_bold() {
        let bold_names = |width: u16, shelf: usize| {
            let (tx, _rx) = std::sync::mpsc::channel();
            let mut app = App::new(tx, "settings".into());
            app.apply(Msg::Library { shelves: shelves(), show: true });
            app.shelf = shelf;
            app.intro_done = true;
            let mut terminal = Terminal::new(TestBackend::new(width, 12)).unwrap();
            terminal.draw(|f| super::draw(f, &mut app)).unwrap();
            let buffer = terminal.backend().buffer().clone();
            // The first letter of each name sits two cells in from the marker.
            (2..4)
                .map(|y| buffer[(2, y)].modifier.contains(Modifier::BOLD))
                .collect::<Vec<bool>>()
        };
        assert_eq!(bold_names(80, 0), [true, false]);
        assert_eq!(bold_names(80, 1), [false, true]);

        // And the same on the track list, where the cursor is a track index.
        let (tx, _rx) = std::sync::mpsc::channel();
        let mut app = App::new(tx, "settings".into());
        app.tracks = vec![track_named(1, "Autobahn"), track_named(2, "Hallogallo")];
        app.done = Some(Ok("finished".into()));
        app.cursor = 1;
        let mut terminal = Terminal::new(TestBackend::new(80, 12)).unwrap();
        terminal.draw(|f| super::draw(f, &mut app)).unwrap();
        let buffer = terminal.backend().buffer().clone();
        let name_at = |y: u16| {
            let row: String = (0..80).map(|x| buffer[(x, y)].symbol().to_string()).collect();
            let x = row.chars().position(|c| c == 'A' || c == 'H').unwrap() as u16;
            buffer[(x, y)].modifier.contains(Modifier::BOLD)
        };
        assert!(!name_at(2), "the row above the cursor is bold");
        assert!(name_at(3), "the cursor row is not bold");
    }

    /* The shelf row says a sync would re-download two files but never which,
       which is the whole reason the pane is there. It follows the cursor
       without a cursor of its own, and a folder longer than the pane says how
       many it could not show rather than ending mid-list. A bordered column
       of filenames with no head to it says nothing about which folder it
       belongs to, so the first line is the counts. */
    #[test]
    fn the_preview_shows_the_selected_folders_tracks() {
        let rows = library_screen_at(120, 1);
        let pane: String = rows[2..10].join("\n");
        assert!(rows[2].contains("9 tracks  ·  2 missing"), "{:?}", rows[2]);
        assert!(pane.contains("Autobahn"), "{pane:?}");
        assert!(pane.contains("Vitamin C"), "{pane:?}");
        // The missing one is marked, not merely coloured.
        let gone = rows[3..10]
            .iter()
            .find(|r| r.contains("Hallogallo"))
            .expect("the missing track is not in the pane");
        assert!(gone.contains("!02 Neu! - Hallogallo"), "{gone:?}");
        // The name is enough; the extension is noise in a preview.
        assert!(!pane.contains(".opus"), "{pane:?}");

        let long = library_screen_at(120, 0);
        let pane: String = long[2..10].join("\n");
        assert!(long[2].contains("42 tracks"), "{:?}", long[2]);
        assert!(!long[2].contains("missing"), "nothing is missing here: {:?}", long[2]);
        assert!(pane.contains("Track 1"), "{pane:?}");
        // The head costs the pane a row, and the tail count is what says so.
        assert!(pane.contains("Track 6"), "{pane:?}");
        assert!(!pane.contains("Track 7"), "a row overflowed the pane: {pane:?}");
        assert!(pane.contains("… 36 more"), "{pane:?}");
    }

    /* The answer to "did my format change take", and the one place on the
       library screen that gives it. Not a column on the row, which reserves
       its width whether or not anything is in it, so a count that is zero for
       everyone not converting would cost every folder name ten columns. */
    #[test]
    fn the_preview_says_how_many_tracks_are_not_in_the_current_format() {
        let screen = |format: &str| {
            let (tx, _rx) = std::sync::mpsc::channel();
            let mut app = App::new(tx, "settings".into());
            app.apply(Msg::Library { shelves: shelves(), show: true });
            app.intro_done = true;
            app.format = format.into();
            let mut terminal = Terminal::new(TestBackend::new(120, 12)).unwrap();
            terminal.draw(|f| super::draw(f, &mut app)).unwrap();
            let buffer = terminal.backend().buffer().clone();
            (0..12)
                .map(|y| {
                    (0..120)
                        .map(|x| buffer[(x, y)].symbol().to_string())
                        .collect::<String>()
                })
                .collect::<Vec<_>>()
                .join("\n")
        };

        /* The fixture's first folder is 42 opus files, so in an opus library
           there is nothing to say and the line must not be there at all. */
        let same = screen("opus");
        assert!(same.contains("42 tracks"), "{same:?}");
        assert!(!same.contains("not opus"), "a line appeared with nothing to report");

        let other = screen("flac");
        assert!(other.contains("42 not flac"), "{other:?}");
        // And the head line it sits under is still there, not pushed off.
        assert!(other.contains("42 tracks"), "{other:?}");
        /* The pane lost a row to it, so the tail count has to have moved
           rather than a track silently falling off the bottom. */
        assert!(other.contains("… 37 more"), "the pane overflowed: {other:?}");
    }

    /* The key deletes files, so the bar names how many, and it is lit only
       for departures a listing proved: one read off the playlist file looks
       identical on the row and must not offer a deletion. */
    #[test]
    fn the_bar_offers_a_purge_only_for_proven_departures() {
        let screen = |proven: bool| {
            let (tx, _rx) = std::sync::mpsc::channel();
            let mut app = App::new(tx, "settings".into());
            app.intro_done = true;
            app.done = Some(Ok(String::new()));
            for (i, status) in [Status::Ok, Status::Gone, Status::Gone].iter().enumerate() {
                let mut t = Track::new(i + 1, "id".into(), format!("T{i}"), std::path::PathBuf::from("/x"));
                t.status = *status;
                t.departure_proven = proven && *status == Status::Gone;
                app.tracks.push(t);
            }
            let mut terminal = Terminal::new(TestBackend::new(120, 12)).unwrap();
            terminal.draw(|f| super::draw(f, &mut app)).unwrap();
            let buffer = terminal.backend().buffer().clone();
            (0..12)
                .map(|y| (0..120).map(|x| buffer[(x, y)].symbol().to_string()).collect::<String>())
                .collect::<Vec<_>>()
                .join("\n")
        };
        let lit = screen(true);
        assert!(lit.contains("D delete 2 departed"), "{lit:?}");
        let dark = screen(false);
        assert!(!dark.contains("D delete"), "offered for offline departures: {dark:?}");
    }

    /* The shelf row already needs its name plus 40 columns of counts, so a
       pane at this width would leave nothing for either. */
    #[test]
    fn a_narrow_library_has_no_preview_at_all() {
        let rows = library_screen(80);
        assert!(
            !rows[2..10].iter().any(|r| r.contains('│')),
            "the pane divider is drawn at 80 columns: {rows:?}"
        );
        assert!(!rows.join("\n").contains("Autobahn"), "{rows:?}");
    }

    /* Only the matching rows, in order, with the marker on the cursor's own
       track. The marker and the command both come from `pos == cursor`, so
       they cannot disagree; what this pins down is which rows get drawn. */
    #[test]
    fn the_highlighted_row_is_the_track_a_command_would_act_on() {
        let (tx, _rx) = std::sync::mpsc::channel();
        let mut app = App::new(tx, "settings".into());
        /* Three matches with the cursor on the middle one, and hidden rows in
           between, so a drawn row taken from the wrong number picks a
           different track instead of clamping onto the right one. */
        for (n, name) in [
            "Aphex Twin - Xtal",
            "Boards - Olson",
            "Aphex Twin - Avril",
            "Boards - Dayvan",
            "Aphex Twin - Rhubarb",
        ]
        .iter()
        .enumerate()
        {
            let mut track = Track::new(
                n + 1,
                "id".into(),
                (*name).into(),
                std::path::PathBuf::from("/tmp/x.opus"),
            );
            track.status = Status::Ok;
            app.tracks.push(track);
        }
        app.done = Some(Ok("finished".into()));
        app.filter = "aphex".into();
        app.cursor = 2;

        let mut terminal = Terminal::new(TestBackend::new(60, 10)).unwrap();
        terminal.draw(|f| super::draw(f, &mut app)).unwrap();
        let buffer = terminal.backend().buffer().clone();
        let rows: Vec<String> = (2..6)
            .map(|y| {
                (0..60)
                    .map(|x| buffer[(x, y)].symbol().to_string())
                    .collect::<String>()
                    .trim_end()
                    .to_string()
            })
            .collect();

        assert!(rows[0].contains("Xtal"), "{rows:?}");
        assert!(rows[1].contains("Avril"), "the filtered-out row was drawn");
        assert!(rows[2].contains("Rhubarb"), "{rows:?}");
        assert!(rows[3].is_empty(), "more rows than the filter allows");

        let highlighted: Vec<&String> = rows.iter().filter(|r| r.starts_with('▌')).collect();
        assert_eq!(highlighted.len(), 1, "{rows:?}");
        assert!(
            highlighted[0].contains("Avril"),
            "the highlight is on a different track than the cursor: {highlighted:?}"
        );
        assert_eq!(app.selected(), Some(3), "and the command would act elsewhere");
    }

    /* `row_of_cursor` is what scrolls the pane, and it is the one place the
       two numberings can be confused. Given the cursor on the first match,
       feeding the list a `tracks` position instead scrolls to the end and the
       selected row leaves the screen entirely. */
    #[test]
    fn a_filtered_list_scrolls_to_the_row_the_cursor_is_on() {
        let (tx, _rx) = std::sync::mpsc::channel();
        let mut app = App::new(tx, "settings".into());
        // Ten rows the filter hides, so a match's position is far from its row.
        for n in 0..10 {
            app.tracks.push(track_named(n + 1, &format!("Boards - {n}")));
        }
        for n in 0..5 {
            app.tracks.push(track_named(n + 11, &format!("Aphex Twin - {n}")));
        }
        app.done = Some(Ok("finished".into()));
        app.filter = "aphex".into();
        app.cursor = 10;
        assert_eq!(app.row_of_cursor(&app.rows()), Some(0), "the first match is row 0");

        // Body is two lines at this height, so five matches have to scroll.
        let mut terminal = Terminal::new(TestBackend::new(60, 6)).unwrap();
        terminal.draw(|f| super::draw(f, &mut app)).unwrap();
        let buffer = terminal.backend().buffer().clone();
        let screen: String = (2..4)
            .map(|y| {
                (0..60)
                    .map(|x| buffer[(x, y)].symbol().to_string())
                    .collect::<String>()
            })
            .collect();
        assert!(
            screen.contains("Aphex Twin - 0"),
            "the selected row scrolled off screen: {screen:?}"
        );
    }

    /* A narrowed list looks exactly like a short run, so the one signal that
       says otherwise has to be unmissable: a band across the full width, not a
       hint sharing the status bar with everything else. */
    #[test]
    fn an_active_filter_gets_a_band_of_its_own() {
        let (tx, _rx) = std::sync::mpsc::channel();
        let mut app = App::new(tx, "settings".into());
        app.tracks = spread();
        app.filter = "one".into();
        let mut terminal = Terminal::new(TestBackend::new(60, 10)).unwrap();
        terminal.draw(|f| super::draw(f, &mut app)).unwrap();
        let buffer = terminal.backend().buffer().clone();

        let band: String = (0..60).map(|x| buffer[(x, 1)].symbol().to_string()).collect();
        assert!(band.contains("FILTER"), "{band:?}");
        assert!(band.contains("/one"), "{band:?}");
        assert!(
            band.contains(&format!("{} of {} shown", app.shown(), app.tracks.len())),
            "{band:?}"
        );
        for x in 0..60u16 {
            assert_eq!(buffer[(x, 1)].bg, crate::theme::WARM.accent, "the band breaks at {x}");
        }

        /* Narrow enough that the hint cannot fit beside the query, which is
           where the band used to stop early and let the gradient back in. */
        let mut narrow = Terminal::new(TestBackend::new(30, 10)).unwrap();
        narrow.draw(|f| super::draw(f, &mut app)).unwrap();
        for x in 0..30u16 {
            assert_eq!(
                narrow.backend().buffer()[(x, 1)].bg,
                crate::theme::WARM.accent,
                "the band breaks at {x} once the hint no longer fits"
            );
        }

        // Narrower than the label itself, where the fill has nothing left to do.
        for w in [1u16, 8, 13] {
            let mut t = Terminal::new(TestBackend::new(w, 10)).unwrap();
            t.draw(|f| super::draw(f, &mut app)).unwrap();
            for x in 0..w {
                assert_eq!(t.backend().buffer()[(x, 1)].bg, crate::theme::WARM.accent, "at width {w}, column {x}");
            }
        }

        // And it is gone once the filter is, so the rule comes back.
        app.clear_filter();
        terminal.draw(|f| super::draw(f, &mut app)).unwrap();
        let rule: String = (0..60)
            .map(|x| terminal.backend().buffer()[(x, 1)].symbol().to_string())
            .collect();
        assert_eq!(rule, "─".repeat(60));
    }

    /* `p` errors unless cliamp is up, so it is offered only then, and the
       status bar has to say why it went rather than leaving a key that was
       there a moment ago unexplained. */
    #[test]
    fn the_play_key_and_its_indicator_follow_cliamp() {
        let bar = |state: crate::app::Player| {
            let (tx, _rx) = std::sync::mpsc::channel();
            let mut app = App::new(tx, "settings".into());
            app.tracks = spread();
            app.done = Some(Ok("finished".into()));
            app.apply(Msg::Player(crate::app::Playback {
                player: state,
                ..Default::default()
            }));
            app.show_help = true;
            let mut terminal = Terminal::new(TestBackend::new(100, 24)).unwrap();
            terminal.draw(|f| super::draw(f, &mut app)).unwrap();
            let buffer = terminal.backend().buffer().clone();
            let screen: String = (0..24)
                .map(|y| {
                    (0..100)
                        .map(|x| buffer[(x, y)].symbol().to_string())
                        .collect::<String>()
                })
                .collect::<Vec<_>>()
                .join("\n");
            (screen, app.can_play())
        };

        let (running, can) = bar(crate::app::Player::Running);
        assert!(can, "cliamp was up and p was still refused");
        assert!(running.contains("♪ cliamp"), "no indicator: {running}");
        assert!(!running.contains("cliamp off"), "{running}");
        assert!(running.contains("play it in cliamp"), "the key went unlisted");

        let (stopped, can) = bar(crate::app::Player::Stopped);
        assert!(!can, "p was offered with nothing to hand it to");
        assert!(stopped.contains("♪ cliamp off"), "no reason given: {stopped}");
        assert!(!stopped.contains("play it in cliamp"), "the key is still listed");

        // Nobody needs telling about a player they have not installed.
        let (missing, can) = bar(crate::app::Player::Missing);
        assert!(!can);
        assert!(!missing.contains("cliamp"), "{missing}");
    }

    /* The keys mean something different while the pick is open and the worker
       is blocked on the answer, so the band has to say both what is selected
       and what Enter will do. */
    #[test]
    fn the_pick_band_says_what_is_selected_and_what_the_keys_do() {
        let (tx, _rx) = std::sync::mpsc::channel();
        let mut app = App::new(tx, "settings".into());
        app.tracks = spread();
        let (reply, _back) = std::sync::mpsc::channel();
        app.apply(Msg::Pick(reply));

        let mut terminal = Terminal::new(TestBackend::new(90, 10)).unwrap();
        terminal.draw(|f| super::draw(f, &mut app)).unwrap();
        let buffer = terminal.backend().buffer().clone();
        let band: String = (0..90).map(|x| buffer[(x, 1)].symbol().to_string()).collect();

        assert!(band.contains("PICK"), "{band:?}");
        assert!(
            band.contains(&format!("{} of {} selected", app.marked.len(), app.tracks.len())),
            "{band:?}"
        );
        assert!(band.contains("enter downloads"), "{band:?}");
        assert!(band.contains("esc none"), "{band:?}");
        for x in 0..90u16 {
            assert_eq!(buffer[(x, 1)].bg, crate::theme::WARM.accent, "the band breaks at {x}");
        }

        // Nothing is running behind a question, so the spinner must not claim so.
        let header: String = (0..90).map(|x| buffer[(x, 0)].symbol().to_string()).collect();
        assert!(
            !super::SPINNER.iter().any(|s| header.contains(s)),
            "the header span while waiting on a keypress: {header:?}"
        );
    }

    /* The pick band replaces the filter band, so it has to carry the filter or
       a narrowed list goes unannounced while `a` acts on it. */
    #[test]
    fn the_pick_band_still_shows_an_active_filter() {
        let (tx, _rx) = std::sync::mpsc::channel();
        let mut app = App::new(tx, "settings".into());
        app.tracks = spread();
        let (reply, _back) = std::sync::mpsc::channel();
        app.apply(Msg::Pick(reply));
        app.filter = "one".into();

        let mut terminal = Terminal::new(TestBackend::new(90, 10)).unwrap();
        terminal.draw(|f| super::draw(f, &mut app)).unwrap();
        let band: String = (0..90)
            .map(|x| terminal.backend().buffer()[(x, 1)].symbol().to_string())
            .collect();
        assert!(band.contains("PICK"), "{band:?}");
        assert!(band.contains("/one"), "{band:?}");
    }

    #[test]
    fn a_filter_that_matches_nothing_says_so_rather_than_going_blank() {
        let (tx, _rx) = std::sync::mpsc::channel();
        let mut app = App::new(tx, "settings".into());
        app.tracks = spread();
        app.filter = "nothing here".into();
        let mut terminal = Terminal::new(TestBackend::new(60, 10)).unwrap();
        terminal.draw(|f| super::draw(f, &mut app)).unwrap();
        let buffer = terminal.backend().buffer().clone();
        let row: String = (0..60).map(|x| buffer[(x, 2)].symbol().to_string()).collect();
        assert!(row.contains("nothing matches"), "{row:?}");
    }

    /* Reachable by emptying --dir with the screen open. The pane was blank,
       and the keys it advertises have to still work or the message is a lie. */
    #[test]
    fn an_emptied_library_says_so_and_still_offers_a_way_out() {
        let (tx, _rx) = std::sync::mpsc::channel();
        let mut app = App::new(tx, "settings".into());
        app.apply(Msg::Library {
            shelves: shelves(),
            show: true,
        });
        app.apply(Msg::Library {
            shelves: Vec::new(),
            show: false,
        });
        assert!(app.can_browse(), "n would do nothing on an empty library");

        app.intro_done = true;
        let mut terminal = Terminal::new(TestBackend::new(80, 12)).unwrap();
        terminal.draw(|f| super::draw(f, &mut app)).unwrap();
        let buffer = terminal.backend().buffer().clone();
        let row: String = (0..80).map(|x| buffer[(x, 2)].symbol().to_string()).collect();
        assert!(row.contains("no playlists"), "{row:?}");
    }

    /* The tally counts track statuses, and the library has no tracks: an
       opened-then-abandoned playlist used to leave its counts on the bar. */
    #[test]
    fn the_library_bar_does_not_tally_a_playlist_it_is_not_showing() {
        let (tx, _rx) = std::sync::mpsc::channel();
        let mut app = App::new(tx, "settings".into());
        app.tracks = spread();
        app.done = Some(Ok("finished".into()));
        app.apply(Msg::Library {
            shelves: shelves(),
            show: true,
        });
        let mut terminal = Terminal::new(TestBackend::new(80, 12)).unwrap();
        terminal.draw(|f| super::draw(f, &mut app)).unwrap();
        let buffer = terminal.backend().buffer().clone();
        let bar: String = (0..80).map(|x| buffer[(x, 11)].symbol().to_string()).collect();
        assert!(!bar.contains("120 ok"), "{bar:?}");
        assert!(bar.contains("h keys"), "{bar:?}");
    }

    /* The tally is recoverable by looking at the list; the key hints are not,
       so they must survive a playlist wide enough to overflow the bar. */
    #[test]
    fn the_key_hints_survive_a_full_status_bar() {
        let logs = vec!["a warning".to_string(); 42];
        for width in [40u16, 60, 80, 100, 200] {
            let row = status_row(width, spread(), logs.clone());
            assert!(
                row.ends_with("h keys"),
                "at {width} columns the hints were clipped: {row:?}"
            );
            assert!(
                row.chars().count() <= width as usize,
                "at {width} columns the bar overflowed: {row:?}"
            );
        }
    }

    /* Widgets patch only the foreground, so a Reset background anywhere means
       something painted over the gradient and would show the terminal's own. */
    #[test]
    fn the_gradient_reaches_every_cell() {
        let (tx, _rx) = std::sync::mpsc::channel();
        let mut app = App::new(tx, "settings".into());
        app.tracks = spread();
        app.logs = vec!["a warning".into()];
        app.show_logs = true;
        let mut terminal = Terminal::new(TestBackend::new(60, 24)).unwrap();
        terminal.draw(|f| super::draw(f, &mut app)).unwrap();
        let buffer = terminal.backend().buffer().clone();

        for y in 0..24u16 {
            for x in 0..60u16 {
                let want = crate::theme::WARM.background(x, y, 60, 24);
                assert_eq!(buffer[(x, y)].bg, want, "at {x},{y}");
            }
        }
        // Bottom-left and top-right are the two ends of the diagonal.
        assert_ne!(
            crate::theme::WARM.background(0, 23, 60, 24),
            crate::theme::WARM.background(59, 0, 60, 24),
            "the gradient has to actually fade"
        );
    }

    /* The spinner is the only sign that a command is running, since the keys
       go quiet while one is in flight. */
    #[test]
    fn the_spinner_returns_while_a_command_runs() {
        let finished = |busy: bool| {
            let (tx, _rx) = std::sync::mpsc::channel();
            let mut app = App::new(tx, "settings".into());
            app.tracks = spread();
            app.done = Some(Ok("finished".into()));
            app.busy = busy;
            let mut terminal = Terminal::new(TestBackend::new(80, 24)).unwrap();
            terminal.draw(|f| super::draw(f, &mut app)).unwrap();
            let buffer = terminal.backend().buffer().clone();
            (0..80)
                .map(|x| buffer[(x, 0)].symbol().to_string())
                .collect::<String>()
        };
        let spinning = super::SPINNER.iter().any(|f| finished(true).contains(f));
        let idle = super::SPINNER.iter().any(|f| finished(false).contains(f));
        assert!(spinning, "no spinner while busy");
        assert!(!idle, "spinner left running with nothing to do");
    }

    fn help_screen(can_command: bool) -> Vec<String> {
        let (tx, _rx) = std::sync::mpsc::channel();
        let mut app = App::new(
            tx,
            "parse on · lookup on · apple on · cover on · m3u8 on · prompts on · rename on".into(),
        );
        app.tracks = spread();
        app.show_help = true;
        app.done = can_command.then(|| Ok("done".into()));
        let mut terminal = Terminal::new(TestBackend::new(120, 30)).unwrap();
        terminal.draw(|f| super::draw(f, &mut app)).unwrap();
        let buffer = terminal.backend().buffer().clone();
        (0..30u16)
            .map(|y| (0..120).map(|x| buffer[(x, y)].symbol().to_string()).collect())
            .filter(|row: &String| row.contains('│'))
            .collect()
    }

    /* Every description used to start wherever the group label left it, and a
       six-character label ate the space after itself, printing "markeds". */
    #[test]
    fn the_help_modal_lines_its_columns_up() {
        for can_command in [true, false] {
            let rows = help_screen(can_command);
            assert!(!rows.is_empty(), "no modal drawn");

            let starts: Vec<usize> = rows
                .iter()
                .filter_map(|row| {
                    // Nothing that is a substring of another row's text.
                    [
                        "first, last",
                        "yt-dlp output",
                        "every track like it",
                        "set one artist",
                        "once the run finishes",
                    ]
                        .iter()
                        .find_map(|what| row.find(what))
                })
                .collect();
            assert!(starts.len() >= 2, "not enough rows to compare");
            assert!(
                starts.windows(2).all(|pair| pair[0] == pair[1]),
                "descriptions start at {starts:?}"
            );

            for row in &rows {
                assert!(!row.contains("markeds"), "label ran into its keys: {row}");
            }
        }
    }

    #[test]
    fn a_popup_covers_the_gradient_with_its_own_surface() {
        let (tx, _rx) = std::sync::mpsc::channel();
        let mut app = App::new(tx, "settings".into());
        app.show_help = true;
        let mut terminal = Terminal::new(TestBackend::new(60, 24)).unwrap();
        terminal.draw(|f| super::draw(f, &mut app)).unwrap();
        let buffer = terminal.backend().buffer().clone();

        let area = super::centred(Rect::new(0, 0, 60, 24), super::popup_width(Rect::new(0, 0, 60, 24)), 8);
        let middle = (area.x + area.width / 2, area.y + 1);
        assert_eq!(buffer[middle].bg, crate::theme::WARM.surface);
    }

    #[test]
    fn popup_width_survives_a_very_wide_terminal() {
        for width in [20u16, 80, 936, 937, 2000, u16::MAX] {
            let area = Rect::new(0, 0, width, 24);
            assert!(popup_width(area) <= width, "at {width}");
        }
    }

    #[test]
    fn wraps_on_words_and_splits_overlong_ones() {
        assert_eq!(wrap("a bb ccc", 10), ["a bb ccc"]);
        assert_eq!(
            wrap("aaaaaa bbbbbb cccccc", 10),
            ["aaaaaa", "bbbbbb", "cccccc"]
        );
        assert_eq!(wrap("supercalifragilistic", 8), ["supercal", "ifragili", "stic"]);
        assert_eq!(wrap("", 10), [""]);
        // The caller's indentation, which is a column and not a separator.
        assert_eq!(wrap("  ab cd", 10), ["  ab cd"]);
        assert_eq!(wrap("  aaaa bbbb", 8), ["  aaaa", "bbbb"]);
        assert_eq!(wrap("   ", 10), ["   "]);
    }

    #[test]
    fn never_returns_a_piece_wider_than_the_box() {
        let long = "Gavin Greenaway, The Lyndhurst Orchestra  (existing)";
        for width in 6..40 {
            assert!(wrap(long, width).iter().all(|p| p.chars().count() <= width));
        }
    }

    #[test]
    fn bar_always_fills_its_column() {
        for percent in 0..=100 {
            let (done, rest) = bar(percent, 8);
            assert_eq!(done.chars().count() + rest.chars().count(), 8, "at {percent}%");
        }
        assert_eq!(bar(0, 8).0, "");
        assert_eq!(bar(100, 8).1, "");
    }

    fn intro_rows(ms: u64, width: u16, height: u16) -> Vec<String> {
        intro_rows_with(ms, width, height, None)
    }


    fn intro_rows_with(ms: u64, width: u16, height: u16, update: Option<&str>) -> Vec<String> {
        let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
        let p = crate::theme::WARM;
        terminal
            .draw(|f| {
                super::draw_background(f, p);
                super::draw_intro(f, ms, update, p);
            })
            .unwrap();
        let buffer = terminal.backend().buffer().clone();
        (0..height)
            .map(|y| {
                (0..width)
                    .map(|x| buffer[(x, y)].symbol().to_string())
                    .collect::<String>()
                    .trim_end()
                    .to_string()
            })
            .collect()
    }

    /* Driven by elapsed milliseconds and not by `tick`, so it has to land the
       same way whether the loop is idling at its poll timeout or spinning on
       messages. These are the three moments the animation is made of. */
    #[test]
    fn the_intro_reveals_the_wordmark_then_the_wave_then_the_signature() {
        let early = intro_rows(100, 64, 14).join("\n");
        assert!(early.contains("█████"), "no wordmark at all: {early:?}");
        assert!(!early.contains("tagged"), "the signature came in first");

        let landed = intro_rows(1400, 64, 14).join("\n");
        // Every letter of EARWORM, which is the row the crossbars all sit on.
        assert!(
            landed.lines().any(|l| l.matches('█').count() > 20),
            "the wordmark never filled out: {landed:?}"
        );
        assert!(
            landed.lines().any(|l| l.chars().any(|c| super::WAVE.contains(&c))),
            "no wave: {landed:?}"
        );
        assert!(landed.contains(TAGLINE), "{landed:?}");

        // The reveal is left to right, so an early frame is strictly shorter.
        let widest = |rows: Vec<String>| rows.iter().map(|r| r.chars().count()).max().unwrap_or(0);
        assert!(
            widest(intro_rows(100, 64, 14)) < widest(intro_rows(700, 64, 14)),
            "the letters did not land one at a time"
        );
    }

    /* The wordmark is 41 columns and would wrap into nonsense below that, and
       the frame can be any size at all while a terminal is being dragged. */
    #[test]
    fn the_intro_survives_any_size_it_is_given() {
        for (w, h) in [(1u16, 1u16), (3, 5), (8, 4), (20, 9), (43, 10), (64, 14), (400, 90)] {
            let rows = intro_rows(1400, w, h);
            assert_eq!(rows.len(), h as usize, "at {w}x{h}");
            assert!(
                rows.iter().all(|r| r.chars().count() <= w as usize),
                "a row ran past the frame at {w}x{h}: {rows:?}"
            );
        }
        // Too narrow for the block letters, so the plain word stands in.
        assert!(intro_rows(1400, 30, 12).join("\n").contains("earworm"));
    }

    /* A newer release is said once, in the one line that was already there:
       the numbers lead and the hint follows, so a narrow frame keeps the
       news and drops the way to act on it rather than the other round. */
    #[test]
    fn the_signature_names_the_update_when_there_is_one() {
        let plain = intro_rows(1400, 100, 14).join("\n");
        assert!(plain.contains(TAGLINE), "{plain:?}");
        assert!(!plain.contains('→'), "nothing newer said something: {plain:?}");

        /* The `v` is added by this line, not carried in: a release tag arrives
           `v0.2.0`, update strips it, and `notice` renders `v0.1.0 → v0.2.0`
           rather than `vv0.2.0`. The current version is read from the crate,
           not hardcoded, or a release bump would fail this test. */
        let told = intro_rows_with(1400, 100, 14, Some("0.2.0")).join("\n");
        let want = format!("v{} → v0.2.0", crate::update::current());
        assert!(told.contains(TAGLINE), "{told:?}");
        assert!(told.contains(&want), "{told} has no {want}");
        assert!(!told.contains("vv"), "doubled the v: {told:?}");
        assert!(told.contains("cargo install"), "{told:?}");

        // The hint yields before the numbers do.
        let narrow = intro_rows_with(1400, 60, 14, Some("0.2.0")).join("\n");
        assert!(narrow.contains(&want), "{narrow:?}");
    }

    /* It covers the scan and nothing else: anything worth reading outranks it,
       and a key skips it, or the animation is in the way rather than polish. */
    #[test]
    fn the_intro_gives_way_to_anything_real() {
        let fresh = || {
            let (tx, _rx) = std::sync::mpsc::channel();
            App::new(tx, "settings".into())
        };
        assert!(fresh().intro(), "a bare start had nothing else to show");

        let mut skipped = fresh();
        skipped.intro_done = true;
        assert!(!skipped.intro(), "a key did not skip it");

        let mut running = fresh();
        running.tracks = spread();
        assert!(!running.intro(), "it sat over the track list");

        let mut helping = fresh();
        helping.show_help = true;
        assert!(!helping.intro(), "it sat under the help modal");

        /* The library is ready in milliseconds on a bare start, so yielding to
           it is how the intro went unseen for anyone who runs `earworm` alone.
           Nothing is waiting on the user behind a list, so it holds. */
        let mut browsing = fresh();
        browsing.apply(Msg::Library { shelves: shelves(), show: true });
        assert_eq!(browsing.view, crate::app::View::Library);
        assert!(browsing.intro(), "the library cut the intro short again");

        /* A prompt is the exception: the worker is blocked on an answer, and
           a question the user cannot see is not polish. */
        let mut asked = fresh();
        let (reply, _back) = std::sync::mpsc::channel();
        asked.apply(Msg::Ask(
            crate::app::Prompt::Input {
                note: String::new(),
                header: "YouTube playlist URL".into(),
                value: String::new(),
                escape: crate::app::Escape::Quit,
            },
            reply,
        ));
        assert!(!asked.intro(), "it hid the question the worker is waiting on");

        let mut finished = fresh();
        finished.done = Some(Ok("finished".into()));
        assert!(!finished.intro(), "it outlived the run");

        // And the frame it leaves behind is the ordinary one, header included.
        let mut app = fresh();
        app.tracks = spread();
        let mut terminal = Terminal::new(TestBackend::new(64, 14)).unwrap();
        terminal.draw(|f| super::draw(f, &mut app)).unwrap();
        let top: String = (0..64)
            .map(|x| terminal.backend().buffer()[(x, 0)].symbol().to_string())
            .collect();
        assert!(top.contains("earworm  ·"), "{top:?}");
    }
}



