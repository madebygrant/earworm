use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::{
    Block, Borders, Clear, List, ListItem, ListState, Paragraph, Scrollbar, ScrollbarOrientation,
    ScrollbarState,
};

use crate::app::{App, Prompt, Status};

use crate::theme::{AMBER, CREAM, DIM, GOLD, GREEN, RED, RULE};

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
    let mut spans = vec![Span::raw(" ")];
    let mut used = 1;
    for (status, count) in app.counts() {
        let text = format!("{count} {}  ", status.label());
        used += text.chars().count();
        spans.push(Span::styled(text, Style::new().fg(status.color())));
    }

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

    // The hints are the part you cannot recover by looking elsewhere, so the
    // summary is what gives way when the bar runs out of room.
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
            Prompt::Input { .. } => "enter confirm   ^u clear   ^w word   esc skip".to_string(),
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
    (area.width * 70 / 100).clamp(20.min(area.width), area.width)
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
                .border_style(Style::new().fg(RULE))
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
    use super::{bar, wrap};

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
