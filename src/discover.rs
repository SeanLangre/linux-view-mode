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
/// Blended searches combine two themes ("retro computing sourdough") to find videos that fit both.
/// Only pairs among the newest themes are used, and only a few per refresh.
pub const BLEND_FROM: usize = 4;
pub const MAX_BLENDS: usize = 3;
pub const BLEND_LIMIT: u32 = 10;
/// Recent videos are preferred: full boost up to a year old, half at two years, none from three.
const RECENT_BOOST: f64 = 0.8;
const YEAR_DAYS: f64 = 365.0;
/// Each dismissed video of a channel lowers that channel's other videos by this much, up to the cap.
const DISMISS_PENALTY: f64 = 0.5;
const MAX_PENALTY: f64 = 1.5;
/// Longer videos are preferred: no boost under 5 minutes, growing to `LONG_BOOST` at 40 minutes.
const LONG_BOOST: f64 = 0.6;
/// Share of the list kept for videos from channels that are not seed channels, so the list
/// is not only the seeded channels' own uploads.
const NEW_CHANNEL_SHARE: f64 = 0.3;
const SHORT_SECS: f64 = 300.0;
const LONG_SECS: f64 = 2400.0;

/// The seeds to query, newest first within each kind: themes, channels, videos.
pub fn pick_sources(seeds: &[Seed]) -> (Vec<&Seed>, Vec<&Seed>, Vec<&Seed>) {
    let newest = |kind: &str, max: usize| -> Vec<&Seed> {
        seeds.iter().rev().filter(|s| s.kind == kind).take(max).collect()
    };
    (newest("theme", MAX_THEMES), newest("channel", MAX_CHANNELS), newest("video", MAX_VIDEOS))
}

/// Pairs of themes to search together: `(label, query)`. Pairs come from the newest `BLEND_FROM`
/// themes; `rotation` (for example the hour) moves which `MAX_BLENDS` of them are used, so
/// different pairs get their turn over time.
pub fn blends(themes: &[&Seed], rotation: usize) -> Vec<(String, String)> {
    let themes = &themes[..themes.len().min(BLEND_FROM)];
    let mut pairs = Vec::new();
    for (i, first) in themes.iter().enumerate() {
        for second in &themes[i + 1..] {
            pairs.push((format!("{} & {}", first.label, second.label), format!("{} {}", first.value, second.value)));
        }
    }
    if pairs.len() <= MAX_BLENDS {
        return pairs;
    }
    let start = rotation % pairs.len();
    (0..MAX_BLENDS).map(|n| pairs[(start + n) % pairs.len()].clone()).collect()
}

/// One list of results and the reason it was fetched, shown on the card ("retro computing").
pub struct Source {
    pub label: String,
    pub videos: Vec<Video>,
}

/// Merges the result lists of several sources into one ranked list.
///
/// A video scores more the higher it sits in a source and the more sources list it; videos
/// uploaded in the last year get 0.8 points extra, fading to half at two years and nothing at three, and videos from channels the
/// user dismissed things from lose points. Videos in `known` are dropped. Each video's
/// `reason` names the sources that found it. About 30% of the list is then reserved for the
/// best videos from channels outside `seed_channels` (the channels the user already follows as seeds).
pub fn rank(
    sources: Vec<Source>,
    known: &HashSet<String>,
    dismissed_channels: &HashMap<String, u32>,
    seed_channels: &HashSet<String>,
    excluded: &[String],
    now: i64,
) -> Vec<Video> {
    struct Entry { score: f64, order: usize, reasons: Vec<String>, video: Video }
    let mut scored: HashMap<String, Entry> = HashMap::new();
    let mut order = 0;
    for source in sources {
        for (position, video) in source.videos.into_iter().enumerate() {
            if known.contains(&video.id) || is_excluded(&video, excluded) {
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
            entry.score += recency(entry.video.timestamp, now) + length_boost(entry.video.duration) - penalty;
            entry.video.reason = Some(entry.reasons.iter().take(2).cloned().collect::<Vec<_>>().join(" + "));
            entry
        })
        .collect();
    ranked.sort_by(|a, b| b.score.total_cmp(&a.score).then(a.order.cmp(&b.order)));
    let (seeded, new): (Vec<_>, Vec<_>) = ranked.into_iter().map(|entry| entry.video).partition(|video| {
        video.channel_id.as_ref().is_some_and(|channel| seed_channels.contains(channel))
    });
    mix(seeded, new, NEW_CHANNEL_SHARE)
}

/// Whether the title or channel name contains an excluded word or phrase, as whole words
/// and ignoring case ("music" does not match "museum").
fn is_excluded(video: &Video, excluded: &[String]) -> bool {
    if excluded.is_empty() {
        return false;
    }
    let words = |text: &str| -> Vec<String> {
        text.split(|c: char| !c.is_alphanumeric()).filter(|w| !w.is_empty()).map(str::to_lowercase).collect()
    };
    let text = [words(&video.title), words(video.channel.as_deref().unwrap_or(""))];
    excluded.iter().any(|term| {
        let term = words(term);
        !term.is_empty() && text.iter().any(|t| t.windows(term.len()).any(|w| w == term.as_slice()))
    })
}

/// Interleaves two ranked lists so `new` takes `share` of the slots, spread evenly; when one
/// list runs out the other fills the rest.
fn mix(seeded: Vec<Video>, new: Vec<Video>, share: f64) -> Vec<Video> {
    let total = seeded.len() + new.len();
    let (mut seeded, mut new) = (seeded.into_iter(), new.into_iter());
    let mut out = Vec::with_capacity(total);
    for i in 0..total {
        let wants_new = ((i + 1) as f64 * share).floor() > (i as f64 * share).floor();
        let next = if wants_new { new.next().or_else(|| seeded.next()) } else { seeded.next().or_else(|| new.next()) };
        out.extend(next);
    }
    out
}

fn recency(timestamp: Option<i64>, now: i64) -> f64 {
    let Some(timestamp) = timestamp else { return 0.0 };
    let age_years = ((now - timestamp) as f64 / 86_400.0 / YEAR_DAYS).max(0.0);
    let share = if age_years <= 1.0 { 1.0 } else { 1.0 - (age_years - 1.0) * 0.5 };
    RECENT_BOOST * share.clamp(0.0, 1.0)
}

fn length_boost(duration: Option<f64>) -> f64 {
    let Some(seconds) = duration else { return 0.0 };
    LONG_BOOST * ((seconds - SHORT_SECS) / (LONG_SECS - SHORT_SECS)).clamp(0.0, 1.0)
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
        rank(sources, known, &HashMap::new(), &HashSet::new(), &[], NOW)
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
    fn recent_uploads_win_ties_and_old_ones_get_no_boost() {
        let old = source("x", vec![video("aaaaaaaaaaa", Some(NOW - 4 * 365 * 86_400))]);
        let fresh = source("y", vec![video("bbbbbbbbbbb", Some(NOW - 86_400))]);
        assert_eq!(ids(&rank_plain(vec![old, fresh], &HashSet::new())), ["bbbbbbbbbbb", "aaaaaaaaaaa"]);
        let at = |years: f64| recency(Some(NOW - (years * 365.0 * 86_400.0) as i64), NOW);
        assert_eq!(at(0.1), RECENT_BOOST);
        assert_eq!(at(1.0), RECENT_BOOST);
        assert!((at(2.0) - RECENT_BOOST / 2.0).abs() < 1e-9);
        assert_eq!(at(3.0), 0.0);
        assert_eq!(at(6.0), 0.0);
        assert_eq!(recency(None, NOW), 0.0);
    }

    #[test]
    fn longer_videos_rank_above_short_ones() {
        let mut short = video("aaaaaaaaaaa", None);
        short.duration = Some(120.0);
        let mut long = video("bbbbbbbbbbb", None);
        long.duration = Some(3000.0);
        let ranked = rank_plain(vec![source("s", vec![short, long])], &HashSet::new());
        assert_eq!(ids(&ranked), ["bbbbbbbbbbb", "aaaaaaaaaaa"]);
        assert_eq!(length_boost(Some(60.0)), 0.0);
        assert_eq!(length_boost(None), 0.0);
        assert_eq!(length_boost(Some(99_999.0)), LONG_BOOST);
    }

    #[test]
    fn dismissing_a_channel_lowers_its_other_videos() {
        let mut liked = video("aaaaaaaaaaa", None);
        liked.channel_id = Some("UCliked".into());
        let mut disliked = video("bbbbbbbbbbb", None);
        disliked.channel_id = Some("UCdisliked".into());
        let s = source("s", vec![disliked, liked]);
        let dismissed = HashMap::from([("UCdisliked".to_string(), 2)]);
        let ranked = rank(vec![s], &HashSet::new(), &dismissed, &HashSet::new(), &[], NOW);
        assert_eq!(ids(&ranked), ["aaaaaaaaaaa", "bbbbbbbbbbb"]);
    }

    #[test]
    fn a_third_of_the_list_comes_from_channels_that_are_not_seeds() {
        let from = |id: &str, channel: &str| {
            let mut v = video(id, None);
            v.channel_id = Some(channel.into());
            v
        };
        let mut found: Vec<Video> = (0..10).map(|i| from(&format!("seeded{i:05}"), "UCseed")).collect();
        found.extend((0..10).map(|i| from(&format!("fresh{i:06}"), "UCnew")));
        let seeds = HashSet::from(["UCseed".to_string()]);
        let ranked = rank(vec![source("s", found)], &HashSet::new(), &HashMap::new(), &seeds, &[], NOW);
        let new_in_top_10 = ranked.iter().take(10).filter(|v| v.channel_id.as_deref() == Some("UCnew")).count();
        assert_eq!(new_in_top_10, 3);
        // With no new-channel videos at all, the seeded ones still fill the list.
        let only = vec![from("aaaaaaaaaaa", "UCseed"), from("bbbbbbbbbbb", "UCseed")];
        assert_eq!(rank(vec![source("s", only)], &HashSet::new(), &HashMap::new(), &seeds, &[], NOW).len(), 2);
    }

    #[test]
    fn excluded_words_hide_whole_word_matches_in_title_or_channel() {
        let mut music = video("aaaaaaaaaaa", None);
        music.title = "Best Lo-Fi MUSIC mix".into();
        let mut museum = video("bbbbbbbbbbb", None);
        museum.title = "A day at the museum".into();
        let mut channel = video("ccccccccccc", None);
        channel.title = "Live set".into();
        channel.channel = Some("Chill Music Records".into());
        let mut essay = video("ddddddddddd", None);
        essay.title = "A video  essay about trains".into();
        let all = || vec![music.clone(), museum.clone(), channel.clone(), essay.clone()];
        let rank_excluding = |terms: &[&str]| {
            let terms: Vec<String> = terms.iter().map(|t| t.to_string()).collect();
            let ranked = rank(vec![source("s", all())], &HashSet::new(), &HashMap::new(), &HashSet::new(), &terms, NOW);
            ranked.iter().map(|v| v.id.clone()).collect::<Vec<_>>()
        };
        assert_eq!(rank_excluding(&["music"]), ["bbbbbbbbbbb", "ddddddddddd"]);
        assert_eq!(rank_excluding(&["video essay"]), ["aaaaaaaaaaa", "bbbbbbbbbbb", "ccccccccccc"]);
        assert_eq!(rank_excluding(&[]).len(), 4);
    }

    #[test]
    fn blends_pair_the_newest_themes_and_rotate() {
        let seed = |id: i64| Seed { id, kind: "theme".into(), value: format!("t{id}"), label: format!("T{id}") };
        let seeds: Vec<Seed> = (1..=6).map(seed).collect();
        let themes: Vec<&Seed> = seeds.iter().collect();
        assert!(blends(&themes[..1], 0).is_empty());
        let two = blends(&themes[..2], 5);
        assert_eq!(two, [("T1 & T2".to_string(), "t1 t2".to_string())]);
        // Six themes: only the first four make six pairs, and three of them are used.
        let first = blends(&themes, 0);
        assert_eq!(first.len(), MAX_BLENDS);
        assert_eq!(first[0].0, "T1 & T2");
        assert!(first.iter().all(|(label, _)| !label.contains("T5") && !label.contains("T6")));
        assert_ne!(blends(&themes, 1), first);
        assert_eq!(blends(&themes, 6), first);
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
