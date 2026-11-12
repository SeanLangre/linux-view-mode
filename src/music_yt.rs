//! yt-dlp for Music mode: song search, playlist import and audio downloads (ported from the
//! desktop music player). yt-dlp is always started directly, never through a shell, with `--`
//! before the user's text.
use std::{path::{Path, PathBuf}, process::Stdio, time::Duration};

use serde::Deserialize;
use tokio::{
    io::{AsyncBufReadExt, AsyncReadExt, BufReader},
    process::Command,
};

use crate::music_db::MusicTrack;

/// Search and playlist import give up after this long.
const LIST_TIMEOUT: Duration = Duration::from_secs(60);
pub const PLAYLIST_MAX: usize = 200;
pub const SEARCH_MAX: usize = 150;
pub const MAX_QUERY_LEN: usize = 200;

#[derive(Deserialize)]
struct Response {
    title: Option<String>,
    #[serde(default)]
    entries: Vec<Entry>,
}

#[derive(Deserialize)]
struct Entry {
    id: Option<String>,
    title: Option<String>,
    channel: Option<String>,
    uploader: Option<String>,
    duration: Option<f64>,
}

fn tracks_from(entries: Vec<Entry>) -> Vec<MusicTrack> {
    entries
        .into_iter()
        .filter_map(|e| {
            let id = e.id.filter(|id| crate::ytdlp::is_video_id(id))?;
            let title = e.title.unwrap_or_else(|| "Untitled".into());
            if title == "[Private video]" || title == "[Deleted video]" {
                return None;
            }
            let artist = e
                .channel
                .or(e.uploader)
                .map(|a| a.trim_end_matches(" - Topic").to_string())
                .filter(|a| !a.is_empty());
            Some(MusicTrack {
                thumbnail_url: Some(format!("https://i.ytimg.com/vi/{id}/mqdefault.jpg")),
                title,
                artist,
                duration_secs: e.duration.filter(|d| d.is_finite() && *d >= 0.0).map(|d| d as u32),
                video_id: id,
                file_path: None,
                downloaded_at: None,
            })
        })
        .collect()
}

pub fn parse_search(json: &[u8]) -> Result<Vec<MusicTrack>, String> {
    let response: Response = serde_json::from_slice(json).map_err(|e| format!("Unexpected search response: {e}"))?;
    Ok(tracks_from(response.entries))
}

pub fn parse_playlist(json: &[u8]) -> Result<(String, Vec<MusicTrack>), String> {
    let response: Response = serde_json::from_slice(json).map_err(|e| format!("Unexpected playlist response: {e}"))?;
    Ok((response.title.unwrap_or_else(|| "YouTube playlist".into()), tracks_from(response.entries)))
}

/// True for a YouTube link that points at a playlist (it contains `list=`). Only YouTube hosts count.
pub fn is_playlist_url(text: &str) -> bool {
    let t = text.trim();
    let Some(rest) = t.strip_prefix("https://").or_else(|| t.strip_prefix("http://")) else { return false };
    let host = rest.split(['/', '?', '#']).next().unwrap_or("");
    let host = host.strip_prefix("www.").or_else(|| host.strip_prefix("m.")).or_else(|| host.strip_prefix("music.")).unwrap_or(host);
    !t.contains(char::is_whitespace) && t.contains("list=") && matches!(host, "youtube.com" | "youtu.be")
}

/// The first `count` search results (YouTube search, like the desktop app: YouTube Music search
/// was too slow and had no artist or length).
pub async fn search(query: &str, count: usize) -> Result<Vec<MusicTrack>, String> {
    let count = count.clamp(1, SEARCH_MAX);
    let output = run(&["--flat-playlist", "--no-warnings", "-J"], &format!("ytsearch{count}:{query}")).await?;
    parse_search(&output)
}

/// The songs of a YouTube playlist link (at most 200).
pub async fn fetch_playlist(url: &str) -> Result<(String, Vec<MusicTrack>), String> {
    if !is_playlist_url(url) {
        return Err("That isn't a YouTube playlist link.".into());
    }
    let limit = PLAYLIST_MAX.to_string();
    let output = run(&["--flat-playlist", "--yes-playlist", "--no-warnings", "-J", "--playlist-end", &limit], url.trim()).await?;
    parse_playlist(&output)
}

/// One song from a YouTube video link (`video_id` must pass `is_video_id`).
pub async fn fetch_video(video_id: &str) -> Result<MusicTrack, String> {
    if !crate::ytdlp::is_video_id(video_id) {
        return Err("That isn't a YouTube video link.".into());
    }
    let output = run(&["--no-playlist", "--no-warnings", "--skip-download", "-J"], &format!("https://www.youtube.com/watch?v={video_id}")).await?;
    parse_video(&output)
}

pub fn parse_video(json: &[u8]) -> Result<MusicTrack, String> {
    let entry: Entry = serde_json::from_slice(json).map_err(|e| format!("Unexpected video response: {e}"))?;
    tracks_from(vec![entry]).into_iter().next().ok_or_else(|| "This video isn't available.".to_string())
}

async fn run(args: &[&str], target: &str) -> Result<Vec<u8>, String> {
    let child = Command::new("yt-dlp")
        .arg("--ignore-config")
        .args(args)
        .arg("--")
        .arg(target)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .map_err(spawn_error)?;
    let output = tokio::time::timeout(LIST_TIMEOUT, child.wait_with_output())
        .await
        .map_err(|_| "YouTube took too long to answer.".to_string())?
        .map_err(|e| format!("Couldn't run yt-dlp: {e}"))?;
    if !output.status.success() {
        return Err(friendly_error(&String::from_utf8_lossy(&output.stderr)));
    }
    Ok(output.stdout)
}

/// Parses a line printed by our `--progress-template`. `None` if it isn't a progress line or the
/// total size is unknown.
pub fn parse_progress(line: &str) -> Option<f32> {
    let mut parts = line.strip_prefix("PROGRESS ")?.split_whitespace();
    let downloaded: f64 = parts.next()?.parse().ok()?;
    let total: f64 = parts.next().and_then(|t| t.parse().ok()).or_else(|| parts.next().and_then(|t| t.parse().ok()))?;
    (total > 0.0).then(|| (downloaded / total).clamp(0.0, 1.0) as f32)
}

/// Downloads the best audio stream (no conversion) into `tmp_dir` and returns its path.
/// `on_progress` gets a fraction from 0.0 to 1.0. Dropping the future kills yt-dlp.
pub async fn download(video_id: &str, tmp_dir: &Path, on_progress: impl Fn(f32)) -> Result<PathBuf, String> {
    if !crate::ytdlp::is_video_id(video_id) {
        return Err("Invalid video id.".into());
    }
    let mut child = Command::new("yt-dlp")
        .args([
            "--ignore-config",
            "-f",
            "bestaudio[acodec=opus]/bestaudio",
            "--no-playlist",
            "--no-warnings",
            "--no-mtime",
            "--newline",
            "--progress",
            "--progress-template",
            "download:PROGRESS %(progress.downloaded_bytes)s %(progress.total_bytes)s %(progress.total_bytes_estimate)s",
            "--print",
            "after_move:filepath",
            "-o",
        ])
        .arg(tmp_dir.join("%(id)s.%(ext)s"))
        .arg("--")
        .arg(format!("https://www.youtube.com/watch?v={video_id}"))
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .map_err(spawn_error)?;

    let stdout = child.stdout.take().ok_or("Couldn't read yt-dlp output")?;
    let mut stderr = child.stderr.take().ok_or("Couldn't read yt-dlp output")?;
    let stderr_text = tokio::spawn(async move {
        let mut text = String::new();
        let _ = stderr.read_to_string(&mut text).await;
        text
    });

    let mut lines = BufReader::new(stdout).lines();
    let mut path = None;
    while let Ok(Some(line)) = lines.next_line().await {
        let line = line.trim();
        if line.starts_with("PROGRESS ") {
            if let Some(p) = parse_progress(line) {
                on_progress(p);
            }
        } else if !line.is_empty() {
            path = Some(PathBuf::from(line));
        }
    }

    let status = child.wait().await.map_err(|e| format!("Couldn't run yt-dlp: {e}"))?;
    if !status.success() {
        let stderr = stderr_text.await.unwrap_or_default();
        return Err(friendly_error(&stderr));
    }
    let path = path.ok_or("yt-dlp didn't report a downloaded file")?;
    if path.is_file() { Ok(path) } else { Err("The downloaded file is missing".into()) }
}

fn spawn_error(e: std::io::Error) -> String {
    if e.kind() == std::io::ErrorKind::NotFound {
        "yt-dlp isn't installed. Install it with: sudo pacman -S yt-dlp".into()
    } else {
        format!("Couldn't run yt-dlp: {e}")
    }
}

pub fn friendly_error(stderr: &str) -> String {
    let s = stderr.to_lowercase();
    if s.contains("not a bot") || s.contains("sign in to confirm") {
        "YouTube is asking for a bot check. Try again later.".into()
    } else if s.contains("unable to download") && s.contains("name resolution")
        || s.contains("network is unreachable")
        || s.contains("temporary failure in name resolution")
    {
        "No internet connection.".into()
    } else if s.contains("video unavailable") || s.contains("private video") {
        "This video isn't available.".into()
    } else if s.contains("http error 429") {
        "YouTube is rate limiting requests. Try again in a few minutes.".into()
    } else if s.contains("unsupported url") || s.contains("nsig") || s.contains("unable to extract") {
        "yt-dlp can't read YouTube right now. Updating it usually fixes this: sudo pacman -Syu yt-dlp".into()
    } else {
        let last = stderr.lines().rev().map(str::trim).find(|l| !l.is_empty()).unwrap_or("unknown error");
        format!("yt-dlp failed: {last}")
    }
}

/// The search text, cleaned: control characters become spaces, at most `MAX_QUERY_LEN` characters.
pub fn clean_query(text: &str) -> Option<String> {
    let cleaned: String = text.chars().map(|c| if c.is_control() { ' ' } else { c }).take(MAX_QUERY_LEN).collect();
    let trimmed = cleaned.trim();
    (!trimmed.is_empty()).then(|| trimmed.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_search_results() {
        let json = br#"{"entries":[
          {"id":"FGBhQbmPwH8","title":"One More Time","channel":"Daft Punk - Topic","duration":322.0},
          {"id":"short","title":"bad id"},
          {"id":"A2VpR8HahKc","uploader":"Someone","duration":null}
        ]}"#;
        let tracks = parse_search(json).unwrap();
        assert_eq!(tracks.len(), 2);
        assert_eq!(tracks[0].artist.as_deref(), Some("Daft Punk"));
        assert_eq!(tracks[0].duration_secs, Some(322));
        assert_eq!(tracks[1].title, "Untitled");
        assert_eq!(tracks[1].duration_secs, None);
    }

    #[test]
    fn recognises_playlist_links() {
        assert!(is_playlist_url("https://www.youtube.com/playlist?list=PL123"));
        assert!(is_playlist_url("  https://music.youtube.com/watch?v=abc&list=RDabc "));
        assert!(!is_playlist_url("https://www.youtube.com/watch?v=FGBhQbmPwH8"));
        assert!(!is_playlist_url("daft punk list=1"));
        assert!(!is_playlist_url("https://example.com/?list=1"));
        assert!(!is_playlist_url("https://example.com/youtube.com/?list=1"));
        assert!(!is_playlist_url("https://youtube.com.evil.example/?list=1"));
        assert!(!is_playlist_url("--exec https://youtube.com/?list=1"));
    }

    #[test]
    fn parses_playlist_and_skips_private_videos() {
        let json = br#"{"title":"Road trip","entries":[
          {"id":"FGBhQbmPwH8","title":"One","channel":"A"},
          {"id":"A2VpR8HahKc","title":"[Private video]"},
          {"id":"fa5IWHDbftI","title":"Two"}
        ]}"#;
        let (title, tracks) = parse_playlist(json).unwrap();
        assert_eq!(title, "Road trip");
        assert_eq!(tracks.len(), 2);
    }

    #[test]
    fn parses_a_single_video() {
        let json = br#"{"id":"FGBhQbmPwH8","title":"One More Time","uploader":"Daft Punk - Topic","duration":322}"#;
        let track = parse_video(json).unwrap();
        assert_eq!(track.video_id, "FGBhQbmPwH8");
        assert_eq!(track.artist.as_deref(), Some("Daft Punk"));
        assert!(parse_video(br#"{"id":"x","title":"bad"}"#).is_err());
    }

    #[test]
    fn parses_progress_lines() {
        assert_eq!(parse_progress("PROGRESS 500 1000 NA"), Some(0.5));
        assert_eq!(parse_progress("PROGRESS 250 NA 1000"), Some(0.25));
        assert_eq!(parse_progress("PROGRESS 5 NA NA"), None);
        assert_eq!(parse_progress("PROGRESS 2000 1000 NA"), Some(1.0));
        assert_eq!(parse_progress("/home/me/file.webm"), None);
    }

    #[test]
    fn explains_common_errors() {
        assert!(friendly_error("ERROR: Sign in to confirm you’re not a bot").contains("bot check"));
        assert!(friendly_error("ERROR: Video unavailable").contains("isn't available"));
        assert!(friendly_error("ERROR: Unable to extract player response").contains("pacman -Syu yt-dlp"));
    }

    #[test]
    fn cleans_search_text() {
        assert_eq!(clean_query("  daft\npunk  ").as_deref(), Some("daft punk"));
        assert_eq!(clean_query(" \n "), None);
        assert_eq!(clean_query(&"a".repeat(500)).unwrap().len(), MAX_QUERY_LEN);
    }
}
