use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::mpsc::{self, Receiver, Sender};
use std::time::Duration;

use anyhow::Result;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::style::Color;
use ratatui::text::Line;

use crate::config::Config;
use crate::cover;
use crate::library::{self, Album, ArtistGroup, Entry, GroupMode, RequiredTag};
use crate::online::{Client, Release};
use crate::organize;
use crate::tags::{self, CoverAction, Field, Metadata};
use crate::theme::{self, UiTheme};
use crate::tui::coverart;
use crate::tui::input::TextInput;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Pane {
    List,
    Detail,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    Browse,
    Search,
    Edit,
    Help,
    Cover,
    Lookup,
    ConfirmDelete,
    Theme,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StatusKind {
    Info,
    Ok,
    Error,
}

#[derive(Clone, Debug)]
pub struct ThemeChoice {
    pub id: String,
    pub name: String,
    pub accent: Color,
    pub background: Color,
}

#[derive(Clone, Debug)]
pub enum Row {
    Artist(usize),
    Album(usize),
    Track { album: usize, track: usize },
}

#[derive(Clone, Debug, Default)]
pub struct DetailRow {
    pub field: Option<Field>,
    pub label: String,
    pub value: String,
    pub album: bool,
    pub mixed: bool,
    pub read_only: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SortMode {
    Artist,
    Album,
    Year,
    Path,
    Duration,
    Format,
}

impl SortMode {
    fn next(self) -> Self {
        match self {
            Self::Artist => Self::Album,
            Self::Album => Self::Year,
            Self::Year => Self::Path,
            Self::Path => Self::Duration,
            Self::Duration => Self::Format,
            Self::Format => Self::Artist,
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            Self::Artist => "artist",
            Self::Album => "album",
            Self::Year => "year",
            Self::Path => "path",
            Self::Duration => "duration",
            Self::Format => "format",
        }
    }
    fn parse(value: &str) -> Self {
        match value
            .split(',')
            .next()
            .unwrap_or("")
            .trim()
            .to_ascii_lowercase()
            .as_str()
        {
            "album" => Self::Album,
            "year" | "date" => Self::Year,
            "path" | "file" => Self::Path,
            "duration" => Self::Duration,
            "format" => Self::Format,
            _ => Self::Artist,
        }
    }
}

#[derive(Clone, Debug)]
struct CoverJob {
    key: String,
    path: PathBuf,
    cover_file: String,
    width: u16,
    height: u16,
}

#[derive(Debug)]
struct CoverResult {
    key: String,
    lines: Vec<Line<'static>>,
}

#[derive(Clone, Debug)]
enum CachedCover {
    Lines(Vec<Line<'static>>),
    Missing,
}

/// One worker thread decodes artwork in the background. Requests are
/// coalesced: when several rows are visited quickly, only the most recent
/// job is rendered, so navigation never blocks on image decoding.
fn cover_worker(jobs: Receiver<CoverJob>, results: Sender<CoverResult>) {
    std::thread::spawn(move || {
        while let Ok(mut job) = jobs.recv() {
            while let Ok(newer) = jobs.try_recv() {
                job = newer;
            }
            let lines = tags::read_artwork(&job.path, &job.cover_file)
                .and_then(|(data, _)| {
                    coverart::render_cover(
                        &data,
                        job.width.max(1),
                        job.height.max(1),
                    )
                })
                .unwrap_or_default();
            if results
                .send(CoverResult {
                    key: job.key,
                    lines,
                })
                .is_err()
            {
                break;
            }
        }
    });
}

pub struct App {
    pub cfg: Config,
    pub theme: UiTheme,
    pub theme_name: String,
    pub dir: PathBuf,
    pub width: u16,
    pub height: u16,
    pub entries: Vec<Entry>,
    pub view: Vec<Entry>,
    pub albums: Vec<Album>,
    pub artists: Vec<ArtistGroup>,
    pub rows: Vec<Row>,
    pub list_index: usize,
    pub list_top: usize,
    pub pane: Pane,
    pub details: Vec<DetailRow>,
    pub detail_index: usize,
    pub detail_top: usize,
    pub selection: HashSet<PathBuf>,
    pub filter: String,
    pub mode: Mode,
    pub input: TextInput,
    pub edit_paths: Vec<PathBuf>,
    pub edit_row: Option<DetailRow>,
    pub cover_editing: bool,
    pub status: String,
    pub status_kind: StatusKind,
    pub sort: SortMode,
    pub expanded_artists: HashSet<String>,
    pub expanded_albums: HashSet<String>,
    pub show_cover: bool,
    pub theme_choices: Vec<ThemeChoice>,
    pub theme_index: usize,
    theme_original: String,
    pub cover_key: String,
    pub cover_lines: Vec<Line<'static>>,
    pub cover_note: String,
    pub cover_loading: bool,
    cover_cache: HashMap<String, CachedCover>,
    cover_jobs: Sender<CoverJob>,
    cover_results: Receiver<CoverResult>,
    pub help_top: usize,
    pub delete_paths: Vec<PathBuf>,
    pub required: Vec<RequiredTag>,
    pub lookup_releases: Vec<Release>,
    pub lookup_index: usize,
    pub lookup_album: Option<(PathBuf, String, String)>,
    pub quit: bool,
    client: Client,
}

impl App {
    pub fn new(dir: &Path, cfg: Config) -> Self {
        let theme = UiTheme::load(&cfg.theme);
        let theme_name = theme.name.clone();
        let required = if cfg.required_tags.is_empty() {
            library::default_required()
        } else {
            let parsed = library::parse_required(&cfg.required_tags);
            if parsed.is_empty() {
                library::default_required()
            } else {
                parsed
            }
        };
        let client = Client::new(&cfg.musicbrainz_user_agent);
        let (cover_jobs, job_rx) = mpsc::channel();
        let (result_tx, cover_results) = mpsc::channel();
        cover_worker(job_rx, result_tx);
        let mut app = Self {
            dir: dir.to_path_buf(),
            width: 80,
            height: 24,
            show_cover: cfg.cover_panel,
            theme_choices: Vec::new(),
            theme_index: 0,
            theme_original: String::new(),
            sort: SortMode::parse(&cfg.sort),
            cfg,
            theme,
            theme_name,
            entries: Vec::new(),
            view: Vec::new(),
            albums: Vec::new(),
            artists: Vec::new(),
            rows: Vec::new(),
            list_index: 0,
            list_top: 0,
            pane: Pane::List,
            details: Vec::new(),
            detail_index: 0,
            detail_top: 0,
            selection: HashSet::new(),
            filter: String::new(),
            mode: Mode::Browse,
            input: TextInput::default(),
            edit_paths: Vec::new(),
            edit_row: None,
            cover_editing: false,
            status: String::new(),
            status_kind: StatusKind::Info,
            expanded_artists: HashSet::new(),
            expanded_albums: HashSet::new(),
            cover_key: String::new(),
            cover_lines: Vec::new(),
            cover_note: String::new(),
            cover_loading: false,
            cover_cache: HashMap::new(),
            cover_jobs,
            cover_results,
            help_top: 0,
            delete_paths: Vec::new(),
            required,
            lookup_releases: Vec::new(),
            lookup_index: 0,
            lookup_album: None,
            quit: false,
            client,
        };
        app.reload();
        app
    }

    pub fn new_file(path: &Path, cfg: Config) -> Self {
        let mut app =
            Self::new(path.parent().unwrap_or_else(|| Path::new(".")), cfg);
        app.entries = vec![library::load_one(path)];
        app.rebuild();
        app.reveal_path(path);
        app
    }

    pub fn reload(&mut self) {
        self.cover_key.clear();
        match library::discover(&self.dir, true) {
            Ok(paths) => {
                self.entries = library::load(&paths);
                self.rebuild();
                self.status = format!(
                    "loaded {} tracks in {} albums",
                    self.entries.len(),
                    self.albums.len()
                );
                self.status_kind = StatusKind::Ok;
            }
            Err(err) => {
                self.status = format!("{err:#}");
                self.status_kind = StatusKind::Error;
            }
        }
    }

    pub fn rebuild(&mut self) {
        self.view = self
            .entries
            .iter()
            .filter(|entry| self.matches_query(entry, &self.filter))
            .cloned()
            .collect();
        let mode = GroupMode::parse(&self.cfg.group_by);
        self.albums = library::group(&self.view, mode);
        self.sort_albums();
        self.artists = library::group_artists(&self.albums);
        self.rows.clear();
        for (artist_index, artist) in self.artists.iter().enumerate() {
            self.rows.push(Row::Artist(artist_index));
            if !self.filter.is_empty()
                || self.expanded_artists.contains(&artist.key)
            {
                for album_index in &artist.albums {
                    let album = &self.albums[*album_index];
                    self.rows.push(Row::Album(*album_index));
                    if !self.filter.is_empty()
                        || self.expanded_albums.contains(&album.key)
                    {
                        for track in 0..album.tracks.len() {
                            self.rows.push(Row::Track {
                                album: *album_index,
                                track,
                            });
                        }
                    }
                }
            }
        }
        if self.rows.is_empty() {
            self.list_index = 0;
        } else {
            self.list_index = self.list_index.min(self.rows.len() - 1);
        }
        self.scroll_list();
        self.rebuild_details();
    }

    fn sort_albums(&mut self) {
        match self.sort {
            SortMode::Artist => {}
            SortMode::Album => self.albums.sort_by(|a, b| {
                a.title
                    .to_ascii_lowercase()
                    .cmp(&b.title.to_ascii_lowercase())
            }),
            SortMode::Year => self.albums.sort_by_key(|a| a.year),
            SortMode::Path => {
                let mut keyed: Vec<(String, Album)> = self
                    .albums
                    .iter()
                    .cloned()
                    .map(|album| (self.album_path(&album), album))
                    .collect();
                keyed.sort_by(|a, b| a.0.cmp(&b.0));
                self.albums =
                    keyed.into_iter().map(|(_, album)| album).collect();
            }
            SortMode::Duration => {
                self.albums.sort_by_key(|a| std::cmp::Reverse(a.duration))
            }
            SortMode::Format => {
                let mut keyed: Vec<(String, Album)> = self
                    .albums
                    .iter()
                    .cloned()
                    .map(|album| (self.album_format(&album), album))
                    .collect();
                keyed.sort_by(|a, b| a.0.cmp(&b.0));
                self.albums =
                    keyed.into_iter().map(|(_, album)| album).collect();
            }
        }
    }

    fn album_path(&self, album: &Album) -> String {
        album
            .tracks
            .first()
            .and_then(|index| self.view.get(*index))
            .map(|entry| entry.path.to_string_lossy().to_string())
            .unwrap_or_default()
    }

    fn album_format(&self, album: &Album) -> String {
        album
            .tracks
            .first()
            .and_then(|index| self.view.get(*index))
            .map(|entry| entry.format())
            .unwrap_or_default()
    }

    pub fn current_row(&self) -> Option<&Row> {
        self.rows.get(self.list_index)
    }

    pub fn album_entry(
        &self,
        album_index: usize,
        track_index: usize,
    ) -> Option<&Entry> {
        self.albums
            .get(album_index)
            .and_then(|album| album.tracks.get(track_index))
            .and_then(|index| self.view.get(*index))
    }

    pub fn current_entry(&self) -> Option<&Entry> {
        match self.current_row()? {
            Row::Track { album, track } => self.album_entry(*album, *track),
            Row::Album(album_index) => {
                let album = self.albums.get(*album_index)?;
                album.tracks.first().and_then(|i| self.view.get(*i))
            }
            Row::Artist(artist_index) => self
                .artists
                .get(*artist_index)
                .and_then(|artist| artist.albums.first())
                .and_then(|album_index| self.albums.get(*album_index))
                .and_then(|album| album.tracks.first())
                .and_then(|i| self.view.get(*i)),
        }
    }

    pub fn entries_for_row(&self, row: &Row) -> Vec<&Entry> {
        match row {
            Row::Track { album, track } => {
                self.album_entry(*album, *track).into_iter().collect()
            }
            Row::Album(index) => self
                .albums
                .get(*index)
                .map(|album| {
                    album
                        .tracks
                        .iter()
                        .filter_map(|i| self.view.get(*i))
                        .collect()
                })
                .unwrap_or_default(),
            Row::Artist(index) => self
                .artists
                .get(*index)
                .map(|artist| {
                    artist
                        .albums
                        .iter()
                        .filter_map(|album_index| self.albums.get(*album_index))
                        .flat_map(|album| album.tracks.iter())
                        .filter_map(|i| self.view.get(*i))
                        .collect()
                })
                .unwrap_or_default(),
        }
    }

    pub fn target_entries(&self) -> Vec<&Entry> {
        if self.selection.len() >= 2 {
            return self
                .entries
                .iter()
                .filter(|entry| self.selection.contains(&entry.path))
                .collect();
        }
        self.current_row()
            .map(|row| self.entries_for_row(row))
            .unwrap_or_default()
    }

    fn target_paths(&self) -> Vec<PathBuf> {
        self.target_entries()
            .into_iter()
            .map(|entry| entry.path.clone())
            .collect()
    }

    pub fn move_list(&mut self, delta: isize) {
        if self.rows.is_empty() {
            return;
        }
        let next = self.list_index as isize + delta;
        self.list_index = next.clamp(0, self.rows.len() as isize - 1) as usize;
        self.scroll_list();
        self.rebuild_details();
    }

    pub fn move_artist(&mut self, delta: isize) {
        if self.rows.is_empty() {
            return;
        }
        let direction = if delta < 0 { -1 } else { 1 };
        let mut index = self.list_index as isize + direction;
        while index >= 0 && (index as usize) < self.rows.len() {
            if matches!(self.rows[index as usize], Row::Artist(_)) {
                self.list_index = index as usize;
                self.scroll_list();
                self.rebuild_details();
                return;
            }
            index += direction;
        }
    }

    pub fn jump_top(&mut self) {
        if self.pane == Pane::List {
            self.list_index = 0;
            self.scroll_list();
            self.rebuild_details();
        } else {
            self.detail_index = 0;
            self.scroll_detail();
        }
    }

    pub fn jump_bottom(&mut self) {
        if self.pane == Pane::List {
            self.list_index = self.rows.len().saturating_sub(1);
            self.scroll_list();
            self.rebuild_details();
        } else {
            self.detail_index = self.details.len().saturating_sub(1);
            self.scroll_detail();
        }
    }

    pub fn scroll_list(&mut self) {
        let height = self.list_height();
        if height == 0 {
            self.list_top = 0;
            return;
        }
        if self.list_index < self.list_top {
            self.list_top = self.list_index;
        }
        if self.list_index >= self.list_top + height {
            self.list_top = self.list_index + 1 - height;
        }
    }

    pub fn scroll_detail(&mut self) {
        let height = self.detail_height();
        if height == 0 {
            self.detail_top = 0;
            return;
        }
        if self.detail_index < self.detail_top {
            self.detail_top = self.detail_index;
        }
        if self.detail_index >= self.detail_top + height {
            self.detail_top = self.detail_index + 1 - height;
        }
    }

    pub fn list_height(&self) -> usize {
        self.body_height().saturating_sub(2)
    }

    pub fn detail_height(&self) -> usize {
        self.body_height().saturating_sub(2)
    }

    fn body_height(&self) -> usize {
        // Header, status and keybar each take one row.
        self.height.saturating_sub(3) as usize
    }

    pub fn toggle_expand(&mut self) {
        let Some(row) = self.current_row().cloned() else {
            return;
        };
        match row {
            Row::Artist(index) => {
                if let Some(artist) = self.artists.get(index) {
                    if !self.expanded_artists.remove(&artist.key) {
                        self.expanded_artists.insert(artist.key.clone());
                    }
                }
            }
            Row::Album(index) => {
                if let Some(album) = self.albums.get(index) {
                    if !self.expanded_albums.remove(&album.key) {
                        self.expanded_albums.insert(album.key.clone());
                    }
                }
            }
            Row::Track { .. } => {}
        }
        self.rebuild();
    }

    pub fn collapse_current(&mut self) {
        let Some(row) = self.current_row().cloned() else {
            return;
        };
        match row {
            Row::Track { album, .. } => {
                if let Some(album) = self.albums.get(album) {
                    let key = album.key.clone();
                    let artist = album.artist.clone();
                    self.expanded_albums.remove(&key);
                    self.rebuild();
                    self.focus_album(&key);
                    let _ = artist;
                }
            }
            Row::Album(index) => {
                let Some(album_key) =
                    self.albums.get(index).map(|a| a.key.clone())
                else {
                    return;
                };
                let artist_key = self
                    .artists
                    .iter()
                    .find(|a| a.albums.contains(&index))
                    .map(|a| a.key.clone());
                if self.expanded_albums.remove(&album_key) {
                    self.rebuild();
                    self.focus_album(&album_key);
                } else if let Some(artist_key) = artist_key {
                    self.expanded_artists.remove(&artist_key);
                    self.rebuild();
                    self.focus_artist(&artist_key);
                }
            }
            Row::Artist(index) => {
                let Some(artist_key) =
                    self.artists.get(index).map(|a| a.key.clone())
                else {
                    return;
                };
                self.expanded_artists.remove(&artist_key);
                self.rebuild();
                self.focus_artist(&artist_key);
            }
        }
    }

    fn focus_album(&mut self, key: &str) {
        for (row_index, row) in self.rows.iter().enumerate() {
            if let Row::Album(index) = row {
                if self.albums.get(*index).map(|a| a.key.as_str()) == Some(key)
                {
                    self.list_index = row_index;
                    self.scroll_list();
                    self.rebuild_details();
                    return;
                }
            }
        }
    }

    fn focus_artist(&mut self, key: &str) {
        for (row_index, row) in self.rows.iter().enumerate() {
            if let Row::Artist(index) = row {
                if self.artists.get(*index).map(|a| a.key.as_str()) == Some(key)
                {
                    self.list_index = row_index;
                    self.scroll_list();
                    self.rebuild_details();
                    return;
                }
            }
        }
    }

    pub fn reveal_path(&mut self, path: &Path) -> bool {
        for (album_index, album) in self.albums.iter().enumerate() {
            for (track_index, entry_index) in album.tracks.iter().enumerate() {
                if self.view.get(*entry_index).map(|e| e.path.as_path())
                    == Some(path)
                {
                    for artist in &self.artists {
                        if artist.albums.contains(&album_index) {
                            self.expanded_artists.insert(artist.key.clone());
                        }
                    }
                    self.expanded_albums.insert(album.key.clone());
                    self.rebuild();
                    for (row_index, row) in self.rows.iter().enumerate() {
                        if let Row::Track { album, track } = row {
                            if *album == album_index && *track == track_index {
                                self.list_index = row_index;
                                self.scroll_list();
                                self.rebuild_details();
                                return true;
                            }
                        }
                    }
                    return false;
                }
            }
        }
        false
    }

    pub fn expand_all(&mut self) {
        for artist in &self.artists {
            self.expanded_artists.insert(artist.key.clone());
        }
        for album in &self.albums {
            self.expanded_albums.insert(album.key.clone());
        }
        self.rebuild();
        self.set_status("expanded all artists and albums", StatusKind::Info);
    }

    pub fn collapse_all(&mut self) {
        self.expanded_artists.clear();
        self.expanded_albums.clear();
        self.rebuild();
        self.set_status("collapsed all artists and albums", StatusKind::Info);
    }

    pub fn toggle_selection(&mut self) {
        let Some(row) = self.current_row().cloned() else {
            return;
        };
        let paths: Vec<PathBuf> = self
            .entries_for_row(&row)
            .into_iter()
            .map(|e| e.path.clone())
            .collect();
        if paths.is_empty() {
            return;
        }
        let all = paths.iter().all(|p| self.selection.contains(p));
        for path in paths {
            if all {
                self.selection.remove(&path);
            } else {
                self.selection.insert(path);
            }
        }
        self.rebuild_details();
    }

    pub fn select_all_visible(&mut self) {
        self.selection = self.view.iter().map(|e| e.path.clone()).collect();
        self.rebuild_details();
    }

    pub fn clear_selection(&mut self) {
        self.selection.clear();
        self.rebuild_details();
    }

    pub fn rebuild_details(&mut self) {
        let keep = self.details.get(self.detail_index).and_then(|r| r.field);
        if self.selection.len() >= 2 {
            self.details = self.bulk_details();
        } else {
            self.details = match self.current_row().cloned() {
                Some(Row::Track { album, track }) => self
                    .album_entry(album, track)
                    .map(|entry| self.track_details(entry))
                    .unwrap_or_default(),
                Some(Row::Album(index)) => self
                    .albums
                    .get(index)
                    .map(|album| self.album_details(album))
                    .unwrap_or_default(),
                Some(Row::Artist(index)) => self
                    .artists
                    .get(index)
                    .map(|artist| self.artist_details(artist))
                    .unwrap_or_default(),
                None => Vec::new(),
            };
        }
        if let Some(field) = keep {
            if let Some(index) =
                self.details.iter().position(|r| r.field == Some(field))
            {
                self.detail_index = index;
                self.scroll_detail();
                return;
            }
        }
        self.detail_index =
            self.detail_index.min(self.details.len().saturating_sub(1));
        self.scroll_detail();
    }

    fn artist_details(&self, artist: &ArtistGroup) -> Vec<DetailRow> {
        let entries: Vec<&Entry> = artist
            .albums
            .iter()
            .filter_map(|index| self.albums.get(*index))
            .flat_map(|album| album.tracks.iter())
            .filter_map(|index| self.view.get(*index))
            .collect();
        let duration: Duration = entries.iter().map(|e| e.duration()).sum();
        let covers =
            entries.iter().filter(|e| e.properties.has_cover()).count();
        vec![
            detail("Artist", artist.name.clone(), true),
            detail("Albums", artist.albums.len().to_string(), true),
            detail("Tracks", entries.len().to_string(), true),
            detail("Duration", library::duration_label(duration), true),
            detail(
                "Cover",
                if covers == 0 {
                    "none".into()
                } else {
                    format!("{covers}/{} tracks", entries.len())
                },
                true,
            ),
        ]
    }

    fn album_details(&self, album: &Album) -> Vec<DetailRow> {
        let entries: Vec<&Entry> = album
            .tracks
            .iter()
            .filter_map(|i| self.view.get(*i))
            .collect();
        let fields = [
            Field::Album,
            Field::AlbumArtist,
            Field::Year,
            Field::Genre,
            Field::Publisher,
            Field::Compilation,
        ];
        let mut rows: Vec<DetailRow> = fields
            .iter()
            .map(|field| {
                let (value, mixed) = aggregate(&entries, *field);
                DetailRow {
                    field: Some(*field),
                    label: field.label().into(),
                    value,
                    album: true,
                    mixed,
                    read_only: false,
                }
            })
            .collect();
        rows.push(detail("Tracks", entries.len().to_string(), true));
        rows.push(detail(
            "Duration",
            library::duration_label(album.duration),
            true,
        ));
        let covers =
            entries.iter().filter(|e| e.properties.has_cover()).count();
        rows.push(detail(
            "Cover",
            if covers == 0 {
                "none".into()
            } else if covers == entries.len() {
                format!("yes ({} tracks)", covers)
            } else {
                format!("{covers}/{} tracks", entries.len())
            },
            true,
        ));
        rows
    }

    fn track_details(&self, entry: &Entry) -> Vec<DetailRow> {
        let fields = [
            Field::Title,
            Field::Artist,
            Field::Track,
            Field::TrackTotal,
            Field::Disc,
            Field::DiscTotal,
            Field::Composer,
            Field::Lyricist,
            Field::Bpm,
            Field::Key,
            Field::Isrc,
            Field::Comment,
            Field::Lyrics,
            Field::Conductor,
            Field::CatalogNumber,
            Field::Barcode,
            Field::Media,
            Field::Grouping,
            Field::Subtitle,
            Field::OriginalDate,
            Field::Copyright,
            Field::EncodedBy,
            Field::Album,
            Field::AlbumArtist,
            Field::Genre,
            Field::Year,
            Field::Publisher,
        ];
        let rows: Vec<DetailRow> = fields
            .iter()
            .map(|field| DetailRow {
                field: Some(*field),
                label: field.label().into(),
                value: entry.metadata.get(*field),
                album: false,
                mixed: false,
                read_only: false,
            })
            .collect();
        rows
    }

    fn bulk_details(&self) -> Vec<DetailRow> {
        let entries: Vec<&Entry> = self
            .entries
            .iter()
            .filter(|entry| self.selection.contains(&entry.path))
            .collect();
        let fields = [
            Field::Album,
            Field::AlbumArtist,
            Field::Genre,
            Field::Year,
            Field::Artist,
            Field::Composer,
            Field::Conductor,
            Field::Lyricist,
            Field::TrackTotal,
            Field::Disc,
            Field::DiscTotal,
            Field::Compilation,
            Field::Comment,
            Field::Publisher,
            Field::CatalogNumber,
            Field::Barcode,
            Field::Media,
            Field::Grouping,
            Field::Subtitle,
            Field::OriginalDate,
            Field::Copyright,
            Field::EncodedBy,
        ];
        fields
            .iter()
            .map(|field| {
                let (value, mixed) = aggregate(&entries, *field);
                DetailRow {
                    field: Some(*field),
                    label: field.label().into(),
                    value,
                    album: false,
                    mixed,
                    read_only: false,
                }
            })
            .collect()
    }

    pub fn current_detail(&self) -> Option<&DetailRow> {
        self.details.get(self.detail_index)
    }

    pub fn start_edit(&mut self) {
        let Some(row) = self.current_detail().cloned() else {
            return;
        };
        if row.read_only {
            self.set_status(
                format!("{} is read-only", row.label),
                StatusKind::Info,
            );
            return;
        }
        let paths = self
            .target_entries()
            .into_iter()
            .map(|entry| entry.path.clone())
            .collect::<Vec<_>>();
        if paths.is_empty() {
            return;
        }
        self.input.set(if row.mixed {
            String::new()
        } else {
            row.value.clone()
        });
        self.edit_paths = paths;
        self.edit_row = Some(row);
        self.mode = Mode::Edit;
    }

    pub fn save_edit(&mut self) {
        let Some(row) = self.edit_row.clone() else {
            return;
        };
        let Some(field) = row.field else {
            return;
        };
        let value = self.input.value.clone();
        let mut failed = 0;
        let mut updated = 0;
        let paths = self.edit_paths.clone();
        for path in &paths {
            let result = (|| -> Result<()> {
                let mut data = tags::read(path)?;
                data.metadata.set(field, &value)?;
                tags::write(path, &data.metadata, CoverAction::Keep)?;
                Ok(())
            })();
            match result {
                Ok(()) => {
                    updated += 1;
                    if let Some(entry) =
                        self.entries.iter_mut().find(|e| &e.path == path)
                    {
                        *entry = library::load_one(path);
                    }
                }
                Err(err) => {
                    failed += 1;
                    self.set_status(
                        format!("{}: {err:#}", path.display()),
                        StatusKind::Error,
                    );
                }
            }
        }
        self.mode = Mode::Browse;
        self.rebuild();
        if failed == 0 {
            self.set_status(
                format!("saved {updated} track(s)"),
                StatusKind::Ok,
            );
        }
    }

    pub fn clear_current_field(&mut self) {
        let Some(row) = self.current_detail().cloned() else {
            return;
        };
        if row.read_only {
            return;
        }
        let Some(field) = row.field else {
            return;
        };
        if row.album && self.selection.len() < 2 {
            self.set_status(
                "use the CLI to clear an album-wide field",
                StatusKind::Info,
            );
            return;
        }
        let paths = self.target_paths();
        if paths.is_empty() {
            return;
        }
        let mut updated = 0;
        for path in &paths {
            if let Ok(mut data) = tags::read(path) {
                if data.metadata.set(field, "").is_ok()
                    && tags::write(path, &data.metadata, CoverAction::Keep)
                        .is_ok()
                {
                    updated += 1;
                    if let Some(entry) =
                        self.entries.iter_mut().find(|e| &e.path == path)
                    {
                        *entry = library::load_one(path);
                    }
                }
            }
        }
        self.rebuild();
        self.set_status(
            format!("cleared {} on {updated} track(s)", row.label),
            StatusKind::Ok,
        );
    }

    pub fn toggle_compilation(&mut self) {
        let entries = self.target_entries();
        if entries.is_empty() {
            return;
        }
        let target = !entries[0].metadata.compilation;
        let paths: Vec<PathBuf> =
            entries.iter().map(|e| e.path.clone()).collect();
        let mut updated = 0;
        for path in &paths {
            if let Ok(mut data) = tags::read(path) {
                data.metadata.compilation = target;
                if tags::write(path, &data.metadata, CoverAction::Keep).is_ok()
                {
                    updated += 1;
                    if let Some(entry) =
                        self.entries.iter_mut().find(|e| &e.path == path)
                    {
                        *entry = library::load_one(path);
                    }
                }
            }
        }
        self.rebuild();
        self.set_status(
            format!(
                "compilation {} for {updated} track(s)",
                if target { "yes" } else { "no" }
            ),
            StatusKind::Ok,
        );
    }

    pub fn guess_track_numbers(&mut self) {
        let paths = self.target_paths();
        if paths.is_empty() {
            return;
        }
        let mut guessed = 0;
        let mut unmatched = 0;
        for path in &paths {
            let Some(guess) = library::guess_track_number(path) else {
                unmatched += 1;
                continue;
            };
            if let Ok(mut data) = tags::read(path) {
                data.metadata.track = guess.track;
                if guess.total > 0 {
                    data.metadata.track_total = guess.total;
                }
                if guess.disc > 0 {
                    data.metadata.disc = guess.disc;
                }
                if tags::write(path, &data.metadata, CoverAction::Keep).is_ok()
                {
                    guessed += 1;
                    if let Some(entry) =
                        self.entries.iter_mut().find(|e| &e.path == path)
                    {
                        *entry = library::load_one(path);
                    }
                }
            }
        }
        self.rebuild();
        self.set_status(
            format!(
                "guessed {guessed} track number(s), {unmatched} without a guess"
            ),
            StatusKind::Ok,
        );
    }

    pub fn jump_missing(&mut self) {
        if self.required.is_empty() {
            self.set_status("no required_tags configured", StatusKind::Info);
            return;
        }
        let mut refs: Vec<(usize, usize, PathBuf)> = Vec::new();
        for (artist_index, artist) in self.artists.iter().enumerate() {
            for album_index in &artist.albums {
                if let Some(album) = self.albums.get(*album_index) {
                    for entry_index in &album.tracks {
                        if let Some(entry) = self.view.get(*entry_index) {
                            refs.push((
                                artist_index,
                                *album_index,
                                entry.path.clone(),
                            ));
                        }
                    }
                }
            }
        }
        if refs.is_empty() {
            return;
        }
        let current = self.current_entry().map(|e| e.path.clone());
        let start = current
            .as_ref()
            .and_then(|path| refs.iter().position(|(_, _, p)| p == path))
            .map(|i| i + 1)
            .unwrap_or(0);
        for offset in 0..refs.len() {
            let (artist_index, album_index, path) =
                refs[(start + offset) % refs.len()].clone();
            let Some(entry) = self.view.iter().find(|e| e.path == path) else {
                continue;
            };
            let missing = library::missing_requirements(entry, &self.required);
            if missing.is_empty() {
                continue;
            }
            if let Some(artist) = self.artists.get(artist_index) {
                let key = artist.key.clone();
                self.expanded_artists.insert(key);
            }
            if let Some(album) = self.albums.get(album_index) {
                let key = album.key.clone();
                self.expanded_albums.insert(key);
            }
            self.rebuild();
            self.reveal_path(&path);
            self.set_status(
                format!(
                    "missing {} · {}",
                    missing
                        .iter()
                        .map(|m| m.label())
                        .collect::<Vec<_>>()
                        .join(", "),
                    path.file_name()
                        .map(|s| s.to_string_lossy().to_string())
                        .unwrap_or_default()
                ),
                StatusKind::Error,
            );
            return;
        }
        self.set_status(
            "all visible tracks have the required tags",
            StatusKind::Ok,
        );
    }

    pub fn rename(&mut self, move_files: bool) {
        let entries = self.target_entries();
        if entries.is_empty() {
            return;
        }
        let pattern = if self.cfg.rename_format.trim().is_empty() {
            if move_files {
                "%A/%L/%n - %t".to_string()
            } else {
                "%n - %t".to_string()
            }
        } else {
            self.cfg.rename_format.clone()
        };
        let root = if move_files {
            None
        } else if pattern.contains('/') {
            if self.cfg.library.is_empty() {
                Some(self.dir.clone())
            } else if entries
                .iter()
                .all(|e| organize::under(&e.path, Path::new(&self.cfg.library)))
            {
                Some(PathBuf::from(&self.cfg.library))
            } else {
                Some(self.dir.clone())
            }
        } else {
            None
        };
        let owned: Vec<Entry> = entries.into_iter().cloned().collect();
        let report = organize::rename(
            &owned,
            &organize::Options {
                pattern,
                root,
                library: if self.cfg.library.is_empty() {
                    None
                } else {
                    Some(PathBuf::from(&self.cfg.library))
                },
                slugify: self.cfg.slugify_rename(),
                move_files,
                dry_run: false,
            },
        );
        self.selection.clear();
        self.reload();
        if report.errors.is_empty() {
            self.set_status(
                format!(
                    "{} {} track(s)",
                    if move_files { "moved" } else { "renamed" },
                    report.results.len()
                ),
                StatusKind::Ok,
            );
        } else {
            self.set_status(report.errors.join("; "), StatusKind::Error);
        }
    }

    pub fn ask_delete(&mut self) {
        let entries = self.target_entries();
        if entries.is_empty() {
            return;
        }
        self.delete_paths = entries.iter().map(|e| e.path.clone()).collect();
        self.mode = Mode::ConfirmDelete;
    }

    pub fn confirm_delete(&mut self) {
        let mut removed = 0;
        for path in &self.delete_paths {
            if std::fs::remove_file(path).is_ok() {
                removed += 1;
                self.selection.remove(path);
            }
        }
        self.delete_paths.clear();
        self.mode = Mode::Browse;
        self.reload();
        self.set_status(format!("deleted {removed} file(s)"), StatusKind::Ok);
    }

    pub fn open_cover(&mut self) {
        if matches!(self.current_row(), Some(Row::Artist(_))) {
            self.set_status(
                "select an album or track to manage cover art",
                StatusKind::Info,
            );
            return;
        }
        if self.target_entries().is_empty() {
            return;
        }
        self.mode = Mode::Cover;
        self.cover_editing = false;
    }

    pub fn toggle_cover_panel(&mut self) {
        self.show_cover = !self.show_cover;
        self.cover_key.clear();
        self.cover_loading = false;
        if !self.show_cover {
            self.cover_lines.clear();
            self.cover_note.clear();
        }
        self.set_status(
            if self.show_cover {
                "album cover panel on"
            } else {
                "album cover panel off"
            },
            StatusKind::Info,
        );
    }

    /// True while artwork for the current row is being decoded, so the event
    /// loop can wake up sooner and display it promptly.
    pub fn cover_loading(&self) -> bool {
        self.cover_loading
    }

    /// Ask the background worker for artwork at the pane's current size.
    /// Cached results are shown immediately; cache misses render off-thread.
    pub fn ensure_cover(&mut self, width: u16, height: u16) {
        if !self.show_cover || width == 0 || height == 0 {
            self.cover_lines.clear();
            self.cover_note.clear();
            self.cover_key.clear();
            self.cover_loading = false;
            return;
        }
        let Some((source, entry_path)) =
            self.current_entry().and_then(|entry| {
                cover_source(entry, &self.cfg.cover_file)
                    .map(|source| (source, entry.path.clone()))
            })
        else {
            self.cover_lines.clear();
            self.cover_note = "no cover art".into();
            self.cover_key.clear();
            self.cover_loading = false;
            return;
        };
        let key = format!("{source}|{width}x{height}");
        if key == self.cover_key {
            return;
        }
        self.cover_key = key.clone();
        match self.cover_cache.get(&key) {
            Some(CachedCover::Lines(lines)) => {
                self.cover_lines = lines.clone();
                self.cover_note.clear();
                self.cover_loading = false;
            }
            Some(CachedCover::Missing) => {
                self.cover_lines.clear();
                self.cover_note = "no cover art".into();
                self.cover_loading = false;
            }
            None => {
                self.cover_lines.clear();
                self.cover_note = "loading cover…".into();
                self.cover_loading = true;
                let _ = self.cover_jobs.send(CoverJob {
                    key,
                    path: entry_path,
                    cover_file: self.cfg.cover_file.clone(),
                    width,
                    height,
                });
            }
        }
    }

    /// Drain finished background artwork and publish it when it matches the
    /// current selection. Older results are still cached for later.
    pub fn poll_cover(&mut self) {
        while let Ok(result) = self.cover_results.try_recv() {
            let current = result.key == self.cover_key;
            let cached = if result.lines.is_empty() {
                CachedCover::Missing
            } else {
                CachedCover::Lines(result.lines.clone())
            };
            if self.cover_cache.len() >= 64 {
                self.cover_cache.clear();
            }
            self.cover_cache.insert(result.key, cached);
            if current {
                self.cover_loading = false;
                if result.lines.is_empty() {
                    self.cover_lines.clear();
                    self.cover_note = "no cover art".into();
                } else {
                    self.cover_lines = result.lines;
                    self.cover_note.clear();
                }
            }
        }
    }

    pub fn download_cover(&mut self) {
        let Some(entry) = self.current_entry() else {
            return;
        };
        let dir = entry
            .path
            .parent()
            .unwrap_or_else(|| Path::new("."))
            .to_path_buf();
        let artist = if entry.metadata.album_artist.trim().is_empty() {
            entry.artist()
        } else {
            entry.metadata.album_artist.clone()
        };
        let album = if entry.metadata.album.trim().is_empty() {
            dir.file_name()
                .map(|s| s.to_string_lossy().to_string())
                .unwrap_or_default()
        } else {
            entry.metadata.album.clone()
        };
        match cover::download_album_cover(
            &self.client,
            &dir,
            &artist,
            &album,
            &self.cfg.cover_file,
        ) {
            Ok(result) => {
                self.mode = Mode::Browse;
                self.reload();
                self.set_status(
                    format!("wrote {}", result.path.display()),
                    StatusKind::Ok,
                );
            }
            Err(err) => self.set_status(format!("{err:#}"), StatusKind::Error),
        }
    }

    pub fn set_cover_from_file(&mut self, path: &Path) {
        let data = match std::fs::read(path) {
            Ok(data) => data,
            Err(err) => {
                self.set_status(format!("{err:#}"), StatusKind::Error);
                return;
            }
        };
        let paths = self.target_paths();
        let mut updated = 0;
        for target in &paths {
            let result = (|| -> Result<()> {
                let meta = tags::read(target)?.metadata;
                let action = if self.cfg.embed_cover() {
                    CoverAction::Set(data.clone())
                } else {
                    CoverAction::Keep
                };
                tags::write(target, &meta, action)?;
                if self.cfg.folder_cover() {
                    let dir = target.parent().unwrap_or_else(|| Path::new("."));
                    tags::write_folder_cover(dir, &self.cfg.cover_file, &data)?;
                }
                Ok(())
            })();
            if result.is_ok() {
                updated += 1;
                if let Some(entry) =
                    self.entries.iter_mut().find(|e| &e.path == target)
                {
                    *entry = library::load_one(target);
                }
            }
        }
        self.mode = Mode::Browse;
        self.rebuild();
        self.set_status(
            format!("set cover on {updated} track(s)"),
            StatusKind::Ok,
        );
    }

    pub fn remove_cover(&mut self) {
        let paths = self.target_paths();
        let mut updated = 0;
        for target in &paths {
            let result = (|| -> Result<()> {
                let meta = tags::read(target)?.metadata;
                let action = if self.cfg.embed_cover() {
                    CoverAction::Remove
                } else {
                    CoverAction::Keep
                };
                tags::write(target, &meta, action)?;
                if self.cfg.folder_cover() {
                    let dir = target.parent().unwrap_or_else(|| Path::new("."));
                    let _ =
                        tags::remove_folder_cover(dir, &self.cfg.cover_file);
                }
                Ok(())
            })();
            if result.is_ok() {
                updated += 1;
                if let Some(entry) =
                    self.entries.iter_mut().find(|e| &e.path == target)
                {
                    *entry = library::load_one(target);
                }
            }
        }
        self.mode = Mode::Browse;
        self.rebuild();
        self.set_status(
            format!("removed cover from {updated} track(s)"),
            StatusKind::Ok,
        );
    }

    pub fn extract_cover(&mut self) {
        let Some(entry) = self.current_entry() else {
            return;
        };
        match tags::read_artwork(&entry.path, &self.cfg.cover_file) {
            Ok((data, mime)) => {
                let ext = if mime.contains("png") { "png" } else { "jpg" };
                let dir = entry.path.parent().unwrap_or_else(|| Path::new("."));
                let mut target = dir.join(format!("cover.{ext}"));
                let mut index = 2;
                while target.exists() {
                    target = dir.join(format!("cover-{index}.{ext}"));
                    index += 1;
                }
                match std::fs::write(&target, data) {
                    Ok(()) => {
                        self.mode = Mode::Browse;
                        self.set_status(
                            format!("extracted {}", target.display()),
                            StatusKind::Ok,
                        );
                    }
                    Err(err) => {
                        self.set_status(format!("{err:#}"), StatusKind::Error)
                    }
                }
            }
            Err(err) => self.set_status(format!("{err:#}"), StatusKind::Error),
        }
    }

    pub fn start_lookup(&mut self) {
        let Some(entry) = self.current_entry() else {
            return;
        };
        if matches!(self.current_row(), Some(Row::Artist(_))) {
            self.set_status(
                "select an album for MusicBrainz lookup",
                StatusKind::Info,
            );
            return;
        }
        let dir = entry
            .path
            .parent()
            .unwrap_or_else(|| Path::new("."))
            .to_path_buf();
        let artist = if entry.metadata.album_artist.trim().is_empty() {
            entry.artist()
        } else {
            entry.metadata.album_artist.clone()
        };
        let album = if entry.metadata.album.trim().is_empty() {
            dir.file_name()
                .map(|s| s.to_string_lossy().to_string())
                .unwrap_or_default()
        } else {
            entry.metadata.album.clone()
        };
        self.status = format!("searching MusicBrainz for {artist} — {album}…");
        self.status_kind = StatusKind::Info;
        match self.client.search_releases(&artist, &album, 8) {
            Ok(releases) => {
                if releases.is_empty() {
                    self.set_status("no releases found", StatusKind::Error);
                    return;
                }
                self.lookup_releases = releases;
                self.lookup_index = 0;
                self.lookup_album = Some((dir, artist, album));
                self.mode = Mode::Lookup;
            }
            Err(err) => self.set_status(format!("{err:#}"), StatusKind::Error),
        }
    }

    pub fn apply_lookup(&mut self) {
        let Some((dir, artist, _album)) = self.lookup_album.clone() else {
            return;
        };
        let Some(release) =
            self.lookup_releases.get(self.lookup_index).cloned()
        else {
            return;
        };
        let detail = match self.client.get_release(&release.id) {
            Ok(detail) => detail,
            Err(err) => {
                self.mode = Mode::Browse;
                self.set_status(format!("{err:#}"), StatusKind::Error);
                return;
            }
        };
        let entries: Vec<Entry> = self
            .albums
            .iter()
            .filter(|album| {
                album.artist.eq_ignore_ascii_case(&artist)
                    && album.title.eq_ignore_ascii_case(&_album)
            })
            .flat_map(|album| album.tracks.iter())
            .filter_map(|i| self.view.get(*i).cloned())
            .collect();
        let plan = cover::plan_release_tracks(&entries, &detail);
        let cover_data = self.client.download_cover(&release.id, 500).ok();
        let label = detail
            .labels
            .first()
            .map(|l| l.label_name().to_string())
            .unwrap_or_default();
        let catalog = detail
            .labels
            .first()
            .map(|l| l.catalog_number.clone())
            .unwrap_or_default();
        let album_title = if detail.title().is_empty() {
            release.title.clone()
        } else {
            detail.title().to_string()
        };
        let album_artist = if detail.artist_name().is_empty() {
            artist
        } else {
            detail.artist_name()
        };
        let year = parse_year(detail.date());
        let mut updated = 0;
        for entry in &entries {
            let assignment = plan.get(&entry.path);
            let result = (|| -> Result<()> {
                let mut data = tags::read(&entry.path)?;
                data.metadata.album = album_title.clone();
                data.metadata.album_artist = album_artist.clone();
                if year > 0 {
                    data.metadata.year = year;
                }
                if !label.is_empty() {
                    data.metadata.publisher = label.clone();
                }
                if !catalog.is_empty() {
                    data.metadata.catalog_number = catalog.clone();
                }
                if let Some(assignment) = assignment {
                    if !assignment.title.is_empty() {
                        data.metadata.title = assignment.title.clone();
                    }
                    data.metadata.set(
                        Field::Track,
                        &cover::track_number_value(
                            assignment.track,
                            assignment.track_total,
                        ),
                    )?;
                    if assignment.set_disc {
                        data.metadata.set(
                            Field::Disc,
                            &cover::track_number_value(
                                assignment.disc,
                                assignment.disc_total,
                            ),
                        )?;
                    }
                }
                let cover_action = if let Some((data, _)) = &cover_data {
                    if self.cfg.embed_cover() {
                        CoverAction::Set(data.clone())
                    } else {
                        CoverAction::Keep
                    }
                } else {
                    CoverAction::Keep
                };
                tags::write(&entry.path, &data.metadata, cover_action)?;
                if self.cfg.folder_cover() {
                    if let Some((data, _)) = &cover_data {
                        tags::write_folder_cover(
                            &dir,
                            &self.cfg.cover_file,
                            data,
                        )?;
                    }
                }
                Ok(())
            })();
            if result.is_ok() {
                updated += 1;
                if let Some(entry_mut) =
                    self.entries.iter_mut().find(|e| e.path == entry.path)
                {
                    *entry_mut = library::load_one(&entry.path);
                }
            }
        }
        self.mode = Mode::Browse;
        self.rebuild();
        self.set_status(
            format!("applied release to {updated} track(s)"),
            StatusKind::Ok,
        );
    }

    pub fn set_status(&mut self, text: impl Into<String>, kind: StatusKind) {
        self.status = text.into();
        self.status_kind = kind;
    }

    pub fn open_theme_dialog(&mut self) {
        self.theme_choices = theme::available_themes()
            .into_iter()
            .map(|(id, name)| {
                let preview = UiTheme::load(&id);
                ThemeChoice {
                    id,
                    name,
                    accent: preview.color("accent.primary"),
                    background: preview.color("bg.base"),
                }
            })
            .collect();
        self.theme_index = self
            .theme_choices
            .iter()
            .position(|choice| choice.id == self.theme_name)
            .unwrap_or(0);
        self.theme_original = self.theme_name.clone();
        self.mode = Mode::Theme;
    }

    fn preview_theme(&mut self, index: usize) {
        let Some(choice) = self.theme_choices.get(index) else {
            return;
        };
        self.theme_index = index;
        self.theme = UiTheme::load(&choice.id);
        self.theme_name = self.theme.name.clone();
    }

    pub fn move_theme(&mut self, delta: isize) {
        if self.theme_choices.is_empty() {
            return;
        }
        let max = self.theme_choices.len().saturating_sub(1) as isize;
        let next = (self.theme_index as isize + delta).clamp(0, max) as usize;
        self.preview_theme(next);
    }

    pub fn jump_theme(&mut self, to_end: bool) {
        if self.theme_choices.is_empty() {
            return;
        }
        let index = if to_end {
            self.theme_choices.len() - 1
        } else {
            0
        };
        self.preview_theme(index);
    }

    pub fn apply_theme(&mut self) {
        self.mode = Mode::Browse;
        self.set_status(
            format!("theme: {}", self.theme.display_name),
            StatusKind::Ok,
        );
    }

    pub fn cancel_theme_dialog(&mut self) {
        self.theme = UiTheme::load(&self.theme_original);
        self.theme_name = self.theme.name.clone();
        self.mode = Mode::Browse;
        self.set_status(
            format!("theme unchanged: {}", self.theme.display_name),
            StatusKind::Info,
        );
    }

    pub fn cycle_sort(&mut self) {
        self.sort = self.sort.next();
        self.rebuild();
        self.set_status(
            format!("sorted by {}", self.sort.label()),
            StatusKind::Info,
        );
    }

    pub fn play(&mut self, album: bool) {
        if self.cfg.player.trim().is_empty() {
            self.set_status(
                "set player in the config to enable playback",
                StatusKind::Error,
            );
            return;
        }
        let entries: Vec<&Entry> = if album {
            match self.current_row() {
                Some(Row::Track { album, .. }) => self
                    .albums
                    .get(*album)
                    .map(|album| {
                        album
                            .tracks
                            .iter()
                            .filter_map(|i| self.view.get(*i))
                            .collect()
                    })
                    .unwrap_or_default(),
                Some(row) => self.entries_for_row(row),
                None => Vec::new(),
            }
        } else {
            self.current_entry().into_iter().collect()
        };
        if entries.is_empty() {
            return;
        }
        let Some(parts) = shlex::split(&self.cfg.player) else {
            self.set_status("invalid player command", StatusKind::Error);
            return;
        };
        let Some((program, args)) = parts.split_first() else {
            return;
        };
        let mut command = Command::new(program);
        command.args(args);
        for entry in &entries {
            command.arg(&entry.path);
        }
        match command.spawn() {
            Ok(_) => self.set_status(
                if entries.len() > 1 {
                    format!("playing {} tracks", entries.len())
                } else {
                    format!("playing {}", entries[0].path.display())
                },
                StatusKind::Ok,
            ),
            Err(err) => {
                self.set_status(format!("play: {err}"), StatusKind::Error)
            }
        }
    }

    pub fn open_folder(&mut self) {
        let Some(entry) = self.current_entry() else {
            return;
        };
        let dir = entry.path.parent().unwrap_or_else(|| Path::new("."));
        if let Err(err) = Command::new("xdg-open").arg(dir).spawn() {
            self.set_status(format!("open folder: {err}"), StatusKind::Error);
        }
    }

    pub fn open_search_url(&mut self) {
        let Some(target) = self.search_url() else {
            self.set_status(
                "set search_url in the config to enable web search",
                StatusKind::Error,
            );
            return;
        };
        if let Err(err) = Command::new("xdg-open").arg(target).spawn() {
            self.set_status(format!("open search: {err}"), StatusKind::Error);
        }
    }

    pub fn search_url(&self) -> Option<String> {
        if self.cfg.search_url.trim().is_empty() {
            return None;
        }
        let entry = self.current_entry()?;
        let mut meta = entry.metadata.clone();
        if !matches!(self.current_row(), Some(Row::Track { .. })) {
            meta.title.clear();
        }
        Some(expand_search_url(&self.cfg.search_url, &meta, entry))
    }

    pub fn handle_key(&mut self, key: KeyEvent) {
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        match self.mode {
            Mode::Help => {
                match key.code {
                    KeyCode::Char('q') | KeyCode::Esc => {
                        self.mode = Mode::Browse
                    }
                    KeyCode::Char('c') if ctrl => self.quit = true,
                    KeyCode::Up | KeyCode::Char('k') => {
                        self.help_top = self.help_top.saturating_sub(1)
                    }
                    KeyCode::Down | KeyCode::Char('j') => self.help_top += 1,
                    KeyCode::PageUp => {
                        self.help_top = self.help_top.saturating_sub(8)
                    }
                    KeyCode::PageDown => self.help_top += 8,
                    KeyCode::Home | KeyCode::Char('g') => self.help_top = 0,
                    KeyCode::End | KeyCode::Char('G') => {
                        self.help_top = usize::MAX / 2
                    }
                    KeyCode::Char('?') => self.mode = Mode::Browse,
                    _ => {}
                }
                return;
            }
            Mode::Theme => {
                match key.code {
                    KeyCode::Esc | KeyCode::Char('q') => {
                        self.cancel_theme_dialog()
                    }
                    KeyCode::Enter => self.apply_theme(),
                    KeyCode::Char('c') if ctrl => self.quit = true,
                    KeyCode::Up | KeyCode::Char('k') => self.move_theme(-1),
                    KeyCode::Down | KeyCode::Char('j') => self.move_theme(1),
                    KeyCode::PageUp => self.move_theme(-10),
                    KeyCode::PageDown => self.move_theme(10),
                    KeyCode::Home | KeyCode::Char('g') => {
                        self.jump_theme(false)
                    }
                    KeyCode::End | KeyCode::Char('G') => self.jump_theme(true),
                    _ => {}
                }
                return;
            }
            Mode::ConfirmDelete => {
                match key.code {
                    KeyCode::Char('y') | KeyCode::Enter => {
                        self.confirm_delete()
                    }
                    KeyCode::Char('n') | KeyCode::Esc => {
                        self.mode = Mode::Browse
                    }
                    _ => {}
                }
                return;
            }
            Mode::Cover => {
                self.handle_cover_key(key);
                return;
            }
            Mode::Lookup => {
                match key.code {
                    KeyCode::Esc | KeyCode::Char('q') => {
                        self.mode = Mode::Browse
                    }
                    KeyCode::Up | KeyCode::Char('k') => {
                        self.lookup_index = self.lookup_index.saturating_sub(1)
                    }
                    KeyCode::Down | KeyCode::Char('j') => {
                        if self.lookup_index + 1 < self.lookup_releases.len() {
                            self.lookup_index += 1;
                        }
                    }
                    KeyCode::Enter => self.apply_lookup(),
                    _ => {}
                }
                return;
            }
            Mode::Search => {
                match key.code {
                    KeyCode::Esc => {
                        self.filter.clear();
                        self.mode = Mode::Browse;
                        self.rebuild();
                    }
                    KeyCode::Enter => {
                        self.mode = Mode::Browse;
                        self.rebuild();
                    }
                    KeyCode::Char('c') if ctrl => self.quit = true,
                    KeyCode::Char('w') if ctrl => {
                        self.input.delete_word();
                        self.filter = self.input.value.clone();
                        self.rebuild();
                    }
                    KeyCode::Backspace => {
                        self.input.backspace();
                        self.filter = self.input.value.clone();
                        self.rebuild();
                    }
                    KeyCode::Delete => {
                        self.input.delete();
                        self.filter = self.input.value.clone();
                        self.rebuild();
                    }
                    KeyCode::Left => self.input.left(),
                    KeyCode::Right => self.input.right(),
                    KeyCode::Home => self.input.home(),
                    KeyCode::End => self.input.end(),
                    KeyCode::Char(ch) => {
                        self.input.insert(ch);
                        self.filter = self.input.value.clone();
                        self.rebuild();
                    }
                    _ => {}
                }
                return;
            }
            Mode::Edit => {
                match key.code {
                    KeyCode::Esc => self.mode = Mode::Browse,
                    KeyCode::Enter => self.save_edit(),
                    KeyCode::Char('c') if ctrl => self.quit = true,
                    KeyCode::Char('w') if ctrl => self.input.delete_word(),
                    KeyCode::Backspace => self.input.backspace(),
                    KeyCode::Delete => self.input.delete(),
                    KeyCode::Left => self.input.left(),
                    KeyCode::Right => self.input.right(),
                    KeyCode::Home => self.input.home(),
                    KeyCode::End => self.input.end(),
                    KeyCode::Char(ch) => self.input.insert(ch),
                    _ => {}
                }
                return;
            }
            Mode::Browse => {}
        }

        if ctrl {
            match key.code {
                KeyCode::Char('c') => self.quit = true,
                KeyCode::Char('d') => {
                    if self.pane == Pane::List {
                        self.move_list(10);
                    } else {
                        self.detail_index = (self.detail_index + 10)
                            .min(self.details.len().saturating_sub(1));
                        self.scroll_detail();
                    }
                }
                KeyCode::Char('u') => {
                    if self.pane == Pane::List {
                        self.move_list(-10);
                    } else {
                        self.detail_index =
                            self.detail_index.saturating_sub(10);
                        self.scroll_detail();
                    }
                }
                KeyCode::Char('a') => self.select_all_visible(),
                KeyCode::Char('r') => self.reload(),
                KeyCode::Char('o') => self.open_folder(),
                KeyCode::Char('k') => self.open_search_url(),
                KeyCode::Char('t') => self.open_theme_dialog(),
                _ => {}
            }
            return;
        }

        match key.code {
            KeyCode::Char('q') => self.quit = true,
            KeyCode::Char('?') => {
                self.mode = Mode::Help;
                self.help_top = 0;
            }
            KeyCode::Tab | KeyCode::BackTab => {
                self.pane = if self.pane == Pane::List {
                    Pane::Detail
                } else {
                    Pane::List
                };
            }
            KeyCode::Esc => {
                if !self.selection.is_empty() {
                    self.clear_selection();
                } else if !self.filter.is_empty() {
                    self.filter.clear();
                    self.rebuild();
                }
            }
            KeyCode::Up | KeyCode::Char('k') => {
                if self.pane == Pane::List {
                    self.move_list(-1);
                } else {
                    self.detail_index = self.detail_index.saturating_sub(1);
                    self.scroll_detail();
                }
            }
            KeyCode::Down | KeyCode::Char('j') => {
                if self.pane == Pane::List {
                    self.move_list(1);
                } else {
                    self.detail_index = (self.detail_index + 1)
                        .min(self.details.len().saturating_sub(1));
                    self.scroll_detail();
                }
            }
            KeyCode::PageDown => {
                if self.pane == Pane::List {
                    self.move_list(self.list_height() as isize);
                } else {
                    self.detail_index = (self.detail_index
                        + self.detail_height())
                    .min(self.details.len().saturating_sub(1));
                    self.scroll_detail();
                }
            }
            KeyCode::PageUp => {
                if self.pane == Pane::List {
                    self.move_list(-(self.list_height() as isize));
                } else {
                    self.detail_index =
                        self.detail_index.saturating_sub(self.detail_height());
                    self.scroll_detail();
                }
            }
            KeyCode::Home | KeyCode::Char('g') => self.jump_top(),
            KeyCode::End | KeyCode::Char('G') => self.jump_bottom(),
            KeyCode::Char('J') => self.move_artist(1),
            KeyCode::Char('K') => self.move_artist(-1),
            KeyCode::Right | KeyCode::Char('l') => {
                if self.pane == Pane::List {
                    if matches!(self.current_row(), Some(Row::Track { .. })) {
                        self.pane = Pane::Detail;
                        self.detail_index = 0;
                        self.scroll_detail();
                    } else {
                        self.toggle_expand();
                    }
                }
            }
            KeyCode::Left | KeyCode::Char('h') => {
                if self.pane == Pane::Detail {
                    self.pane = Pane::List;
                } else {
                    self.collapse_current();
                }
            }
            KeyCode::Enter => {
                if self.pane == Pane::List {
                    if matches!(self.current_row(), Some(Row::Track { .. })) {
                        self.pane = Pane::Detail;
                        self.detail_index = 0;
                        self.scroll_detail();
                    } else {
                        self.toggle_expand();
                    }
                } else {
                    self.start_edit();
                }
            }
            KeyCode::Char('e') => {
                if self.pane == Pane::List {
                    self.pane = Pane::Detail;
                    self.detail_index = 0;
                    self.scroll_detail();
                }
                self.start_edit();
            }
            KeyCode::Char(' ') => {
                self.toggle_selection();
                if self.pane == Pane::List {
                    self.move_list(1);
                }
            }
            KeyCode::Char('/') => {
                self.mode = Mode::Search;
                self.input.set(self.filter.clone());
            }
            KeyCode::Char('S') => self.cycle_sort(),
            KeyCode::Char('R') => self.reload(),
            KeyCode::Char('v') => self.toggle_cover_panel(),
            KeyCode::Char('E') => self.expand_all(),
            KeyCode::Char('C') => self.collapse_all(),
            KeyCode::Char('!') => self.jump_missing(),
            KeyCode::Char('T') => self.guess_track_numbers(),
            KeyCode::Char('x') => self.toggle_compilation(),
            KeyCode::Char('d') => self.clear_current_field(),
            KeyCode::Char('D') => self.ask_delete(),
            KeyCode::Char('c') => self.open_cover(),
            KeyCode::Char('p') => self.play(false),
            KeyCode::Char('P') => self.play(true),
            KeyCode::Char('N') => self.rename(false),
            KeyCode::Char('M') => self.rename(true),
            KeyCode::Char('m') => self.start_lookup(),
            _ => {}
        }
    }

    fn handle_cover_key(&mut self, key: KeyEvent) {
        if self.cover_editing {
            match key.code {
                KeyCode::Esc => {
                    self.cover_editing = false;
                }
                KeyCode::Enter => {
                    let path = PathBuf::from(self.input.value.trim());
                    self.cover_editing = false;
                    if !path.as_os_str().is_empty() {
                        self.set_cover_from_file(&path);
                    }
                }
                KeyCode::Char('w')
                    if key.modifiers.contains(KeyModifiers::CONTROL) =>
                {
                    self.input.delete_word()
                }
                KeyCode::Backspace => self.input.backspace(),
                KeyCode::Char(ch) => self.input.insert(ch),
                _ => {}
            }
            return;
        }
        match key.code {
            KeyCode::Esc | KeyCode::Char('c') | KeyCode::Char('q') => {
                self.mode = Mode::Browse
            }
            KeyCode::Char('d') => self.download_cover(),
            KeyCode::Char('s') => {
                self.cover_editing = true;
                self.input.set(String::new());
            }
            KeyCode::Char('x') => self.remove_cover(),
            KeyCode::Char('e') => self.extract_cover(),
            _ => {}
        }
    }

    fn matches_query(&self, entry: &Entry, query: &str) -> bool {
        let groups = parse_query(query);
        if groups.is_empty() {
            return true;
        }
        groups.iter().any(|group| {
            group.iter().all(|clause| {
                clause.neg != matches_clause(entry, clause, &self.required)
            })
        })
    }
}

fn cover_source(entry: &Entry, configured: &str) -> Option<String> {
    if !entry.properties.images.is_empty() {
        return Some(format!(
            "embed:{}:{}:{}",
            entry.path.display(),
            entry.properties.images.len(),
            file_stamp(&entry.path)
        ));
    }
    let dir = entry.path.parent().unwrap_or_else(|| Path::new("."));
    let mut candidates = Vec::new();
    if let Some(found) = tags::find_folder_cover(dir) {
        candidates.push(found);
    }
    if !configured.trim().is_empty() {
        let name = Path::new(configured.trim())
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or(configured.trim());
        candidates.push(dir.join(name));
    }
    for candidate in candidates {
        if let Ok(meta) = std::fs::metadata(&candidate) {
            if meta.is_file() {
                return Some(format!(
                    "file:{}:{}:{}",
                    candidate.display(),
                    meta.len(),
                    file_stamp(&candidate)
                ));
            }
        }
    }
    None
}

fn file_stamp(path: &Path) -> String {
    let Ok(meta) = std::fs::metadata(path) else {
        return String::new();
    };
    let modified = meta
        .modified()
        .ok()
        .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|delta| format!("{}.{}", delta.as_secs(), delta.subsec_nanos()))
        .unwrap_or_default();
    format!("{}-{modified}", meta.len())
}

fn detail(label: &str, value: String, read_only: bool) -> DetailRow {
    DetailRow {
        field: None,
        label: label.to_string(),
        value,
        album: false,
        mixed: false,
        read_only,
    }
}

fn aggregate(entries: &[&Entry], field: Field) -> (String, bool) {
    let Some(first) = entries.first() else {
        return (String::new(), false);
    };
    let value = first.metadata.get(field);
    for entry in &entries[1..] {
        if entry.metadata.get(field) != value {
            return (String::new(), true);
        }
    }
    (value, false)
}

fn parse_year(value: &str) -> u32 {
    value
        .trim()
        .chars()
        .take_while(|c| c.is_ascii_digit())
        .collect::<String>()
        .get(..4)
        .and_then(|s| s.parse().ok())
        .unwrap_or(0)
}

fn expand_search_url(pattern: &str, meta: &Metadata, entry: &Entry) -> String {
    let mut out = String::new();
    let chars: Vec<char> = pattern.chars().collect();
    let mut index = 0;
    while index < chars.len() {
        if chars[index] != '%' {
            out.push(chars[index]);
            index += 1;
            continue;
        }
        if index + 1 >= chars.len() {
            out.push('%');
            break;
        }
        if chars[index + 1] == '%' {
            out.push('%');
            index += 2;
            continue;
        }
        let start = index + 1;
        let mut end = start;
        while end < chars.len()
            && (chars[end].is_ascii_alphanumeric() || chars[end] == '-')
        {
            end += 1;
        }
        let word: String = chars[start..end].iter().collect();
        let value = if word.eq_ignore_ascii_case("f")
            || word.eq_ignore_ascii_case("format")
        {
            entry.format()
        } else {
            let field = if word.len() == 1 {
                match word.as_str() {
                    "t" => Some(Field::Title),
                    "a" => Some(Field::Artist),
                    "A" => Some(Field::AlbumArtist),
                    "L" => Some(Field::Album),
                    "g" => Some(Field::Genre),
                    "y" => Some(Field::Year),
                    "m" => Some(Field::Composer),
                    "l" => Some(Field::Publisher),
                    "i" => Some(Field::Isrc),
                    "c" => Some(Field::Comment),
                    _ => Field::parse(&word),
                }
            } else {
                Field::parse(&word)
            };
            field.map(|f| meta.get(f)).unwrap_or_default()
        };
        out.push_str(
            &url::form_urlencoded::byte_serialize(value.as_bytes())
                .collect::<String>(),
        );
        index = end;
    }
    out
}

#[derive(Clone, Debug)]
struct QueryClause {
    neg: bool,
    field: String,
    op: String,
    value: String,
}

fn parse_query(query: &str) -> Vec<Vec<QueryClause>> {
    let tokens = tokenize(query);
    let mut groups = Vec::new();
    let mut current = Vec::new();
    for token in tokens {
        if token.eq_ignore_ascii_case("OR") {
            if !current.is_empty() {
                groups.push(std::mem::take(&mut current));
            }
            continue;
        }
        if let Some(clause) = parse_clause(&token) {
            current.push(clause);
        }
    }
    if !current.is_empty() {
        groups.push(current);
    }
    groups
}

fn tokenize(query: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut current = String::new();
    let mut quote = None;
    let mut escaped = false;
    for ch in query.chars() {
        if escaped {
            current.push(ch);
            escaped = false;
        } else if ch == '\\' {
            escaped = true;
        } else if let Some(q) = quote {
            if ch == q {
                quote = None;
            } else {
                current.push(ch);
            }
        } else if ch == '"' || ch == '\'' {
            quote = Some(ch);
        } else if ch.is_whitespace() {
            if !current.is_empty() {
                out.push(std::mem::take(&mut current));
            }
        } else {
            current.push(ch);
        }
    }
    if !current.is_empty() {
        out.push(current);
    }
    out
}

fn parse_clause(token: &str) -> Option<QueryClause> {
    let mut token = token;
    let mut neg = false;
    if let Some(rest) = token.strip_prefix('-') {
        if !rest.is_empty() {
            neg = true;
            token = rest;
        }
    }
    let Some((field, op, value)) = split_field(token) else {
        return Some(QueryClause {
            neg,
            field: String::new(),
            op: ":".into(),
            value: token.to_string(),
        });
    };
    Some(QueryClause {
        neg,
        field: field.to_ascii_lowercase(),
        op,
        value,
    })
}

fn split_field(token: &str) -> Option<(String, String, String)> {
    let mut best: Option<(usize, &str)> = None;
    for op in [">=", "<=", "!=", "=", ">", "<", ":"] {
        if let Some(index) = token.find(op) {
            if best.map(|(i, _)| index < i).unwrap_or(true) {
                best = Some((index, op));
            }
        }
    }
    let (index, mut op) = best?;
    let field = token[..index].to_string();
    let mut value = token[index + op.len()..].to_string();
    if op == ":" {
        for prefix in [">=", "<=", "!=", ">", "<", "="] {
            if let Some(rest) = value.strip_prefix(prefix) {
                op = prefix;
                value = rest.to_string();
                break;
            }
        }
    }
    Some((field, op.to_string(), value))
}

fn matches_clause(
    entry: &Entry,
    clause: &QueryClause,
    required: &[RequiredTag],
) -> bool {
    match clause.field.as_str() {
        "path" => contains(&entry.path.to_string_lossy(), &clause.value),
        "file" => contains(&entry.title(), &clause.value),
        "format" | "container" => {
            contains(&entry.properties.container, &clause.value)
        }
        "codec" => contains(&entry.properties.codec, &clause.value),
        "cover" | "art" => bool_match(entry.properties.has_cover(), clause),
        "lossless" => bool_match(
            matches!(
                entry.properties.container.as_str(),
                "flac" | "wavpack" | "ape"
            ),
            clause,
        ),
        "has" => match clause.value.to_ascii_lowercase().as_str() {
            "cover" | "art" | "artwork" => entry.properties.has_cover(),
            "lyrics" => !entry.metadata.lyrics.trim().is_empty(),
            "genre" | "genres" => !entry.metadata.genres.is_empty(),
            "artist" => !entry.metadata.artists.is_empty(),
            "album" => !entry.metadata.album.trim().is_empty(),
            _ => false,
        },
        "duration" | "length" => duration_match(entry.duration(), clause),
        "bitrate" => number_match(entry.properties.bitrate_kbps as i64, clause),
        "size" => number_match(entry.properties.size as i64, clause),
        "compilation" | "comp" => {
            bool_match(entry.metadata.compilation, clause)
        }
        "missing" | "required" => {
            let missing = library::missing_requirements(entry, required);
            match clause.value.to_ascii_lowercase().as_str() {
                "" | "yes" | "true" | "1" => !missing.is_empty(),
                "no" | "false" | "0" => missing.is_empty(),
                value => RequiredTag::parse(value)
                    .map(|requirement| {
                        library::missing_requirement(entry, &requirement)
                    })
                    .unwrap_or(false),
            }
        }
        field => {
            let Some(field) = Field::parse(field) else {
                return search_text(entry)
                    .to_ascii_lowercase()
                    .contains(&clause.value.to_ascii_lowercase());
            };
            let value = entry.metadata.get(field);
            if matches!(
                field,
                Field::Year
                    | Field::Track
                    | Field::TrackTotal
                    | Field::Disc
                    | Field::DiscTotal
            ) && matches!(
                clause.op.as_str(),
                "=" | "!=" | ">" | ">=" | "<" | "<=" | ":"
            ) {
                return number_match(value.parse().unwrap_or(0), clause);
            }
            contains(&value, &clause.value)
        }
    }
}

fn search_text(entry: &Entry) -> String {
    format!(
        "{} {} {} {} {} {}",
        entry.metadata.title,
        entry.metadata.artist_string(),
        entry.metadata.album,
        entry.metadata.album_artist,
        entry.metadata.genre_string(),
        entry.path.display()
    )
}

fn contains(value: &str, needle: &str) -> bool {
    value
        .to_ascii_lowercase()
        .contains(&needle.to_ascii_lowercase())
}

fn bool_match(value: bool, clause: &QueryClause) -> bool {
    if clause.value.is_empty() {
        return value;
    }
    let wanted = matches!(
        clause.value.to_ascii_lowercase().as_str(),
        "yes" | "true" | "1" | "on" | "y"
    );
    if clause.op == "!=" {
        value != wanted
    } else {
        value == wanted
    }
}

fn number_match(value: i64, clause: &QueryClause) -> bool {
    if clause.value.is_empty() {
        return value != 0;
    }
    let Ok(wanted) = clause.value.parse::<i64>() else {
        return false;
    };
    match clause.op.as_str() {
        ">" => value > wanted,
        ">=" => value >= wanted,
        "<" => value < wanted,
        "<=" => value <= wanted,
        "!=" => value != wanted,
        _ => value == wanted,
    }
}

fn duration_match(duration: Duration, clause: &QueryClause) -> bool {
    let seconds = duration.as_secs() as i64;
    if clause.value.is_empty() {
        return seconds > 0;
    }
    let mut value = clause.value.to_ascii_lowercase();
    let multiplier = if value.ends_with('m') {
        value.pop();
        60
    } else if value.ends_with('s') {
        value.pop();
        1
    } else if value.contains(':') {
        let parts: Vec<_> = value.split(':').collect();
        if parts.len() == 2 {
            let minutes: i64 = parts[0].parse().unwrap_or(0);
            let secs: i64 = parts[1].parse().unwrap_or(0);
            return compare(seconds, minutes * 60 + secs, &clause.op);
        }
        1
    } else {
        1
    };
    let Ok(wanted) = value.parse::<i64>() else {
        return false;
    };
    compare(seconds, wanted * multiplier, &clause.op)
}

fn compare(value: i64, wanted: i64, op: &str) -> bool {
    match op {
        ">" => value > wanted,
        ">=" => value >= wanted,
        "<" => value < wanted,
        "<=" => value <= wanted,
        "!=" => value != wanted,
        _ => value == wanted,
    }
}
