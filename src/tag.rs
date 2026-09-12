use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};

use anyhow::{Context, Result};
use lofty::config::WriteOptions;
use lofty::file::{AudioFile, TaggedFile, TaggedFileExt};
use lofty::picture::{MimeType, Picture, PictureType};
use lofty::probe::Probe;
use lofty::tag::{Accessor, Tag, TagExt};

use crate::app::{Asker, Status};
use crate::config::Config;
use crate::lookup::{self, Match};

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
    pub duration: u64,
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

pub fn set_fields(path: &Path, artist: &str, title: &str) -> Result<()> {
    with_tag(path, |tag| {
        tag.set_artist(artist.to_string());
        tag.set_title(title.to_string());
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
        && let Some(typed) = manual(path, asker, &out.artist, &out.title, index)
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

fn manual(path: &Path, asker: &Asker, artist: &str, title: &str, index: usize) -> Option<Match> {
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
    title did, so retry for album and artwork. */
    let extra = lookup::deezer(&new_artist, &new_title)
        .filter(|m| lookup::plausible(m, &new_artist, &new_title));
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

fn apply(
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

    let jpeg = cfg.cover.then(|| lookup::cover_bytes(m)).flatten();
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
    use super::{image_extension, open, read, set_fields};
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
            set_fields(&file, "Kraftwerk", "Autobahn").unwrap_or_else(|e| panic!("{ext}: {e}"));
            let back = read(&file).unwrap();
            assert_eq!((back.artist.as_str(), back.title.as_str()), ("Kraftwerk", "Autobahn"), "{ext}");
        }
        /* The branch under test only runs for a file with no tag, so a
           fixture that arrives tagged would pass while saying nothing. */
        assert!(!untagged.is_empty(), "every fixture came with a tag already");
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

