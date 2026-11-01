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
/// The boosts multiply a video's relevance (`1 + boosts`), so they reorder close matches but
/// never lift a weak match above a strong one.
/// Recent videos are preferred: full boost up to a year old, half at two years, none from three.
const RECENT_BOOST: f64 = 0.4;
const YEAR_DAYS: f64 = 365.0;
/// Each dismissed video of a channel lowers that channel's other videos by this much, up to the cap.
const DISMISS_PENALTY: f64 = 0.5;
const MAX_PENALTY: f64 = 1.5;
/// Longer videos are preferred: no boost under 5 minutes, growing to `LONG_BOOST` at 40 minutes.
const LONG_BOOST: f64 = 0.3;
/// Videos watched more per day since upload are preferred, on a log scale: full boost at
/// `POPULAR_PER_DAY` views a day, half at its square root.
const POPULAR_BOOST: f64 = 0.3;
const POPULAR_PER_DAY: f64 = 10_000.0;
/// Each further video from the same channel keeps this share of its score, so one channel
/// cannot fill the top of the list.
const SAME_CHANNEL_DECAY: f64 = 0.7;
/// Shorts are never suggested. Search results do not mark them, so anything this short counts.
const SHORTS_MAX_SECS: f64 = 180.0;
/// Share of the list kept for videos from channels that are not seed channels, so the list
/// is not only the seeded channels' own uploads.
const NEW_CHANNEL_SHARE: f64 = 0.5;
/// Most missing view counts looked up (one yt-dlp request each) per refresh.
pub const MAX_VIEW_LOOKUPS: usize = 8;
/// Videos from channels that are not seeds need at least this many views, only to keep out spam;
/// popularity otherwise counts through `POPULAR_BOOST`. A video whose view count is unknown, as for
/// some live streams and premieres, does not pass. Seed channels are exempt.
const MIN_NEW_CHANNEL_VIEWS: u64 = 2_000;
const SHORT_SECS: f64 = 300.0;
const LONG_SECS: f64 = 2400.0;

/// The seeds to query per kind: themes, channels, videos. When a kind has more seeds than its
/// maximum, the newest half of the slots always go to the newest seeds and the rest rotate
/// through the older ones with `rotation` (for example the hour), so no seed is left out for good.
pub fn pick_sources(seeds: &[Seed], rotation: usize) -> (Vec<&Seed>, Vec<&Seed>, Vec<&Seed>) {
    let pick = |kind: &str, max: usize| -> Vec<&Seed> {
        let all: Vec<&Seed> = seeds.iter().rev().filter(|s| s.kind == kind).collect();
        if all.len() <= max {
            return all;
        }
        let fixed = max / 2;
        let older = &all[fixed..];
        let start = rotation * (max - fixed) % older.len();
        all[..fixed].iter().copied().chain((0..max - fixed).map(|n| older[(start + n) % older.len()])).collect()
    };
    (pick("theme", MAX_THEMES), pick("channel", MAX_CHANNELS), pick("video", MAX_VIDEOS))
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
/// A video's relevance grows the higher it sits in a source and the more sources list it. That is
/// multiplied by boosts for recent, long and much-watched videos; videos from channels the user
/// dismissed things from lose points, and each further video of one channel keeps less of its
/// score. Videos in `known`, Shorts and excluded videos are dropped. Each video's `reason` names
/// the sources that found it. Half of the list is then reserved for the best videos from channels
/// outside `seed_channels` (the channels the user already follows as seeds).
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
            if known.contains(&video.id) || is_short(&video) || is_excluded(&video, excluded) {
                continue;
            }
            let points = 1.0 / (1.0 + position as f64 * 0.15);
            let (views, timestamp) = (video.view_count, video.timestamp);
            let entry = scored.entry(video.id.clone()).or_insert_with(|| {
                order += 1;
                Entry { score: 0.0, order, reasons: vec![], video }
            });
            entry.score += points;
            // Channel pages sometimes leave out the view count or date that a search shows.
            if entry.video.view_count.is_none() { entry.video.view_count = views; }
            if entry.video.timestamp.is_none() { entry.video.timestamp = timestamp; }
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
            let video = &entry.video;
            let boosts = RECENT_BOOST * recency(video.timestamp, now)
                + LONG_BOOST * length(video.duration)
                + POPULAR_BOOST * popularity(video.view_count, video.timestamp, now);
            entry.score = entry.score * (1.0 + boosts) - penalty;
            entry.video.reason = Some(entry.reasons.iter().take(2).cloned().collect::<Vec<_>>().join(" + "));
            entry
        })
        .collect();
    ranked.sort_by(|a, b| b.score.total_cmp(&a.score).then(a.order.cmp(&b.order)));
    // Walking down the list, each channel's second video keeps 70% of its score, the third 49%...
    let mut seen: HashMap<String, i32> = HashMap::new();
    for entry in &mut ranked {
        if let Some(channel) = &entry.video.channel_id {
            let count = seen.entry(channel.clone()).or_insert(0);
            entry.score *= SAME_CHANNEL_DECAY.powi(*count);
            *count += 1;
        }
    }
    ranked.sort_by(|a, b| b.score.total_cmp(&a.score).then(a.order.cmp(&b.order)));
    let (seeded, new): (Vec<_>, Vec<_>) = ranked.into_iter().map(|entry| entry.video).partition(|video| {
        video.channel_id.as_ref().is_some_and(|channel| seed_channels.contains(channel))
    });
    let new = new.into_iter().filter(|v| v.view_count.is_some_and(|views| views >= MIN_NEW_CHANNEL_VIEWS)).collect();
    mix(seeded, new, NEW_CHANNEL_SHARE)
}

/// Words that usually come with an excluded topic, since a title rarely says "kids" outright.
const RELATED: &[(&str, &[&str])] = &[
    ("kid", &["child", "children", "toddler", "nursery", "preschool", "sing along", "singalong"]),
    (
        "music",
        &[
            "song", "lyrics", "lyric", "remix", "album", "official audio", "official music video",
            "lofi", "lo-fi", "phonk", "nightcore", "sped up", "slowed", "instrumental", "type beat", "mashup",
            "edm", "synthwave", "vaporwave", "soundtrack", "ost",
        ],
    ),
];

/// Lowercase words, with a trailing plural "s" removed so "kids" and "kid" are the same word.
fn stems(text: &str) -> Vec<String> {
    text.split(|c: char| !c.is_alphanumeric())
        .filter(|w| !w.is_empty())
        .map(|w| {
            let w = w.to_lowercase();
            if w.chars().count() > 3 && w.ends_with('s') && !w.ends_with("ss") { w[..w.len() - 1].to_string() } else { w }
        })
        .collect()
}

/// Whether the title or channel name contains an excluded word or phrase (whole words, any case,
/// singular or plural), or a word that usually comes with it ("not kids" also hides "sing along").
fn is_excluded(video: &Video, excluded: &[String]) -> bool {
    if excluded.is_empty() {
        return false;
    }
    let channel = video.channel.as_deref().unwrap_or("");
    let text = [stems(&video.title), stems(channel)];
    // Channel names run words together ("kidstv123"), so there a word counts inside a longer one.
    let squashed = channel.to_lowercase();
    excluded.iter().any(|term| {
        let typed = term.trim().to_lowercase();
        if !typed.contains(char::is_whitespace) && typed.chars().count() >= 4 && squashed.contains(&typed) {
            return true;
        }
        let term = stems(term);
        let related = RELATED
            .iter()
            .filter(|(key, _)| term.len() == 1 && term[0] == *key)
            .flat_map(|(_, words)| words.iter().map(|w| stems(w)));
        std::iter::once(term.clone()).chain(related).any(|phrase| {
            !phrase.is_empty() && text.iter().any(|t| t.windows(phrase.len()).any(|w| w == phrase.as_slice()))
        })
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

/// Shorts, recognised by length (live streams and premieres have none and pass).
fn is_short(video: &Video) -> bool {
    video.duration.is_some_and(|seconds| seconds <= SHORTS_MAX_SECS)
}

/// 1 up to a year old, 0.5 at two years, 0 from three; 0 when the date is unknown.
fn recency(timestamp: Option<i64>, now: i64) -> f64 {
    let Some(timestamp) = timestamp else { return 0.0 };
    let age_years = ((now - timestamp) as f64 / 86_400.0 / YEAR_DAYS).max(0.0);
    let share = if age_years <= 1.0 { 1.0 } else { 1.0 - (age_years - 1.0) * 0.5 };
    share.clamp(0.0, 1.0)
}

/// 0 under 5 minutes, growing to 1 at 40 minutes.
fn length(duration: Option<f64>) -> f64 {
    let Some(seconds) = duration else { return 0.0 };
    ((seconds - SHORT_SECS) / (LONG_SECS - SHORT_SECS)).clamp(0.0, 1.0)
}

/// Views per day since upload on a log scale: 1 at `POPULAR_PER_DAY` or more, 0.5 at its square
/// root (100 a day), 0 when the count is unknown. An unknown date counts as a year old.
fn popularity(views: Option<u64>, timestamp: Option<i64>, now: i64) -> f64 {
    let Some(views) = views else { return 0.0 };
    let days = timestamp.map_or(YEAR_DAYS, |t| ((now - t) as f64 / 86_400.0).max(1.0));
    ((views as f64 / days + 1.0).log10() / POPULAR_PER_DAY.log10()).clamp(0.0, 1.0)
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
        video.view_count = Some(1_000_000);
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
        assert_eq!(at(0.1), 1.0);
        assert_eq!(at(1.0), 1.0);
        assert!((at(2.0) - 0.5).abs() < 1e-9);
        assert_eq!(at(3.0), 0.0);
        assert_eq!(at(6.0), 0.0);
        assert_eq!(recency(None, NOW), 0.0);
    }

    #[test]
    fn longer_videos_rank_above_short_ones() {
        let mut short = video("aaaaaaaaaaa", None);
        short.duration = Some(240.0);
        let mut long = video("bbbbbbbbbbb", None);
        long.duration = Some(3000.0);
        let ranked = rank_plain(vec![source("s", vec![short, long])], &HashSet::new());
        assert_eq!(ids(&ranked), ["bbbbbbbbbbb", "aaaaaaaaaaa"]);
        assert_eq!(length(Some(200.0)), 0.0);
        assert_eq!(length(None), 0.0);
        assert_eq!(length(Some(99_999.0)), 1.0);
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
    fn half_of_the_list_comes_from_channels_that_are_not_seeds() {
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
        assert_eq!(new_in_top_10, 5);
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
    fn an_excluded_topic_also_hides_words_that_come_with_it() {
        let mut ninimo = video("aaaaaaaaaaa", None);
        ninimo.title = "BOO! Sing Along with Spooky Ninimo".into();
        let mut plural = video("bbbbbbbbbbb", None);
        plural.title = "Best Kids Songs".into();
        let mut documentary = video("ccccccccccc", None);
        documentary.title = "The Haunting of a Hotel".into();
        let rank_excluding = |term: &str| {
            let all = vec![ninimo.clone(), plural.clone(), documentary.clone()];
            let ranked = rank(vec![source("s", all)], &HashSet::new(), &HashMap::new(), &HashSet::new(), &[term.to_string()], NOW);
            ranked.iter().map(|v| v.id.clone()).collect::<Vec<_>>()
        };
        assert_eq!(rank_excluding("kids"), ["ccccccccccc"]);
        assert_eq!(rank_excluding("kid"), ["ccccccccccc"]);
        assert_eq!(rank_excluding("music"), ["aaaaaaaaaaa", "ccccccccccc"]);
        let mut phonk = video("ddddddddddd", None);
        phonk.title = "Spooky (phonk)".into();
        let ranked = rank(vec![source("s", vec![phonk, documentary.clone()])], &HashSet::new(), &HashMap::new(), &HashSet::new(), &["music".to_string()], NOW);
        assert_eq!(ids(&ranked), ["ccccccccccc"]);
        assert_eq!(rank_excluding("haunting"), ["aaaaaaaaaaa", "bbbbbbbbbbb"]);
        // A channel name with the word run together is caught too, a title only on whole words.
        let mut tv = video("eeeeeeeeeee", None);
        tv.title = "spooky spooky halloween song".into();
        tv.channel = Some("kidstv123".into());
        let mut plain = video("fffffffffff", None);
        plain.title = "A day at the museum".into();
        // Counts the videos that remain after excluding `term`.
        let hide = |term: &str, videos: Vec<Video>| {
            ids(&rank(vec![source("s", videos)], &HashSet::new(), &HashMap::new(), &HashSet::new(), &[term.to_string()], NOW)).len()
        };
        assert_eq!(hide("kids", vec![tv.clone(), plain.clone()]), 1);
        assert_eq!(hide("music", vec![tv.clone(), plain.clone()]), 1);
        assert_eq!(hide("muse", vec![plain.clone()]), 1);
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
    fn videos_from_new_channels_need_2000_views_but_seed_channels_do_not() {
        let from = |id: &str, channel: &str, views: Option<u64>| {
            let mut v = video(id, None);
            v.channel_id = Some(channel.into());
            v.view_count = views;
            v
        };
        let found = vec![
            from("aaaaaaaaaaa", "UCnew", Some(1_999)),
            from("bbbbbbbbbbb", "UCnew2", Some(2_000)),
            from("ccccccccccc", "UCnew", None),
            from("ddddddddddd", "UCseed", Some(12)),
            from("eeeeeeeeeee", "UCseed", None),
        ];
        let seeds = HashSet::from(["UCseed".to_string()]);
        let ranked = rank(vec![source("s", found)], &HashSet::new(), &HashMap::new(), &seeds, &[], NOW);
        let mut kept = ids(&ranked);
        kept.sort();
        assert_eq!(kept, ["bbbbbbbbbbb", "ddddddddddd", "eeeeeeeeeee"]);
    }

    #[test]
    fn a_missing_view_count_is_filled_in_from_another_source() {
        let mut channel_page = video("aaaaaaaaaaa", None);
        channel_page.view_count = None;
        let mut search = video("aaaaaaaaaaa", Some(NOW - 86_400));
        search.view_count = Some(1_613_240);
        let ranked = rank_plain(vec![source("channel", vec![channel_page]), source("search", vec![search])], &HashSet::new());
        assert_eq!(ranked.len(), 1);
        assert_eq!((ranked[0].view_count, ranked[0].timestamp), (Some(1_613_240), Some(NOW - 86_400)));
    }

    #[test]
    fn sources_keep_the_newest_seeds_and_rotate_through_the_older_ones() {
        let seed = |id: i64, kind: &str| Seed { id, kind: kind.into(), value: id.to_string(), label: id.to_string() };
        let mut seeds: Vec<Seed> = (1..=12).map(|i| seed(i, "theme")).collect();
        seeds.push(seed(13, "channel"));
        seeds.push(seed(14, "video"));
        let (themes, channels, videos) = pick_sources(&seeds, 0);
        assert_eq!(themes.len(), MAX_THEMES);
        assert_eq!((channels.len(), videos.len()), (1, 1));
        let ids = |rotation| pick_sources(&seeds, rotation).0.iter().map(|s| s.id).collect::<Vec<_>>();
        // The three newest are always there; every older theme gets its turn within a few hours.
        let mut seen = HashSet::new();
        for hour in 0..3 {
            let picked = ids(hour);
            assert_eq!(picked[..3], [12, 11, 10]);
            assert_eq!(picked.iter().collect::<HashSet<_>>().len(), MAX_THEMES);
            seen.extend(picked);
        }
        assert_eq!(seen.len(), 12);
        // Few enough seeds are all used, newest first.
        let few: Vec<Seed> = (1..=3).map(|i| seed(i, "theme")).collect();
        assert_eq!(pick_sources(&few, 7).0.iter().map(|s| s.id).collect::<Vec<_>>(), [3, 2, 1]);
    }

    #[test]
    fn shorts_are_never_suggested() {
        let mut short = video("aaaaaaaaaaa", None);
        short.duration = Some(59.0);
        let mut vertical = video("bbbbbbbbbbb", None);
        vertical.duration = Some(180.0);
        let mut live = video("ccccccccccc", None);
        live.duration = None;
        let mut normal = video("ddddddddddd", None);
        normal.duration = Some(181.0);
        let ranked = rank_plain(vec![source("s", vec![short, vertical, live, normal])], &HashSet::new());
        let mut kept = ids(&ranked);
        kept.sort();
        assert_eq!(kept, ["ccccccccccc", "ddddddddddd"]);
    }

    #[test]
    fn boosts_reorder_close_matches_but_do_not_lift_a_weak_one_over_the_top_result() {
        let mut top = video("aaaaaaaaaaa", Some(NOW - 4 * 365 * 86_400));
        top.duration = Some(480.0);
        let filler = (0..10).map(|i| video(&format!("filler{i:05}"), Some(NOW - 4 * 365 * 86_400)));
        let mut last = video("bbbbbbbbbbb", Some(NOW - 30 * 86_400));
        last.duration = Some(2400.0);
        let mut found = vec![top];
        found.extend(filler);
        found.push(last);
        let ranked = rank_plain(vec![source("s", found)], &HashSet::new());
        assert_eq!(ranked[0].id, "aaaaaaaaaaa");
        assert_ne!(ranked[1].id, "bbbbbbbbbbb");
    }

    #[test]
    fn one_channel_does_not_fill_the_top_of_the_list() {
        let from = |id: &str, channel: &str| {
            let mut v = video(id, None);
            v.channel_id = Some(channel.into());
            v
        };
        let found = vec![
            from("big00000001", "UCbig"),
            from("big00000002", "UCbig"),
            from("big00000003", "UCbig"),
            from("other000001", "UCother"),
        ];
        let ranked = rank_plain(vec![source("s", found)], &HashSet::new());
        assert_eq!(ids(&ranked), ["big00000001", "other000001", "big00000002", "big00000003"]);
    }

    #[test]
    fn popularity_counts_views_per_day_on_a_log_scale() {
        let day = 86_400;
        assert_eq!(popularity(None, Some(NOW - day), NOW), 0.0);
        assert_eq!(popularity(Some(1_000_000), Some(NOW - day), NOW), 1.0);
        // 3,000 views in 30 days is 100 a day: half way.
        assert!((popularity(Some(3_000), Some(NOW - 30 * day), NOW) - 0.5).abs() < 0.01);
        // A young niche video is not far behind an old popular one.
        let young = popularity(Some(15_000), Some(NOW - 60 * day), NOW);
        let old = popularity(Some(400_000), Some(NOW - 5 * 365 * day), NOW);
        assert!(young > old);
    }

    #[test]
    fn titles_become_queries_without_bracketed_tags() {
        assert_eq!(query_from_title("Amiga 500 Restoration [4K] (Part 2)  "), "Amiga 500 Restoration");
        assert_eq!(query_from_title("[Official]"), "[Official]");
    }
}
