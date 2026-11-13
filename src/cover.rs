//! Album artwork downloads and MusicBrainz tracklist mapping.

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};

use crate::library::Entry;
use crate::online::{Client, ReleaseDetail, choose_release, search_label};
use crate::tags::DEFAULT_COVER;

#[derive(Clone, Debug)]
pub struct DownloadResult {
    pub path: PathBuf,
    pub release_id: String,
}

pub fn download_album_cover(
    client: &Client,
    dir: &Path,
    artist: &str,
    album: &str,
    filename: &str,
) -> Result<DownloadResult> {
    if album.trim().is_empty() {
        bail!("album title is required");
    }
    let releases = client.search_releases(artist, album, 8)?;
    let release =
        choose_release(&releases, artist, album).with_context(|| {
            format!(
                "no MusicBrainz release found for {}",
                search_label(artist, album)
            )
        })?;
    download_release_cover(client, dir, &release.id, filename)
}

pub fn download_release_cover(
    client: &Client,
    dir: &Path,
    release_id: &str,
    filename: &str,
) -> Result<DownloadResult> {
    if release_id.trim().is_empty() {
        bail!("release ID is required");
    }
    let (data, mime) = client
        .download_cover(release_id, 500)
        .or_else(|_| client.download_cover(release_id, 0))?;
    if !mime.starts_with("image/") {
        bail!("cover art archive returned {mime}");
    }
    let path = write_cover(dir, filename, &data)?;
    Ok(DownloadResult {
        path,
        release_id: release_id.to_string(),
    })
}

pub fn write_cover(dir: &Path, filename: &str, data: &[u8]) -> Result<PathBuf> {
    let name = Path::new(filename.trim())
        .file_name()
        .and_then(|s| s.to_str())
        .filter(|s| !s.is_empty())
        .unwrap_or(DEFAULT_COVER);
    fs::create_dir_all(dir)?;
    let target = dir.join(name);
    let tmp = dir.join(format!(".tuigr-cover-{}.tmp", std::process::id()));
    fs::write(&tmp, data)?;
    fs::rename(&tmp, &target)?;
    Ok(target)
}

#[derive(Clone, Debug, Default)]
pub struct TrackAssignment {
    pub title: String,
    pub track: u32,
    pub track_total: u32,
    pub disc: u32,
    pub disc_total: u32,
    pub set_disc: bool,
}

#[derive(Clone, Debug)]
struct ReleaseTrack {
    medium: u32,
    medium_total: u32,
    number: u32,
    total: u32,
    title: String,
}

pub fn plan_release_tracks(
    entries: &[Entry],
    detail: &ReleaseDetail,
) -> HashMap<PathBuf, TrackAssignment> {
    let mut out = HashMap::new();
    if detail.media.is_empty() {
        return out;
    }
    let mut tracks: Vec<ReleaseTrack> = Vec::new();
    for (index, medium) in detail.media.iter().enumerate() {
        let medium_position = if medium.position == 0 {
            index as u32 + 1
        } else {
            medium.position
        };
        let total = if medium.tracks.is_empty() {
            medium.track_count
        } else {
            medium.tracks.len() as u32
        };
        for (track_index, track) in medium.tracks.iter().enumerate() {
            let mut number = track.position;
            if number == 0 {
                number = track.number.parse::<u32>().unwrap_or(0);
            }
            if number == 0 {
                number = track_index as u32 + 1;
            }
            let title = if track.title.trim().is_empty() {
                track.recording.title.trim().to_string()
            } else {
                track.title.trim().to_string()
            };
            tracks.push(ReleaseTrack {
                medium: medium_position,
                medium_total: detail.media.len() as u32,
                number,
                total,
                title,
            });
        }
    }
    if tracks.is_empty() {
        return out;
    }

    let mut order: Vec<usize> = (0..entries.len()).collect();
    order.sort_by(|a, b| {
        let ea = &entries[*a];
        let eb = &entries[*b];
        if ea.metadata.track > 0 && eb.metadata.track > 0 {
            disc(ea.metadata.disc)
                .cmp(&disc(eb.metadata.disc))
                .then(ea.metadata.track.cmp(&eb.metadata.track))
        } else {
            ea.path.cmp(&eb.path)
        }
    });

    let mut used_local = vec![false; entries.len()];
    let mut used_tracks = vec![false; tracks.len()];
    let mut by_title: HashMap<String, Vec<usize>> = HashMap::new();
    for index in &order {
        let key = normalize_title(&entries[*index].metadata.title);
        let key = if key.is_empty() {
            normalize_title(&entries[*index].title())
        } else {
            key
        };
        if !key.is_empty() {
            by_title.entry(key).or_default().push(*index);
        }
    }

    for (track_index, track) in tracks.iter().enumerate() {
        let key = normalize_title(&track.title);
        if key.is_empty() {
            continue;
        }
        if let Some(candidates) = by_title.get(&key) {
            for local in candidates {
                if !used_local[*local] {
                    assign(&mut out, &mut used_local, entries, *local, track);
                    used_tracks[track_index] = true;
                    break;
                }
            }
        }
    }
    let mut next = 0;
    for (track_index, track) in tracks.iter().enumerate() {
        if used_tracks[track_index] {
            continue;
        }
        while next < order.len() && used_local[order[next]] {
            next += 1;
        }
        if next >= order.len() {
            break;
        }
        let local = order[next];
        assign(&mut out, &mut used_local, entries, local, track);
        next += 1;
    }
    out
}

fn assign(
    out: &mut HashMap<PathBuf, TrackAssignment>,
    used_local: &mut [bool],
    entries: &[Entry],
    local: usize,
    track: &ReleaseTrack,
) {
    used_local[local] = true;
    out.insert(
        entries[local].path.clone(),
        TrackAssignment {
            title: track.title.clone(),
            track: track.number,
            track_total: track.total,
            disc: track.medium,
            disc_total: track.medium_total,
            set_disc: track.medium_total > 1,
        },
    );
}

fn disc(value: u32) -> u32 {
    if value == 0 { 1 } else { value }
}

fn normalize_title(value: &str) -> String {
    value
        .to_ascii_lowercase()
        .chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .collect()
}

pub fn track_number_value(track: u32, total: u32) -> String {
    if total > 0 {
        format!("{track}/{total}")
    } else {
        track.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::online::{Medium, Release, ReleaseDetail, Track};
    use crate::tags::Metadata;

    fn entry(path: &str, title: &str, track: u32, disc: u32) -> Entry {
        Entry {
            path: PathBuf::from(path),
            metadata: Metadata {
                title: title.into(),
                track,
                disc,
                ..Default::default()
            },
            properties: Default::default(),
            error: None,
        }
    }

    fn release(media: Vec<Vec<&str>>) -> ReleaseDetail {
        ReleaseDetail {
            release: Release {
                title: "Album".into(),
                ..Default::default()
            },
            media: media
                .into_iter()
                .enumerate()
                .map(|(index, titles)| Medium {
                    position: index as u32 + 1,
                    track_count: titles.len() as u32,
                    tracks: titles
                        .into_iter()
                        .enumerate()
                        .map(|(track_index, title)| Track {
                            title: title.into(),
                            position: track_index as u32 + 1,
                            ..Default::default()
                        })
                        .collect(),
                    ..Default::default()
                })
                .collect(),
            labels: Vec::new(),
        }
    }

    #[test]
    fn maps_by_title_then_order() {
        let entries = vec![
            entry("/a/01.flac", "Two", 0, 0),
            entry("/a/02.flac", "One", 0, 0),
            entry("/a/03.flac", "Three", 0, 0),
        ];
        let plan = plan_release_tracks(
            &entries,
            &release(vec![vec!["One", "Two", "Three"]]),
        );
        assert_eq!(plan[Path::new("/a/02.flac")].track, 1);
        assert_eq!(plan[Path::new("/a/01.flac")].track, 2);
        assert_eq!(plan[Path::new("/a/03.flac")].track, 3);
        assert!(!plan[Path::new("/a/01.flac")].set_disc);
    }

    #[test]
    fn maps_multi_disc() {
        let entries = vec![
            entry("/a/d1/01.flac", "One", 1, 1),
            entry("/a/d1/02.flac", "Two", 2, 1),
            entry("/a/d2/01.flac", "Three", 1, 2),
            entry("/a/d2/02.flac", "Four", 2, 2),
        ];
        let plan = plan_release_tracks(
            &entries,
            &release(vec![vec!["One", "Two"], vec!["Three", "Four"]]),
        );
        assert_eq!(plan[Path::new("/a/d2/01.flac")].disc, 2);
        assert!(plan[Path::new("/a/d2/01.flac")].set_disc);
        assert_eq!(plan[Path::new("/a/d1/02.flac")].track_total, 2);
    }
}
