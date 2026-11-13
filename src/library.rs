//! File discovery, metadata loading, album/artist grouping and validation
//! helpers.

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{Context, Result};
use rayon::prelude::*;
use walkdir::WalkDir;

use crate::tags::{self, Field, Metadata, Properties, TrackData};

pub const AUDIO_EXTENSIONS: &[&str] =
    &["mp3", "flac", "m4a", "m4b", "mp4", "ogg", "oga", "opus"];

#[derive(Clone, Debug)]
pub struct Entry {
    pub path: PathBuf,
    pub metadata: Metadata,
    pub properties: Properties,
    pub error: Option<String>,
}

impl Entry {
    pub fn title(&self) -> String {
        let title = self.metadata.title.trim();
        if !title.is_empty() {
            title.to_string()
        } else {
            self.path
                .file_stem()
                .map(|s| s.to_string_lossy().to_string())
                .unwrap_or_default()
        }
    }

    pub fn artist(&self) -> String {
        self.metadata.artist_string()
    }

    pub fn album_title(&self) -> String {
        let album = self.metadata.album.trim();
        if !album.is_empty() {
            album.to_string()
        } else {
            self.path
                .parent()
                .and_then(|p| p.file_name())
                .map(|s| s.to_string_lossy().to_string())
                .unwrap_or_default()
        }
    }

    pub fn album_artist(&self) -> String {
        let artist = self.metadata.album_artist.trim();
        if !artist.is_empty() {
            artist.to_string()
        } else if !self.artist().trim().is_empty() {
            self.artist()
        } else {
            "Unknown Artist".into()
        }
    }

    pub fn track_label(&self) -> String {
        match (self.metadata.track, self.metadata.track_total) {
            (0, _) => String::new(),
            (n, 0) => n.to_string(),
            (n, total) => format!("{n}/{total}"),
        }
    }

    pub fn duration(&self) -> Duration {
        self.properties.duration
    }

    pub fn format(&self) -> String {
        if self.properties.container.is_empty() {
            self.path
                .extension()
                .map(|e| e.to_string_lossy().to_ascii_uppercase())
                .unwrap_or_default()
        } else {
            self.properties.container.clone()
        }
    }
}

pub fn is_audio(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .map(|e| AUDIO_EXTENSIONS.contains(&e.to_ascii_lowercase().as_str()))
        .unwrap_or(false)
}

pub fn discover(dir: &Path, recursive: bool) -> Result<Vec<PathBuf>> {
    let mut paths = Vec::new();
    if recursive {
        let walker = WalkDir::new(dir).into_iter().filter_entry(|entry| {
            entry.depth() == 0
                || !entry.file_name().to_string_lossy().starts_with('.')
        });
        for entry in walker.filter_map(Result::ok) {
            if entry.file_type().is_dir() {
                continue;
            }
            let path = entry.path();
            if is_audio(path) {
                paths.push(path.to_path_buf());
            }
        }
    } else {
        for entry in
            fs::read_dir(dir).with_context(|| format!("{}", dir.display()))?
        {
            let entry = entry?;
            if entry.file_type()?.is_dir() {
                continue;
            }
            let path = entry.path();
            if is_audio(&path) {
                paths.push(path);
            }
        }
    }
    paths.sort();
    Ok(paths)
}

pub fn expand(args: &[PathBuf], recursive: bool) -> Result<Vec<PathBuf>> {
    let mut out = Vec::new();
    for arg in args {
        let st =
            fs::metadata(arg).with_context(|| format!("{}", arg.display()))?;
        if st.is_dir() {
            out.extend(discover(arg, recursive)?);
        } else {
            out.push(arg.clone());
        }
    }
    Ok(out)
}

pub fn load_one(path: &Path) -> Entry {
    match tags::read(path) {
        Ok(TrackData {
            metadata,
            properties,
        }) => Entry {
            path: path.to_path_buf(),
            metadata,
            properties,
            error: None,
        },
        Err(err) => Entry {
            path: path.to_path_buf(),
            metadata: Metadata::default(),
            properties: Properties::default(),
            error: Some(format!("{err:#}")),
        },
    }
}

pub fn load(paths: &[PathBuf]) -> Vec<Entry> {
    paths.par_iter().map(|path| load_one(path)).collect()
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GroupMode {
    Tags,
    Folder,
}

impl GroupMode {
    pub fn parse(value: &str) -> Self {
        match value.trim().to_ascii_lowercase().as_str() {
            "folder" | "folders" | "directory" | "dir" => Self::Folder,
            _ => Self::Tags,
        }
    }
}

#[derive(Clone, Debug)]
pub struct Album {
    pub key: String,
    pub artist: String,
    pub title: String,
    pub year: u32,
    pub tracks: Vec<usize>,
    pub duration: Duration,
    pub has_cover: bool,
    pub various: bool,
}

impl Album {
    pub fn label(&self) -> String {
        let mut label = if self.artist.is_empty() {
            self.title.clone()
        } else {
            format!("{} — {}", self.artist, self.title)
        };
        if self.year > 0 {
            label.push_str(&format!(" ({})", self.year));
        }
        label
    }
}

#[derive(Clone, Debug)]
pub struct ArtistGroup {
    pub key: String,
    pub name: String,
    pub albums: Vec<usize>,
}

pub fn group(entries: &[Entry], mode: GroupMode) -> Vec<Album> {
    let mut albums: HashMap<String, Album> = HashMap::new();
    let mut order = Vec::new();
    for (index, entry) in entries.iter().enumerate() {
        let key = album_key(entry, mode);
        let album = albums.entry(key.clone()).or_insert_with(|| {
            order.push(key.clone());
            Album {
                key: key.clone(),
                artist: String::new(),
                title: String::new(),
                year: 0,
                tracks: Vec::new(),
                duration: Duration::ZERO,
                has_cover: false,
                various: false,
            }
        });
        album.tracks.push(index);
    }
    let mut out: Vec<Album> = order
        .into_iter()
        .filter_map(|key| albums.remove(&key))
        .map(|album| finish_album(album, entries))
        .collect();
    out.sort_by(|a, b| {
        a.artist
            .to_ascii_lowercase()
            .cmp(&b.artist.to_ascii_lowercase())
            .then(a.year.cmp(&b.year))
            .then(
                a.title
                    .to_ascii_lowercase()
                    .cmp(&b.title.to_ascii_lowercase()),
            )
            .then(a.key.cmp(&b.key))
    });
    out
}

fn album_key(entry: &Entry, mode: GroupMode) -> String {
    if mode == GroupMode::Folder {
        return format!(
            "dir:{}",
            entry
                .path
                .parent()
                .unwrap_or_else(|| Path::new(""))
                .display()
        );
    }
    let album = entry.metadata.album.trim().to_ascii_lowercase();
    let album_artist = entry.metadata.album_artist.trim().to_ascii_lowercase();
    if album.is_empty() {
        return format!(
            "dir:{}",
            entry
                .path
                .parent()
                .unwrap_or_else(|| Path::new(""))
                .display()
        );
    }
    if album_artist.is_empty() {
        format!("tag:album\x00{album}")
    } else {
        format!("tag:{album_artist}\x00{album}")
    }
}

fn finish_album(mut album: Album, entries: &[Entry]) -> Album {
    let Some(first_index) = album.tracks.first().copied() else {
        return album;
    };
    let first = &entries[first_index];
    album.title = first.album_title();
    album.artist = first.metadata.album_artist.trim().to_string();
    if album.artist.is_empty() {
        album.artist = first.artist().trim().to_string();
    }
    album.year = first.metadata.year;
    let mut artists = HashMap::new();
    let mut has_album_artist = false;
    for index in &album.tracks {
        let entry = &entries[*index];
        if !entry.metadata.album_artist.trim().is_empty() {
            has_album_artist = true;
        }
        if !entry.artist().trim().is_empty() {
            artists.insert(entry.artist().to_ascii_lowercase(), ());
        }
        album.duration += entry.duration();
        album.has_cover |= entry.properties.has_cover();
        if album.year == 0 {
            album.year = entry.metadata.year;
        }
    }
    if !has_album_artist && artists.len() > 1 {
        album.artist = "Various Artists".into();
        album.various = true;
    }
    if album.artist.is_empty() {
        album.artist = "Unknown Artist".into();
    }
    album.tracks.sort_by(|a, b| {
        let ea = &entries[*a];
        let eb = &entries[*b];
        disc_sort(ea.metadata.disc)
            .cmp(&disc_sort(eb.metadata.disc))
            .then(
                track_sort(ea.metadata.track)
                    .cmp(&track_sort(eb.metadata.track)),
            )
            .then(
                ea.title()
                    .to_ascii_lowercase()
                    .cmp(&eb.title().to_ascii_lowercase()),
            )
            .then(ea.path.cmp(&eb.path))
    });
    album
}

fn disc_sort(value: u32) -> u32 {
    if value == 0 { 1 } else { value }
}

fn track_sort(value: u32) -> u32 {
    if value == 0 { u32::MAX } else { value }
}

pub fn group_artists(albums: &[Album]) -> Vec<ArtistGroup> {
    let mut index: HashMap<String, usize> = HashMap::new();
    let mut artists: Vec<ArtistGroup> = Vec::new();
    for (album_index, album) in albums.iter().enumerate() {
        let key = album.artist.trim().to_ascii_lowercase();
        let key = if key.is_empty() {
            "unknown artist".to_string()
        } else {
            key
        };
        let artist_index = *index.entry(key.clone()).or_insert_with(|| {
            artists.push(ArtistGroup {
                key: key.clone(),
                name: if album.artist.trim().is_empty() {
                    "Unknown Artist".into()
                } else {
                    album.artist.clone()
                },
                albums: Vec::new(),
            });
            artists.len() - 1
        });
        artists[artist_index].albums.push(album_index);
    }
    artists
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RequiredTag {
    pub field: Option<Field>,
    pub cover: bool,
}

impl RequiredTag {
    pub fn label(&self) -> &'static str {
        if self.cover {
            "Cover"
        } else {
            self.field.map(Field::label).unwrap_or("Tag")
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        let value = value.trim();
        let lower = value.to_ascii_lowercase();
        if matches!(lower.as_str(), "cover" | "art" | "artwork" | "cover art") {
            return Some(Self {
                field: None,
                cover: true,
            });
        }
        let normalized = value.replace([' ', '_'], "-");
        let field =
            Field::parse(value).or_else(|| Field::parse(&normalized))?;
        Some(Self {
            field: Some(field),
            cover: false,
        })
    }
}

pub fn default_required() -> Vec<RequiredTag> {
    ["artist", "album", "title", "track"]
        .iter()
        .filter_map(|name| RequiredTag::parse(name))
        .collect()
}

pub fn parse_required(names: &[String]) -> Vec<RequiredTag> {
    names
        .iter()
        .filter_map(|name| RequiredTag::parse(name))
        .collect()
}

pub fn missing_requirements(
    entry: &Entry,
    requirements: &[RequiredTag],
) -> Vec<RequiredTag> {
    requirements
        .iter()
        .filter(|requirement| missing_requirement(entry, requirement))
        .cloned()
        .collect()
}

pub fn missing_requirement(entry: &Entry, requirement: &RequiredTag) -> bool {
    if requirement.cover {
        return !entry.properties.has_cover();
    }
    requirement
        .field
        .map(|field| entry.metadata.get(field).trim().is_empty())
        .unwrap_or(false)
}

/// A track/disc number parsed from a file name.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct TrackGuess {
    pub disc: u32,
    pub track: u32,
    pub total: u32,
}

pub fn guess_track_number(name: &Path) -> Option<TrackGuess> {
    let base = name.file_stem()?.to_string_lossy().to_string();
    let base = base.trim();
    if base.is_empty() {
        return None;
    }
    let patterns: &[(regex::Regex, &str)] = &[];
    let _ = patterns;
    // Ordered parser, equivalent to the Go implementation.
    if let Some(caps) = regex::Regex::new(r"(?i)^\s*(?:cd|disc)\s*(\d{1,2})\s*(?:[-._]\s*)?(?:track\s*)?(\d{1,3})\b").ok()?.captures(base) {
        return finish_guess(caps.get(2)?.as_str(), "", caps.get(1)?.as_str());
    }
    if let Some(caps) = regex::Regex::new(r"^\s*(\d{1,2})[-._](\d{1,3})\b")
        .ok()?
        .captures(base)
    {
        return finish_guess(caps.get(2)?.as_str(), "", caps.get(1)?.as_str());
    }
    if let Some(caps) = regex::Regex::new(
        r"^\s*[\[\(]?(\d{1,3})[\]\)]?\s*(?:/|of)\s*(\d{1,3})\b",
    )
    .ok()?
    .captures(base)
    {
        return finish_guess(caps.get(1)?.as_str(), caps.get(2)?.as_str(), "");
    }
    if let Some(caps) =
        regex::Regex::new(r"(?i)^\s*(?:track|trk)\s*[#.]?\s*(\d{1,3})\b")
            .ok()?
            .captures(base)
    {
        return finish_guess(caps.get(1)?.as_str(), "", "");
    }
    if let Some(caps) =
        regex::Regex::new(r"^\s*[\[\(]?(\d{1,3})[\]\)]?\s*(?:[-._)]|\s+)\s*")
            .ok()?
            .captures(base)
    {
        return finish_guess(caps.get(1)?.as_str(), "", "");
    }
    if let Some(caps) = regex::Regex::new(r"(?:^|[\s._-])(\d{1,3})\s*$")
        .ok()?
        .captures(base)
    {
        if !regex::Regex::new(
            r"^\s*(?:19|20)\d{2}[-._/]\d{1,2}(?:[-._/]\d{1,2})?",
        )
        .ok()?
        .is_match(base)
        {
            return finish_guess(caps.get(1)?.as_str(), "", "");
        }
    }
    None
}

fn finish_guess(
    track_text: &str,
    total_text: &str,
    disc_text: &str,
) -> Option<TrackGuess> {
    let track = parse_small(track_text)?;
    if track == 0 || track > 999 {
        return None;
    }
    let mut guess = TrackGuess {
        track,
        ..Default::default()
    };
    if let Some(total) = parse_small(total_text) {
        if total >= track && total <= 999 {
            guess.total = total;
        }
    }
    if let Some(disc) = parse_small(disc_text) {
        if disc > 0 && disc <= 99 {
            guess.disc = disc;
        }
    }
    Some(guess)
}

fn parse_small(value: &str) -> Option<u32> {
    let value = value.trim();
    if value.is_empty() || !value.chars().all(|c| c.is_ascii_digit()) {
        return None;
    }
    value.parse().ok().filter(|n| *n <= 999)
}

pub fn duration_label(duration: Duration) -> String {
    let total = duration.as_secs();
    let hours = total / 3600;
    let minutes = (total % 3600) / 60;
    let seconds = total % 60;
    if hours > 0 {
        format!("{hours}:{minutes:02}:{seconds:02}")
    } else {
        format!("{minutes}:{seconds:02}")
    }
}

pub fn size_label(bytes: u64) -> String {
    const UNITS: [&str; 6] = ["B", "KB", "MB", "GB", "TB", "PB"];
    let mut size = bytes as f64;
    let mut unit = 0;
    while size >= 1024.0 && unit + 1 < UNITS.len() {
        size /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{bytes} B")
    } else {
        format!("{size:.1} {}", UNITS[unit])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn guesses_numbers() {
        for (name, track, total, disc) in [
            ("01 - Song.flac", 1, 0, 0),
            ("[03] Title.flac", 3, 0, 0),
            ("Track 05 - Title.flac", 5, 0, 0),
            ("Disc 2 - 03 Title.flac", 3, 0, 2),
            ("1-02 Title.flac", 2, 0, 1),
            ("06 of 12 Title.flac", 6, 12, 0),
            ("Title - 07.flac", 7, 0, 0),
        ] {
            let guess = guess_track_number(Path::new(name)).unwrap();
            assert_eq!(
                (guess.track, guess.total, guess.disc),
                (track, total, disc),
                "{name}"
            );
        }
        for name in [
            "1999.flac",
            "2024-01-01.flac",
            "00 - bad.flac",
            "no number.flac",
        ] {
            assert!(guess_track_number(Path::new(name)).is_none(), "{name}");
        }
    }

    #[test]
    fn required_tags_default() {
        let tags = default_required();
        assert_eq!(tags.len(), 4);
        assert!(RequiredTag::parse("track number").is_some());
        assert!(RequiredTag::parse("cover").is_some());
        assert!(RequiredTag::parse("not-a-tag").is_none());
    }
}
