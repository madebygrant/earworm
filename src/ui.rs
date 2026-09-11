use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{
    Block, Borders, Clear, List, ListItem, ListState, Paragraph, Scrollbar, ScrollbarOrientation,
    ScrollbarState,
};

use crate::app::{App, Player, Prompt, Shelf, Status, View};
use crate::manifest;

use crate::theme::{self, AMBER, CREAM, DIM, GOLD, GREEN, INK, RED, RULE, SURFACE};

const SPINNER: [&str; 8] = ["⣷", "⣯", "⣟", "⡿", "⢿", "⣻", "⣽", "⣾"];
const STATUS_WIDTH: usize = 8;
/// Narrowest library screen that still has room for a preview beside the rows.
const PREVIEW_FROM: u16 = 100;

fn dim(text: impl Into<String>) -> Span<'static> {
    Span::styled(text.into(), Style::new().fg(DIM))
}

pub fn draw(frame: &mut Frame, app: &mut App) {
    // Takes the whole frame: a header above it would be the thing you read.
    if app.intro() {
        draw_background(frame);
        draw_intro(frame, app.started.elapsed().as_millis() as u64);
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

    draw_background(frame);
    draw_header(frame, app, header);
    // The pick wins: it says what the keys do, and it carries the filter.
    if app.picking.is_some() {
        draw_pick_bar(frame, app, rule);
    } else if app.filtering() {
        draw_filter_bar(frame, app, rule);
    } else {
        draw_rule(frame, rule);
    }
    if app.show_logs && !app.logs.is_empty() {
        let [list, logs] =
            Layout::vertical([Constraint::Min(1), Constraint::Length(8)]).areas(body);
        draw_body(frame, app, list);
        draw_logs(frame, app, logs);
    } else {
        draw_body(frame, app, body);
    }
    draw_rule(frame, footrule);
    draw_status(frame, app, status);

    if app.show_help {
        draw_help(frame, app);
    }
    if app.prompt.is_some() {
        draw_prompt(frame, app);
    }
}

/* Painted before anything else: every other widget styles only its foreground,
   so the gradient survives underneath them. */
fn draw_background(frame: &mut Frame) {
    let area = frame.area();
    let buffer = frame.buffer_mut();
    for y in area.top()..area.bottom() {
        for x in area.left()..area.right() {
            let bg = theme::background(x - area.left(), y - area.top(), area.width, area.height);
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
fn draw_intro(frame: &mut Frame, ms: u64) {
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
                let lit = theme::glow((ms - due) as f32 / FADE);
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
                Style::new().fg(theme::glow(ms as f32 / FADE)),
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
            Paragraph::new(Line::from(Span::styled(wave, Style::new().fg(AMBER)))),
            Rect::new(left, top + mark_rows + 1, width, 1),
        );
    }

    // Last, so it reads as a signature rather than part of the animation.
    if ms > wave_at + 250 {
        let sign = format!("youtube playlists, tagged  ·  v{}", env!("CARGO_PKG_VERSION"));
        let sign = truncate(&sign, area.width as usize);
        let at = area.x + (area.width - sign.chars().count() as u16) / 2;
        frame.render_widget(
            Paragraph::new(Line::from(dim(sign))),
            Rect::new(at, top + mark_rows + 3, area.right() - at, 1),
        );
    }
}

fn draw_rule(frame: &mut Frame, area: Rect) {
    frame.render_widget(
        Paragraph::new(Span::styled(
            "─".repeat(area.width as usize),
            Style::new().fg(RULE),
        )),
        area,
    );
}

/* A mode where the list is not the whole playlist, or the keys do not mean
   what they usually mean, takes the rule's row as a filled band rather than a
   hint among hints. It deliberately covers the gradient, which is what makes
   it impossible to miss. */
fn draw_band(frame: &mut Frame, area: Rect, left: String, mut right: String) {
    let width = area.width as usize;
    let band = Style::new().fg(INK).bg(GOLD);
    let used = left.chars().count();
    let mut spans = vec![Span::styled(left, band.add_modifier(Modifier::BOLD))];
    /* Paragraph styles only the cells it writes, so the padding is what keeps
       the gradient out. It runs to the full width even when that costs the
       hint: a torn band reads as a rendering fault. */
    let rest = width.saturating_sub(used);
    let gap = if used + right.chars().count() <= width {
        rest - right.chars().count()
    } else {
        right.clear();
        rest
    };
    spans.push(Span::styled(" ".repeat(gap), band));
    spans.push(Span::styled(right, band));
    frame.render_widget(Paragraph::new(Line::from(spans)), area);
}

fn draw_filter_bar(frame: &mut Frame, app: &App, area: Rect) {
    let caret = if app.typing_filter { "\u{2588}" } else { "" };
    let right = if app.typing_filter {
        "enter keeps  ·  esc clears ".to_string()
    } else {
        format!("{} of {} shown  ·  esc clears ", app.shown(), app.tracks.len())
    };
    draw_band(
        frame,
        area,
        format!(" FILTER  /{}{}", app.filter, caret),
        right,
    );
}

/* Carries the filter too, since the pick band replaces it: a narrowed list
   with nothing saying so is what `a` would then mark the wrong amount of. */
fn draw_pick_bar(frame: &mut Frame, app: &App, area: Rect) {
    let mut left = format!(" PICK  {} of {} selected", app.marked.len(), app.tracks.len());
    if !app.filter.is_empty() || app.typing_filter {
        let caret = if app.typing_filter { "\u{2588}" } else { "" };
        left = format!("{left}  /{}{}", app.filter, caret);
    }
    draw_band(
        frame,
        area,
        left,
        "space  ·  a all  ·  enter downloads  ·  esc none ".to_string(),
    );
}

fn draw_header(frame: &mut Frame, app: &App, area: Rect) {
    let mut spans = vec![Span::styled(" earworm", Style::new().fg(GOLD))];
    // The open playlist is not what is on screen while the library is.
    if !app.playlist.is_empty() && app.view == View::Tracks {
        spans.push(dim("  ·  "));
        spans.push(Span::styled(app.playlist.clone(), Style::new().fg(CREAM)));
    }
    spans.push(dim("  ·  "));
    // The stage belongs to the last run, which the library screen is not.
    let stage = if app.view == View::Library && !app.busy {
        "library".to_string()
    } else {
        app.stage.clone()
    };
    spans.push(Span::styled(stage, Style::new().fg(CREAM)));

    if app.view == View::Library {
        let count = app.library.len();
        let plural = if count == 1 { "playlist" } else { "playlists" };
        spans.push(dim("  ·  "));
        spans.push(Span::styled(
            format!("{count} {plural}"),
            Style::new().fg(CREAM),
        ));
    } else if !app.tracks.is_empty() {
        spans.push(dim("  ·  "));
        // How much of the run is showing is the filter band's to say.
        spans.push(Span::styled(
            format!("{}/{}", app.settled(), app.tracks.len()),
            Style::new().fg(CREAM),
        ));
    }
    /* Movement the track rows cannot show: a slow lookup still looks alive,
       and a bulk action over fifty tracks explains why the keys went quiet.
       The library has no run behind it, so an idle one must not appear busy.
       A pick is waiting on a keypress, so nothing is moving behind it. */
    if app.picking.is_none() && ((app.done.is_none() && app.view == View::Tracks) || app.busy) {
        spans.push(Span::styled(
            format!("  {}", SPINNER[(app.tick / 2) % SPINNER.len()]),
            Style::new().fg(GOLD),
        ));
    }
    frame.render_widget(Paragraph::new(Line::from(spans)), area);
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
    let rest = width.saturating_sub(done.chars().count());
    (done, "─".repeat(rest))
}

fn draw_body(frame: &mut Frame, app: &mut App, area: Rect) {
    match app.view {
        View::Tracks => draw_tracks(frame, app, area),
        View::Library => draw_library(frame, app, area),
    }
}

/* One row per playlist folder, all of it from the manifests: the counts are
   what is on disk, and the sync time is the only thing that says whether that
   still matches the playlist upstream. */
fn draw_library(frame: &mut Frame, app: &mut App, area: Rect) {
    /* Only reachable by emptying --dir while the screen is open: a bare start
       with no playlists asks for a URL instead of showing this. */
    if app.library.is_empty() {
        frame.render_widget(
            Paragraph::new(Line::from(dim(" no playlists here now   n syncs one"))),
            area,
        );
        return;
    }

    /* Resolved once and shared with the pane below, so the row the marker is
       on and the folder the pane describes cannot become two questions. */
    let selected = app.shelf.min(app.library.len() - 1);

    /* The shelf row already needs name plus 28 columns of counts and sync
       time, so the pane only appears where both fit without squeezing names
       down to nothing. Below that it is simply absent. */
    let area = if area.width >= PREVIEW_FROM {
        // u32: the product overflows u16 past 32767 columns.
        let pane = (u32::from(area.width) * 2 / 5).min(60) as u16;
        let [list, preview] =
            Layout::horizontal([Constraint::Min(1), Constraint::Length(pane)]).areas(area);
        draw_preview(frame, &app.library[selected], preview);
        list
    } else {
        area
    };

    let width = area.width as usize;
    let longest = app
        .library
        .iter()
        .map(|s| s.name.chars().count())
        .max()
        .unwrap_or(0);
    let name_width = longest.min(width.saturating_sub(28).max(12));

    let items: Vec<ListItem> = app
        .library
        .iter()
        .enumerate()
        .map(|(row, shelf)| {
            let name = truncate(&shelf.name, name_width);
            let pad = name_width.saturating_sub(name.chars().count());
            let mut spans = vec![
                Span::styled(
                    if row == selected { "▌" } else { " " },
                    Style::new().fg(GREEN),
                ),
                Span::styled(format!(" {name}{:pad$}", ""), Style::new().fg(CREAM)),
                dim(format!("  {:>4} tracks", shelf.tracks)),
            ];
            /* Files the manifest lists that are no longer there, which a sync
               would download again. Worth colour: it is the one thing on this
               screen that is a problem. */
            if shelf.missing > 0 {
                spans.push(Span::styled(
                    format!("  {} missing", shelf.missing),
                    Style::new().fg(AMBER),
                ));
            }
            spans.push(dim(match shelf.synced {
                Some(at) => format!("  synced {}", manifest::ago(at)),
                None => "  never synced".into(),
            }));
            ListItem::new(Line::from(spans))
        })
        .collect();

    let mut state = ListState::default();
    state.select(Some(selected));
    frame.render_stateful_widget(List::new(items), area, &mut state);

    if app.library.len() > area.height as usize {
        let mut bar_state = ScrollbarState::new(app.library.len()).position(selected);
        frame.render_stateful_widget(
            Scrollbar::new(ScrollbarOrientation::VerticalRight)
                .begin_symbol(None)
                .end_symbol(None)
                .thumb_style(Style::new().fg(DIM))
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
fn draw_preview(frame: &mut Frame, shelf: &Shelf, area: Rect) {
    let block = Block::new()
        .borders(Borders::LEFT)
        .border_style(Style::new().fg(RULE));
    let inner = block.inner(area);
    frame.render_widget(block, area);
    if inner.width == 0 || inner.height == 0 {
        return;
    }

    if shelf.files.is_empty() {
        frame.render_widget(Paragraph::new(Line::from(dim(" nothing recorded"))), inner);
        return;
    }

    let width = inner.width as usize;
    let height = inner.height as usize;
    // The last line goes to the tail count, so no track is silently dropped.
    let room = if shelf.files.len() > height {
        height - 1
    } else {
        shelf.files.len()
    };

    let mut lines: Vec<Line> = shelf.files[..room]
        .iter()
        .map(|(name, here)| {
            let title = name.rsplit_once('.').map_or(name.as_str(), |(stem, _)| stem);
            /* Missing files are the reason the pane exists, so they get a
               gutter mark as well as the colour. */
            let mark = if *here { ' ' } else { '!' };
            Line::from(Span::styled(
                format!("{mark}{}", truncate(title, width.saturating_sub(1))),
                Style::new().fg(if *here { CREAM } else { AMBER }),
            ))
        })
        .collect();
    if room < shelf.files.len() {
        lines.push(Line::from(dim(format!(
            " … {} more",
            shelf.files.len() - room
        ))));
    }
    frame.render_widget(Paragraph::new(lines), inner);
}

fn draw_tracks(frame: &mut Frame, app: &mut App, area: Rect) {
    let width = area.width as usize;
    let rows = app.rows();
    if rows.is_empty() && !app.tracks.is_empty() {
        frame.render_widget(
            Paragraph::new(Line::from(dim(format!(
                " nothing matches {}   esc clears it",
                app.filter
            )))),
            area,
        );
        return;
    }

    /* One name column for the whole list, so the dim source and note columns
       line up instead of stepping in and out with each title's length. */
    let longest = rows
        .iter()
        .map(|pos| app.tracks[*pos].name.chars().count())
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
                    Style::new().fg(GREEN),
                ),
                // Its own column, so a marked track under the cursor shows both.
                Span::styled(
                    if app.marked.contains(&track.index) {
                        "•"
                    } else {
                        " "
                    },
                    Style::new().fg(GOLD),
                ),
                dim(format!("{:>3} ", track.index)),
            ];

            if track.status == Status::Downloading {
                let (done, rest) = bar(track.percent, STATUS_WIDTH);
                spans.push(Span::styled(done, Style::new().fg(GOLD)));
                spans.push(dim(rest));
            } else {
                spans.push(Span::styled(
                    format!("{:<STATUS_WIDTH$}", track.status.label()),
                    Style::new().fg(track.status.color()),
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
            let pad = name_width.saturating_sub(name.chars().count());
            spans.push(Span::styled(format!("  {name}"), Style::new().fg(CREAM)));
            if !tail.is_empty() {
                spans.push(dim(format!("{:pad$}  {tail}", "")));
            }
            ListItem::new(Line::from(spans))
        })
        .collect();

    let mut state = ListState::default();
    let at = app.row_of_cursor(&rows);
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
                .thumb_style(Style::new().fg(DIM))
                .track_symbol(None),
            area,
            &mut bar_state,
        );
    }
}

fn draw_logs(frame: &mut Frame, app: &App, area: Rect) {
    let height = area.height.saturating_sub(1) as usize;
    let start = app.logs.len().saturating_sub(height);
    let mut lines = vec![Line::from(dim(" yt-dlp output"))];
    lines.extend(
        app.logs[start..]
            .iter()
            .map(|l| Line::styled(format!(" {l}"), Style::new().fg(RED))),
    );
    frame.render_widget(Paragraph::new(lines), area);
}

fn draw_status(frame: &mut Frame, app: &App, area: Rect) {
    let width = area.width as usize;

    /* yt-dlp's stderr is otherwise invisible until you happen to press l, so
       say it is there. Dimmed once the pane is open: nothing left to find. */
    let mut hints = Vec::new();
    if !app.logs.is_empty() {
        let count = app.logs.len();
        let plural = if count == 1 { "line" } else { "lines" };
        hints.push((
            format!("   l  {count} yt-dlp {plural}"),
            Style::new().fg(if app.show_logs { DIM } else { AMBER }),
        ));
    }
    if !app.marked.is_empty() && app.view == View::Tracks && app.picking.is_none() {
        hints.push((
            format!("   {} marked", app.marked.len()),
            Style::new().fg(GOLD),
        ));
    }
    /* The library is the first screen a bare run shows, and a list of folders
       gives no clue that Enter opens one. The track view has a finished run
       behind it and a summary worth the same space. */
    if app.view == View::Library {
        hints.push(("   enter open".to_string(), Style::new().fg(GOLD)));
        hints.push(("   R resync all".to_string(), Style::new().fg(DIM)));
        hints.push(("   n new URL".to_string(), Style::new().fg(DIM)));
    }
    /* Absent entirely when cliamp is not installed, since a player nobody has
       is not news. Installed but stopped is worth saying: it is the answer to
       "where did `p` go". */
    match app.player {
        Player::Running => hints.push(("   ♪ cliamp".to_string(), Style::new().fg(GREEN))),
        Player::Stopped => hints.push(("   ♪ cliamp off".to_string(), Style::new().fg(DIM))),
        Player::Missing => {}
    }
    hints.push(("   h keys".to_string(), Style::new().fg(DIM)));
    let reserved: usize = hints.iter().map(|(t, _)| t.chars().count()).sum();

    /* The hints are the part you cannot recover by looking elsewhere, so both
       the summary and the tally give way before they do. Every status in the
       tally is still readable in the list itself. */
    // Counted from the tracks, which are not what the library screen lists.
    let tally: Vec<(Status, String)> = if app.view == View::Library {
        Vec::new()
    } else {
        app.counts()
            .into_iter()
            .map(|(status, count)| (status, format!("{count} {}  ", status.label())))
            .collect()
    };
    let total: usize = tally.iter().map(|(_, t)| t.chars().count()).sum();
    let budget = width.saturating_sub(reserved);
    // One cell for the ellipsis, and only when something is actually dropped.
    let limit = budget.saturating_sub(if 1 + total > budget { 1 } else { 0 });

    let mut spans = vec![Span::raw(" ")];
    let mut used = 1;
    let mut dropped = false;
    for (status, text) in tally {
        if used + text.chars().count() > limit {
            dropped = true;
            break;
        }
        used += text.chars().count();
        spans.push(Span::styled(text, Style::new().fg(status.color())));
    }
    if dropped && used < budget {
        spans.push(dim("…"));
        used += 1;
    }

    let room = width.saturating_sub(used + reserved);
    // Also the last run's, and it names a folder that is one row of many here.
    let (middle, style) = match app.done.as_ref().filter(|_| app.view == View::Tracks) {
        Some(Ok(summary)) => (truncate(summary, room), Style::new().fg(DIM)),
        Some(Err(err)) => (
            truncate(&format!("failed: {err}"), room),
            Style::new().fg(RED),
        ),
        None => (String::new(), Style::new()),
    };
    let pad = room.saturating_sub(middle.chars().count());
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
    let mut rows: Vec<(&str, &str, &str)> = vec![
        ("move", "j k  ↑ ↓", ""),
        ("", "g G", "first, last"),
    ];
    if app.view == View::Library {
        rows.extend([("open", "enter", "read this playlist off disk")]);
        if app.can_play() {
            rows.push(("play", "p", "hand this playlist to cliamp"));
        }
        rows.extend([
            ("sync", "R", "sync every playlist"),
            ("", "n", "sync a new URL"),
            ("view", "l", "yt-dlp output"),
            ("quit", "q  Esc  ^c", ""),
        ]);
    } else if app.picking.is_some() {
        rows.extend([
            ("pick", "space", "this track"),
            ("", "m", "every track like it"),
            ("", "a", "everything the filter shows"),
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
            ("", "/", "filter by name or status"),
            ("mark", "space", "this track"),
            ("", "m", "every track like it"),
        ]);
        if app.can_command() {
            rows.extend([
                ("act", "e", "edit this track"),
                ("", "c", "choose cover art"),
                ("", "r", "retry failures"),
                ("", "S", "sync this playlist"),
            ]);
            if app.can_play() {
                rows.push(("", "p", "play it in cliamp"));
            }
            rows.extend([
                ("marked", "s", "swap artist and title"),
                ("", "A", "set one artist"),
            ]);
        } else {
            rows.push(("act", "e c r s S A", "once the run finishes"));
        }
        rows.push((
            "back",
            "Esc",
            if app.library.is_empty() {
                "quit"
            } else {
                "the library"
            },
        ));
        rows.push(("quit", "q  ^c", ""));
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
                Span::styled(format!(" {label:group$}"), Style::new().fg(DIM)),
                Span::styled(format!("{keys:key$}"), Style::new().fg(GOLD)),
                Span::styled((*what).to_string(), Style::new().fg(CREAM)),
            ])
        })
        .collect();

    lines.push(Line::default());

    // Wrapped to the table it sits under, or one long line sets the width.
    let settings = widest(|r| r.2.chars().count()) + key;
    for (n, piece) in wrap(&app.settings, settings.max(20)).into_iter().enumerate() {
        lines.push(Line::from(vec![
            Span::styled(
                format!(" {:group$}", if n == 0 { "run" } else { "" }),
                Style::new().fg(DIM),
            ),
            Span::styled(piece, Style::new().fg(CREAM)),
        ]));
    }

    let content = lines
        .iter()
        .map(|l| l.width() as u16)
        .max()
        .unwrap_or(0);
    popup(frame, "keys", lines, content + 3);
}

fn draw_prompt(frame: &mut Frame, app: &App) {
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
                rows.push((note.clone(), Style::new().fg(DIM)));
                rows.push((String::new(), Style::new()));
            }
            rows.extend(options.iter().enumerate().map(|(i, opt)| {
                let selected = i == app.choice;
                let style = if selected {
                    Style::new().fg(GREEN)
                } else {
                    Style::new()
                };
                (format!("{} {opt}", if selected { "▌" } else { " " }), style)
            }));
            (header, rows)
        }
        Prompt::Input { header, .. } => (
            header,
            vec![(format!("{}\u{2588}", app.input), Style::new().fg(CREAM))],
        ),
    };
    rows.push((String::new(), Style::new()));
    rows.push((
        match prompt {
            Prompt::Choice { escape, .. } => {
                format!("j/k select   enter confirm   {}", escape.hint())
            }
            crate::app::Prompt::Input { escape, .. } => {
                format!("enter confirm   ^u clear   ^w word   {}", escape.hint())
            }
        },
        Style::new().fg(DIM),
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
    popup(frame, header, lines, width);
}

fn popup_width(area: Rect) -> u16 {
    // Widened to u32 first: 937 columns is enough to overflow the multiply.
    let seventy = (area.width as u32 * 70 / 100) as u16;
    seventy.clamp(20.min(area.width), area.width)
}

fn popup(frame: &mut Frame, title: &str, lines: Vec<Line>, width: u16) {
    let screen = frame.area();
    let height = (lines.len() as u16 + 2).min(screen.height);
    // Never narrower than the title it has to fit, never wider than the screen.
    let width = width.clamp((title.chars().count() as u16 + 4).min(screen.width), screen.width);
    let area = centred(screen, width, height);
    frame.render_widget(Clear, area);
    frame.render_widget(
        Paragraph::new(lines).block(
            Block::default()
                .borders(Borders::ALL)
                .style(Style::new().bg(SURFACE))
                .border_style(Style::new().fg(RULE).bg(SURFACE))
                .title(Span::styled(format!(" {title} "), Style::new().fg(CREAM))),
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
    if text.chars().count() <= width {
        return text.to_string();
    }
    text.chars()
        .take(width.saturating_sub(1))
        .collect::<String>()
        + "…"
}

/// Greedy word wrap, hard-splitting any single word too long for the box.
fn wrap(text: &str, width: usize) -> Vec<String> {
    if width == 0 {
        return vec![text.to_string()];
    }
    let mut out = Vec::new();
    let mut line = String::new();
    for word in text.split(' ') {
        let mut word = word;
        while word.chars().count() > width {
            if !line.is_empty() {
                out.push(std::mem::take(&mut line));
            }
            let head: String = word.chars().take(width).collect();
            word = &word[head.len()..];
            out.push(head);
        }
        let joined = line.chars().count() + 1 + word.chars().count();
        if !line.is_empty() && joined > width {
            out.push(std::mem::take(&mut line));
        }
        if !line.is_empty() {
            line.push(' ');
        }
        line.push_str(word);
    }
    out.push(line);
    out
}

#[cfg(test)]
mod tests {
    use super::{bar, popup_width, wrap};
    use crate::app::{App, Msg, Shelf, Status, Track};
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;
    use ratatui::layout::Rect;

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

    fn library_screen(width: u16) -> Vec<String> {
        library_screen_at(width, 0)
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

    /* The shelf row says a sync would re-download two files but never which,
       which is the whole reason the pane is there. It follows the cursor
       without a cursor of its own, and a folder longer than the pane says how
       many it could not show rather than ending mid-list. */
    #[test]
    fn the_preview_shows_the_selected_folders_tracks() {
        let rows = library_screen_at(120, 1);
        let pane: String = rows[2..10].join("\n");
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
        assert!(pane.contains("Track 1"), "{pane:?}");
        assert!(pane.contains("Track 7"), "{pane:?}");
        assert!(!pane.contains("Track 8"), "a row overflowed the pane: {pane:?}");
        assert!(pane.contains("… 35 more"), "{pane:?}");
    }

    /* The shelf row already needs its name plus 28 columns of counts, so a
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
            assert_eq!(buffer[(x, 1)].bg, crate::theme::GOLD, "the band breaks at {x}");
        }

        /* Narrow enough that the hint cannot fit beside the query, which is
           where the band used to stop early and let the gradient back in. */
        let mut narrow = Terminal::new(TestBackend::new(30, 10)).unwrap();
        narrow.draw(|f| super::draw(f, &mut app)).unwrap();
        for x in 0..30u16 {
            assert_eq!(
                narrow.backend().buffer()[(x, 1)].bg,
                crate::theme::GOLD,
                "the band breaks at {x} once the hint no longer fits"
            );
        }

        // Narrower than the label itself, where the fill has nothing left to do.
        for w in [1u16, 8, 13] {
            let mut t = Terminal::new(TestBackend::new(w, 10)).unwrap();
            t.draw(|f| super::draw(f, &mut app)).unwrap();
            for x in 0..w {
                assert_eq!(t.backend().buffer()[(x, 1)].bg, crate::theme::GOLD, "at width {w}, column {x}");
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
            app.apply(Msg::Player(state));
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
            assert_eq!(buffer[(x, 1)].bg, crate::theme::GOLD, "the band breaks at {x}");
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
                let want = crate::theme::background(x, y, 60, 24);
                assert_eq!(buffer[(x, y)].bg, want, "at {x},{y}");
            }
        }
        // Bottom-left and top-right are the two ends of the diagonal.
        assert_ne!(
            crate::theme::background(0, 23, 60, 24),
            crate::theme::background(59, 0, 60, 24),
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
            "parse on · lookup on · cover on · m3u8 on · prompts on · rename on".into(),
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
        assert_eq!(buffer[middle].bg, crate::theme::SURFACE);
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
        let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
        terminal
            .draw(|f| {
                super::draw_background(f);
                super::draw_intro(f, ms);
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
        assert!(landed.contains("youtube playlists, tagged"), "{landed:?}");

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
