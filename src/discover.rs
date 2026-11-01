//! Our own recommendations ("For You"): videos found from the seeds a profile gave us.
//!
//! Discovery runs as Guest (no account cookies), so it never touches the account's history.
//! The active profile is only used to leave out videos it has already watched.

use crate::{db::Seed, ytdlp::Video};
use std::collections::{HashMap, HashSet};

/// Most recent themes and channels used per refresh; bounds the number of yt-dlp requests.
pub const MAX_THEMES: usize = 6;
pub const MAX_CHANNELS: usize = 6;
/// Videos taken from each theme search and each channel.
pub const THEME_LIMIT: u32 = 12;
pub const CHANNEL_LIMIT: usize = 8;
/// Videos newer than this get a recency boost.
const FRESH_DAYS: f64 = 30.0;

/// The seeds to query, newest first within each kind.
pub fn pick_sources(seeds: &[Seed]) -> (Vec<&Seed>, Vec<&Seed>) {
    let newest = |kind: &str, max: usize| -> Vec<&Seed> {
        seeds.iter().rev().filter(|s| s.kind == kind).take(max).collect()
    };
    (newest("theme", MAX_THEMES), newest("channel", MAX_CHANNELS))
}

/// Merges the result lists of several sources into one ranked list.
///
/// A video scores more the higher it sits in a source and the more sources list it; videos
/// uploaded in the last 30 days get up to half a point extra. Videos in `known` are dropped.
pub fn rank(sources: Vec<Vec<Video>>, known: &HashSet<String>, now: i64) -> Vec<Video> {
    let mut scored: HashMap<String, (f64, usize, Video)> = HashMap::new();
    let mut order = 0;
    for source in sources {
        for (position, video) in source.into_iter().enumerate() {
            if known.contains(&video.id) {
                continue;
            }
            let points = 1.0 / (1.0 + position as f64 * 0.15);
            scored
                .entry(video.id.clone())
                .and_modify(|entry| entry.0 += points)
                .or_insert_with(|| {
                    order += 1;
                    (points, order, video)
                });
        }
    }
    let mut ranked: Vec<(f64, usize, Video)> = scored
        .into_values()
        .map(|(score, order, video)| (score + recency(video.timestamp, now), order, video))
        .collect();
    ranked.sort_by(|a, b| b.0.total_cmp(&a.0).then(a.1.cmp(&b.1)));
    ranked.into_iter().map(|(_, _, video)| video).collect()
}

fn recency(timestamp: Option<i64>, now: i64) -> f64 {
    let Some(timestamp) = timestamp else { return 0.0 };
    let age_days = (now - timestamp) as f64 / 86_400.0;
    0.5 * (1.0 - age_days / FRESH_DAYS).clamp(0.0, 1.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn video(id: &str, timestamp: Option<i64>) -> Video {
        let mut video = Video::placeholder(id);
        video.timestamp = timestamp;
        video
    }
    fn ids(videos: &[Video]) -> Vec<&str> {
        videos.iter().map(|v| v.id.as_str()).collect()
    }
    const NOW: i64 = 1_800_000_000;

    #[test]
    fn a_video_found_by_two_sources_beats_a_first_place_in_one() {
        let a = vec![video("aaaaaaaaaaa", None), video("bbbbbbbbbbb", None)];
        let b = vec![video("ccccccccccc", None), video("bbbbbbbbbbb", None)];
        let ranked = rank(vec![a, b], &HashSet::new(), NOW);
        assert_eq!(ids(&ranked), ["bbbbbbbbbbb", "aaaaaaaaaaa", "ccccccccccc"]);
    }

    #[test]
    fn known_videos_are_dropped_and_duplicates_collapse() {
        let source = vec![video("aaaaaaaaaaa", None), video("aaaaaaaaaaa", None), video("bbbbbbbbbbb", None)];
        let known = HashSet::from(["aaaaaaaaaaa".to_string()]);
        assert_eq!(ids(&rank(vec![source], &known, NOW)), ["bbbbbbbbbbb"]);
    }

    #[test]
    fn newer_uploads_win_ties_and_old_ones_get_no_boost() {
        let old = video("aaaaaaaaaaa", Some(NOW - 400 * 86_400));
        let fresh = video("bbbbbbbbbbb", Some(NOW - 86_400));
        // Same rank in two different sources, so only recency separates them.
        let ranked = rank(vec![vec![old], vec![fresh]], &HashSet::new(), NOW);
        assert_eq!(ids(&ranked), ["bbbbbbbbbbb", "aaaaaaaaaaa"]);
        assert_eq!(recency(Some(NOW - 400 * 86_400), NOW), 0.0);
        assert_eq!(recency(None, NOW), 0.0);
    }

    #[test]
    fn sources_are_the_newest_seeds_of_each_kind() {
        let seed = |id: i64, kind: &str| Seed { id, kind: kind.into(), value: id.to_string(), label: id.to_string() };
        let mut seeds: Vec<Seed> = (1..=8).map(|i| seed(i, "theme")).collect();
        seeds.push(seed(9, "channel"));
        seeds.push(seed(10, "video"));
        let (themes, channels) = pick_sources(&seeds);
        assert_eq!(themes.len(), MAX_THEMES);
        assert_eq!(themes[0].id, 8);
        assert_eq!(channels.len(), 1);
    }
}
