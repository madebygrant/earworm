use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::mpsc::Sender;

use anyhow::{Context, Result, bail};

use crate::app::{Msg, Status, Track};
use crate::config::Config;
use crate::manifest;

static CHILD: AtomicU32 = AtomicU32::new(0);

/// Quitting the UI must take the download with it; an orphaned yt-dlp would
/// keep writing into the same folder after the terminal is restored.
pub fn stop() {
    let pid = CHILD.swap(0, Ordering::SeqCst);
    if pid != 0 {
        let _ = Command::new("kill").arg(pid.to_string()).status();
    }
}

const TITLE_CLEANUP: &str = " *[\\(\\[][^)\\]]*(?i:official|lyric|audio|video|visualiser|visualizer|hd|4k|remaster)[^)\\]]*[\\)\\]]";

fn output_template(dir: &Path, ext: &str) -> String {
    format!(
        "{}/%(playlist)s/%(playlist_index)02d - %(title)s.{ext}",
        dir.display()
    )
}

/// Resolves ids, titles and destination paths from the playlist listing alone.
/// One request instead of one per video, which is a second rather than half a
/// minute; the paths match what the real download computes.
pub fn scan(cfg: &Config) -> Result<Vec<Track>> {
    let out = Command::new("yt-dlp")
        .args([
            "--skip-download",
            "--quiet",
            "--no-warnings",
            "--flat-playlist",
        ])
        // Same rewrite the download applies, or the scanned paths miss every
        // title that carries an "(Official Video)" suffix.
        .args(["--replace-in-metadata", "track,title", TITLE_CLEANUP, ""])
        .arg("--output")
        .arg(output_template(&cfg.dir, "opus"))
        .arg("--print")
        .arg("%(id)s\t%(playlist_index)s\t%(title)s\t%(filename)s")
        .args(&cfg.extra)
        .arg(&cfg.url)
        .output()
        .context("yt-dlp not found on PATH")?;

    let mut tracks = Vec::new();
    for line in String::from_utf8_lossy(&out.stdout).lines() {
        let parts: Vec<&str> = line.splitn(4, '\t').collect();
        if parts.len() != 4 {
            continue;
        }
        let index: usize = parts[1].parse().unwrap_or(tracks.len() + 1);
        let mut track = Track::new(
            index,
            parts[0].to_string(),
            parts[2].to_string(),
            PathBuf::from(parts[3]),
        );
        // is_file, not exists: a directory sitting on the destination path
        // would otherwise count as a track that had already downloaded.
        if track.path.as_ref().is_some_and(|p| p.is_file()) {
            track.status = Status::Have;
        }
        tracks.push(track);
    }
    /* Renaming moves a track off the path this scan predicts, so the manifest
       is what says "already downloaded" once the filename no longer matches.
       It also survives a title changing on YouTube. */
    if let Some(folder) = tracks
        .first()
        .and_then(|t| t.path.as_deref())
        .and_then(Path::parent)
        .map(Path::to_path_buf)
    {
        let known = manifest::read(&folder);
        for track in &mut tracks {
            if let Some(file) = known.get(&track.id) {
                track.path = Some(file.clone());
                track.status = Status::Have;
            }
        }
    }

    /* A non-zero exit only matters when nothing came back: one unavailable
       video in an otherwise readable playlist still fails the process. */
    if tracks.is_empty() {
        // A private playlist, a dead network and a typo'd URL are otherwise
        // indistinguishable to whoever has to read the message.
        let stderr = String::from_utf8_lossy(&out.stderr);
        let detail = stderr
            .lines()
            .rev()
            .find(|l| !l.trim().is_empty())
            .unwrap_or("no detail from yt-dlp");
        bail!("could not read the playlist: {}", detail.trim());
    }
    Ok(tracks)
}

/// Re-running over finished tracks makes yt-dlp remux them, and its metadata
/// step cannot copy the cover-art stream --embed-thumbnail added, which kills
/// the whole run. Naming those ids up front makes yt-dlp skip them instead.
///
/// Keyed on the file being there rather than on a status, because a retry runs
/// this again once the earlier tracks have moved on to `ok` or `manual`.
fn write_archive(tracks: &[Track], path: &Path) -> Result<usize> {
    let mut file = std::fs::File::create(path)?;
    let mut count = 0;
    for track in tracks {
        if track.path.as_deref().is_some_and(Path::is_file) {
            writeln!(file, "youtube {}", track.id)?;
            count += 1;
        }
    }
    Ok(count)
}

/// Owns the run's scratch directory: removing the root takes the archive and
/// the discarded per-track thumbnails with it.
pub struct Download {
    pub scratch: PathBuf,
    pub archive: PathBuf,
    pub thumbs: Option<PathBuf>,
}

impl Drop for Download {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.scratch);
    }
}

pub fn run(cfg: &Config, tracks: &mut [Track], tx: &Sender<Msg>) -> Result<()> {
    /* A retry downloads a second time in the same process, and the guard
       removes this whole directory on the way out, so the runs cannot share
       a name. */
    static RUN: AtomicU32 = AtomicU32::new(0);
    let scratch = std::env::temp_dir().join(format!(
        "earworm-{}-{}",
        std::process::id(),
        RUN.fetch_add(1, Ordering::SeqCst)
    ));
    std::fs::create_dir_all(&scratch)?;
    let guard = Download {
        archive: scratch.join("archive"),
        thumbs: cfg.cover.then(|| scratch.join("thumbs")),
        scratch,
    };
    write_archive(&*tracks, &guard.archive)?;
    if let Some(dir) = &guard.thumbs {
        std::fs::create_dir_all(dir)?;
    }

    let mut cmd = Command::new("yt-dlp");
    cmd.args([
        "--yes-playlist",
        "--extract-audio",
        "--audio-format",
        "opus",
        "--embed-thumbnail",
        "--embed-metadata",
        "--quiet",
        "--newline",
        "--progress",
        "--progress-delta",
        "0.5",
    ]);
    cmd.arg("--progress-template")
        .arg("download:@P\t%(info.playlist_index)s\t%(progress._percent_str)s");
    cmd.arg("--print")
        .arg("after_move:@D\t%(playlist_index)s\t%(filepath)s");

    // Also rewrites the filename, since title feeds the output template.
    cmd.args(["--replace-in-metadata", "track,title", TITLE_CLEANUP, ""]);

    /* Separator predicts field order: dashes run artist-first, pipes and
    bullets put the artist last. The classes are mutually exclusive, so only
    one rule can match a title. Overwrites YouTube's own tags. */
    if cfg.parse {
        cmd.args([
            "--parse-metadata",
            "title:^(?P<artist>.+?) [-–—] (?P<track>.+)$",
        ]);
        cmd.args([
            "--parse-metadata",
            "title:^(?!.* [-–—] )(?P<track>.+) [|•] (?P<artist>[^|•]+)$",
        ]);
    }

    /* --write-thumbnail has no playlist-only mode, so per-track images go to a
    temp dir and are dropped; only pl_thumbnail is kept. */
    if let Some(dir) = &guard.thumbs {
        cmd.args(["--write-thumbnail", "--convert-thumbnails", "jpg"]);
        cmd.arg("--output")
            .arg(format!("thumbnail:{}/%(id)s.%(ext)s", dir.display()));
        cmd.arg("--output").arg(format!(
            "pl_thumbnail:{}/%(playlist)s/cover.%(ext)s",
            cfg.dir.display()
        ));
    }

    cmd.arg("--download-archive").arg(&guard.archive);
    cmd.arg("--output")
        .arg(output_template(&cfg.dir, "%(ext)s"));
    cmd.args(&cfg.extra);
    cmd.arg(&cfg.url);
    cmd.stdout(Stdio::piped()).stderr(Stdio::piped());

    let mut child = cmd.spawn().context("yt-dlp not found on PATH")?;
    CHILD.store(child.id(), Ordering::SeqCst);

    let errors = child.stderr.take().unwrap();
    let err_tx = tx.clone();
    let pump = std::thread::spawn(move || {
        for line in BufReader::new(errors).lines().map_while(Result::ok) {
            let _ = err_tx.send(Msg::Log(line));
        }
    });

    let stdout = child.stdout.take().unwrap();
    for line in BufReader::new(stdout).lines().map_while(Result::ok) {
        let parts: Vec<&str> = line.splitn(3, '\t').collect();
        match parts.as_slice() {
            ["@P", index, percent] => {
                if let Ok(index) = index.parse() {
                    let percent = percent
                        .trim()
                        .trim_end_matches('%')
                        .parse::<f64>()
                        .unwrap_or(0.0);
                    let _ = tx.send(Msg::Progress {
                        index,
                        percent: percent as u16,
                    });
                }
            }
            ["@D", index, path] => {
                if let Ok(index) = index.parse::<usize>() {
                    let path = PathBuf::from(*path);
                    if let Some(track) = tracks.iter_mut().find(|t| t.index == index) {
                        track.path = Some(path.clone());
                        track.status = Status::Downloaded;
                    }
                    let _ = tx.send(Msg::Update {
                        index,
                        status: Status::Downloaded,
                        source: None,
                        note: None,
                        name: None,
                    });
                    let _ = tx.send(Msg::Path { index, path });
                }
            }
            _ => {
                if !line.trim().is_empty() {
                    let _ = tx.send(Msg::Log(line));
                }
            }
        }
    }

    let status = child.wait()?;
    CHILD.store(0, Ordering::SeqCst);
    let _ = pump.join();
    if !status.success() {
        bail!("yt-dlp exited with {status}");
    }
    Ok(())
}
