//! Runs yt-dlp and turns its JSON output into `Video`s.
//!
//! yt-dlp is always started directly (never through a shell), with `--` before
//! the positional argument, and is killed if it runs longer than `TIMEOUT`.

use std::process::Stdio;
use std::time::Duration;

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use tokio::process::Command;

const TIMEOUT: Duration = Duration::from_secs(30);
pub const MAX_QUERY_LEN: usize = 200;

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct Video {
    pub id: String,
    pub title: String,
    pub channel: Option<String>,
    pub channel_id: Option<String>,
    /// Seconds; `None` for live streams and some premieres.
    pub duration: Option<f64>,
    pub view_count: Option<u64>,
    pub thumb: String,
}

/// Searches YouTube (signed out) and returns up to `limit` videos.
pub async fn search(query: &str, limit: u32) -> Result<Vec<Video>> {
    let json = run(&format!("ytsearch{limit}:{query}")).await?;
    parse_playlist(&json)
}

pub fn is_video_id(id: &str) -> bool {
    id.len() == 11
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

async fn run(target: &str) -> Result<Vec<u8>> {
    let child = Command::new("yt-dlp")
        .args(["--flat-playlist", "-J", "--no-warnings", "--"])
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
        let stderr = String::from_utf8_lossy(&output.stderr);
        bail!("yt-dlp failed: {}", stderr.trim());
    }
    Ok(output.stdout)
}

#[derive(Deserialize)]
struct RawPlaylist {
    #[serde(default)]
    entries: Vec<RawEntry>,
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

fn parse_playlist(json: &[u8]) -> Result<Vec<Video>> {
    let playlist: RawPlaylist =
        serde_json::from_slice(json).context("unexpected yt-dlp output")?;

    Ok(playlist
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
                channel: e.channel.or(e.uploader),
                channel_id: e.channel_id,
                duration: e.duration,
                view_count: e.view_count,
                thumb,
                id,
            })
        })
        .collect())
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
    fn query_cleaning() {
        assert_eq!(clean_query("  lofi \n beats "), Some("lofi   beats".to_string()));
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

        let videos = parse_playlist(json).unwrap();
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
}
