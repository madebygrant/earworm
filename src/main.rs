mod app;
mod config;
mod lookup;
mod tag;
mod theme;
mod ui;
mod worker;
mod ytdlp;

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, TryRecvError};
use std::time::{Duration, Instant};

use anyhow::Result;
use clap::{CommandFactory, FromArgMatches};
use ratatui::crossterm::event::{self, Event, KeyCode, KeyEventKind, KeyModifiers};

use app::{App, Cmd, Prompt, Reply};
use config::{Cli, Config};

fn main() -> Result<()> {
    let matches = Cli::command().get_matches();
    let cfg = Config::build(Cli::from_arg_matches(&matches)?, &matches)?;
    let settings = cfg.describe();
    let (tx, rx) = mpsc::channel();
    let (cmd_tx, cmd_rx) = mpsc::channel();
    let cancel = Arc::new(AtomicBool::new(false));
    let worker_cancel = Arc::clone(&cancel);
    std::thread::spawn(move || worker::run(cfg, tx, worker_cancel, cmd_rx));

    let mut terminal = ratatui::init();
    let mut app = App::new(cmd_tx, settings);
    let result = run(&mut terminal, &mut app, rx);

    cancel.store(true, Ordering::SeqCst);
    ytdlp::stop();
    // Closes the command channel, which is what ends the worker's service loop.
    app.cmds.take();
    /* Exiting part-way through an in-place tag rewrite truncates the file, so
       give the worker a moment to finish one it had already started. */
    let deadline = Instant::now() + Duration::from_secs(5);
    while tag::writing() && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(20));
    }
    ratatui::restore();
    result?;

    match &app.done {
        Some(Ok(summary)) => println!("{summary}"),
        Some(Err(err)) => {
            eprintln!("earworm: {err}");
            std::process::exit(1);
        }
        None => {}
    }
    Ok(())
}

fn run(
    terminal: &mut ratatui::DefaultTerminal,
    app: &mut App,
    rx: mpsc::Receiver<app::Msg>,
) -> Result<()> {
    while !app.quit {
        loop {
            match rx.try_recv() {
                Ok(msg) => app.apply(msg),
                Err(TryRecvError::Empty) => break,
                /* The worker drops its sender on the way out, so a disconnect
                   before Done means it died. Without this the UI would keep
                   drawing a run that had already stopped. */
                Err(TryRecvError::Disconnected) => {
                    // A worker that asked to quit has already said its piece.
                    if app.done.is_none() && !app.quit {
                        app.done = Some(Err("worker stopped unexpectedly".into()));
                        app.quit = true;
                    }
                    break;
                }
            }
        }
        terminal.draw(|frame| ui::draw(frame, app))?;
        app.tick = app.tick.wrapping_add(1);

        if event::poll(Duration::from_millis(120))?
            && let Event::Key(key) = event::read()?
            && key.kind == KeyEventKind::Press
        {
            handle_key(app, key.code, key.modifiers);
        }
    }
    Ok(())
}

fn handle_key(app: &mut App, code: KeyCode, mods: KeyModifiers) {
    if app.prompt.is_some() {
        handle_prompt_key(app, code, mods);
        return;
    }
    /* The overlay swallows the next key rather than acting on it: anything
       else makes dismissing it a guess about what the key also did. */
    if app.show_help && !matches!(code, KeyCode::Char('q')) {
        app.show_help = false;
        return;
    }
    match code {
        KeyCode::Char('c') if mods.contains(KeyModifiers::CONTROL) => app.quit = true,
        KeyCode::Char('h') | KeyCode::Char('?') => app.show_help = true,
        KeyCode::Char('q') | KeyCode::Esc => app.quit = true,
        KeyCode::Char('f') => app.follow = !app.follow,
        KeyCode::Char('l') => app.show_logs = !app.show_logs,
        KeyCode::Char('j') | KeyCode::Down => {
            app.follow = false;
            app.cursor = (app.cursor + 1).min(app.tracks.len().saturating_sub(1));
        }
        KeyCode::Char('k') | KeyCode::Up => {
            app.follow = false;
            app.cursor = app.cursor.saturating_sub(1);
        }
        KeyCode::Char('e') if app.can_command() => {
            if let Some(index) = app.selected() {
                app.send(Cmd::Edit(index));
            }
        }
        KeyCode::Char('c') if app.can_command() => {
            if let Some(index) = app.selected() {
                app.send(Cmd::Cover(index));
            }
        }
        KeyCode::Char('g') => app.cursor = 0,
        KeyCode::Char('G') => app.cursor = app.tracks.len().saturating_sub(1),
        _ => {}
    }
}

/// Esc cancels the question rather than the run: the worker treats a cancel as
/// "leave this track alone", which is the safe answer for every prompt here.
fn handle_prompt_key(app: &mut App, code: KeyCode, mods: KeyModifiers) {
    let Some((prompt, _)) = &app.prompt else {
        return;
    };
    let options = match prompt {
        Prompt::Choice { options, .. } => options.len(),
        Prompt::Input { .. } => 0,
    };

    let ctrl = mods.contains(KeyModifiers::CONTROL);
    let reply = match (code, options) {
        (KeyCode::Esc, _) => Some(Reply::Cancel),
        // A prefilled field is unusable without a way to clear it.
        (KeyCode::Char('u'), 0) if ctrl => {
            app.input.clear();
            None
        }
        (KeyCode::Char('w'), 0) if ctrl => {
            let kept = app.input.trim_end();
            let cut = kept.rfind(' ').map_or(0, |i| i + 1);
            app.input.truncate(cut);
            None
        }
        (KeyCode::Enter, 0) => Some(Reply::Text(app.input.clone())),
        (KeyCode::Enter, _) => Some(Reply::Choice(app.choice)),
        (KeyCode::Down | KeyCode::Tab | KeyCode::Char('j'), n) if n > 0 => {
            app.choice = (app.choice + 1) % n;
            None
        }
        (KeyCode::Up | KeyCode::BackTab | KeyCode::Char('k'), n) if n > 0 => {
            app.choice = (app.choice + n - 1) % n;
            None
        }
        (KeyCode::Char(c), 0) if !ctrl => {
            app.input.push(c);
            None
        }
        (KeyCode::Backspace, 0) => {
            app.input.pop();
            None
        }
        _ => None,
    };

    if let Some(reply) = reply {
        if let Some((_, tx)) = app.prompt.take() {
            let _ = tx.send(reply);
        }
        app.input.clear();
        app.choice = 0;
    }
}
