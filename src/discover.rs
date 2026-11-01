//! Our own recommendations ("For You"): videos found from the seeds a profile gave us.
//!
//! Discovery runs as Guest (no account cookies), so it never touches the account's history.
//! The active profile is only used to leave out videos it has already watched or dismissed.

use crate::{db::Seed, ytdlp::Video};
use std::collections::{HashMap, HashSet};

/// Most recent themes, channels and videos used per refresh; bounds the number of yt-dlp requests.
pub const MAX_THEMES: usize = 6;
pub const MAX_CHANNELS: usize = 6;
pub const MAX_VIDEOS: usize = 4;
/// Videos taken from each search and each channel.
pub const THEME_LIMIT: u32 = 12;
pub const CHANNEL_LIMIT: usize = 8;
/// Videos newer than this get a recency boost.
const FRESH_DAYS: f64 = 30.0;
/// Each dismissed video of a channel lowers that channel's other videos by this much, up to the cap.
const DISMISS_PENALTY: f64 = 0.5;
const MAX_PENALTY: f64 = 1.5;

/// The seeds to query, newest first within each kind: themes, channels, videos.
pub fn pick_sources(seeds: &[Seed]) -> (Vec<&Seed>, Vec<&Seed>, Vec<&Seed>) {
    let newest = |kind: &str, max: usize| -> Vec<&Seed> {
        seeds.iter().rev().filter(|s| s.kind == kind).take(max).collect()
    };
    (newest("theme", MAX_THEMES), newest("channel", MAX_CHANNELS), newest("video", MAX_VIDEOS))
}

/// One list of results and the reason it was fetched, shown on the card ("retro computing").
pub struct Source {
    pub label: String,
    pub videos: Vec<Video>,
}

/// Merges the result lists of several sources into one ranked list.
///
/// A video scores more the higher it sits in a source and the more sources list it; videos
/// uploaded in the last 30 days get up to half a point extra, and videos from channels the
/// user dismissed things from lose points. Videos in `known` are dropped. Each video's
/// `reason` names the sources that found it.
pub fn rank(
    sources: Vec<Source>,
    known: &HashSet<String>,
    dismissed_channels: &HashMap<String, u32>,
    now: i64,
) -> Vec<Video> {
    struct Entry { score: f64, order: usize, reasons: Vec<String>, video: Video }
    let mut scored: HashMap<String, Entry> = HashMap::new();
    let mut order = 0;
    for source in sources {
        for (position, video) in source.videos.into_iter().enumerate() {
            if known.contains(&video.id) {
                continue;
            }
            let points = 1.0 / (1.0 + position as f64 * 0.15);
            let entry = scored.entry(video.id.clone()).or_insert_with(|| {
                order += 1;
                Entry { score: 0.0, order, reasons: vec![], video }
            });
            entry.score += points;
            if !entry.reasons.contains(&source.label) {
                entry.reasons.push(source.label.clone());
            }
        }
    }
    let mut ranked: Vec<Entry> = scored
        .into_values()
        .map(|mut entry| {
            let penalty = entry.video.channel_id.as_ref()
                .and_then(|channel| dismissed_channels.get(channel))
                .map_or(0.0, |count| (*count as f64 * DISMISS_PENALTY).min(MAX_PENALTY));
            entry.score += recency(entry.video.timestamp, now) - penalty;
            entry.video.reason = Some(entry.reasons.iter().take(2).cloned().collect::<Vec<_>>().join(" + "));
            entry
        })
        .collect();
    ranked.sort_by(|a, b| b.score.total_cmp(&a.score).then(a.order.cmp(&b.order)));
    ranked.into_iter().map(|entry| entry.video).collect()
}

fn recency(timestamp: Option<i64>, now: i64) -> f64 {
    let Some(timestamp) = timestamp else { return 0.0 };
    let age_days = (now - timestamp) as f64 / 86_400.0;
    0.5 * (1.0 - age_days / FRESH_DAYS).clamp(0.0, 1.0)
}

/// A search query for "videos like this one": the title without bracketed tags and extra spaces.
pub fn query_from_title(title: &str) -> String {
    let mut depth = 0;
    let plain: String = title
        .chars()
        .filter(|c| match c {
            '(' | '[' | '【' => { depth += 1; false }
            ')' | ']' | '】' => { depth = (depth - 1).max(0); false }
            _ => depth == 0,
        })
        .collect();
    let query = plain.split_whitespace().collect::<Vec<_>>().join(" ");
    if query.is_empty() { title.trim().to_string() } else { query }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn video(id: &str, timestamp: Option<i64>) -> Video {
        let mut video = Video::placeholder(id);
        video.timestamp = timestamp;
        video
    }
    fn source(label: &str, videos: Vec<Video>) -> Source {
        Source { label: label.into(), videos }
    }
    fn ids(videos: &[Video]) -> Vec<&str> {
        videos.iter().map(|v| v.id.as_str()).collect()
    }
    fn rank_plain(sources: Vec<Source>, known: &HashSet<String>) -> Vec<Video> {
        rank(sources, known, &HashMap::new(), NOW)
    }
    const NOW: i64 = 1_800_000_000;

    #[test]
    fn a_video_found_by_two_sources_beats_a_first_place_in_one() {
        let a = source("a", vec![video("aaaaaaaaaaa", None), video("bbbbbbbbbbb", None)]);
        let b = source("b", vec![video("ccccccccccc", None), video("bbbbbbbbbbb", None)]);
        let ranked = rank_plain(vec![a, b], &HashSet::new());
        assert_eq!(ids(&ranked), ["bbbbbbbbbbb", "aaaaaaaaaaa", "ccccccccccc"]);
        assert_eq!(ranked[0].reason.as_deref(), Some("a + b"));
        assert_eq!(ranked[1].reason.as_deref(), Some("a"));
    }

    #[test]
    fn known_videos_are_dropped_and_duplicates_collapse() {
        let s = source("s", vec![video("aaaaaaaaaaa", None), video("aaaaaaaaaaa", None), video("bbbbbbbbbbb", None)]);
        let known = HashSet::from(["aaaaaaaaaaa".to_string()]);
        assert_eq!(ids(&rank_plain(vec![s], &known)), ["bbbbbbbbbbb"]);
    }

    #[test]
    fn newer_uploads_win_ties_and_old_ones_get_no_boost() {
        let old = source("x", vec![video("aaaaaaaaaaa", Some(NOW - 400 * 86_400))]);
        let fresh = source("y", vec![video("bbbbbbbbbbb", Some(NOW - 86_400))]);
        assert_eq!(ids(&rank_plain(vec![old, fresh], &HashSet::new())), ["bbbbbbbbbbb", "aaaaaaaaaaa"]);
        assert_eq!(recency(Some(NOW - 400 * 86_400), NOW), 0.0);
        assert_eq!(recency(None, NOW), 0.0);
    }

    #[test]
    fn dismissing_a_channel_lowers_its_other_videos() {
        let mut liked = video("aaaaaaaaaaa", None);
        liked.channel_id = Some("UCliked".into());
        let mut disliked = video("bbbbbbbbbbb", None);
        disliked.channel_id = Some("UCdisliked".into());
        let s = source("s", vec![disliked, liked]);
        let dismissed = HashMap::from([("UCdisliked".to_string(), 2)]);
        let ranked = rank(vec![s], &HashSet::new(), &dismissed, NOW);
        assert_eq!(ids(&ranked), ["aaaaaaaaaaa", "bbbbbbbbbbb"]);
    }

    #[test]
    fn sources_are_the_newest_seeds_of_each_kind() {
        let seed = |id: i64, kind: &str| Seed { id, kind: kind.into(), value: id.to_string(), label: id.to_string() };
        let mut seeds: Vec<Seed> = (1..=8).map(|i| seed(i, "theme")).collect();
        seeds.push(seed(9, "channel"));
        seeds.push(seed(10, "video"));
        let (themes, channels, videos) = pick_sources(&seeds);
        assert_eq!(themes.len(), MAX_THEMES);
        assert_eq!(themes[0].id, 8);
        assert_eq!((channels.len(), videos.len()), (1, 1));
    }

    #[test]
    fn titles_become_queries_without_bracketed_tags() {
        assert_eq!(query_from_title("Amiga 500 Restoration [4K] (Part 2)  "), "Amiga 500 Restoration");
        assert_eq!(query_from_title("[Official]"), "[Official]");
    }
}
