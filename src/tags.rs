//! Tag reading and writing built on lofty.
//!
//! Reads parse only metadata and audio properties. Writes are read-modify-write:
//! fields that did not change are left alone, and format-specific items we do
//! not understand stay on the tag.

use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{Context, Result, bail};
use lofty::config::WriteOptions;
use lofty::file::{AudioFile, FileType, TaggedFile, TaggedFileExt};
use lofty::picture::{Picture, PictureType};
use lofty::prelude::{Accessor, ItemKey};
use lofty::probe::Probe;
use lofty::tag::{ItemValue, Tag, TagItem, TagType};

/// Editable, format-neutral metadata.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Metadata {
    pub title: String,
    pub artists: Vec<String>,
    pub album: String,
    pub album_artist: String,
    pub genres: Vec<String>,
    pub year: u32,
    pub date: String,
    pub track: u32,
    pub track_total: u32,
    pub disc: u32,
    pub disc_total: u32,
    pub compilation: bool,
    pub comment: String,
    pub lyrics: String,
    pub composer: String,
    pub conductor: String,
    pub lyricist: String,
    pub bpm: String,
    pub key: String,
    pub isrc: String,
    pub publisher: String,
    pub catalog_number: String,
    pub barcode: String,
    pub media: String,
    pub grouping: String,
    pub subtitle: String,
    pub original_date: String,
    pub copyright: String,
    pub encoded_by: String,
    pub musicbrainz_track_id: String,
    pub musicbrainz_album_id: String,
}

/// Audio and container properties.
#[derive(Clone, Debug, Default)]
pub struct Properties {
    pub container: String,
    pub codec: String,
    pub duration: Duration,
    pub bitrate_kbps: u32,
    pub sample_rate: u32,
    pub channels: u32,
    pub bit_depth: u32,
    pub size: u64,
    pub images: Vec<ImageInfo>,
    pub folder_cover: Option<PathBuf>,
}

#[derive(Clone, Debug)]
pub struct ImageInfo {
    pub kind: String,
    pub mime: String,
    pub size: usize,
}

impl Properties {
    pub fn has_cover(&self) -> bool {
        !self.images.is_empty() || self.folder_cover.is_some()
    }
}

#[derive(Clone, Debug, Default)]
pub struct TrackData {
    pub metadata: Metadata,
    pub properties: Properties,
}

#[derive(Clone, Debug)]
pub enum CoverAction {
    Keep,
    Set(Vec<u8>),
    Remove,
}

/// All editable fields in display order.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Field {
    Title,
    Artist,
    Album,
    AlbumArtist,
    Genre,
    Year,
    Track,
    TrackTotal,
    Disc,
    DiscTotal,
    Compilation,
    Comment,
    Lyrics,
    Composer,
    Conductor,
    Lyricist,
    Bpm,
    Key,
    Isrc,
    Publisher,
    CatalogNumber,
    Barcode,
    Media,
    Grouping,
    Subtitle,
    OriginalDate,
    Copyright,
    EncodedBy,
    MusicBrainzTrackId,
    MusicBrainzAlbumId,
}

pub const ALL_FIELDS: &[Field] = &[
    Field::Title,
    Field::Artist,
    Field::Album,
    Field::AlbumArtist,
    Field::Genre,
    Field::Year,
    Field::Track,
    Field::TrackTotal,
    Field::Disc,
    Field::DiscTotal,
    Field::Compilation,
    Field::Composer,
    Field::Conductor,
    Field::Lyricist,
    Field::Bpm,
    Field::Key,
    Field::Isrc,
    Field::Comment,
    Field::Lyrics,
    Field::Publisher,
    Field::CatalogNumber,
    Field::Barcode,
    Field::Media,
    Field::Grouping,
    Field::Subtitle,
    Field::OriginalDate,
    Field::Copyright,
    Field::EncodedBy,
    Field::MusicBrainzTrackId,
    Field::MusicBrainzAlbumId,
];

impl Field {
    pub fn label(self) -> &'static str {
        match self {
            Field::Title => "Title",
            Field::Artist => "Artist",
            Field::Album => "Album",
            Field::AlbumArtist => "Album artist",
            Field::Genre => "Genre",
            Field::Year => "Year",
            Field::Track => "Track",
            Field::TrackTotal => "Track total",
            Field::Disc => "Disc",
            Field::DiscTotal => "Disc total",
            Field::Compilation => "Compilation",
            Field::Comment => "Comment",
            Field::Lyrics => "Lyrics",
            Field::Composer => "Composer",
            Field::Conductor => "Conductor",
            Field::Lyricist => "Lyricist",
            Field::Bpm => "BPM",
            Field::Key => "Key",
            Field::Isrc => "ISRC",
            Field::Publisher => "Label",
            Field::CatalogNumber => "Catalog #",
            Field::Barcode => "Barcode",
            Field::Media => "Media",
            Field::Grouping => "Grouping",
            Field::Subtitle => "Subtitle",
            Field::OriginalDate => "Original date",
            Field::Copyright => "Copyright",
            Field::EncodedBy => "Encoded by",
            Field::MusicBrainzTrackId => "MusicBrainz track ID",
            Field::MusicBrainzAlbumId => "MusicBrainz album ID",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        let value = value.trim().to_ascii_lowercase();
        Some(match value.as_str() {
            "title" => Field::Title,
            "artist" | "artists" | "performer" | "performers" => Field::Artist,
            "album" => Field::Album,
            "albumartist" | "album-artist" | "album_artist" | "band"
            | "album artist" => Field::AlbumArtist,
            "genre" | "genres" | "style" => Field::Genre,
            "year" | "date" | "published" | "release-date" => Field::Year,
            "track" | "tracknumber" | "track-number" | "trackno"
            | "track-no" => Field::Track,
            "track-total" | "tracktotal" | "total-tracks" | "totaltracks"
            | "tracks" => Field::TrackTotal,
            "disc" | "discnumber" | "disc-number" | "discno" | "disc-no"
            | "disk" => Field::Disc,
            "disc-total" | "disctotal" | "total-discs" | "totaldiscs"
            | "discs" | "disk-total" => Field::DiscTotal,
            "compilation" | "comp" | "itunescompilation" => Field::Compilation,
            "comment" | "comments" => Field::Comment,
            "lyrics" | "unsyncedlyrics" | "unsynced-lyrics" => Field::Lyrics,
            "composer" => Field::Composer,
            "conductor" => Field::Conductor,
            "lyricist" => Field::Lyricist,
            "bpm" | "tempo" => Field::Bpm,
            "key" | "initialkey" | "initial-key" | "initial key" => Field::Key,
            "isrc" => Field::Isrc,
            "publisher" | "label" | "record-label" | "recordlabel" => {
                Field::Publisher
            }
            "catalog-number" | "catalognumber" | "catalog#" | "catalog"
            | "catno" => Field::CatalogNumber,
            "barcode" | "upc" => Field::Barcode,
            "media" | "mediatype" | "media-type" => Field::Media,
            "grouping" | "contentgroup" | "content-group" => Field::Grouping,
            "subtitle" => Field::Subtitle,
            "original-date" | "originaldate" | "originalyear"
            | "original-year" => Field::OriginalDate,
            "copyright" => Field::Copyright,
            "encoded-by" | "encodedby" | "encoder" => Field::EncodedBy,
            "musicbrainz-track-id" | "musicbrainz_trackid" | "mbtrackid" => {
                Field::MusicBrainzTrackId
            }
            "musicbrainz-album-id" | "musicbrainz_albumid" | "mbalbumid" => {
                Field::MusicBrainzAlbumId
            }
            _ => return None,
        })
    }
}

impl Metadata {
    pub fn artist_string(&self) -> String {
        self.artists.join(", ")
    }

    pub fn genre_string(&self) -> String {
        self.genres.join(", ")
    }

    pub fn get(&self, field: Field) -> String {
        match field {
            Field::Title => self.title.clone(),
            Field::Artist => self.artists.join("; "),
            Field::Album => self.album.clone(),
            Field::AlbumArtist => self.album_artist.clone(),
            Field::Genre => self.genres.join(", "),
            Field::Year => number(self.year),
            Field::Track => number(self.track),
            Field::TrackTotal => number(self.track_total),
            Field::Disc => number(self.disc),
            Field::DiscTotal => number(self.disc_total),
            Field::Compilation => {
                if self.compilation { "yes" } else { "no" }.to_string()
            }
            Field::Comment => self.comment.clone(),
            Field::Lyrics => self.lyrics.clone(),
            Field::Composer => self.composer.clone(),
            Field::Conductor => self.conductor.clone(),
            Field::Lyricist => self.lyricist.clone(),
            Field::Bpm => self.bpm.clone(),
            Field::Key => self.key.clone(),
            Field::Isrc => self.isrc.clone(),
            Field::Publisher => self.publisher.clone(),
            Field::CatalogNumber => self.catalog_number.clone(),
            Field::Barcode => self.barcode.clone(),
            Field::Media => self.media.clone(),
            Field::Grouping => self.grouping.clone(),
            Field::Subtitle => self.subtitle.clone(),
            Field::OriginalDate => self.original_date.clone(),
            Field::Copyright => self.copyright.clone(),
            Field::EncodedBy => self.encoded_by.clone(),
            Field::MusicBrainzTrackId => self.musicbrainz_track_id.clone(),
            Field::MusicBrainzAlbumId => self.musicbrainz_album_id.clone(),
        }
    }

    pub fn set(&mut self, field: Field, value: &str) -> Result<()> {
        let value = value.trim();
        match field {
            Field::Title => self.title = value.to_string(),
            Field::Artist => self.artists = split_list(value, ';'),
            Field::Album => self.album = value.to_string(),
            Field::AlbumArtist => self.album_artist = value.to_string(),
            Field::Genre => self.genres = split_list(value, ','),
            Field::Year => self.year = parse_number(value).context("year")?,
            Field::Track => {
                if value.is_empty() {
                    self.track = 0;
                    self.track_total = 0;
                } else {
                    let (track, total) = parse_pair(value)?;
                    self.track = track;
                    if total > 0 {
                        self.track_total = total;
                    }
                }
            }
            Field::TrackTotal => {
                self.track_total = parse_number(value).context("track total")?
            }
            Field::Disc => {
                if value.is_empty() {
                    self.disc = 0;
                    self.disc_total = 0;
                } else {
                    let (disc, total) = parse_pair(value)?;
                    self.disc = disc;
                    if total > 0 {
                        self.disc_total = total;
                    }
                }
            }
            Field::DiscTotal => {
                self.disc_total = parse_number(value).context("disc total")?
            }
            Field::Compilation => self.compilation = parse_bool(value)?,
            Field::Comment => self.comment = value.to_string(),
            Field::Lyrics => self.lyrics = value.to_string(),
            Field::Composer => self.composer = value.to_string(),
            Field::Conductor => self.conductor = value.to_string(),
            Field::Lyricist => self.lyricist = value.to_string(),
            Field::Bpm => self.bpm = value.to_string(),
            Field::Key => self.key = value.to_string(),
            Field::Isrc => self.isrc = value.to_string(),
            Field::Publisher => self.publisher = value.to_string(),
            Field::CatalogNumber => self.catalog_number = value.to_string(),
            Field::Barcode => self.barcode = value.to_string(),
            Field::Media => self.media = value.to_string(),
            Field::Grouping => self.grouping = value.to_string(),
            Field::Subtitle => self.subtitle = value.to_string(),
            Field::OriginalDate => self.original_date = value.to_string(),
            Field::Copyright => self.copyright = value.to_string(),
            Field::EncodedBy => self.encoded_by = value.to_string(),
            Field::MusicBrainzTrackId => {
                self.musicbrainz_track_id = value.to_string()
            }
            Field::MusicBrainzAlbumId => {
                self.musicbrainz_album_id = value.to_string()
            }
        }
        Ok(())
    }
}

fn number(value: u32) -> String {
    if value == 0 {
        String::new()
    } else {
        value.to_string()
    }
}

fn split_list(value: &str, sep: char) -> Vec<String> {
    value
        .split(sep)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(ToOwned::to_owned)
        .collect()
}

fn parse_number(value: &str) -> Result<u32> {
    let value = value.trim();
    if value.is_empty() {
        return Ok(0);
    }
    value
        .parse::<u32>()
        .with_context(|| format!("invalid number {value:?}"))
}

fn parse_pair(value: &str) -> Result<(u32, u32)> {
    let (a, b) = value.split_once('/').unwrap_or((value, ""));
    Ok((parse_number(a)?, parse_number(b)?))
}

fn parse_bool(value: &str) -> Result<bool> {
    Ok(match value.trim().to_ascii_lowercase().as_str() {
        "yes" | "true" | "1" | "on" | "y" => true,
        "no" | "false" | "0" | "off" | "n" | "" => false,
        other => bail!("invalid boolean {other:?}"),
    })
}

/// Read a track's tags and properties.
pub fn read(path: &Path) -> Result<TrackData> {
    let tagged = open(path)?;
    Ok(track_data(path, &tagged))
}

fn open(path: &Path) -> Result<TaggedFile> {
    Probe::open(path)
        .with_context(|| format!("{}: cannot open", path.display()))?
        .guess_file_type()
        .with_context(|| format!("{}: cannot detect format", path.display()))?
        .read()
        .with_context(|| format!("{}: cannot read metadata", path.display()))
}

fn track_data(path: &Path, tagged: &TaggedFile) -> TrackData {
    let tag = tagged.primary_tag().or_else(|| tagged.first_tag());
    let metadata = tag.map(metadata_from).unwrap_or_default();
    TrackData {
        metadata,
        properties: properties_from(path, tagged, tag),
    }
}

#[allow(clippy::field_reassign_with_default)]
fn metadata_from(tag: &Tag) -> Metadata {
    let mut meta = Metadata::default();
    meta.title = first(tag, ItemKey::TrackTitle);
    meta.artists = values(tag, ItemKey::TrackArtists);
    if meta.artists.is_empty() {
        meta.artists = values(tag, ItemKey::TrackArtist);
    }
    if meta.artists.is_empty() {
        if let Some(artist) = tag.artist() {
            meta.artists.push(artist.trim().to_string());
        }
    }
    meta.album = first(tag, ItemKey::AlbumTitle);
    meta.album_artist = first(tag, ItemKey::AlbumArtist);
    if meta.album_artist.is_empty() {
        meta.album_artist = first(tag, ItemKey::AlbumArtists);
    }
    meta.genres = values(tag, ItemKey::Genre);
    if meta.genres.is_empty() {
        if let Some(genre) = tag.genre() {
            meta.genres.push(genre.trim().to_string());
        }
    }
    let date = first(tag, ItemKey::RecordingDate);
    meta.date = date.clone();
    meta.year = parse_year(&date)
        .or_else(|| parse_year(&first(tag, ItemKey::Year)))
        .unwrap_or_default();
    meta.track = tag.track().unwrap_or_default();
    meta.track_total = tag.track_total().unwrap_or_default();
    meta.disc = tag.disk().unwrap_or_default();
    meta.disc_total = tag.disk_total().unwrap_or_default();
    meta.compilation =
        parse_bool(&first(tag, ItemKey::FlagCompilation)).unwrap_or(false);
    meta.comment = first(tag, ItemKey::Comment);
    meta.lyrics = first(tag, ItemKey::Lyrics);
    if meta.lyrics.is_empty() {
        meta.lyrics = first(tag, ItemKey::UnsyncLyrics);
    }
    meta.composer = first(tag, ItemKey::Composer);
    meta.conductor = first(tag, ItemKey::Conductor);
    meta.lyricist = first(tag, ItemKey::Lyricist);
    meta.bpm = first(tag, ItemKey::Bpm);
    if meta.bpm.is_empty() {
        meta.bpm = first(tag, ItemKey::IntegerBpm);
    }
    meta.key = first(tag, ItemKey::InitialKey);
    meta.isrc = first(tag, ItemKey::Isrc);
    meta.publisher = first(tag, ItemKey::Label);
    if meta.publisher.is_empty() {
        meta.publisher = first(tag, ItemKey::Publisher);
    }
    meta.catalog_number = first(tag, ItemKey::CatalogNumber);
    meta.barcode = first(tag, ItemKey::Barcode);
    meta.media = first(tag, ItemKey::OriginalMediaType);
    meta.grouping = first(tag, ItemKey::ContentGroup);
    meta.subtitle = first(tag, ItemKey::TrackSubtitle);
    if meta.subtitle.is_empty() {
        meta.subtitle = first(tag, ItemKey::SetSubtitle);
    }
    meta.original_date = first(tag, ItemKey::OriginalReleaseDate);
    meta.copyright = first(tag, ItemKey::CopyrightMessage);
    meta.encoded_by = first(tag, ItemKey::EncodedBy);
    if meta.encoded_by.is_empty() {
        meta.encoded_by = first(tag, ItemKey::EncoderSoftware);
    }
    meta.musicbrainz_track_id = first(tag, ItemKey::MusicBrainzTrackId);
    if meta.musicbrainz_track_id.is_empty() {
        meta.musicbrainz_track_id = first(tag, ItemKey::MusicBrainzRecordingId);
    }
    meta.musicbrainz_album_id = first(tag, ItemKey::MusicBrainzReleaseId);
    meta
}

fn first(tag: &Tag, key: ItemKey) -> String {
    tag.get_string(key)
        .map(str::trim)
        .unwrap_or_default()
        .to_string()
}

fn values(tag: &Tag, key: ItemKey) -> Vec<String> {
    tag.get_strings(key)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(ToOwned::to_owned)
        .collect()
}

fn parse_year(value: &str) -> Option<u32> {
    let digits: String = value
        .trim()
        .chars()
        .take_while(|c| c.is_ascii_digit())
        .collect();
    if digits.len() >= 4 {
        digits[..4].parse().ok()
    } else {
        None
    }
}

fn properties_from(
    path: &Path,
    tagged: &TaggedFile,
    tag: Option<&Tag>,
) -> Properties {
    let props = tagged.properties();
    let images: Vec<ImageInfo> = tag
        .map(|tag| {
            tag.pictures()
                .iter()
                .map(|picture| ImageInfo {
                    kind: format!("{:?}", picture.pic_type()),
                    mime: picture
                        .mime_type()
                        .map(|mime| format!("{mime:?}"))
                        .unwrap_or_default(),
                    size: picture.data().len(),
                })
                .collect()
        })
        .unwrap_or_default();
    let folder_cover = if images.is_empty() {
        find_folder_cover(path.parent().unwrap_or_else(|| Path::new(".")))
    } else {
        None
    };
    Properties {
        container: file_type_name(tagged.file_type()),
        codec: file_type_name(tagged.file_type()),
        duration: props.duration(),
        bitrate_kbps: props.audio_bitrate().unwrap_or_default(),
        sample_rate: props.sample_rate().unwrap_or_default(),
        channels: props.channels().map(u32::from).unwrap_or_default(),
        bit_depth: props.bit_depth().map(u32::from).unwrap_or_default(),
        size: fs::metadata(path).map(|m| m.len()).unwrap_or_default(),
        images,
        folder_cover,
    }
}

/// Apply metadata changes and an optional cover edit, preserving untouched
/// items and format-specific tags.
pub fn write(
    path: &Path,
    metadata: &Metadata,
    cover: CoverAction,
) -> Result<TrackData> {
    let mut tagged = open(path)?;
    let old = tagged
        .primary_tag()
        .or_else(|| tagged.first_tag())
        .map(metadata_from)
        .unwrap_or_default();
    ensure_tag(&mut tagged)?;
    let tag = if let Some(tag) = tagged.primary_tag_mut() {
        tag
    } else if let Some(tag) = tagged.first_tag_mut() {
        tag
    } else {
        bail!("{}: no writable tag", path.display())
    };
    apply_metadata(tag, &old, metadata);
    apply_cover(tag, cover);
    tagged
        .save_to_path(path, WriteOptions::default())
        .with_context(|| format!("{}: cannot save metadata", path.display()))?;
    read(path)
}

fn ensure_tag(tagged: &mut TaggedFile) -> Result<()> {
    if tagged.primary_tag().is_some() || tagged.first_tag().is_some() {
        return Ok(());
    }
    let tag_type = tag_type_for(tagged.file_type());
    tagged.insert_tag(Tag::new(tag_type));
    Ok(())
}

fn file_type_name(file_type: FileType) -> String {
    match file_type {
        FileType::Mpeg => "mp3",
        FileType::Flac => "flac",
        FileType::Vorbis => "ogg",
        FileType::Opus => "opus",
        FileType::Mp4 => "m4a",
        FileType::Aac => "aac",
        FileType::Aiff => "aiff",
        FileType::Ape => "ape",
        FileType::Mpc => "mpc",
        FileType::Speex => "speex",
        FileType::Wav => "wav",
        FileType::WavPack => "wavpack",
        FileType::Custom(name) => name,
        _ => "audio",
    }
    .to_string()
}

fn tag_type_for(file_type: FileType) -> TagType {
    match file_type {
        FileType::Flac
        | FileType::Vorbis
        | FileType::Opus
        | FileType::Speex => TagType::VorbisComments,
        FileType::Mp4 => TagType::Mp4Ilst,
        _ => TagType::Id3v2,
    }
}

fn apply_metadata(tag: &mut Tag, old: &Metadata, new: &Metadata) {
    if old.title != new.title {
        replace_text(tag, ItemKey::TrackTitle, &new.title);
    }
    if old.artists != new.artists {
        replace_values(
            tag,
            &[ItemKey::TrackArtist, ItemKey::TrackArtists],
            &new.artists,
        );
    }
    if old.album != new.album {
        replace_text(tag, ItemKey::AlbumTitle, &new.album);
    }
    if old.album_artist != new.album_artist {
        replace_values(
            tag,
            &[ItemKey::AlbumArtist, ItemKey::AlbumArtists],
            &if new.album_artist.is_empty() {
                Vec::new()
            } else {
                vec![new.album_artist.clone()]
            },
        );
    }
    if old.genres != new.genres {
        replace_values(tag, &[ItemKey::Genre], &new.genres);
    }
    if old.year != new.year {
        tag.remove_key(ItemKey::Year);
        tag.remove_key(ItemKey::RecordingDate);
        if new.year > 0 {
            tag.insert_text(ItemKey::RecordingDate, new.year.to_string());
        }
    }
    if old.track != new.track {
        tag.remove_track();
        if new.track > 0 {
            tag.set_track(new.track);
        }
    }
    if old.track_total != new.track_total {
        tag.remove_track_total();
        if new.track_total > 0 {
            tag.set_track_total(new.track_total);
        }
    }
    if old.disc != new.disc {
        tag.remove_disk();
        if new.disc > 0 {
            tag.set_disk(new.disc);
        }
    }
    if old.disc_total != new.disc_total {
        tag.remove_disk_total();
        if new.disc_total > 0 {
            tag.set_disk_total(new.disc_total);
        }
    }
    if old.compilation != new.compilation {
        tag.remove_key(ItemKey::FlagCompilation);
        if new.compilation {
            tag.insert_text(ItemKey::FlagCompilation, "1".into());
        }
    }
    replace_if_changed(tag, ItemKey::Comment, &old.comment, &new.comment);
    replace_if_changed(tag, ItemKey::Lyrics, &old.lyrics, &new.lyrics);
    replace_if_changed(tag, ItemKey::Composer, &old.composer, &new.composer);
    replace_if_changed(tag, ItemKey::Conductor, &old.conductor, &new.conductor);
    replace_if_changed(tag, ItemKey::Lyricist, &old.lyricist, &new.lyricist);
    replace_if_changed(tag, ItemKey::Bpm, &old.bpm, &new.bpm);
    replace_if_changed(tag, ItemKey::InitialKey, &old.key, &new.key);
    replace_if_changed(tag, ItemKey::Isrc, &old.isrc, &new.isrc);
    replace_if_changed(tag, ItemKey::Label, &old.publisher, &new.publisher);
    replace_if_changed(
        tag,
        ItemKey::CatalogNumber,
        &old.catalog_number,
        &new.catalog_number,
    );
    replace_if_changed(tag, ItemKey::Barcode, &old.barcode, &new.barcode);
    replace_if_changed(tag, ItemKey::OriginalMediaType, &old.media, &new.media);
    replace_if_changed(
        tag,
        ItemKey::ContentGroup,
        &old.grouping,
        &new.grouping,
    );
    replace_if_changed(
        tag,
        ItemKey::TrackSubtitle,
        &old.subtitle,
        &new.subtitle,
    );
    replace_if_changed(
        tag,
        ItemKey::OriginalReleaseDate,
        &old.original_date,
        &new.original_date,
    );
    replace_if_changed(
        tag,
        ItemKey::CopyrightMessage,
        &old.copyright,
        &new.copyright,
    );
    replace_if_changed(
        tag,
        ItemKey::EncodedBy,
        &old.encoded_by,
        &new.encoded_by,
    );
    replace_if_changed(
        tag,
        ItemKey::MusicBrainzTrackId,
        &old.musicbrainz_track_id,
        &new.musicbrainz_track_id,
    );
    replace_if_changed(
        tag,
        ItemKey::MusicBrainzReleaseId,
        &old.musicbrainz_album_id,
        &new.musicbrainz_album_id,
    );
}

fn replace_if_changed(tag: &mut Tag, key: ItemKey, old: &str, new: &str) {
    if old != new {
        replace_text(tag, key, new);
    }
}

fn replace_text(tag: &mut Tag, key: ItemKey, value: &str) {
    tag.remove_key(key);
    if !value.trim().is_empty() {
        tag.insert_text(key, value.trim().to_string());
    }
}

fn replace_values(tag: &mut Tag, keys: &[ItemKey], values: &[String]) {
    for key in keys {
        tag.remove_key(*key);
    }
    for value in values.iter().filter(|v| !v.trim().is_empty()) {
        tag.push(TagItem::new(
            keys[0],
            ItemValue::Text(value.trim().to_string()),
        ));
    }
}

fn apply_cover(tag: &mut Tag, cover: CoverAction) {
    match cover {
        CoverAction::Keep => {}
        CoverAction::Remove => {
            tag.remove_picture_type(PictureType::CoverFront);
            if tag.tag_type() == TagType::Mp4Ilst {
                tag.remove_picture_type(PictureType::Other);
            }
        }
        CoverAction::Set(data) => {
            tag.remove_picture_type(PictureType::CoverFront);
            if tag.tag_type() == TagType::Mp4Ilst {
                tag.remove_picture_type(PictureType::Other);
            }
            if !data.is_empty() {
                tag.push_picture(
                    Picture::unchecked(data)
                        .pic_type(PictureType::CoverFront)
                        .build(),
                );
            }
        }
    }
}

// ── covers ───────────────────────────────────────────────────────────────

pub const DEFAULT_COVER: &str = "albumart.jpg";

pub const COMMON_COVERS: &[&str] = &[
    "albumart.jpg",
    "albumart.jpeg",
    "albumart.png",
    "cover.jpg",
    "cover.jpeg",
    "cover.png",
    "folder.jpg",
    "folder.jpeg",
    "folder.png",
    "front.jpg",
    "front.jpeg",
    "front.png",
    "album.jpg",
    "album.jpeg",
    "album.png",
];

pub fn find_folder_cover(dir: &Path) -> Option<PathBuf> {
    COMMON_COVERS
        .iter()
        .map(|name| dir.join(name))
        .find(|path| path.is_file())
}

/// Read embedded artwork, then folder artwork.
pub fn read_artwork(
    path: &Path,
    configured: &str,
) -> Result<(Vec<u8>, String)> {
    if let Ok((data, mime)) = read_cover(path) {
        return Ok((data, mime));
    }
    let dir = path.parent().unwrap_or_else(|| Path::new("."));
    let mut candidates = Vec::new();
    if let Some(found) = find_folder_cover(dir) {
        candidates.push(found);
    }
    if !configured.trim().is_empty() {
        candidates.push(dir.join(filepath_base(configured)));
    }
    for candidate in candidates {
        if let Ok(data) = fs::read(&candidate) {
            let mime = detect_image_mime(&data);
            if mime.starts_with("image/") {
                return Ok((data, mime));
            }
        }
    }
    bail!("{}: no cover art", path.display())
}

pub fn read_cover(path: &Path) -> Result<(Vec<u8>, String)> {
    let tagged = open(path)?;
    let tag = tagged
        .primary_tag()
        .or_else(|| tagged.first_tag())
        .context("no tag")?;
    let picture = tag
        .pictures()
        .iter()
        .find(|p| p.pic_type() == PictureType::CoverFront)
        .or_else(|| tag.pictures().first())
        .context("no embedded cover")?;
    Ok((
        picture.data().to_vec(),
        picture
            .mime_type()
            .map(|mime| format!("{mime:?}").to_ascii_lowercase())
            .unwrap_or_else(|| detect_image_mime(picture.data())),
    ))
}

pub fn write_folder_cover(
    dir: &Path,
    configured: &str,
    data: &[u8],
) -> Result<PathBuf> {
    let name = if configured.trim().is_empty() {
        DEFAULT_COVER
    } else {
        filepath_base(configured)
    };
    fs::create_dir_all(dir)?;
    let target = dir.join(name);
    let tmp = dir.join(format!(".tuigr-cover-{}.tmp", std::process::id()));
    fs::write(&tmp, data)?;
    fs::rename(&tmp, &target)?;
    Ok(target)
}

pub fn remove_folder_cover(dir: &Path, configured: &str) -> Result<()> {
    let name = if configured.trim().is_empty() {
        DEFAULT_COVER
    } else {
        filepath_base(configured)
    };
    let path = dir.join(name);
    if path.exists() {
        fs::remove_file(&path)?;
    }
    let base = path.with_extension("");
    for ext in ["jpg", "jpeg", "png"] {
        let candidate = base.with_extension(ext);
        if candidate.exists() {
            let _ = fs::remove_file(candidate);
        }
    }
    Ok(())
}

fn filepath_base(value: &str) -> &str {
    Path::new(value)
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or(value)
}

pub fn detect_image_mime(data: &[u8]) -> String {
    if data.starts_with(&[0x89, b'P', b'N', b'G']) {
        "image/png".into()
    } else if data.starts_with(&[0xff, 0xd8, 0xff]) {
        "image/jpeg".into()
    } else if data.starts_with(b"GIF8") {
        "image/gif".into()
    } else {
        "application/octet-stream".into()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture(name: &str) -> (tempfile::TempDir, PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(name);
        fs::copy(Path::new("testdata").join(name), &path).unwrap();
        (dir, path)
    }

    #[test]
    fn round_trip_core_fields() {
        for name in [
            "silence.flac",
            "silence.mp3",
            "silence.m4a",
            "silence.ogg",
            "silence.opus",
        ] {
            let (_dir, path) = fixture(name);
            let mut data = read(&path).unwrap();
            data.metadata.title = "Title".into();
            data.metadata.artists = vec!["One".into(), "Two".into()];
            data.metadata.album = "Album".into();
            data.metadata.album_artist = "Band".into();
            data.metadata.genres = vec!["Rock".into(), "Jazz".into()];
            data.metadata.year = 1993;
            data.metadata.track = 3;
            data.metadata.track_total = 12;
            data.metadata.disc = 1;
            data.metadata.disc_total = 2;
            data.metadata.compilation = true;
            data.metadata.comment = "comment".into();
            data.metadata.composer = "Composer".into();
            data.metadata.publisher = "Label".into();
            data.metadata.catalog_number = "CAT-1".into();
            write(&path, &data.metadata, CoverAction::Keep).unwrap();
            let read_back = read(&path).unwrap().metadata;
            assert_eq!(read_back.title, "Title", "{name}");
            assert_eq!(read_back.album, "Album", "{name}");
            assert_eq!(read_back.album_artist, "Band", "{name}");
            assert_eq!(read_back.year, 1993, "{name}");
            assert_eq!(read_back.track, 3, "{name}");
            assert_eq!(read_back.track_total, 12, "{name}");
            assert_eq!(read_back.disc, 1, "{name}");
            assert_eq!(read_back.disc_total, 2, "{name}");
            assert!(read_back.compilation, "{name}");
            assert!(read_back.artists.contains(&"One".to_string()), "{name}");
            assert!(read_back.artists.contains(&"Two".to_string()), "{name}");
            assert!(read_back.genres.contains(&"Rock".to_string()), "{name}");
            assert_eq!(read_back.comment, "comment", "{name}");
            assert_eq!(read_back.composer, "Composer", "{name}");
            assert_eq!(read_back.publisher, "Label", "{name}");
        }
    }

    #[test]
    fn cover_round_trip() {
        let mut png = Vec::new();
        let image =
            image::RgbaImage::from_pixel(2, 2, image::Rgba([220, 40, 40, 255]));
        image::DynamicImage::ImageRgba8(image)
            .write_to(
                &mut std::io::Cursor::new(&mut png),
                image::ImageFormat::Png,
            )
            .unwrap();
        for name in
            ["silence.flac", "silence.mp3", "silence.m4a", "silence.ogg"]
        {
            let (_dir, path) = fixture(name);
            let data = read(&path).unwrap();
            write(&path, &data.metadata, CoverAction::Set(png.to_vec()))
                .unwrap();
            let (read_png, _) = read_cover(&path).unwrap();
            assert_eq!(read_png, png, "{name}");
            write(&path, &data.metadata, CoverAction::Remove).unwrap();
            assert!(read_cover(&path).is_err(), "{name}");
        }
    }
}
