//! Command line interface.

use std::collections::HashMap;
use std::fs;
use std::io::Write as _;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use clap::{Args, CommandFactory, Parser, Subcommand};
use clap_complete::Shell;

use crate::config::Config;
use crate::cover::{self, DownloadResult};
use crate::library::{self, Entry, GroupMode};
use crate::online::Client;
use crate::organize;
use crate::tags::{self, CoverAction, Field, Metadata};
use crate::theme;
use crate::tui;

#[derive(Parser, Debug)]
#[command(
    name = "tuigr",
    version,
    about = "Browse and edit music tags in the terminal"
)]
pub struct Cli {
    /// Music directory (overrides the configured library)
    #[arg(short = 'C', long = "dir", global = true)]
    pub dir: Option<PathBuf>,
    /// Open this file or directory instead of the library
    pub path: Option<PathBuf>,
    #[command(subcommand)]
    pub command: Option<Command>,
}

#[derive(Subcommand, Debug)]
#[allow(clippy::large_enum_variant)]
pub enum Command {
    /// List music files with their tags
    List(ListArgs),
    /// Print a single tag field
    Get(GetArgs),
    /// Set tag fields on one or more tracks
    Set(SetArgs),
    /// Apply a regular expression to existing tag values
    Modify(ModifyArgs),
    /// Rename files after their tags
    Rename(RenameArgs),
    /// Guess track numbers from file names
    Guess(GuessArgs),
    /// Download album artwork
    Cover(CoverArgs),
    /// Show library statistics
    Stats(StatsArgs),
    /// Validate tags and album consistency
    Check(CheckArgs),
    /// Search MusicBrainz for album metadata
    Lookup(LookupArgs),
    /// List color themes
    Themes(ThemesArgs),
    /// Generate shell completions
    Completion(CompletionArgs),
    /// Generate a man page
    Man(ManArgs),
}

#[derive(Args, Debug, Default)]
pub struct CommonArgs {
    /// Recurse into directories
    #[arg(short = 'r', long)]
    pub recursive: bool,
    #[arg(value_name = "FILE|DIR")]
    pub paths: Vec<PathBuf>,
}

#[derive(Args, Debug, Default)]
pub struct ListArgs {
    #[arg(long)]
    pub json: bool,
    #[arg(long)]
    pub flat: bool,
    #[arg(long)]
    pub albums: bool,
    #[arg(short = 'r', long)]
    pub recursive: bool,
    #[arg(value_name = "FILE|DIR")]
    pub paths: Vec<PathBuf>,
}

#[derive(Args, Debug, Default)]
pub struct GetArgs {
    pub field: String,
    #[arg(short = 'r', long)]
    pub recursive: bool,
    #[arg(value_name = "FILE|DIR")]
    pub paths: Vec<PathBuf>,
}

#[derive(Args, Debug, Default)]
pub struct SetArgs {
    #[arg(long)]
    pub title: Option<String>,
    #[arg(short = 'a', long)]
    pub artist: Option<String>,
    #[arg(short = 'b', long)]
    pub album: Option<String>,
    #[arg(short = 'A', long, alias = "album-artist")]
    pub albumartist: Option<String>,
    #[arg(short = 'g', long)]
    pub genre: Option<String>,
    #[arg(short = 'y', long, alias = "date")]
    pub year: Option<String>,
    #[arg(short = 't', long, alias = "track-number")]
    pub track: Option<String>,
    #[arg(long)]
    pub track_total: Option<String>,
    #[arg(long, alias = "disc-number")]
    pub disc: Option<String>,
    #[arg(long)]
    pub disc_total: Option<String>,
    #[arg(long)]
    pub compilation: Option<String>,
    #[arg(long)]
    pub comment: Option<String>,
    #[arg(long)]
    pub lyrics: Option<String>,
    #[arg(long)]
    pub composer: Option<String>,
    #[arg(long)]
    pub conductor: Option<String>,
    #[arg(long)]
    pub lyricist: Option<String>,
    #[arg(long)]
    pub bpm: Option<String>,
    #[arg(long)]
    pub key: Option<String>,
    #[arg(long)]
    pub isrc: Option<String>,
    #[arg(short = 'l', long, alias = "label")]
    pub publisher: Option<String>,
    #[arg(long)]
    pub catalog_number: Option<String>,
    #[arg(long)]
    pub barcode: Option<String>,
    #[arg(long)]
    pub media: Option<String>,
    #[arg(long)]
    pub grouping: Option<String>,
    #[arg(long)]
    pub subtitle: Option<String>,
    #[arg(long)]
    pub original_date: Option<String>,
    #[arg(long)]
    pub copyright: Option<String>,
    #[arg(long)]
    pub encoded_by: Option<String>,
    #[arg(long)]
    pub cover: Option<PathBuf>,
    #[arg(long)]
    pub remove_cover: bool,
    #[arg(long)]
    pub append: bool,
    #[arg(long)]
    pub renumber: bool,
    #[arg(short = 'n', long)]
    pub dry_run: bool,
    #[arg(short = 'r', long)]
    pub recursive: bool,
    #[arg(value_name = "FILE|DIR")]
    pub paths: Vec<PathBuf>,
}

#[derive(Args, Debug, Default)]
pub struct ModifyArgs {
    #[arg(short = 'e', long)]
    pub regex: String,
    #[arg(long, default_value = "")]
    pub replacement: String,
    #[arg(long)]
    pub title: bool,
    #[arg(short = 'a', long)]
    pub artist: bool,
    #[arg(short = 'b', long)]
    pub album: bool,
    #[arg(short = 'A', long)]
    pub albumartist: bool,
    #[arg(short = 'g', long)]
    pub genre: bool,
    #[arg(short = 'y', long)]
    pub year: bool,
    #[arg(short = 't', long)]
    pub track: bool,
    #[arg(long)]
    pub comment: bool,
    #[arg(long)]
    pub lyrics: bool,
    #[arg(long)]
    pub composer: bool,
    #[arg(long)]
    pub bpm: bool,
    #[arg(long)]
    pub key: bool,
    #[arg(long)]
    pub isrc: bool,
    #[arg(short = 'l', long)]
    pub publisher: bool,
    #[arg(short = 'n', long)]
    pub dry_run: bool,
    #[arg(short = 'r', long)]
    pub recursive: bool,
    #[arg(value_name = "FILE|DIR")]
    pub paths: Vec<PathBuf>,
}

#[derive(Args, Debug, Default)]
pub struct RenameArgs {
    #[arg(short = 'f', long)]
    pub format: Option<String>,
    #[arg(long)]
    pub root: Option<PathBuf>,
    #[arg(long)]
    pub r#move: bool,
    #[arg(short = 's', long)]
    pub slugify: bool,
    #[arg(long)]
    pub no_slugify: bool,
    #[arg(short = 'n', long)]
    pub dry_run: bool,
    #[arg(short = 'r', long)]
    pub recursive: bool,
    #[arg(value_name = "FILE|DIR")]
    pub paths: Vec<PathBuf>,
}

#[derive(Args, Debug, Default)]
pub struct GuessArgs {
    #[arg(short = 'n', long)]
    pub dry_run: bool,
    #[arg(short = 'r', long)]
    pub recursive: bool,
    #[arg(value_name = "FILE|DIR")]
    pub paths: Vec<PathBuf>,
}

#[derive(Args, Debug, Default)]
pub struct CoverArgs {
    #[arg(long)]
    pub release: Option<String>,
    #[arg(long, default_value = "")]
    pub artist: String,
    #[arg(long, default_value = "")]
    pub album: String,
    #[arg(long)]
    pub file: Option<String>,
    #[arg(short = 'r', long)]
    pub recursive: bool,
    #[arg(value_name = "FILE|DIR")]
    pub paths: Vec<PathBuf>,
}

#[derive(Args, Debug, Default)]
pub struct StatsArgs {
    #[arg(long)]
    pub json: bool,
    #[arg(long, default_value_t = 10)]
    pub top: usize,
    #[arg(short = 'r', long)]
    pub recursive: bool,
    #[arg(value_name = "FILE|DIR")]
    pub paths: Vec<PathBuf>,
}

#[derive(Args, Debug, Default)]
pub struct CheckArgs {
    #[arg(long)]
    pub json: bool,
    #[arg(short = 'r', long)]
    pub recursive: bool,
    #[arg(value_name = "FILE|DIR")]
    pub paths: Vec<PathBuf>,
}

#[derive(Args, Debug, Default)]
pub struct LookupArgs {
    #[arg(long)]
    pub release: Option<String>,
    #[arg(long, default_value = "")]
    pub artist: String,
    #[arg(long, default_value = "")]
    pub album: String,
    #[arg(long)]
    pub json: bool,
    #[arg(long, default_value_t = 5)]
    pub limit: usize,
    #[arg(short = 'r', long)]
    pub recursive: bool,
    #[arg(value_name = "FILE|DIR")]
    pub paths: Vec<PathBuf>,
}

#[derive(Args, Debug, Default)]
pub struct ThemesArgs {
    #[arg(long)]
    pub json: bool,
}

#[derive(Args, Debug)]
pub struct CompletionArgs {
    pub shell: Shell,
}

#[derive(Args, Debug)]
pub struct ManArgs {
    #[arg(long)]
    pub dir: Option<PathBuf>,
}

pub fn run() -> Result<()> {
    let cli = Cli::parse();
    let config = Config::load()?;
    match cli.command {
        None => {
            let path = cli
                .path
                .clone()
                .unwrap_or_else(|| config.library_dir(Path::new(".")));
            if path.is_dir() {
                tui::run(&path, config)
            } else {
                tui::run_file(&path, config)
            }
        }
        Some(command) => dispatch(command, cli.dir, config),
    }
}

fn dispatch(
    command: Command,
    dir: Option<PathBuf>,
    config: Config,
) -> Result<()> {
    match command {
        Command::List(args) => list(&args, dir, &config),
        Command::Get(args) => get(&args, dir, &config),
        Command::Set(args) => set(&args, dir, &config),
        Command::Modify(args) => modify(&args, dir, &config),
        Command::Rename(args) => rename(&args, dir, &config),
        Command::Guess(args) => guess(&args, dir, &config),
        Command::Cover(args) => cover_command(&args, dir, &config),
        Command::Stats(args) => stats(&args, dir, &config),
        Command::Check(args) => check(&args, dir, &config),
        Command::Lookup(args) => lookup(&args, dir, &config),
        Command::Themes(args) => themes(&args, &config),
        Command::Completion(args) => completion(&args),
        Command::Man(args) => man(&args),
    }
}

fn target_paths(
    dir: Option<PathBuf>,
    paths: &[PathBuf],
    config: &Config,
) -> Vec<PathBuf> {
    if !paths.is_empty() {
        return paths.to_vec();
    }
    if let Some(dir) = dir {
        return vec![dir];
    }
    vec![config.library_dir(Path::new("."))]
}

fn expand_paths(
    dir: Option<PathBuf>,
    paths: &[PathBuf],
    recursive: bool,
    config: &Config,
) -> Result<Vec<PathBuf>> {
    let targets = target_paths(dir, paths, config);
    library::expand(&targets, recursive)
}

fn rel(path: &Path) -> String {
    path.to_string_lossy().to_string()
}

fn list(args: &ListArgs, dir: Option<PathBuf>, config: &Config) -> Result<()> {
    let paths = expand_paths(dir, &args.paths, args.recursive, config)?;
    let entries = library::load(&paths);
    if args.json {
        let values: Vec<_> = entries
            .iter()
            .map(|entry| {
                let mut value = serde_json::json!({
                    "path": entry.path,
                    "title": entry.title(),
                    "artists": entry.metadata.artists,
                    "album": entry.metadata.album,
                    "albumArtist": entry.metadata.album_artist,
                    "genres": entry.metadata.genres,
                    "year": entry.metadata.year,
                    "track": entry.metadata.track,
                    "trackTotal": entry.metadata.track_total,
                    "format": entry.properties.container,
                    "durationMs": entry.properties.duration.as_millis(),
                    "bitrate": entry.properties.bitrate_kbps,
                    "size": entry.properties.size,
                    "hasCover": entry.properties.has_cover(),
                });
                if let Some(err) = &entry.error {
                    value["error"] = serde_json::Value::String(err.clone());
                }
                value
            })
            .collect();
        println!("{}", serde_json::to_string_pretty(&values)?);
        return Ok(());
    }
    if args.flat {
        for entry in &entries {
            println!(
                "{}\t{}\t{}\t{}\t{}",
                rel(&entry.path),
                entry.artist(),
                entry.metadata.album,
                entry.track_label(),
                entry.title()
            );
        }
        return Ok(());
    }
    let albums = library::group(&entries, GroupMode::Tags);
    if args.albums {
        for album in &albums {
            println!("{}", album.label());
            println!(
                "  {} tracks · {}",
                album.tracks.len(),
                library::duration_label(album.duration)
            );
        }
        return Ok(());
    }
    for album in &albums {
        println!("{}", album.label());
        println!(
            "  {} tracks · {}",
            album.tracks.len(),
            library::duration_label(album.duration)
        );
        for index in &album.tracks {
            let entry = &entries[*index];
            if let Some(err) = &entry.error {
                println!("    ! {}: {err}", rel(&entry.path));
                continue;
            }
            println!(
                "    {:<5} {:<40} {}",
                if entry.track_label().is_empty() {
                    "--".into()
                } else {
                    entry.track_label()
                },
                truncate(&entry.title(), 40),
                library::duration_label(entry.duration())
            );
            println!("          {}", rel(&entry.path));
        }
    }
    Ok(())
}

fn get(args: &GetArgs, dir: Option<PathBuf>, config: &Config) -> Result<()> {
    let field = Field::parse(&args.field)
        .with_context(|| format!("unknown field {:?}", args.field))?;
    let paths = expand_paths(dir, &args.paths, args.recursive, config)?;
    for path in &paths {
        match tags::read(path) {
            Ok(data) => {
                if paths.len() > 1 {
                    println!("{}\t{}", rel(path), data.metadata.get(field));
                } else {
                    println!("{}", data.metadata.get(field));
                }
            }
            Err(err) => eprintln!("{}: {err:#}", rel(path)),
        }
    }
    Ok(())
}

fn set(args: &SetArgs, dir: Option<PathBuf>, config: &Config) -> Result<()> {
    if args.renumber
        && (has_set_values(args) || args.cover.is_some() || args.remove_cover)
    {
        bail!("--renumber cannot be combined with tag or cover edits");
    }
    let paths = expand_paths(dir, &args.paths, args.recursive, config)?;
    if args.renumber {
        return renumber(&paths, args.dry_run, config);
    }
    if !has_set_values(args) && args.cover.is_none() && !args.remove_cover {
        bail!("no fields to set");
    }
    let cover_data = match &args.cover {
        Some(path) => Some(
            fs::read(path).with_context(|| format!("{}", path.display()))?,
        ),
        None => None,
    };
    let mut updated = 0;
    let mut failed = 0;
    for path in &paths {
        let result = (|| -> Result<()> {
            let mut data = tags::read(path)?;
            let old = data.metadata.clone();
            if args.append {
                if let Some(value) = &args.artist {
                    for item in value
                        .split(';')
                        .map(str::trim)
                        .filter(|s| !s.is_empty())
                    {
                        if !data
                            .metadata
                            .artists
                            .iter()
                            .any(|a| a.eq_ignore_ascii_case(item))
                        {
                            data.metadata.artists.push(item.to_string());
                        }
                    }
                }
                if let Some(value) = &args.genre {
                    for item in value
                        .split(',')
                        .map(str::trim)
                        .filter(|s| !s.is_empty())
                    {
                        if !data
                            .metadata
                            .genres
                            .iter()
                            .any(|g| g.eq_ignore_ascii_case(item))
                        {
                            data.metadata.genres.push(item.to_string());
                        }
                    }
                }
            }
            apply_set_fields(&mut data.metadata, args, !args.append)?;
            if args.dry_run {
                print_changes(path, &old, &data.metadata);
                return Ok(());
            }
            let cover_action = if args.remove_cover {
                if config.embed_cover() {
                    CoverAction::Remove
                } else {
                    CoverAction::Keep
                }
            } else if let Some(data) = &cover_data {
                if config.embed_cover() {
                    CoverAction::Set(data.clone())
                } else {
                    CoverAction::Keep
                }
            } else {
                CoverAction::Keep
            };
            tags::write(path, &data.metadata, cover_action)?;
            if config.folder_cover() {
                let dir = path.parent().unwrap_or_else(|| Path::new("."));
                if args.remove_cover {
                    let _ = tags::remove_folder_cover(dir, &config.cover_file);
                } else if let Some(data) = &cover_data {
                    tags::write_folder_cover(dir, &config.cover_file, data)?;
                }
            }
            Ok(())
        })();
        match result {
            Ok(()) => updated += 1,
            Err(err) => {
                failed += 1;
                eprintln!("{}: {err:#}", rel(path));
            }
        }
    }
    println!("updated {updated} file(s)");
    if failed > 0 {
        bail!("{failed} file(s) failed");
    }
    Ok(())
}

fn has_set_values(args: &SetArgs) -> bool {
    args.title.is_some()
        || args.artist.is_some()
        || args.album.is_some()
        || args.albumartist.is_some()
        || args.genre.is_some()
        || args.year.is_some()
        || args.track.is_some()
        || args.track_total.is_some()
        || args.disc.is_some()
        || args.disc_total.is_some()
        || args.compilation.is_some()
        || args.comment.is_some()
        || args.lyrics.is_some()
        || args.composer.is_some()
        || args.conductor.is_some()
        || args.lyricist.is_some()
        || args.bpm.is_some()
        || args.key.is_some()
        || args.isrc.is_some()
        || args.publisher.is_some()
        || args.catalog_number.is_some()
        || args.barcode.is_some()
        || args.media.is_some()
        || args.grouping.is_some()
        || args.subtitle.is_some()
        || args.original_date.is_some()
        || args.copyright.is_some()
        || args.encoded_by.is_some()
}

fn apply_set_fields(
    meta: &mut Metadata,
    args: &SetArgs,
    replace: bool,
) -> Result<()> {
    let pairs: [(Option<&String>, Field); 27] = [
        (args.title.as_ref(), Field::Title),
        (args.artist.as_ref(), Field::Artist),
        (args.album.as_ref(), Field::Album),
        (args.albumartist.as_ref(), Field::AlbumArtist),
        (args.genre.as_ref(), Field::Genre),
        (args.year.as_ref(), Field::Year),
        (args.track.as_ref(), Field::Track),
        (args.track_total.as_ref(), Field::TrackTotal),
        (args.disc.as_ref(), Field::Disc),
        (args.disc_total.as_ref(), Field::DiscTotal),
        (args.compilation.as_ref(), Field::Compilation),
        (args.comment.as_ref(), Field::Comment),
        (args.lyrics.as_ref(), Field::Lyrics),
        (args.composer.as_ref(), Field::Composer),
        (args.conductor.as_ref(), Field::Conductor),
        (args.lyricist.as_ref(), Field::Lyricist),
        (args.bpm.as_ref(), Field::Bpm),
        (args.key.as_ref(), Field::Key),
        (args.isrc.as_ref(), Field::Isrc),
        (args.publisher.as_ref(), Field::Publisher),
        (args.catalog_number.as_ref(), Field::CatalogNumber),
        (args.barcode.as_ref(), Field::Barcode),
        (args.media.as_ref(), Field::Media),
        (args.grouping.as_ref(), Field::Grouping),
        (args.subtitle.as_ref(), Field::Subtitle),
        (args.original_date.as_ref(), Field::OriginalDate),
        (args.copyright.as_ref(), Field::Copyright),
    ];
    for (value, field) in pairs {
        if let Some(value) = value {
            let value = if args.append
                && matches!(field, Field::Artist | Field::Genre)
            {
                continue;
            } else {
                value.as_str()
            };
            if replace || !matches!(field, Field::Artist | Field::Genre) {
                meta.set(field, value)?;
            }
        }
    }
    if let Some(value) = &args.encoded_by {
        meta.set(Field::EncodedBy, value)?;
    }
    Ok(())
}

fn print_changes(path: &Path, old: &Metadata, new: &Metadata) {
    println!("{}", rel(path));
    for field in tags::ALL_FIELDS {
        let before = old.get(*field);
        let after = new.get(*field);
        if before != after {
            println!("  {:<16} {:?} -> {:?}", field.label(), before, after);
        }
    }
}

fn renumber(paths: &[PathBuf], dry_run: bool, config: &Config) -> Result<()> {
    let entries = library::load(paths);
    let albums = library::group(&entries, GroupMode::Folder);
    let mut jobs = Vec::new();
    for album in &albums {
        let mut by_disc: HashMap<u32, Vec<&Entry>> = HashMap::new();
        for index in &album.tracks {
            let entry = &entries[*index];
            if entry.error.is_none() {
                by_disc.entry(entry.metadata.disc).or_default().push(entry);
            }
        }
        for list in by_disc.values_mut() {
            list.sort_by(|a, b| a.path.cmp(&b.path));
            for (i, entry) in list.iter().enumerate() {
                jobs.push((
                    entry.path.clone(),
                    i as u32 + 1,
                    list.len() as u32,
                ));
            }
        }
    }
    let mut updated = 0;
    for (path, track, total) in jobs {
        if dry_run {
            println!("{}\t{track}/{total}", rel(&path));
            continue;
        }
        let mut data = tags::read(&path)?;
        data.metadata.track = track;
        data.metadata.track_total = total;
        tags::write(&path, &data.metadata, CoverAction::Keep)?;
        updated += 1;
    }
    if dry_run {
        println!("{} track(s) would be renumbered", updated);
    } else {
        println!("renumbered {updated} track(s)");
    }
    let _ = config;
    Ok(())
}

fn modify(
    args: &ModifyArgs,
    dir: Option<PathBuf>,
    config: &Config,
) -> Result<()> {
    if args.regex.trim().is_empty() {
        bail!("-e/--regex is required");
    }
    let regex = regex::Regex::new(&args.regex).context("invalid regex")?;
    let fields = selected_modify_fields(args);
    if fields.is_empty() {
        bail!("no fields selected");
    }
    let paths = expand_paths(dir, &args.paths, args.recursive, config)?;
    let mut changed = 0;
    for path in &paths {
        let result = (|| -> Result<bool> {
            let mut data = tags::read(path)?;
            let old = data.metadata.clone();
            for field in &fields {
                let before = data.metadata.get(*field);
                let after = regex
                    .replace_all(&before, args.replacement.as_str())
                    .to_string();
                if before != after {
                    data.metadata.set(*field, &after)?;
                }
            }
            if data.metadata == old {
                return Ok(false);
            }
            if args.dry_run {
                print_changes(path, &old, &data.metadata);
                return Ok(true);
            }
            tags::write(path, &data.metadata, CoverAction::Keep)?;
            Ok(true)
        })();
        match result {
            Ok(true) => changed += 1,
            Ok(false) => {}
            Err(err) => eprintln!("{}: {err:#}", rel(path)),
        }
    }
    println!("modified {changed} file(s)");
    Ok(())
}

fn selected_modify_fields(args: &ModifyArgs) -> Vec<Field> {
    let mut fields = Vec::new();
    if args.title {
        fields.push(Field::Title);
    }
    if args.artist {
        fields.push(Field::Artist);
    }
    if args.album {
        fields.push(Field::Album);
    }
    if args.albumartist {
        fields.push(Field::AlbumArtist);
    }
    if args.genre {
        fields.push(Field::Genre);
    }
    if args.year {
        fields.push(Field::Year);
    }
    if args.track {
        fields.push(Field::Track);
    }
    if args.comment {
        fields.push(Field::Comment);
    }
    if args.lyrics {
        fields.push(Field::Lyrics);
    }
    if args.composer {
        fields.push(Field::Composer);
    }
    if args.bpm {
        fields.push(Field::Bpm);
    }
    if args.key {
        fields.push(Field::Key);
    }
    if args.isrc {
        fields.push(Field::Isrc);
    }
    if args.publisher {
        fields.push(Field::Publisher);
    }
    fields
}

fn rename(
    args: &RenameArgs,
    dir: Option<PathBuf>,
    config: &Config,
) -> Result<()> {
    let paths = expand_paths(dir, &args.paths, args.recursive, config)?;
    let entries = library::load(&paths);
    let pattern = args
        .format
        .clone()
        .or_else(|| {
            if config.rename_format.trim().is_empty() {
                None
            } else {
                Some(config.rename_format.clone())
            }
        })
        .unwrap_or_else(|| {
            if args.r#move {
                "%A/%L/%n - %t".into()
            } else {
                "%n - %t".into()
            }
        });
    let slugify = if args.no_slugify {
        false
    } else if args.slugify {
        true
    } else {
        config.slugify_rename()
    };
    let root = args.root.clone().or_else(|| {
        if args.r#move {
            None
        } else if pattern.contains('/') {
            if let Some(path) = args.paths.first().filter(|p| p.is_dir()) {
                Some(path.clone())
            } else if config.library.is_empty() {
                None
            } else if entries
                .iter()
                .all(|e| organize::under(&e.path, Path::new(&config.library)))
            {
                Some(PathBuf::from(&config.library))
            } else {
                None
            }
        } else {
            None
        }
    });
    let report = organize::rename(
        &entries,
        &organize::Options {
            pattern,
            root,
            library: if config.library.is_empty() {
                None
            } else {
                Some(PathBuf::from(&config.library))
            },
            slugify,
            move_files: args.r#move,
            dry_run: args.dry_run,
        },
    );
    for result in &report.results {
        println!("{} -> {}", rel(&result.source), rel(&result.target));
    }
    for error in &report.errors {
        eprintln!("{error}");
    }
    println!(
        "{} {} file(s)",
        if args.dry_run {
            "would rename"
        } else {
            "renamed"
        },
        report.results.len()
    );
    if !report.errors.is_empty() {
        bail!("{} file(s) failed", report.errors.len());
    }
    Ok(())
}

fn guess(
    args: &GuessArgs,
    dir: Option<PathBuf>,
    config: &Config,
) -> Result<()> {
    let paths = expand_paths(dir, &args.paths, args.recursive, config)?;
    let mut changed = 0;
    let mut unmatched = 0;
    for path in &paths {
        let Some(guess) = library::guess_track_number(path) else {
            unmatched += 1;
            println!("{}: no track number found", rel(path));
            continue;
        };
        let label = guess_label(guess);
        if args.dry_run {
            changed += 1;
            println!("{}: {label}", rel(path));
            continue;
        }
        let mut data = tags::read(path)?;
        data.metadata.track = guess.track;
        if guess.total > 0 {
            data.metadata.track_total = guess.total;
        }
        if guess.disc > 0 {
            data.metadata.disc = guess.disc;
        }
        tags::write(path, &data.metadata, CoverAction::Keep)?;
        changed += 1;
        println!("{}: {label}", rel(path));
    }
    println!(
        "{} {changed} track number(s), {unmatched} file(s) without a guess",
        if args.dry_run {
            "would update"
        } else {
            "updated"
        }
    );
    Ok(())
}

fn guess_label(guess: library::TrackGuess) -> String {
    let mut label = format!("track {}", guess.track);
    if guess.total > 0 {
        label.push_str(&format!("/{}", guess.total));
    }
    if guess.disc > 0 {
        label = format!("disc {}, {label}", guess.disc);
    }
    label
}

fn cover_command(
    args: &CoverArgs,
    dir: Option<PathBuf>,
    config: &Config,
) -> Result<()> {
    let client = Client::new(&config.musicbrainz_user_agent);
    let filename = args
        .file
        .clone()
        .unwrap_or_else(|| config.cover_file.clone());
    if let Some(release) = &args.release {
        let album_dir = cover_directory(dir, &args.paths, config);
        let result = cover::download_release_cover(
            &client, &album_dir, release, &filename,
        )?;
        print_download(&result);
        return Ok(());
    }
    if !args.album.is_empty() {
        let album_dir = cover_directory(dir, &args.paths, config);
        let result = cover::download_album_cover(
            &client,
            &album_dir,
            &args.artist,
            &args.album,
            &filename,
        )?;
        print_download(&result);
        return Ok(());
    }
    let paths = expand_paths(dir, &args.paths, args.recursive, config)?;
    let entries = library::load(&paths);
    let albums = library::group(&entries, GroupMode::Folder);
    let mut wrote = 0;
    let mut failed = 0;
    for album in &albums {
        let Some(first_index) = album.tracks.first().copied() else {
            continue;
        };
        let first = &entries[first_index];
        if let Some(err) = &first.error {
            eprintln!("{}: {err}", rel(&first.path));
            failed += 1;
            continue;
        }
        let dir = first.path.parent().unwrap_or_else(|| Path::new("."));
        let artist = if first.metadata.album_artist.trim().is_empty() {
            first.artist()
        } else {
            first.metadata.album_artist.clone()
        };
        let album_title = if first.metadata.album.trim().is_empty() {
            dir.file_name()
                .map(|s| s.to_string_lossy().to_string())
                .unwrap_or_default()
        } else {
            first.metadata.album.clone()
        };
        match cover::download_album_cover(
            &client,
            dir,
            &artist,
            &album_title,
            &filename,
        ) {
            Ok(result) => {
                print_download(&result);
                wrote += 1;
            }
            Err(err) => {
                eprintln!("{}: {err:#}", dir.display());
                failed += 1;
            }
        }
    }
    println!("wrote {wrote} cover(s)");
    if failed > 0 {
        bail!("{failed} folder(s) failed");
    }
    Ok(())
}

fn cover_directory(
    dir: Option<PathBuf>,
    paths: &[PathBuf],
    config: &Config,
) -> PathBuf {
    let target = paths
        .first()
        .cloned()
        .or(dir)
        .unwrap_or_else(|| config.library_dir(Path::new(".")));
    if target.is_dir() {
        target
    } else {
        target
            .parent()
            .unwrap_or_else(|| Path::new("."))
            .to_path_buf()
    }
}

fn print_download(result: &DownloadResult) {
    println!("{}: wrote {}", rel(&result.path), result.release_id);
}

fn stats(
    args: &StatsArgs,
    dir: Option<PathBuf>,
    config: &Config,
) -> Result<()> {
    let paths = expand_paths(dir, &args.paths, args.recursive, config)?;
    let entries = library::load(&paths);
    let readable: Vec<_> =
        entries.iter().filter(|e| e.error.is_none()).collect();
    let albums = library::group(&entries, GroupMode::Tags);
    let mut artists = HashMap::new();
    let mut genres = HashMap::new();
    let mut formats = HashMap::new();
    let mut missing = HashMap::new();
    let mut size = 0u64;
    let mut duration = std::time::Duration::ZERO;
    let mut lossless = 0;
    for entry in &entries {
        if let Some(err) = &entry.error {
            eprintln!("{}: {err}", rel(&entry.path));
            continue;
        }
        size += entry.properties.size;
        duration += entry.properties.duration;
        *formats
            .entry(entry.properties.container.clone())
            .or_insert(0) += 1;
        if matches!(
            entry.properties.container.as_str(),
            "flac" | "wavpack" | "ape"
        ) {
            lossless += 1;
        }
        *artists.entry(entry.album_artist()).or_insert(0) += 1;
        for genre in &entry.metadata.genres {
            *genres.entry(genre.clone()).or_insert(0) += 1;
        }
        if entry.metadata.title.trim().is_empty() {
            *missing.entry("title").or_insert(0) += 1;
        }
        if entry.metadata.artists.is_empty() {
            *missing.entry("artist").or_insert(0) += 1;
        }
        if entry.metadata.album.trim().is_empty() {
            *missing.entry("album").or_insert(0) += 1;
        }
        if !entry.properties.has_cover() {
            *missing.entry("cover").or_insert(0) += 1;
        }
    }
    let top_counts = |map: HashMap<String, usize>| -> Vec<serde_json::Value> {
        let mut items: Vec<_> = map.into_iter().collect();
        items.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
        items
            .into_iter()
            .take(args.top)
            .map(|(name, count)| serde_json::json!({"name": name, "count": count}))
            .collect()
    };
    let report = serde_json::json!({
        "tracks": entries.len(),
        "readable": readable.len(),
        "unreadable": entries.len() - readable.len(),
        "albums": albums.len(),
        "artists": artists.len(),
        "durationMs": duration.as_millis(),
        "size": size,
        "formats": formats,
        "lossless": lossless,
        "missing": missing,
        "topArtists": top_counts(artists),
        "topGenres": top_counts(genres),
    });
    if args.json {
        println!("{}", serde_json::to_string_pretty(&report)?);
        return Ok(());
    }
    println!("tracks      {}", entries.len());
    println!("readable    {}", readable.len());
    println!("unreadable  {}", entries.len() - readable.len());
    println!("albums      {}", albums.len());
    println!("artists     {}", report["artists"]);
    println!("duration    {}", library::duration_label(duration));
    println!("size        {}", library::size_label(size));
    if !readable.is_empty() {
        println!(
            "lossless    {lossless} ({}%)",
            lossless * 100 / readable.len()
        );
    }
    for (title, value) in [
        ("formats", &report["formats"]),
        ("missing", &report["missing"]),
    ] {
        if let Some(map) = value.as_object() {
            if !map.is_empty() {
                println!("\n{title}");
                for (name, count) in map {
                    println!("  {:<14} {count}", name);
                }
            }
        }
    }
    for (title, key) in
        [("top artists", "topArtists"), ("top genres", "topGenres")]
    {
        if let Some(items) = report[key].as_array() {
            if items.is_empty() {
                continue;
            }
            println!("\n{title}");
            for item in items {
                println!(
                    "  {:<24} {}",
                    item["name"].as_str().unwrap_or(""),
                    item["count"]
                );
            }
        }
    }
    Ok(())
}

fn check(
    args: &CheckArgs,
    dir: Option<PathBuf>,
    config: &Config,
) -> Result<()> {
    let paths = expand_paths(dir, &args.paths, args.recursive, config)?;
    let entries = library::load(&paths);
    let mut issues: Vec<serde_json::Value> = Vec::new();
    let mut add = |path: &Path, message: String| {
        issues.push(serde_json::json!({"path": path, "message": message}));
    };
    for entry in &entries {
        if let Some(err) = &entry.error {
            add(&entry.path, format!("unreadable metadata: {err}"));
            continue;
        }
        if entry.metadata.title.trim().is_empty() {
            add(&entry.path, "missing title".into());
        }
        if entry.metadata.artists.is_empty() {
            add(&entry.path, "missing artist".into());
        }
        if entry.metadata.album.trim().is_empty() {
            add(&entry.path, "missing album".into());
        }
        if entry.metadata.year == 0 {
            add(&entry.path, "missing year".into());
        }
        if entry.metadata.track == 0 {
            add(&entry.path, "missing track number".into());
        }
        if entry.metadata.track_total > 0
            && entry.metadata.track > entry.metadata.track_total
        {
            add(&entry.path, "track exceeds total".into());
        }
        if !entry.properties.has_cover() {
            add(&entry.path, "missing cover art".into());
        }
    }
    for album in library::group(&entries, GroupMode::Folder) {
        let mut seen: HashMap<(u32, u32), &Path> = HashMap::new();
        let mut album_artist = "";
        let mut album_title = "";
        let mut album_year = 0;
        for index in &album.tracks {
            let entry = &entries[*index];
            if entry.error.is_some() {
                continue;
            }
            if entry.metadata.track > 0 {
                let key = (entry.metadata.disc, entry.metadata.track);
                if let Some(previous) = seen.get(&key) {
                    add(
                        &entry.path,
                        format!(
                            "duplicate track number {} (also {})",
                            entry.metadata.track,
                            previous.display()
                        ),
                    );
                } else {
                    seen.insert(key, &entry.path);
                }
            }
            let artist = if entry.metadata.album_artist.trim().is_empty() {
                entry.artist()
            } else {
                entry.metadata.album_artist.clone()
            };
            let title = entry.metadata.album.trim();
            if album_artist.is_empty() {
                album_artist = Box::leak(artist.into_boxed_str());
                album_title = Box::leak(title.to_string().into_boxed_str());
                album_year = entry.metadata.year;
            } else {
                if !artist.eq_ignore_ascii_case(album_artist) {
                    add(
                        &entry.path,
                        format!(
                            "album artist {artist:?} differs from {album_artist:?}"
                        ),
                    );
                }
                if !title.eq_ignore_ascii_case(album_title) {
                    add(
                        &entry.path,
                        format!(
                            "album {:?} differs from {:?}",
                            title, album_title
                        ),
                    );
                }
                if entry.metadata.year != 0
                    && album_year != 0
                    && entry.metadata.year != album_year
                {
                    add(
                        &entry.path,
                        format!(
                            "year {} differs from {}",
                            entry.metadata.year, album_year
                        ),
                    );
                }
            }
        }
    }
    if args.json {
        println!("{}", serde_json::to_string_pretty(&issues)?);
    } else {
        for issue in &issues {
            println!(
                "{}: {}",
                issue["path"].as_str().unwrap_or(""),
                issue["message"].as_str().unwrap_or("")
            );
        }
    }
    if !issues.is_empty() {
        bail!("{} issue(s) found", issues.len());
    }
    Ok(())
}

fn lookup(
    args: &LookupArgs,
    dir: Option<PathBuf>,
    config: &Config,
) -> Result<()> {
    let client = Client::new(&config.musicbrainz_user_agent);
    if let Some(release) = &args.release {
        let detail = client.get_release(release)?;
        if args.json {
            println!("{}", serde_json::to_string_pretty(&detail)?);
            return Ok(());
        }
        write_release(&detail);
        return Ok(());
    }
    if !args.artist.is_empty() || !args.album.is_empty() {
        let releases =
            client.search_releases(&args.artist, &args.album, args.limit)?;
        return write_releases(&releases, args.json);
    }
    let paths = expand_paths(dir, &args.paths, args.recursive, config)?;
    let entries = library::load(&paths);
    let albums = library::group(&entries, GroupMode::Tags);
    let mut all = Vec::new();
    for album in &albums {
        let releases =
            client.search_releases(&album.artist, &album.title, args.limit)?;
        all.extend(releases);
    }
    write_releases(&all, args.json)
}

fn write_releases(
    releases: &[crate::online::Release],
    json: bool,
) -> Result<()> {
    if json {
        println!("{}", serde_json::to_string_pretty(releases)?);
        return Ok(());
    }
    for (index, release) in releases.iter().enumerate() {
        println!(
            "{}. {} — {}",
            index + 1,
            release.artist_name(),
            release.title
        );
        let mut detail = release.date.clone();
        if !release.country.is_empty() {
            detail.push_str(&format!(" · {}", release.country));
        }
        if release.track_count > 0 {
            detail.push_str(&format!(" · {} tracks", release.track_count));
        }
        println!("   {}", detail.trim_start_matches(" · "));
        println!("   MBID {}", release.id);
    }
    if releases.is_empty() {
        println!("no releases found");
    }
    Ok(())
}

fn write_release(detail: &crate::online::ReleaseDetail) {
    println!("{} — {}", detail.artist_name(), detail.title());
    if !detail.date().is_empty() {
        println!("  {}", detail.date());
    }
    if let Some(label) = detail.labels.first() {
        let name = label.label_name();
        if !name.is_empty() {
            println!(
                "  {name}{}",
                if label.catalog_number.is_empty() {
                    String::new()
                } else {
                    format!(" · {}", label.catalog_number)
                }
            );
        }
    }
    println!("  MBID {}", detail.release.id);
    println!();
    for medium in &detail.media {
        for track in &medium.tracks {
            let length = if track.length > 0 {
                format!(
                    " {}",
                    library::duration_label(std::time::Duration::from_millis(
                        track.length
                    ))
                )
            } else {
                String::new()
            };
            let number = if detail.media.len() > 1 {
                format!("{}-{}", medium.position, track.number)
            } else {
                track.number.clone()
            };
            println!("  {:<6} {}{length}", number, track.title);
        }
    }
}

fn themes(args: &ThemesArgs, config: &Config) -> Result<()> {
    let active = theme::canonical_theme(&config.theme);
    let themes = theme::available_themes();
    if args.json {
        let values: Vec<_> = themes
            .iter()
            .map(|(id, name)| serde_json::json!({"id": id, "name": name, "active": id == &active}))
            .collect();
        println!("{}", serde_json::to_string_pretty(&values)?);
    } else {
        for (id, name) in themes {
            println!("{} {}", if id == active { "*" } else { " " }, name);
        }
    }
    Ok(())
}

fn completion(args: &CompletionArgs) -> Result<()> {
    let mut command = Cli::command();
    clap_complete::generate(
        args.shell,
        &mut command,
        "tuigr",
        &mut std::io::stdout(),
    );
    Ok(())
}

fn man(args: &ManArgs) -> Result<()> {
    let command = Cli::command();
    if let Some(dir) = &args.dir {
        fs::create_dir_all(dir)?;
        clap_mangen::generate_to(command, dir)?;
    } else {
        let mut buffer = Vec::new();
        clap_mangen::Man::new(command).render(&mut buffer)?;
        std::io::stdout().write_all(&buffer)?;
    }
    Ok(())
}

fn truncate(value: &str, width: usize) -> String {
    if value.chars().count() <= width {
        return value.to_string();
    }
    if width == 0 {
        return String::new();
    }
    let mut out: String = value.chars().take(width.saturating_sub(1)).collect();
    out.push('…');
    out
}
