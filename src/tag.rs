use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};

use anyhow::{Context, Result};
use lofty::config::WriteOptions;
use lofty::file::{AudioFile, TaggedFile, TaggedFileExt};
use lofty::picture::{MimeType, Picture, PictureType};
use lofty::probe::Probe;
use lofty::tag::items::Timestamp;
use lofty::tag::{Accessor, Tag, TagExt};

use std::time::Duration;

use crate::app::{Asker, Status};
use crate::config::Config;
use crate::lookup::{self, Match};

/// One image through ffmpeg, which is a decode and an encode of a few
/// megabytes. Generous, since the alternative is leaving the art oversized.
const COVER_RESIZE: Duration = Duration::from_secs(20);

static WRITING: AtomicBool = AtomicBool::new(false);

/// True while a track's tags are being rewritten in place. Quitting during one
/// truncates the file, so the shutdown path waits this out.
pub fn writing() -> bool {
    WRITING.load(Ordering::SeqCst)
}

struct WriteGuard;

impl WriteGuard {
    fn new() -> Self {
        WRITING.store(true, Ordering::SeqCst);
        WriteGuard
    }
}

impl Drop for WriteGuard {
    fn drop(&mut self) {
        WRITING.store(false, Ordering::SeqCst);
    }
}

pub struct Info {
    pub artist: String,
    pub title: String,
    pub album: String,
    /// `None` is a file with no year tag, which is not the same as a year of
    /// zero: one is unknown and the other is a claim.
    pub year: Option<u16>,
    pub duration: u64,
}

/* What an edit writes. Album and year travel beside artist and title because
   `set_fields` writes the whole set: a caller changing one has to carry the
   others, or a swap would silently drop the album off every track it touched. */
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Fields {
    pub artist: String,
    pub title: String,
    pub album: String,
    pub year: Option<u16>,
}

fn open(path: &Path) -> Result<TaggedFile> {
    Probe::open(path)
        .context("cannot open file")?
        .read()
        .context("cannot read audio")
}

pub fn read(path: &Path) -> Result<Info> {
    let file = open(path)?;
    let tag = file.primary_tag().or_else(|| file.first_tag());
    Ok(Info {
        artist: tag.and_then(|t| t.artist()).unwrap_or_default().to_string(),
        title: tag.and_then(|t| t.title()).unwrap_or_default().to_string(),
        album: tag.and_then(|t| t.album()).unwrap_or_default().to_string(),
        /* `date` reads the full recording date and falls back to a bare
           year, which is the only one of the two earworm ever writes. */
        year: tag.and_then(|t| t.date()).map(|d| d.year),
        duration: file.properties().duration().as_secs(),
    })
}

pub struct Outcome {
    pub status: Status,
    pub source: String,
    pub note: String,
    pub artist: String,
    pub title: String,
    pub mbid: Option<String>,
}

fn with_tag<F: FnOnce(&mut Tag)>(path: &Path, edit: F) -> Result<()> {
    let mut file = open(path)?;
    /* Taken from the container, never assumed: inserting Vorbis comments
       into an mp3 is refused outright by lofty, and the write then fails on
       any untagged file in a format other than opus or flac. */
    if file.primary_tag().is_none() {
        let kind = file.file_type().primary_tag_type();
        file.insert_tag(Tag::new(kind));
    }
    let _writing = WriteGuard::new();
    let tag = file.primary_tag_mut().context("no writable tag")?;
    edit(tag);
    tag.save_to_path(path, WriteOptions::default())
        .context("cannot write tags")
}

pub fn set_album(path: &Path, album: &str) -> Result<()> {
    with_tag(path, |tag| tag.set_album(album.to_string()))
}

pub fn set_fields(path: &Path, fields: &Fields) -> Result<()> {
    with_tag(path, |tag| {
        tag.set_artist(fields.artist.clone());
        tag.set_title(fields.title.clone());
        /* Removed rather than set to nothing: lofty writes an empty album as
           a present-but-blank frame, which reads back as a tagged file whose
           album happens to be "" and stops `tag_tracks` filling it in. */
        if fields.album.is_empty() {
            tag.remove_album();
        } else {
            tag.set_album(fields.album.clone());
        }
        /* A date with only a year in it, which is what the box asks for and
           what every source earworm has gives. Written through `set_date`
           because lofty has no year of its own: it keys the bare `Year` field
           and the full recording date to the same accessor. */
        match fields.year {
            Some(year) => tag.set_date(Timestamp {
                year,
                ..Timestamp::default()
            }),
            None => {
                tag.remove_date();
            }
        }
    })
}

/// Embedding a PNG as image/jpeg produces a file players silently refuse to
/// show art for, so the bytes decide the mime type.
pub fn image_extension(data: &[u8]) -> Option<&'static str> {
    if data.starts_with(&[0xFF, 0xD8]) {
        Some("jpg")
    } else if data.starts_with(b"\x89PNG") {
        Some("png")
    } else {
        None
    }
}

/* No bigger than Deezer's `cover_xl`, which is the largest earworm would
   ever choose for itself: the cap is "not larger than our own best source"
   rather than a number to argue about. YouTube serves an Art Track's
   thumbnail at 2048 square, and `--embed-thumbnail` puts that in every file
   the lookup could not place, where it is several times the size of the
   audio it is attached to. */
pub const COVER_MAX: u16 = 1000;

/// The embedded front cover, for carrying art across a rewrite or measuring
/// what is already there. `None` when the file has no picture.
pub fn read_cover(path: &Path) -> Option<Vec<u8>> {
    let file = open(path).ok()?;
    let tag = file.primary_tag().or_else(|| file.first_tag())?;
    let front = tag
        .pictures()
        .iter()
        .find(|p| p.pic_type() == PictureType::CoverFront)
        .or_else(|| tag.pictures().first())?;
    Some(front.data().to_vec())
}

/* Shrinks an oversized embedded cover in place, and says whether it had to.
   Measured with `jpeg_size`, so a picture that is not a JPEG is left alone:
   the only ones that are not come from the `c` picker, where somebody chose
   the file on purpose. */
pub fn cap_cover(path: &Path) -> Result<bool> {
    let Some(image) = read_cover(path) else {
        return Ok(false);
    };
    let (w, h) = lookup::jpeg_size(&image);
    if w == 0 || (w <= COVER_MAX && h <= COVER_MAX) {
        return Ok(false);
    }
    let smaller = shrink(&image).context("could not resize the cover")?;
    set_cover(path, &smaller)?;
    Ok(true)
}

/* Through files rather than pipes: `run_bounded` polls `try_wait` and never
   writes to the child, so a piped stdin nobody closes would leave ffmpeg
   waiting for input that is not coming. */
fn shrink(image: &[u8]) -> Option<Vec<u8>> {
    let dir = std::env::temp_dir().join(format!(
        "earworm-cover-{}-{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    std::fs::create_dir_all(&dir).ok()?;
    let (from, to) = (dir.join("in.jpg"), dir.join("out.jpg"));
    let _ = std::fs::remove_file(&to);
    std::fs::write(&from, image).ok()?;

    let out = lookup::run_bounded(
        std::process::Command::new("ffmpeg")
            .args(["-v", "error", "-y", "-i"])
            .arg(&from)
            // Never upscales, and keeps whatever aspect the art came with.
            .args([
                "-vf",
                &format!("scale={COVER_MAX}:{COVER_MAX}:force_original_aspect_ratio=decrease"),
            ])
            .args(["-q:v", "3"])
            .arg(&to)
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null()),
        COVER_RESIZE,
    );
    let smaller = out
        .filter(|o| o.status.success())
        .and_then(|_| std::fs::read(&to).ok())
        // A truncated write and a wrong format both fail here rather than
        // replacing good art with something no player will show.
        .filter(|bytes| image_extension(bytes) == Some("jpg"));
    let _ = std::fs::remove_dir_all(&dir);
    smaller
}

pub fn set_cover(path: &Path, image: &[u8]) -> Result<()> {
    let mime = match image_extension(image) {
        Some("png") => MimeType::Png,
        _ => MimeType::Jpeg,
    };
    with_tag(path, |tag| {
        tag.remove_picture_type(PictureType::CoverFront);
        tag.push_picture(
            Picture::unchecked(image.to_vec())
                .pic_type(PictureType::CoverFront)
                .mime_type(mime)
                .build(),
        );
    })
}

pub struct CoverState {
    pub written: bool,
}

pub fn identify(
    path: &Path,
    cfg: &Config,
    asker: &Asker,
    cover: &mut CoverState,
    index: usize,
) -> Result<Outcome> {
    let info = read(path)?;
    let mut out = Outcome {
        status: Status::NoMatch,
        source: String::new(),
        note: String::new(),
        artist: info.artist.clone(),
        title: info.title.clone(),
        mbid: None,
    };

    let mut found: Option<Match> = None;
    if cfg.lookup {
        out.status = Status::NoMatch;
        if let Some(key) = &cfg.acoustid_key {
            found = lookup::acoustid(path, key);
        }
        if found.is_none() {
            match lookup::deezer(&info.artist, &info.title) {
                Some(m) if lookup::plausible(&m, &info.artist, &info.title) => found = Some(m),
                Some(m) => {
                    out.status = Status::Weak;
                    out.source = m.source.into();
                    out.note = format!("rejected \"{}\"", m.title);
                }
                None => {}
            }
        }
        /* Apple is the fallback, not a second opinion: Deezer's plausible hit
        already won, and an implausible Deezer hit already owns the rejection
        note. */
        if found.is_none() && cfg.apple {
            match lookup::itunes(&info.artist, &info.title) {
                Some(m) if lookup::plausible(&m, &info.artist, &info.title) => found = Some(m),
                Some(m) if out.status == Status::NoMatch => {
                    out.status = Status::Weak;
                    out.source = m.source.into();
                    out.note = format!("rejected \"{}\"", m.title);
                }
                _ => {}
            }
        }
    } else {
        out.status = Status::Kept;
    }

    if let Some(m) = &found {
        out.status = Status::Ok;
        out.source = m.source.into();
        out.mbid = m.mbid.clone();
        if let Some(score) = m.score {
            out.note = format!("score {score:.2}");
        }
        let (artist, title, note) = apply(path, m, cfg, cover, asker, index, &info.artist)?;
        out.artist = artist;
        out.title = title;
        if !note.is_empty() {
            out.note = note;
        }
    }

    /* Unresolved tracks are the only ones worth asking about, and with
    --no-lookup that is all of them. */
    let unresolved = matches!(
        out.status,
        Status::NoMatch | Status::Weak | Status::Kept
    );
    if cfg.fix
        && asker.enabled
        && unresolved
        && let Some(typed) = manual(path, asker, &out.artist, &out.title, index, cfg.apple)
    {
        out.status = Status::Manual;
        out.source = "typed".into();
        out.note.clear();
        let (artist, title, _) = apply(path, &typed, cfg, cover, asker, index, "")?;
        out.artist = artist;
        out.title = title;
    }

    Ok(out)
}

/// The text-search half of `identify`, for words already worth searching
/// with: Deezer first, Apple fallback, the plausibility gate on both. No
/// fingerprint and no prompts; a miss is just `None`.
pub fn search(artist: &str, title: &str, apple: bool) -> Option<Match> {
    lookup::deezer(artist, title)
        .filter(|m| lookup::plausible(m, artist, title))
        .or_else(|| {
            apple
                .then(|| lookup::itunes(artist, title))
                .flatten()
                .filter(|m| lookup::plausible(m, artist, title))
        })
}

fn manual(
    path: &Path,
    asker: &Asker,
    artist: &str,
    title: &str,
    index: usize,
    apple: bool,
) -> Option<Match> {
    let name = path.file_name()?.to_string_lossy().to_string();
    let answers = asker.form(
        &format!("track {index}"),
        &name,
        vec![
            ("artist".into(), artist.to_string()),
            ("title".into(), title.to_string()),
        ],
        crate::app::Escape::Skip,
    )?;
    let [new_artist, new_title] = answers.as_slice() else {
        return None;
    };
    let (new_artist, new_title) = (new_artist.clone(), new_title.clone());
    if new_artist.is_empty() || new_title.is_empty() {
        return None;
    }
    if new_artist == artist && new_title == title {
        return None;
    }
    /* A hand-typed artist and title make a far better search key than the video
    title did, so retry for album and artwork through the same search a `T`
    re-run would use. */
    let extra = search(&new_artist, &new_title, apple);
    Some(Match {
        title: new_title,
        artist: new_artist,
        album: extra.as_ref().and_then(|m| m.album.clone()),
        cover_url: extra.as_ref().and_then(|m| m.cover_url.clone()),
        mbid: None,
        score: None,
        source: "manual",
    })
}

/// Writes a lookup hit into the file: tags, embedded art, and the folder
/// image unless the caller already wrote one. Returns the artist and title
/// as written, plus why the artist differs when it does.
pub fn apply(
    path: &Path,
    m: &Match,
    cfg: &Config,
    cover: &mut CoverState,
    asker: &Asker,
    index: usize,
    old_artist: &str,
) -> Result<(String, String, String)> {
    let mut artist = m.artist.clone();
    let mut note = String::new();

    // Title and album are always taken; only the artist can lose information.
    if !old_artist.is_empty() && lookup::downgrades(&artist, old_artist) {
        let (chosen, why) = choose_artist(asker, index, old_artist, &m.artist);
        artist = chosen;
        note = why;
    }

    let jpeg = cfg
        .cover
        .then(|| lookup::cover_bytes(m, cfg.apple))
        .flatten();
    with_tag(path, |tag| {
        tag.set_title(m.title.clone());
        tag.set_artist(artist.clone());
        if let Some(album) = &m.album {
            tag.set_album(album.clone());
        }
        if let Some(data) = &jpeg {
            tag.remove_picture_type(PictureType::CoverFront);
            tag.push_picture(
                Picture::unchecked(data.clone())
                    .pic_type(PictureType::CoverFront)
                    .mime_type(MimeType::Jpeg)
                    .build(),
            );
        }
    })?;

    /* Folder image is written once per run, by whichever track resolves art
    first; every track still gets the image embedded. */
    if let (Some(data), false) = (&jpeg, cover.written)
        && let Some(dir) = path.parent()
    {
        std::fs::write(dir.join("cover.jpg"), data)?;
        cover.written = true;
    }

    Ok((artist, m.title.clone(), note))
}

/// Only reached on a real conflict, which keeps the prompt rare enough to be
/// worth answering. Unattended runs keep the fuller existing credit.
fn choose_artist(asker: &Asker, index: usize, old: &str, new: &str) -> (String, String) {
    let options = vec![
        format!("{old}  (existing)"),
        format!("{new}  (lookup)"),
        "type it myself".to_string(),
    ];
    match asker.choose(&format!("Track {index}: artist conflict"), options) {
        Some(1) => (new.to_string(), "chose lookup".into()),
        Some(2) => match asker.input("Artist", old) {
            Some(typed) if !typed.is_empty() => (typed, "typed artist".into()),
            _ => (old.to_string(), "kept artist".into()),
        },
        _ => (old.to_string(), "kept artist".into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lookup;
    use std::path::{Path, PathBuf};

    /* Silent, untagged and in whatever container the codec implies, which is
       the case `with_tag` has to insert a tag for. Several candidates per
       format because the encoder's name is a property of the ffmpeg build:
       this one has no `libvorbis` and ships the native `vorbis` instead. */
    fn bare(at: &Path, codecs: &[&str]) -> Option<()> {
        codecs.iter().any(|codec| encode(at, codec)).then_some(())
    }

    fn encode(at: &Path, codec: &str) -> bool {
        let mut cmd = std::process::Command::new("ffmpeg");
        // Stereo: ffmpeg's own vorbis encoder refuses anything else.
        cmd.args(["-v", "error", "-f", "lavfi", "-i", "anullsrc=r=44100:cl=stereo"])
            .args(["-t", "0.2", "-map_metadata", "-1", "-c:a", codec])
            // That encoder is also marked experimental in some builds.
            .args(["-strict", "-2"])
            /* Otherwise ffmpeg stamps an encoder tag of its own and the file
               arrives already tagged, which is not the case under test. */
            .args(["-fflags", "+bitexact", "-flags:a", "+bitexact"]);
        // The only one of them that can be written with no tag at all.
        if codec == "libmp3lame" {
            cmd.args(["-id3v2_version", "0", "-write_id3v1", "0"]);
        }
        cmd.arg("-y")
            .arg(at)
            .status()
            .is_ok_and(|status| status.success())
    }

    /* A tag type the container does not take is refused outright, and a file
       with no tag at all is where the type has to be guessed: assuming Vorbis
       comments left an untagged mp3 or m4a unwritable while the download that
       produced it reported success. */
    #[test]
    fn writes_tags_into_every_container_earworm_downloads() {
        let dir = std::env::temp_dir().join(format!("earworm-tagtypes-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        // Named rather than counted: every container but mp3 carries a tag
        // block by definition, so this says which fixture is doing the work.
        let mut untagged: Vec<&str> = Vec::new();
        /* Driven off FORMATS, so a format added to the table without a
           container earworm can tag fails here rather than at the end of
           somebody's download. */
        for (format, ext) in crate::config::FORMATS {
            let codecs: &[&str] = match format {
                "opus" => &["libopus", "opus"],
                "m4a" => &["aac", "libfdk_aac"],
                "mp3" => &["libmp3lame", "mp3"],
                "flac" => &["flac"],
                "vorbis" => &["libvorbis", "vorbis"],
                "alac" => &["alac"],
                other => panic!("{other} has no codec here, so nothing tests its container"),
            };
            let file: PathBuf = dir.join(format!("{format}.{ext}"));
            if bare(&file, codecs).is_none() {
                eprintln!("skipped {ext}: ffmpeg could not write it");
                continue;
            }
            use lofty::file::TaggedFileExt;
            if open(&file).unwrap().primary_tag().is_none() {
                untagged.push(format);
            }
            let fields = Fields {
                artist: "Kraftwerk".into(),
                title: "Autobahn".into(),
                album: "Autobahn".into(),
                year: Some(1974),
            };
            set_fields(&file, &fields).unwrap_or_else(|e| panic!("{ext}: {e}"));
            let back = read(&file).unwrap();
            assert_eq!((back.artist.as_str(), back.title.as_str()), ("Kraftwerk", "Autobahn"), "{ext}");
            /* Album and year go into the same tag block, so every container
               that takes the first two has to take these as well. */
            assert_eq!(back.album, "Autobahn", "{ext}");
            assert_eq!(back.year, Some(1974), "{ext}");
        }
        /* The branch under test only runs for a file with no tag, so a
           fixture that arrives tagged would pass while saying nothing. */
        assert!(!untagged.is_empty(), "every fixture came with a tag already");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// A square JPEG of `px`, the shape YouTube serves an Art Track's
    /// thumbnail in. `None` if ffmpeg is not installed.
    fn square_jpeg(at: &Path, px: u32) -> Option<()> {
        std::process::Command::new("ffmpeg")
            .args(["-v", "error", "-f", "lavfi", "-i"])
            .arg(format!("testsrc=size={px}x{px}:duration=1:rate=1"))
            .args(["-frames:v", "1", "-y"])
            .arg(at)
            .status()
            .ok()
            .filter(|s| s.success())
            .map(|_| ())
    }

    /* YouTube serves an Art Track's thumbnail at 2048 square, and
       `--embed-thumbnail` puts it in every file the lookup could not place:
       on a real folder that was 2.9MB of art on 3.5MB of audio, more than a
       third of every file and the same image in all twelve. */
    #[test]
    fn an_oversized_cover_is_shrunk_and_a_small_one_is_left_alone() {
        let dir = std::env::temp_dir().join(format!("earworm-cap-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let track = dir.join("01 - A.opus");
        let big = dir.join("big.jpg");
        if crate::worker::tests::tiny_opus(&track).is_none() || square_jpeg(&big, 2048).is_none() {
            eprintln!("no ffmpeg, skipping");
            let _ = std::fs::remove_dir_all(&dir);
            return;
        }
        let art = std::fs::read(&big).unwrap();
        assert_eq!(lookup::jpeg_size(&art).0, 2048, "the fixture is not 2048 wide");
        set_cover(&track, &art).unwrap();
        let before = std::fs::metadata(&track).unwrap().len();

        assert!(cap_cover(&track).unwrap(), "an oversized cover was left alone");

        let capped = read_cover(&track).expect("the cover went entirely");
        let (w, h) = lookup::jpeg_size(&capped);
        assert!(w <= COVER_MAX && h <= COVER_MAX, "still {w}x{h}");
        assert_eq!(w, COVER_MAX, "it shrank past the cap rather than to it");
        // Still a picture a player will show, not a truncated write.
        assert_eq!(image_extension(&capped), Some("jpg"));
        assert!(
            std::fs::metadata(&track).unwrap().len() < before,
            "the file did not get smaller"
        );
        /* Idempotent: a second pass has nothing to do, which is what keeps
           this off the cost of every later sync. */
        assert!(!cap_cover(&track).unwrap(), "it resized an already-capped cover");

        // Under the cap is left exactly as it was, bytes included.
        let small = dir.join("small.jpg");
        square_jpeg(&small, 600).unwrap();
        let art = std::fs::read(&small).unwrap();
        set_cover(&track, &art).unwrap();
        assert!(!cap_cover(&track).unwrap());
        assert_eq!(read_cover(&track).unwrap(), art, "a small cover was re-encoded");

        // And a file with no picture at all is not an error.
        let bare = dir.join("02 - B.opus");
        crate::worker::tests::tiny_opus(&bare).unwrap();
        assert!(read_cover(&bare).is_none());
        assert!(!cap_cover(&bare).unwrap());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn detects_image_types_by_magic_bytes() {
        assert_eq!(image_extension(&[0xFF, 0xD8, 0xFF, 0xE0]), Some("jpg"));
        assert_eq!(image_extension(b"\x89PNG\r\n\x1a\n"), Some("png"));
        assert_eq!(image_extension(b"GIF89a"), None);
        assert_eq!(image_extension(b"<html>"), None);
        assert_eq!(image_extension(b""), None);
    }
}


