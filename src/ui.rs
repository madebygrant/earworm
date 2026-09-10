use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::{
    Block, Borders, Clear, List, ListItem, ListState, Paragraph, Scrollbar, ScrollbarOrientation,
    ScrollbarState,
};

use crate::app::{App, Escape, Prompt, Status};

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
        draw_tracks(frame, app, list);
        draw_logs(frame, app, logs);
    } else {
        draw_tracks(frame, app, body);
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
    if !app.playlist.is_empty() {
        spans.push(dim("  ·  "));
        spans.push(Span::styled(app.playlist.clone(), Style::new().fg(CREAM)));
    }
    spans.push(dim("  ·  "));
    spans.push(Span::styled(app.stage.clone(), Style::new().fg(CREAM)));

    if !app.tracks.is_empty() {
        spans.push(dim("  ·  "));
        spans.push(Span::styled(
            format!("{}/{}", app.settled(), app.tracks.len()),
            Style::new().fg(CREAM),
        ));
    }
    // Movement the track rows cannot show: a slow lookup still looks alive.
    if app.done.is_none() {
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

fn draw_tracks(frame: &mut Frame, app: &mut App, area: Rect) {
    let width = area.width as usize;
    /* One name column for the whole list, so the dim source and note columns
       line up instead of stepping in and out with each title's length. */
    let longest = app
        .tracks
        .iter()
        .map(|t| t.name.chars().count())
        .max()
        .unwrap_or(0);
    let name_width = longest.min(width.saturating_sub(STATUS_WIDTH + 8).max(12));

    let items: Vec<ListItem> = app
        .tracks
        .iter()
        .enumerate()
        .map(|(row, track)| {
            let selected = row == app.cursor;
            let mut spans = vec![
                Span::styled(
                    if selected { "▌" } else { " " },
                    Style::new().fg(GREEN),
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
    if !app.tracks.is_empty() {
        state.select(Some(app.cursor.min(app.tracks.len() - 1)));
    }
    frame.render_stateful_widget(List::new(items), area, &mut state);

    // Only worth the column when the list actually runs off the pane.
    if app.tracks.len() > area.height as usize {
        let mut bar_state = ScrollbarState::new(app.tracks.len()).position(app.cursor);
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
    hints.push(("   h keys".to_string(), Style::new().fg(DIM)));
    let reserved: usize = hints.iter().map(|(t, _)| t.chars().count()).sum();

    /* The hints are the part you cannot recover by looking elsewhere, so both
       the summary and the tally give way before they do. Every status in the
       tally is still readable in the list itself. */
    let tally: Vec<(Status, String)> = app
        .counts()
        .into_iter()
        .map(|(status, count)| (status, format!("{count} {}  ", status.label())))
        .collect();
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
    let (middle, style) = match &app.done {
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

fn draw_help(frame: &mut Frame, app: &App) {
    let actions = if app.can_command() {
        "e  edit artist and title      c  choose cover art"
    } else {
        "e / c  available once the run finishes"
    };
    let rows = vec![
        ("move", "j k  ↑ ↓      g G  first last".to_string()),
        ("view", "f  follow the active track     l  yt-dlp output".to_string()),
        ("act", actions.to_string()),
        ("quit", "q  ctrl+c".to_string()),
        ("", String::new()),
        ("run", app.settings.clone()),
    ];
    let lines: Vec<Line> = rows
        .into_iter()
        .map(|(key, text)| {
            Line::from(vec![
                Span::styled(format!(" {key:<6}"), Style::new().fg(GOLD)),
                Span::styled(text, Style::new().fg(CREAM)),
            ])
        })
        .collect();
    popup(frame, "keys", lines);
}

fn draw_prompt(frame: &mut Frame, app: &App) {
    let Some((prompt, _)) = &app.prompt else {
        return;
    };
    let width = popup_width(frame.area());
    let inner = (width as usize).saturating_sub(4);

    let (header, mut rows) = match prompt {
        Prompt::Choice { header, options } => (
            header,
            options
                .iter()
                .enumerate()
                .map(|(i, opt)| {
                    let selected = i == app.choice;
                    let style = if selected {
                        Style::new().fg(GREEN)
                    } else {
                        Style::new()
                    };
                    (format!("{} {opt}", if selected { "▌" } else { " " }), style)
                })
                .collect::<Vec<_>>(),
        ),
        Prompt::Input { header, .. } => (
            header,
            vec![(format!("{}\u{2588}", app.input), Style::new().fg(CREAM))],
        ),
    };
    rows.push((String::new(), Style::new()));
    rows.push((
        match prompt {
            Prompt::Choice { .. } => "j/k select   enter confirm   esc skip".to_string(),
            Prompt::Input { escape, .. } => {
                let esc = match escape {
                    Escape::Skip => "esc skip",
                    Escape::Quit => "esc quit",
                };
                format!("enter confirm   ^u clear   ^w word   {esc}")
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
    popup(frame, header, lines);
}

fn popup_width(area: Rect) -> u16 {
    // Widened to u32 first: 937 columns is enough to overflow the multiply.
    let seventy = (area.width as u32 * 70 / 100) as u16;
    seventy.clamp(20.min(area.width), area.width)
}

fn popup(frame: &mut Frame, title: &str, lines: Vec<Line>) {
    let screen = frame.area();
    let height = (lines.len() as u16 + 2).min(screen.height);
    let area = centred(screen, popup_width(screen), height);
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
    use crate::app::{App, Status, Track};
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
