//! Optional TOML configuration.

use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    pub library: String,
    pub player: String,
    pub sort: String,
    pub theme: String,
    pub required_tags: Vec<String>,
    pub group_by: String,
    pub search_url: String,
    pub rename_format: String,
    pub slugify: bool,
    pub sluggify: bool,
    pub cover_art: String,
    pub cover_file: String,
    pub cover_panel: bool,
    pub id3_version: String,
    pub scan_workers: usize,
    pub musicbrainz_user_agent: String,
}

impl Default for Config {
    fn default() -> Self {
        let workers = std::thread::available_parallelism()
            .map(|n| n.get())
            .unwrap_or(1)
            .min(8);
        Self {
            library: String::new(),
            player: String::new(),
            sort: "artist,album,track".into(),
            theme: "ayu-dark".into(),
            required_tags: vec![
                "artist".into(),
                "album".into(),
                "title".into(),
                "track".into(),
            ],
            group_by: "tags".into(),
            search_url:
                "https://musicbrainz.org/search?query=%a+%L&type=release".into(),
            rename_format: String::new(),
            slugify: false,
            sluggify: false,
            cover_art: "embed".into(),
            cover_file: "albumart.jpg".into(),
            cover_panel: false,
            id3_version: "preserve".into(),
            scan_workers: workers.max(1),
            musicbrainz_user_agent:
                "tuigr/0.2 (https://github.com/opencode/tuigr)".into(),
        }
    }
}

impl Config {
    pub fn path() -> Result<PathBuf> {
        let base = dirs::config_dir()
            .context("cannot locate the configuration directory")?;
        Ok(base.join("tuigr").join("config.toml"))
    }

    pub fn load() -> Result<Self> {
        Self::load_file(&Self::path()?)
    }

    pub fn load_file(path: &Path) -> Result<Self> {
        let text = match fs::read_to_string(path) {
            Ok(text) => text,
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
                return Ok(Self::default());
            }
            Err(err) => {
                return Err(err)
                    .with_context(|| format!("config {}", path.display()));
            }
        };
        let mut config: Self = toml::from_str(&text)
            .with_context(|| format!("config {}", path.display()))?;
        config.library = expand_home(config.library.trim());
        config.player = config.player.trim().to_string();
        config.sort = config.sort.trim().to_string();
        config.theme = config.theme.trim().to_string();
        config.group_by = config.group_by.trim().to_string();
        config.search_url = config.search_url.trim().to_string();
        config.rename_format = config.rename_format.trim().to_string();
        config.cover_art = config.cover_art.trim().to_string();
        config.cover_file = config.cover_file.trim().to_string();
        config.id3_version = config.id3_version.trim().to_string();
        config.musicbrainz_user_agent =
            config.musicbrainz_user_agent.trim().to_string();
        config.required_tags = config
            .required_tags
            .iter()
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect();
        if config.cover_file.is_empty() {
            config.cover_file = crate::tags::DEFAULT_COVER.into();
        }
        if config.theme.is_empty() {
            config.theme = "ayu-dark".into();
        }
        if config.sort.is_empty() {
            config.sort = "artist,album,track".into();
        }
        if config.scan_workers == 0 {
            config.scan_workers = Self::default().scan_workers;
        }
        Ok(config)
    }

    pub fn library_dir(&self, fallback: &Path) -> PathBuf {
        if self.library.is_empty() {
            fallback.to_path_buf()
        } else {
            PathBuf::from(&self.library)
        }
    }

    pub fn slugify_rename(&self) -> bool {
        self.slugify || self.sluggify
    }

    pub fn embed_cover(&self) -> bool {
        !matches!(
            self.cover_art.to_ascii_lowercase().as_str(),
            "folder" | "file" | "folder-only"
        )
    }

    pub fn folder_cover(&self) -> bool {
        matches!(
            self.cover_art.to_ascii_lowercase().as_str(),
            "folder"
                | "file"
                | "folder-only"
                | "both"
                | "embed+folder"
                | "folder+embed"
        )
    }
}

fn expand_home(value: &str) -> String {
    if value == "~" {
        return dirs::home_dir()
            .map(|p| p.to_string_lossy().to_string())
            .unwrap_or_else(|| value.to_string());
    }
    if let Some(rest) = value.strip_prefix("~/") {
        if let Some(home) = dirs::home_dir() {
            return home.join(rest).to_string_lossy().to_string();
        }
    }
    value.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults() {
        let cfg = Config::default();
        assert_eq!(cfg.theme, "ayu-dark");
        assert_eq!(cfg.cover_file, "albumart.jpg");
        assert_eq!(cfg.required_tags.len(), 4);
    }

    #[test]
    fn loads_toml() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        fs::write(
            &path,
            r#"
library = "/music"
theme = "dracula"
cover_file = "art.jpg"
cover_panel = true
required_tags = ["artist", "album", "title", "track"]
"#,
        )
        .unwrap();
        let cfg = Config::load_file(&path).unwrap();
        assert_eq!(cfg.library, "/music");
        assert_eq!(cfg.theme, "dracula");
        assert_eq!(cfg.cover_file, "art.jpg");
        assert!(cfg.cover_panel);
    }
}
