use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::{
    Block, Borders, Clear, List, ListItem, ListState, Paragraph, Scrollbar, ScrollbarOrientation,
    ScrollbarState,
};

use crate::app::{App, Prompt, Status, View};
use crate::manifest;

use crate::theme::{self, AMBER, CREAM, DIM, GOLD, GREEN, RED, RULE, SURFACE};

const SPINNER: [&str; 8] = ["⣷", "⣯", "⣟", "⡿", "⢿", "⣻", "⣽", "⣾"];
const STATUS_WIDTH: usize = 8;

fn dim(text: impl Into<String>) -> Span<'static> {
    Span::styled(text.into(), Style::new().fg(DIM))
}

pub fn draw(frame: &mut Frame, app: &mut App) {
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
    draw_rule(frame, rule);
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
        let bg = theme::background(y - area.top(), area.height);
        for x in area.left()..area.right() {
            buffer[(x, y)].set_bg(bg);
        }
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
        /* Under a filter the settled count describes the whole run, which is
           not what the list is showing, so say how much of it is. */
        spans.push(Span::styled(
            if app.filter.is_empty() {
                format!("{}/{}", app.settled(), app.tracks.len())
            } else {
                format!("{} of {}", app.shown(), app.tracks.len())
            },
            Style::new().fg(CREAM),
        ));
    }
    /* Movement the track rows cannot show: a slow lookup still looks alive,
       and a bulk action over fifty tracks explains why the keys went quiet.
       The library has no run behind it, so an idle one must not appear busy. */
    if (app.done.is_none() && app.view == View::Tracks) || app.busy {
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
                    if row == app.shelf { "▌" } else { " " },
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
    state.select(Some(app.shelf.min(app.library.len() - 1)));
    frame.render_stateful_widget(List::new(items), area, &mut state);

    if app.library.len() > area.height as usize {
        let mut bar_state = ScrollbarState::new(app.library.len()).position(app.shelf);
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
    if !app.marked.is_empty() && app.view == View::Tracks {
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
    /* The box is the only thing on screen that says the list is narrowed, so
       it holds its place ahead of the tally and the summary. */
    if app.typing_filter {
        hints.push((
            format!("   /{}\u{2588}", app.filter),
            Style::new().fg(GOLD),
        ));
        hints.push(("   enter keeps   esc clears".to_string(), Style::new().fg(DIM)));
    } else if !app.filter.is_empty() {
        hints.push((format!("   /{}", app.filter), Style::new().fg(GOLD)));
        hints.push(("   esc clears".to_string(), Style::new().fg(DIM)));
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
        rows.extend([
            ("open", "enter", "read this playlist off disk"),
            ("sync", "R", "sync every playlist"),
            ("", "n", "sync a new URL"),
            ("view", "l", "yt-dlp output"),
            ("quit", "q  Esc  ^c", ""),
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
            Prompt::Input { escape, .. } => {
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
            },
            Shelf {
                path: std::path::PathBuf::from("/music/Road trip"),
                name: "Road trip".into(),
                url: "u2".into(),
                tracks: 9,
                missing: 2,
                synced: None,
            },
        ]
    }

    fn library_screen(width: u16) -> Vec<String> {
        let (tx, _rx) = std::sync::mpsc::channel();
        let mut app = App::new(tx, "settings".into());
        app.apply(Msg::Library {
            shelves: shelves(),
            show: true,
        });
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
            let want = crate::theme::background(y, 24);
            for x in 0..60u16 {
                assert_eq!(buffer[(x, y)].bg, want, "at {x},{y}");
            }
        }
        assert_ne!(
            crate::theme::background(0, 24),
            crate::theme::background(23, 24),
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
}
