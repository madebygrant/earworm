use std::collections::HashSet;
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::mpsc::Sender;

use anyhow::{Result, bail};

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

fn output_template(cfg: &Config, ext: &str) -> String {
    format!(
        "{}/{}/%(playlist_index)02d - %(title)s.{ext}",
        cfg.dir.display(),
        folder_field(cfg)
    )
}

/* The playlist's own title, unless this run belongs to a folder somebody has
   named: then the name is a literal, and `%` in it has to be escaped or
   yt-dlp reads it as the start of a field. */
fn folder_field(cfg: &Config) -> String {
    match &cfg.folder {
        Some(name) => name.replace('%', "%%"),
        None => "%(playlist)s".to_string(),
    }
}

/// The scan predicts the filenames the download will write, so both have to
/// agree about the format: the download names it and the scan needs the
/// extension it lands on.
fn scan_template(cfg: &Config) -> String {
    output_template(cfg, crate::config::extension(&cfg.format))
}

pub struct Listing {
    pub tracks: Vec<Track>,
    /// yt-dlp exited cleanly, so the listing is the whole playlist. A
    /// truncated one looks exactly like a playlist that lost tracks, which is
    /// why nothing may act on an absence unless this is true.
    pub complete: bool,
    /// The playlist's own id. On `Listing` and not on `Track` because it
    /// describes the playlist rather than the song, and every entry repeats
    /// it. `None` for a single video, which has no playlist to name.
    pub playlist_id: Option<String>,
}

/// Resolves ids, titles and destination paths from the playlist listing alone.
/// One request instead of one per video, which is a second rather than half a
/// minute; the paths match what the real download computes.
pub fn scan(cfg: &Config) -> Result<Listing> {
    scan_with("yt-dlp", cfg)
}

/// Takes the binary so a test can hand it a stub and read back the arguments
/// this actually ran with. Visible to the crate for the one test that has to
/// span a scan and the tag pass either side of it, which no single module
/// can reach. The predicted filename is the whole point of the
/// scan, and a test of the helper that builds the template passes whatever
/// the call site chooses to do with it.
pub(crate) fn scan_with(bin: &str, cfg: &Config) -> Result<Listing> {
    let out = Command::new(bin)
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
        .arg(scan_template(cfg))
        .arg("--print")
        /* Duration is here for one caller: the pass that attaches a URL to a
           folder earworm did not download and has to work out which file on
           disk is which video. It breaks a tie between two tracks whose
           titles match and is never enough on its own, since every other song
           is three minutes. The playlist id is here for another: its prefix is
           what says whether this is an album. The filename stays last, because
           it is the one field that could hold a tab and `splitn` lets the last
           one keep it. */
        .arg("%(id)s\t%(playlist_index)s\t%(title)s\t%(duration)s\t%(playlist_id)s\t%(filename)s")
        .args(&cfg.extra)
        .arg(&cfg.url)
        .output()
        .map_err(|e| anyhow::anyhow!(crate::deps::launch_error(bin, &e)))?;

    let mut tracks = Vec::new();
    let mut playlist_id: Option<String> = None;
    for line in String::from_utf8_lossy(&out.stdout).lines() {
        let parts: Vec<&str> = line.splitn(6, '\t').collect();
        if parts.len() != 6 {
            continue;
        }
        let index: usize = parts[1].parse().unwrap_or(tracks.len() + 1);
        // Every entry repeats it, so the first one that is not yt-dlp's "NA"
        // is the answer for the whole listing.
        if playlist_id.is_none() && parts[4] != "NA" && !parts[4].is_empty() {
            playlist_id = Some(parts[4].to_string());
        }
        let mut track = Track::new(
            index,
            parts[0].to_string(),
            parts[2].to_string(),
            PathBuf::from(parts[5]),
        );
        // yt-dlp prints "NA" for a video whose duration it does not know,
        // and 0 is what the rest of the tool already means by "no duration".
        track.duration = parts[3].parse().unwrap_or(0);
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
            /* Nothing here needs to exclude the ids earworm invents for a
               folder it did not download: this is a lookup by video id, and
               `manifest::local_id` puts a `~` in front, which no video id
               carries. A `~` entry is therefore unreachable from here, and
               the reconciling pass is what places those files instead, by
               name and tag. The other two readers of ids do need a guard,
               because they walk the manifest rather than index into it. */
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
        bail!(
            "could not read the playlist: {}  ·  check the link is public",
            detail.trim()
        );
    }
    Ok(Listing {
        tracks,
        complete: out.status.success(),
        playlist_id,
    })
}

/// Re-running over finished tracks makes yt-dlp remux them, and its metadata
/// step cannot copy the cover-art stream --embed-thumbnail added, which kills
/// the whole run. Naming those ids up front makes yt-dlp skip them instead.
///
/// Keyed on the file being there rather than on a status, because a retry runs
/// this again once the earlier tracks have moved on to `ok` or `manual`.
///
/// `Skipped` is the one status that also belongs here: it is how the run says
/// it was never asked for this track, and the archive is what makes yt-dlp
/// honour that without narrowing the listing the way `--playlist-items` would.
///
/// `refetch` names the track indices a conversion wants downloaded again
/// because their format cannot be converted without stacking a second lossy
/// generation. Their files are on disk, so without this they would be exactly
/// the tracks yt-dlp skipped. A parameter rather than a `Status`, because
/// `counts()` is hand-kept and a variant missing from it does not fail to
/// compile.
pub(crate) fn write_archive(
    tracks: &[Track],
    path: &Path,
    refetch: &HashSet<usize>,
) -> Result<usize> {
    let mut file = std::fs::File::create(path)?;
    let mut count = 0;
    for track in tracks {
        if refetch.contains(&track.index) {
            continue;
        }
        /* The archive is read by yt-dlp, which knows nothing about ids
           earworm invented for files it never downloaded. Harmless there
           either way, but a file written for another tool gets only what
           that tool understands. */
        if manifest::is_local(&track.id) {
            continue;
        }
        if track.status == Status::Skipped || track.path.as_deref().is_some_and(Path::is_file) {
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

pub fn run(
    cfg: &Config,
    tracks: &mut [Track],
    tx: &Sender<Msg>,
    refetch: &HashSet<usize>,
) -> Result<()> {
    run_with("yt-dlp", cfg, tracks, tx, refetch)
}

/// Takes the binary for the same reason `scan_with` does: a test that
/// asserted on a helper building these arguments would leave the call site
/// free to pass anything, which is how the scan's format came to be right in
/// the helper and hardcoded at the call.
fn run_with(
    bin: &str,
    cfg: &Config,
    tracks: &mut [Track],
    tx: &Sender<Msg>,
    refetch: &HashSet<usize>,
) -> Result<()> {
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
    write_archive(&*tracks, &guard.archive, refetch)?;
    if let Some(dir) = &guard.thumbs {
        std::fs::create_dir_all(dir)?;
    }

    let mut cmd = Command::new(bin);
    cmd.args([
        "--yes-playlist",
        "--extract-audio",
        "--embed-metadata",
        "--quiet",
        "--newline",
        "--progress",
        "--progress-delta",
        "0.5",
    ]);
    cmd.args(["--audio-format", &cfg.format]);
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
        /* Here rather than in the unconditional block above, or --no-cover
           embeds YouTube's thumbnail in every track while its own help
           promises nothing is embedded. It is the fallback that carries a
           track the lookup could not place, so it stays on by default. */
        cmd.arg("--embed-thumbnail");
        cmd.args(["--write-thumbnail", "--convert-thumbnails", "jpg"]);
        cmd.arg("--output")
            .arg(format!("thumbnail:{}/%(id)s.%(ext)s", dir.display()));
        cmd.arg("--output").arg(format!(
            "pl_thumbnail:{}/{}/cover.%(ext)s",
            cfg.dir.display(),
            folder_field(cfg)
        ));
    }

    cmd.arg("--download-archive").arg(&guard.archive);
    cmd.arg("--output").arg(output_template(cfg, "%(ext)s"));
    cmd.args(&cfg.extra);
    cmd.arg(&cfg.url);
    cmd.stdout(Stdio::piped()).stderr(Stdio::piped());

    let mut child = cmd
        .spawn()
        .map_err(|e| anyhow::anyhow!(crate::deps::launch_error(bin, &e)))?;
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
        bail!("yt-dlp exited with {status}  ·  l has its output");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /* Without this the folder is named from the playlist's title on every
       download, so a renamed folder is synced back under the old name and
       the whole playlist arrives beside the renamed copy. */
    #[test]
    fn a_named_folder_is_what_the_download_writes_into() {
        let mut cfg = crate::worker::tests::config(true);
        cfg.dir = PathBuf::from("/music");

        let upstream = output_template(&cfg, "opus");
        assert!(upstream.contains("/%(playlist)s/"), "{upstream}");

        cfg.folder = Some("Morning".into());
        let pinned = output_template(&cfg, "opus");
        assert!(pinned.starts_with("/music/Morning/"), "{pinned}");
        assert!(!pinned.contains("%(playlist)s"), "{pinned}");
        // The rest of the name is still yt-dlp's to fill in.
        assert!(pinned.contains("%(playlist_index)02d - %(title)s.opus"), "{pinned}");

        /* A literal name goes into a template, where `%` opens a field: a
           playlist called "100% Hits" would otherwise take the folder
           somewhere nobody asked for. */
        cfg.folder = Some("100% Hits".into());
        assert!(output_template(&cfg, "opus").contains("/100%% Hits/"));
    }

    /* A predicted filename that misses the extension makes every track look
       absent, so the run downloads a folder it already has. Driven through a
       stub rather than against `scan_template`: asserting on the helper left
       the call site free to pass anything, which is exactly the regression
       this is here to catch. */
    #[test]
    fn the_scan_predicts_filenames_in_the_chosen_format() {
        let output_for = |format: &str| {
            let dir = std::env::temp_dir()
                .join(format!("earworm-scan-{format}-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(&dir).unwrap();
            let bin = dir.join("yt-dlp");
            let log = dir.join("log");
            // One argument per line: a template carries spaces, so a
            // space-joined line could not be split back into arguments.
            std::fs::write(
                &bin,
                format!(
                    "#!/bin/sh\nfor a in \"$@\"; do echo \"$a\" >> \"{}\"; done\n\
                     printf 'id1\t1\tTitle\t213\tPL1\t{}
'\n",
                    log.display(),
                    dir.join("music").join("track").display()
                ),
            )
            .unwrap();
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                std::fs::set_permissions(&bin, std::fs::Permissions::from_mode(0o755)).unwrap();
            }

            let mut cfg = crate::worker::tests::config(true);
            cfg.dir = dir.join("music");
            cfg.format = format.into();
            scan_with(&bin.display().to_string(), &cfg).unwrap();

            let args: Vec<String> = std::fs::read_to_string(&log)
                .unwrap()
                .lines()
                .map(str::to_string)
                .collect();
            let at = args
                .iter()
                .position(|a| a == "--output")
                .expect("the scan named no output template");
            let template = args[at + 1].clone();
            let _ = std::fs::remove_dir_all(&dir);
            template
        };

        for (format, ext) in [("vorbis", ".ogg"), ("opus", ".opus"), ("mp3", ".mp3")] {
            let template = output_for(format);
            assert!(template.ends_with(ext), "{format} scanned for {template}");
        }
    }

    /* `--no-cover` promises "nothing fetched, nothing embedded in the
       tracks", and `--embed-thumbnail` sat in the unconditional block, so
       every track still carried YouTube's thumbnail. For an Art Track that is
       2048 square and bigger than the audio it is attached to. */
    #[test]
    fn no_cover_embeds_no_thumbnail() {
        let args_for = |cover: bool| {
            let dir = std::env::temp_dir()
                .join(format!("earworm-thumb-{cover}-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(&dir).unwrap();
            let bin = dir.join("yt-dlp");
            let log = dir.join("log");
            std::fs::write(
                &bin,
                format!(
                    "#!/bin/sh\nfor a in \"$@\"; do echo \"$a\" >> \"{}\"; done\nexit 0\n",
                    log.display()
                ),
            )
            .unwrap();
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                std::fs::set_permissions(&bin, std::fs::Permissions::from_mode(0o755)).unwrap();
            }
            let mut cfg = crate::worker::tests::config(true);
            cfg.dir = dir.join("music");
            cfg.cover = cover;
            let mut tracks = vec![Track::new(1, "id".into(), "n".into(), dir.join("a.opus"))];
            let (tx, _rx) = std::sync::mpsc::channel();
            run_with(&bin.display().to_string(), &cfg, &mut tracks, &tx, &HashSet::new()).unwrap();
            let args = std::fs::read_to_string(&log).unwrap_or_default();
            let _ = std::fs::remove_dir_all(&dir);
            args
        };

        let on = args_for(true);
        assert!(on.contains("--embed-thumbnail"), "the default lost its art: {on}");
        assert!(on.contains("--write-thumbnail"), "{on}");

        let off = args_for(false);
        assert!(
            !off.contains("--embed-thumbnail"),
            "--no-cover embedded a thumbnail anyway: {off}"
        );
        assert!(!off.contains("--write-thumbnail"), "{off}");
        // The rest of the download is untouched by the flag.
        assert!(off.contains("--extract-audio"), "{off}");
    }

    /* The archive is what keeps yt-dlp off a track the run was not asked for,
       and it has to work without a file behind it: that is the whole
       difference from an already-downloaded one. Narrowing the listing with
       --playlist-items would silence departure detection instead. */
    #[test]
    fn the_archive_names_skipped_tracks_as_well_as_downloaded_ones() {
        let dir = std::env::temp_dir().join(format!("earworm-archive-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let here = dir.join("01 - A.opus");
        std::fs::write(&here, "audio").unwrap();

        let mut tracks = vec![
            Track::new(1, "have".into(), "01 - A.opus".into(), here),
            Track::new(2, "want".into(), "02 - B.opus".into(), dir.join("02 - B.opus")),
            Track::new(3, "skip".into(), "03 - C.opus".into(), dir.join("03 - C.opus")),
        ];
        tracks[2].status = Status::Skipped;

        let path = dir.join("archive");
        assert_eq!(write_archive(&tracks, &path, &HashSet::new()).unwrap(), 2);
        let body = std::fs::read_to_string(&path).unwrap();
        assert!(body.contains("youtube have"), "{body:?}");
        assert!(body.contains("youtube skip"), "{body:?}");
        assert!(!body.contains("youtube want"), "the run would not fetch it: {body:?}");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /* The refetch set is the whole mechanism behind converting an mp3: its
       file is on disk, so without being left out of the archive it is exactly
       the track yt-dlp skips, and nothing is ever downloaded again. Read back
       off the file the download was handed, not off the helper that built it. */
    #[test]
    fn a_refetched_track_is_left_out_of_the_archive() {
        let dir = std::env::temp_dir().join(format!("earworm-refetch-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let (one, two) = (dir.join("01 - A.mp3"), dir.join("02 - B.opus"));
        std::fs::write(&one, "audio").unwrap();
        std::fs::write(&two, "audio").unwrap();

        let tracks = vec![
            Track::new(1, "stale".into(), "01 - A.mp3".into(), one),
            Track::new(2, "fine".into(), "02 - B.opus".into(), two),
        ];
        let path = dir.join("archive");
        let refetch = HashSet::from([1usize]);
        assert_eq!(write_archive(&tracks, &path, &refetch).unwrap(), 1);
        let body = std::fs::read_to_string(&path).unwrap();
        assert!(
            !body.contains("youtube stale"),
            "the track being converted would have been skipped: {body:?}"
        );
        assert!(body.contains("youtube fine"), "{body:?}");
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
