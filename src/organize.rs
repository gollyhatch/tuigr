//! Rename and move audio files after their tags.

use std::fs;
use std::path::{Path, PathBuf};

use crate::library::Entry;
use crate::tags::{Field, Metadata, Properties};

#[derive(Clone, Debug, Default)]
pub struct Options {
    pub pattern: String,
    pub root: Option<PathBuf>,
    pub library: Option<PathBuf>,
    pub slugify: bool,
    pub move_files: bool,
    pub dry_run: bool,
}

#[derive(Clone, Debug)]
pub struct RenameResult {
    pub source: PathBuf,
    pub target: PathBuf,
}

#[derive(Clone, Debug, Default)]
pub struct RenameReport {
    pub results: Vec<RenameResult>,
    pub errors: Vec<String>,
}

/// Rename files according to a pattern. Existing targets are never overwritten.
pub fn rename(entries: &[Entry], options: &Options) -> RenameReport {
    let pattern = if options.pattern.trim().is_empty() {
        "%n - %t"
    } else {
        &options.pattern
    };
    let mut report = RenameReport::default();
    if options.move_files && options.library.is_none() {
        report
            .errors
            .push("--move requires a configured library".into());
        return report;
    }
    let mut claimed: std::collections::HashMap<PathBuf, PathBuf> =
        Default::default();
    for entry in entries {
        let expanded = expand_pattern(
            pattern,
            &entry.metadata,
            &entry.properties,
            &entry.path,
        );
        let cleaned = clean_path(&apply_slug(&expanded, options.slugify));
        let cleaned = match cleaned {
            Ok(value) => value,
            Err(err) => {
                report
                    .errors
                    .push(format!("{}: {err}", entry.path.display()));
                continue;
            }
        };
        let mut target = if options.move_files {
            options.library.clone().unwrap_or_default().join(&cleaned)
        } else if let Some(root) = &options.root {
            root.join(&cleaned)
        } else {
            entry
                .path
                .parent()
                .unwrap_or_else(|| Path::new("."))
                .join(&cleaned)
        };
        if target.extension().is_none() {
            if let Some(ext) = entry.path.extension() {
                target.set_extension(ext);
            }
        }
        if same_path(&entry.path, &target) {
            continue;
        }
        if let Some(previous) = claimed.get(&target) {
            report.errors.push(format!(
                "{}: target {} already claimed by {}",
                entry.path.display(),
                target.display(),
                previous.display()
            ));
            continue;
        }
        claimed.insert(target.clone(), entry.path.clone());
        if target.exists() {
            report.errors.push(format!(
                "{}: target exists: {}",
                entry.path.display(),
                target.display()
            ));
            continue;
        }
        if !options.dry_run {
            if let Some(parent) = target.parent() {
                if let Err(err) = fs::create_dir_all(parent) {
                    report
                        .errors
                        .push(format!("{}: {err}", entry.path.display()));
                    continue;
                }
            }
            if let Err(err) = fs::rename(&entry.path, &target) {
                if options.move_files {
                    if let Err(copy_err) = move_file(&entry.path, &target) {
                        report.errors.push(format!(
                            "{}: {err}; copy fallback: {copy_err}",
                            entry.path.display()
                        ));
                        continue;
                    }
                } else {
                    report
                        .errors
                        .push(format!("{}: {err}", entry.path.display()));
                    continue;
                }
            }
        }
        report.results.push(RenameResult {
            source: entry.path.clone(),
            target,
        });
    }
    report
}

/// Expand rename placeholders.
pub fn expand_pattern(
    pattern: &str,
    meta: &Metadata,
    props: &Properties,
    path: &Path,
) -> String {
    let mut out = String::new();
    let bytes = pattern.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] != b'%' {
            out.push(bytes[i] as char);
            i += 1;
            continue;
        }
        if i + 1 >= bytes.len() {
            out.push('%');
            break;
        }
        if bytes[i + 1] == b'%' {
            out.push('%');
            i += 2;
            continue;
        }
        let start = i + 1;
        let mut end = start;
        while end < bytes.len()
            && ((bytes[end] as char).is_ascii_alphanumeric()
                || bytes[end] == b'-')
        {
            end += 1;
        }
        let mut word = &pattern[start..end];
        // Patterns such as %d-%n should treat the hyphen as a literal
        // separator rather than part of the placeholder name.
        if field_for_word(word).is_none() && word.len() > 1 {
            let first = &word[..1];
            if field_for_word(first).is_some() || first == "f" {
                word = first;
                end = start + 1;
            }
        }
        let value = rename_value(meta, props, path, word);
        out.push_str(&sanitize_name(&value));
        i = end;
    }
    cleanup_pattern(out)
}

fn field_for_word(word: &str) -> Option<Field> {
    if word.len() == 1 {
        match word {
            "t" => Some(Field::Title),
            "a" => Some(Field::Artist),
            "A" => Some(Field::AlbumArtist),
            "L" => Some(Field::Album),
            "n" => Some(Field::Track),
            "N" => Some(Field::TrackTotal),
            "d" => Some(Field::Disc),
            "D" => Some(Field::DiscTotal),
            "y" => Some(Field::Year),
            "g" => Some(Field::Genre),
            "m" => Some(Field::Composer),
            "l" => Some(Field::Publisher),
            "i" => Some(Field::Isrc),
            "c" => Some(Field::Comment),
            _ => Field::parse(word),
        }
    } else {
        Field::parse(word)
    }
}

fn rename_value(
    meta: &Metadata,
    props: &Properties,
    path: &Path,
    word: &str,
) -> String {
    let lower = word.to_ascii_lowercase();
    if lower == "f" || lower == "format" || lower == "ext" {
        return if props.container.is_empty() {
            path.extension()
                .map(|e| e.to_string_lossy().to_ascii_lowercase())
                .unwrap_or_default()
        } else {
            props.container.to_ascii_lowercase()
        };
    }
    let Some(field) = field_for_word(word) else {
        return String::new();
    };
    match field {
        Field::Title => {
            if !meta.title.trim().is_empty() {
                meta.title.clone()
            } else {
                path.file_stem()
                    .map(|s| s.to_string_lossy().to_string())
                    .unwrap_or_default()
            }
        }
        Field::Artist => {
            if meta.artists.is_empty() {
                "Unknown Artist".into()
            } else {
                meta.artist_string()
            }
        }
        Field::AlbumArtist => {
            if !meta.album_artist.is_empty() {
                meta.album_artist.clone()
            } else if meta.artists.is_empty() {
                "Unknown Artist".into()
            } else {
                meta.artist_string()
            }
        }
        Field::Album => {
            if !meta.album.is_empty() {
                meta.album.clone()
            } else {
                path.parent()
                    .and_then(|p| p.file_name())
                    .map(|s| s.to_string_lossy().to_string())
                    .unwrap_or_default()
            }
        }
        Field::Track => pad_number(meta.track, meta.track_total),
        Field::TrackTotal => number(meta.track_total),
        Field::Disc => pad_number(meta.disc, meta.disc_total),
        Field::DiscTotal => number(meta.disc_total),
        other => meta.get(other),
    }
}

fn pad_number(value: u32, total: u32) -> String {
    if value == 0 {
        return String::new();
    }
    let width = if total > 0 {
        total.to_string().len().max(2)
    } else {
        2
    };
    format!("{value:0width$}")
}

fn number(value: u32) -> String {
    if value == 0 {
        String::new()
    } else {
        value.to_string()
    }
}

fn cleanup_pattern(value: String) -> String {
    let mut value = value.replace("//", "/");
    value = value.trim().trim_matches('/').to_string();
    value = value.replace("/ - ", "/");
    value = value.trim_start_matches(" - ").to_string();
    while value.contains(" - - ") {
        value = value.replace(" - - ", " - ");
    }
    value
}

fn apply_slug(value: &str, slugify: bool) -> String {
    if !slugify {
        return value.to_string();
    }
    value
        .split('/')
        .map(slugify_segment)
        .collect::<Vec<_>>()
        .join("/")
}

pub fn slugify(value: &str) -> String {
    let value = value
        .replace('ä', "ae")
        .replace('ö', "oe")
        .replace('ü', "ue")
        .replace('Ä', "Ae")
        .replace('Ö', "Oe")
        .replace('Ü', "Ue")
        .replace('ß', "ss")
        .replace('æ', "ae")
        .replace('ø', "o")
        .replace('å', "a")
        .replace('œ', "oe");
    let value = deunicode::deunicode(&value).to_ascii_lowercase();
    let mut out = String::new();
    let mut dash = false;
    for ch in value.chars() {
        if ch.is_ascii_alphanumeric() {
            out.push(ch);
            dash = false;
        } else if !dash && !out.is_empty() {
            out.push('-');
            dash = true;
        }
    }
    out.trim_matches('-').to_string()
}

fn slugify_segment(value: &str) -> String {
    slugify(value)
}

fn sanitize_name(value: &str) -> String {
    let mut out = String::new();
    for ch in value.trim().chars() {
        match ch {
            '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|' | '\0' => {
                out.push('-')
            }
            c if c.is_control() => {}
            c => out.push(c),
        }
    }
    let out = out.split_whitespace().collect::<Vec<_>>().join(" ");
    let out = out.trim_matches(|c| c == ' ' || c == '.').to_string();
    let mut cleaned = out;
    while cleaned.contains("--") {
        cleaned = cleaned.replace("--", "-");
    }
    cleaned
}

fn clean_path(value: &str) -> Result<String, String> {
    let value = value.replace('\\', "/");
    let parts: Vec<&str> = value
        .split('/')
        .map(str::trim)
        .filter(|p| !p.is_empty() && *p != ".")
        .collect();
    if parts.contains(&"..") {
        return Err("rename pattern escapes its directory".into());
    }
    if parts.is_empty() {
        return Err("rename pattern expands to an empty name".into());
    }
    Ok(parts.join("/"))
}

fn same_path(a: &Path, b: &Path) -> bool {
    match (a.canonicalize(), b.canonicalize()) {
        (Ok(a), Ok(b)) => a == b,
        _ => a == b,
    }
}

fn move_file(source: &Path, target: &Path) -> std::io::Result<()> {
    fs::copy(source, target)?;
    fs::remove_file(source)
}

pub fn under(path: &Path, root: &Path) -> bool {
    let path = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
    let root = root.canonicalize().unwrap_or_else(|_| root.to_path_buf());
    path.starts_with(root)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slugify_values() {
        assert_eq!(slugify("Jürgen Müller"), "juergen-mueller");
        assert_eq!(slugify("Straße"), "strasse");
        assert_eq!(slugify("Hello, World!"), "hello-world");
    }

    #[test]
    fn expands_placeholders() {
        let meta = Metadata {
            title: "Song".into(),
            artists: vec!["Artist".into()],
            album: "Album".into(),
            album_artist: "Band".into(),
            track: 3,
            track_total: 12,
            disc: 1,
            disc_total: 2,
            year: 1993,
            ..Default::default()
        };
        let props = Properties::default();
        let value = expand_pattern(
            "%A/%L/%d-%n - %t",
            &meta,
            &props,
            Path::new("x.flac"),
        );
        assert_eq!(value, "Band/Album/01-03 - Song");
    }
}
