//! MusicBrainz and Cover Art Archive client.

use std::sync::Mutex;
use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};

#[derive(Debug)]
pub struct Client {
    http: reqwest::blocking::Client,
    pub user_agent: String,
    pub base_url: String,
    pub cover_base_url: String,
    rate_limit: Duration,
    last: Mutex<Option<Instant>>,
}

impl Client {
    pub fn new(user_agent: &str) -> Self {
        let user_agent = if user_agent.trim().is_empty() {
            "tuigr/0.2 (https://github.com/opencode/tuigr)"
        } else {
            user_agent.trim()
        };
        Self {
            http: reqwest::blocking::Client::builder()
                .timeout(Duration::from_secs(20))
                .build()
                .expect("http client"),
            user_agent: user_agent.to_string(),
            base_url: "https://musicbrainz.org".into(),
            cover_base_url: "https://coverartarchive.org".into(),
            rate_limit: Duration::from_secs(1),
            last: Mutex::new(None),
        }
    }

    fn wait(&self) -> Result<()> {
        let mut last = self.last.lock().unwrap();
        if let Some(previous) = *last {
            let elapsed = previous.elapsed();
            if elapsed < self.rate_limit {
                std::thread::sleep(self.rate_limit - elapsed);
            }
        }
        *last = Some(Instant::now());
        Ok(())
    }

    pub fn search_releases(
        &self,
        artist: &str,
        album: &str,
        limit: usize,
    ) -> Result<Vec<Release>> {
        if album.trim().is_empty() && artist.trim().is_empty() {
            bail!("nothing to search for");
        }
        let mut terms = Vec::new();
        if !album.trim().is_empty() {
            terms.push(format!("release:\"{}\"", escape_query(album)));
        }
        if !artist.trim().is_empty() {
            terms.push(format!("artist:\"{}\"", escape_query(artist)));
        }
        let query = url::form_urlencoded::Serializer::new(String::new())
            .append_pair("query", &terms.join(" AND "))
            .append_pair("fmt", "json")
            .append_pair("limit", &limit.max(1).to_string())
            .finish();
        let url = format!("{}/ws/2/release/?{}", self.base_url, query);
        let response: SearchResponse = self.get_json(&url)?;
        Ok(response.releases)
    }

    pub fn get_release(&self, mbid: &str) -> Result<ReleaseDetail> {
        let url = format!(
            "{}/ws/2/release/{}?inc=recordings+artist-credits+labels+release-groups&fmt=json",
            self.base_url,
            url::form_urlencoded::byte_serialize(mbid.as_bytes())
                .collect::<String>()
        );
        self.get_json(&url)
    }

    pub fn download_cover(
        &self,
        release_mbid: &str,
        size: u32,
    ) -> Result<(Vec<u8>, String)> {
        self.wait()?;
        let suffix = if size > 0 {
            format!("front-{size}")
        } else {
            "front".to_string()
        };
        let url = format!(
            "{}/release/{}/{}",
            self.cover_base_url,
            url::form_urlencoded::byte_serialize(release_mbid.as_bytes())
                .collect::<String>(),
            suffix
        );
        let response = self
            .http
            .get(&url)
            .header(reqwest::header::USER_AGENT, &self.user_agent)
            .send()
            .with_context(|| format!("cover art archive: {url}"))?;
        if !response.status().is_success() {
            bail!("cover art archive: {}", response.status());
        }
        let mime = response
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .unwrap_or("image/jpeg")
            .split(';')
            .next()
            .unwrap_or("image/jpeg")
            .trim()
            .to_string();
        let data = response.bytes()?.to_vec();
        Ok((data, mime))
    }

    fn get_json<T: for<'de> Deserialize<'de>>(&self, url: &str) -> Result<T> {
        self.wait()?;
        let response = self
            .http
            .get(url)
            .header(reqwest::header::USER_AGENT, &self.user_agent)
            .header(reqwest::header::ACCEPT, "application/json")
            .send()
            .with_context(|| format!("musicbrainz: {url}"))?;
        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().unwrap_or_default();
            bail!("musicbrainz: {status}: {}", body.trim());
        }
        Ok(response.json()?)
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ArtistCredit {
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub artist: Artist,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Artist {
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub name: String,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Release {
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub date: String,
    #[serde(default)]
    pub country: String,
    #[serde(default)]
    pub status: String,
    #[serde(rename = "artist-credit", default)]
    pub artist_credit: Vec<ArtistCredit>,
    #[serde(rename = "track-count", default)]
    pub track_count: u32,
    #[serde(default)]
    pub media: Vec<Medium>,
}

impl Release {
    pub fn artist_name(&self) -> String {
        self.artist_credit
            .iter()
            .map(|credit| {
                let name = credit.name.trim();
                if name.is_empty() {
                    credit.artist.name.trim()
                } else {
                    name
                }
            })
            .filter(|name| !name.is_empty())
            .collect::<Vec<_>>()
            .join(", ")
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Label {
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub label: LabelName,
    #[serde(rename = "catalog-number", default)]
    pub catalog_number: String,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct LabelName {
    #[serde(default)]
    pub name: String,
}

impl Label {
    pub fn label_name(&self) -> &str {
        if !self.label.name.is_empty() {
            &self.label.name
        } else {
            &self.name
        }
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Medium {
    #[serde(default)]
    pub position: u32,
    #[serde(default)]
    pub format: String,
    #[serde(rename = "track-count", default)]
    pub track_count: u32,
    #[serde(default)]
    pub tracks: Vec<Track>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Track {
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub number: String,
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub position: u32,
    #[serde(default)]
    pub length: u64,
    #[serde(default)]
    pub recording: Recording,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Recording {
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub title: String,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct ReleaseDetail {
    #[serde(flatten)]
    pub release: Release,
    #[serde(default)]
    pub media: Vec<Medium>,
    #[serde(rename = "label-info", default)]
    pub labels: Vec<Label>,
}

impl ReleaseDetail {
    pub fn artist_name(&self) -> String {
        self.release.artist_name()
    }
    pub fn title(&self) -> &str {
        &self.release.title
    }
    pub fn date(&self) -> &str {
        &self.release.date
    }
}

#[derive(Deserialize)]
struct SearchResponse {
    #[serde(default)]
    releases: Vec<Release>,
}

pub fn escape_query(value: &str) -> String {
    value.replace('\\', "\\\\").replace('"', "\\\"")
}

pub fn search_label(artist: &str, album: &str) -> String {
    if artist.is_empty() {
        album.to_string()
    } else {
        format!("{artist} — {album}")
    }
}

pub fn choose_release<'a>(
    releases: &'a [Release],
    artist: &str,
    album: &str,
) -> Option<&'a Release> {
    let album = album.trim();
    let artist = artist.trim();
    releases
        .iter()
        .find(|release| {
            release.title.eq_ignore_ascii_case(album)
                && (artist.is_empty()
                    || release
                        .artist_name()
                        .to_ascii_lowercase()
                        .contains(&artist.to_ascii_lowercase()))
        })
        .or_else(|| {
            releases
                .iter()
                .find(|release| release.title.eq_ignore_ascii_case(album))
        })
        .or_else(|| releases.first())
}
