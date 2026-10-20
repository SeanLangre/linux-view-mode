//! Runs yt-dlp and turns its JSON output into `Video`s.
//!
//! yt-dlp is always started directly (never through a shell), with `--` before
//! the positional argument, and is killed if it runs longer than `TIMEOUT`.

use std::time::Duration;
use std::{path::Path, process::Stdio};

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use tokio::process::Command;

const TIMEOUT: Duration = Duration::from_secs(30);
pub const MAX_QUERY_LEN: usize = 200;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Video {
    pub id: String,
    pub title: String,
    pub channel: Option<String>,
    pub channel_id: Option<String>,
    /// Seconds; `None` for live streams and some premieres.
    pub duration: Option<f64>,
    pub view_count: Option<u64>,
    pub thumb: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resume_position: Option<f64>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Channel {
    pub id: String,
    pub name: Option<String>,
    pub videos: Vec<Video>,
}

/// Searches YouTube with the selected profile and returns up to `limit` videos.
pub async fn search(query: &str, limit: u32, firefox: Option<&Path>) -> Result<Vec<Video>> {
    let json = run(&format!("ytsearch{limit}:{query}"), &[], firefox).await?;
    Ok(parse_playlist(&json)?.videos)
}

/// A channel's latest `limit` uploads. `id` must pass `is_channel_id`.
pub async fn channel(id: &str, limit: u32, firefox: Option<&Path>) -> Result<Channel> {
    debug_assert!(is_channel_id(id));
    let url = format!("https://www.youtube.com/channel/{id}/videos");
    let json = run(&url, &["--playlist-end", &limit.to_string()], firefox).await?;
    let playlist = parse_playlist(&json)?;
    Ok(Channel {
        id: id.to_string(),
        name: playlist.channel,
        videos: playlist.videos,
    })
}

impl Video {
    pub fn placeholder(id: &str) -> Self {
        Self {
            id: id.into(),
            title: format!("Video {id}"),
            channel: None,
            channel_id: None,
            duration: None,
            view_count: None,
            thumb: format!("https://i.ytimg.com/vi/{id}/hqdefault.jpg"),
            resume_position: None,
        }
    }
}

pub fn feed_target(kind: &str) -> Option<&'static str> {
    match kind {
        "recommended" => Some("https://www.youtube.com/feed/recommended"),
        "subscriptions" => Some("https://www.youtube.com/feed/subscriptions"),
        "history" => Some("https://www.youtube.com/feed/history"),
        "watchlater" => Some("https://www.youtube.com/playlist?list=WL"),
        "liked" => Some("https://www.youtube.com/playlist?list=LL"),
        _ => None,
    }
}

pub async fn feed(kind: &str, firefox: &Path) -> Result<Vec<Video>> {
    let target = feed_target(kind).context("unknown feed")?;
    let json = run(target, &["--playlist-end", "24"], Some(firefox)).await?;
    Ok(parse_playlist(&json)?.videos)
}

pub async fn mark_watched(id: &str, firefox: &Path) -> Result<()> {
    if !is_video_id(id) {
        bail!("invalid video id");
    }
    run(
        &format!("https://www.youtube.com/watch?v={id}"),
        &[
            "--no-flat-playlist",
            "--no-playlist",
            "--simulate",
            "--mark-watched",
        ],
        Some(firefox),
    )
    .await?;
    Ok(())
}

pub fn is_video_id(id: &str) -> bool {
    id.len() == 11
        && id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}

pub fn is_channel_id(id: &str) -> bool {
    id.len() == 24
        && id.starts_with("UC")
        && id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}

/// Trims the query and removes control characters. Returns `None` if nothing is left.
pub fn clean_query(raw: &str) -> Option<String> {
    let cleaned: String = raw
        .chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .take(MAX_QUERY_LEN)
        .collect();
    let trimmed = cleaned.trim();
    (!trimmed.is_empty()).then(|| trimmed.to_string())
}

async fn run(target: &str, extra_args: &[&str], firefox: Option<&Path>) -> Result<Vec<u8>> {
    let mut command = Command::new("yt-dlp");
    command.args(["--ignore-config", "--flat-playlist", "-J", "--no-warnings"]);
    if let Some(folder) = firefox {
        command
            .arg("--cookies-from-browser")
            .arg(format!("firefox:{}", folder.display()));
    }
    let child = command
        .args(extra_args)
        .arg("--")
        .arg(target)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .context("could not start yt-dlp (is it installed?)")?;

    let output = tokio::time::timeout(TIMEOUT, child.wait_with_output())
        .await
        .context("yt-dlp timed out")??;

    if !output.status.success() {
        // Avoid leaking account details from yt-dlp output into logs.
        bail!("yt-dlp exited with {}", output.status);
    }
    Ok(output.stdout)
}

pub async fn check_login(firefox: &Path) -> Result<()> {
    let json = run(":ytsubs", &["--playlist-end", "1"], Some(firefox)).await?;
    parse_playlist(&json)?;
    Ok(())
}

#[derive(Deserialize)]
struct RawPlaylist {
    channel: Option<String>,
    channel_id: Option<String>,
    #[serde(default)]
    entries: Vec<RawEntry>,
}

struct Playlist {
    channel: Option<String>,
    videos: Vec<Video>,
}

#[derive(Deserialize)]
struct RawEntry {
    id: Option<String>,
    title: Option<String>,
    channel: Option<String>,
    uploader: Option<String>,
    channel_id: Option<String>,
    duration: Option<f64>,
    view_count: Option<u64>,
    #[serde(default)]
    thumbnails: Vec<RawThumb>,
}

#[derive(Deserialize)]
struct RawThumb {
    url: String,
    width: Option<u32>,
}

fn parse_playlist(json: &[u8]) -> Result<Playlist> {
    let playlist: RawPlaylist = serde_json::from_slice(json).context("unexpected yt-dlp output")?;

    // Channel pages only name the channel once, at the top level.
    let videos = playlist
        .entries
        .into_iter()
        .filter_map(|e| {
            // Skip channels, playlists and anything else that isn't a plain video.
            let id = e.id.filter(|id| is_video_id(id))?;
            let thumb = e
                .thumbnails
                .into_iter()
                .max_by_key(|t| t.width.unwrap_or(0))
                .map(|t| t.url)
                .unwrap_or_else(|| format!("https://i.ytimg.com/vi/{id}/hqdefault.jpg"));
            Some(Video {
                title: e.title.unwrap_or_else(|| "Untitled".to_string()),
                channel: e
                    .channel
                    .or(e.uploader)
                    .or_else(|| playlist.channel.clone()),
                channel_id: e.channel_id.or_else(|| playlist.channel_id.clone()),
                duration: e.duration,
                view_count: e.view_count,
                thumb,
                id,
                resume_position: None,
            })
        })
        .collect();

    Ok(Playlist {
        channel: playlist.channel,
        videos,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn video_id_validation() {
        assert!(is_video_id("rFZHOHl-L8A"));
        assert!(is_video_id("aqz-KE-bpKQ"));
        assert!(!is_video_id("short"));
        assert!(!is_video_id("UCSJ4gkVC6NrvII8umztf0Ow"));
        assert!(!is_video_id("--exec=rm -"));
    }

    #[test]
    fn channel_id_validation() {
        assert!(is_channel_id("UCSMOQeBJ2RAnuFungnQOxLg"));
        assert!(!is_channel_id("XXSMOQeBJ2RAnuFungnQOxLg"));
        assert!(!is_channel_id("UC/../../etc/passwd/xxxxx"));
        assert!(!is_channel_id("rFZHOHl-L8A"));
    }

    #[test]
    fn query_cleaning() {
        assert_eq!(
            clean_query("  lofi \n beats "),
            Some("lofi   beats".to_string())
        );
        assert_eq!(clean_query(" \t "), None);
        assert_eq!(clean_query(&"x".repeat(500)).unwrap().len(), MAX_QUERY_LEN);
    }

    #[test]
    fn parses_search_output() {
        let json = br#"{
            "_type": "playlist",
            "entries": [
                {
                    "id": "rFZHOHl-L8A",
                    "title": "lofi hip hop radio",
                    "channel": "Lofi Girl",
                    "channel_id": "UCSJ4gkVC6NrvII8umztf0Ow",
                    "duration": null,
                    "view_count": 16684606,
                    "thumbnails": [
                        {"url": "https://i.ytimg.com/small.jpg", "width": 360},
                        {"url": "https://i.ytimg.com/big.jpg", "width": 720}
                    ]
                },
                {"id": "UCSJ4gkVC6NrvII8umztf0Ow", "title": "A channel"},
                {"id": "aqz-KE-bpKQ", "uploader": "Blender", "duration": 634.0}
            ]
        }"#;

        let videos = parse_playlist(json).unwrap().videos;
        assert_eq!(videos.len(), 2);
        assert_eq!(videos[0].thumb, "https://i.ytimg.com/big.jpg");
        assert_eq!(videos[0].duration, None);
        assert_eq!(videos[1].channel.as_deref(), Some("Blender"));
        assert_eq!(videos[1].title, "Untitled");
        assert_eq!(
            videos[1].thumb,
            "https://i.ytimg.com/vi/aqz-KE-bpKQ/hqdefault.jpg"
        );
    }

    #[test]
    fn channel_entries_inherit_channel() {
        let json = br#"{
            "channel": "Blender",
            "channel_id": "UCSMOQeBJ2RAnuFungnQOxLg",
            "entries": [{"id": "sO_lhW8av3U", "title": "Recap", "duration": 149}]
        }"#;

        let playlist = parse_playlist(json).unwrap();
        assert_eq!(playlist.channel.as_deref(), Some("Blender"));
        assert_eq!(playlist.videos[0].channel.as_deref(), Some("Blender"));
        assert_eq!(
            playlist.videos[0].channel_id.as_deref(),
            Some("UCSMOQeBJ2RAnuFungnQOxLg")
        );
        assert_eq!(playlist.videos[0].duration, Some(149.0));
    }
}
