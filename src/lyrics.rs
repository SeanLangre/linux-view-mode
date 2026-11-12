//! Lyrics from LRCLIB (https://lrclib.net), ported from the desktop music player. Only the song
//! title, artist and length are sent to the service. HTTP goes through `curl`, like the rest of
//! CouchTube.

use serde::Deserialize;

const BASE: &str = "https://lrclib.net/api";
const USER_AGENT: &str = concat!("couchtube/", env!("CARGO_PKG_VERSION"), " (personal TV music player)");
/// How far a result's length may differ from ours before we treat it as a different version.
const DURATION_TOLERANCE_SECS: f64 = 8.0;

#[derive(Clone, Debug, PartialEq, Default)]
pub struct Lyrics {
    /// Raw LRC text (`[mm:ss.xx] line`), if the lyrics are timed.
    pub synced: Option<String>,
    pub plain: Option<String>,
}

impl Lyrics {
    /// Timed lines, empty when the lyrics aren't timed.
    #[cfg(test)]
    pub fn lines(&self) -> Vec<(f64, String)> {
        self.synced.as_deref().map(parse_lrc).unwrap_or_default()
    }
}

#[derive(Deserialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
struct Candidate {
    #[serde(default)]
    duration: Option<f64>,
    #[serde(default)]
    instrumental: bool,
    plain_lyrics: Option<String>,
    synced_lyrics: Option<String>,
}

impl Candidate {
    fn has_lyrics(&self) -> bool {
        self.instrumental
            || self.synced_lyrics.as_deref().is_some_and(|s| !s.trim().is_empty())
            || self.plain_lyrics.as_deref().is_some_and(|s| !s.trim().is_empty())
    }

    fn into_lyrics(self) -> Lyrics {
        if self.instrumental {
            return Lyrics {
                synced: None,
                plain: Some("♪ Instrumental ♪".into()),
            };
        }
        Lyrics {
            synced: self.synced_lyrics.filter(|s| !s.trim().is_empty()),
            plain: self.plain_lyrics.filter(|s| !s.trim().is_empty()),
        }
    }
}

/// YouTube titles are messy ("Artist - Song (Official Video) [HD]"). Returns
/// `(artist, title)` as best we can tell.
pub fn clean(title: &str, channel: Option<&str>) -> (String, String) {
    // Drop (...), [...] and 【...】 parts, and anything after a '|'.
    let mut out = String::new();
    let mut depth = 0usize;
    for c in title.chars() {
        match c {
            '(' | '[' | '【' => depth += 1,
            ')' | ']' | '】' => depth = depth.saturating_sub(1),
            '|' if depth == 0 => break,
            _ if depth == 0 => out.push(c),
            _ => {}
        }
    }
    let out = out.split_whitespace().collect::<Vec<_>>().join(" ");

    let channel = channel
        .map(|c| c.trim_end_matches(" - Topic").trim_end_matches("VEVO").trim().to_string())
        .unwrap_or_default();
    match out.split_once(" - ") {
        Some((artist, song)) if !artist.trim().is_empty() && !song.trim().is_empty() => {
            (artist.trim().to_string(), song.trim().to_string())
        }
        _ => (channel, out),
    }
}

/// Parses LRC text into `(seconds, line)`, sorted by time. A line may carry
/// several timestamps. Metadata tags like `[ar: Artist]` are ignored.
pub fn parse_lrc(text: &str) -> Vec<(f64, String)> {
    let mut lines = vec![];
    for raw in text.lines() {
        let mut rest = raw.trim();
        let mut stamps = vec![];
        while let Some(stripped) = rest.strip_prefix('[') {
            let Some((tag, after)) = stripped.split_once(']') else { break };
            match parse_stamp(tag) {
                Some(t) => stamps.push(t),
                None => break,
            }
            rest = after.trim_start();
        }
        for t in stamps {
            lines.push((t, rest.trim().to_string()));
        }
    }
    lines.sort_by(|a, b| a.0.total_cmp(&b.0));
    lines
}

fn parse_stamp(tag: &str) -> Option<f64> {
    let (min, sec) = tag.split_once(':')?;
    let min: f64 = min.trim().parse().ok()?;
    let sec: f64 = sec.trim().parse().ok()?;
    (min >= 0.0 && (0.0..60.0).contains(&sec)).then_some(min * 60.0 + sec)
}

/// Index of the line being sung at `position`, or `None` before the first line (the screen does
/// this itself; kept for the tests that pin the rule down).
#[cfg(test)]
pub fn current_line(lines: &[(f64, String)], position: f64) -> Option<usize> {
    lines.partition_point(|(t, _)| *t <= position).checked_sub(1)
}

fn pick_best(candidates: Vec<Candidate>, duration: Option<f64>) -> Option<Candidate> {
    candidates
        .into_iter()
        .filter(Candidate::has_lyrics)
        .filter_map(|c| {
            let diff = match (duration, c.duration) {
                (Some(ours), Some(theirs)) => (ours - theirs).abs(),
                _ => 0.0,
            };
            (diff <= DURATION_TOLERANCE_SECS).then_some((diff, c))
        })
        // Closest length first; timed lyrics win ties.
        .min_by(|(da, a), (db, b)| {
            da.total_cmp(db)
                .then(b.synced_lyrics.is_some().cmp(&a.synced_lyrics.is_some()))
        })
        .map(|(_, c)| c)
}

async fn get<T: for<'de> Deserialize<'de>>(path: &str, query: &[(&str, &str)]) -> Result<Option<T>, String> {
    let mut config = format!(
        "silent\nshow-error\nmax-time = 15\nget\nurl = \"{BASE}/{path}\"\nheader = \"User-Agent: {USER_AGENT}\"\nwrite-out = \"\\n%{{http_code}}\"\n"
    );
    for (key, value) in query {
        config.push_str(&format!("data-urlencode = \"{}={}\"\n", crate::gql::quote(key), crate::gql::quote(value)));
    }
    let (status, body) = crate::helix::curl(config).await.map_err(|_| "Couldn't reach the lyrics service.".to_string())?;
    match status {
        200 => serde_json::from_str::<T>(&body).map(Some).map_err(|e| format!("Unexpected lyrics response: {e}")),
        404 => Ok(None),
        code => Err(format!("The lyrics service returned an error ({code}).")),
    }
}

async fn lookup(title: &str, channel: Option<&str>, duration_secs: Option<u32>) -> Result<Option<Lyrics>, String> {
    let (artist, song) = clean(title, channel);
    let duration = duration_secs.map(f64::from);
    let duration_text = duration_secs.map(|d| d.to_string());

    if song.trim().is_empty() {
        return Ok(None);
    }
    // An exact match first (it needs the artist and the length), then a looser search.
    if let (Some(d), false) = (&duration_text, artist.is_empty()) {
        let query = [("artist_name", artist.as_str()), ("track_name", song.as_str()), ("duration", d.as_str())];
        if let Some(c) = get::<Candidate>("get", &query).await? {
            if c.has_lyrics() {
                return Ok(Some(c.into_lyrics()));
            }
        }
    }
    let q = format!("{artist} {song}");
    let found: Vec<Candidate> = get("search", &[("q", q.trim())]).await?.unwrap_or_default();
    Ok(pick_best(found, duration).map(Candidate::into_lyrics))
}

/// Look up lyrics. `Ok(None)` means the service has none for this song.
pub async fn fetch(title: String, channel: Option<String>, duration_secs: Option<u32>) -> Result<Option<Lyrics>, String> {
    lookup(&title, channel.as_deref(), duration_secs).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cleans_messy_titles() {
        assert_eq!(
            clean("Daft Punk - One More Time (Official Video) [HD]", Some("Daft Punk")),
            ("Daft Punk".into(), "One More Time".into())
        );
        assert_eq!(
            clean("One More Time", Some("Daft Punk - Topic")),
            ("Daft Punk".into(), "One More Time".into())
        );
        assert_eq!(
            clean("Song Name | Official Music Video", Some("ArtistVEVO")),
            ("Artist".into(), "Song Name".into())
        );
        assert_eq!(clean("A - B - C (live)", None), ("A".into(), "B - C".into()));
    }

    #[test]
    fn parses_timed_lyrics() {
        let lrc = "[ar: Someone]\n[00:12.50] Hello\n[00:05.00][01:00.25] Chorus\n[00:20.00]\nno stamp";
        let lines = parse_lrc(lrc);
        assert_eq!(
            lines,
            vec![
                (5.0, "Chorus".to_string()),
                (12.5, "Hello".to_string()),
                (20.0, String::new()),
                (60.25, "Chorus".to_string())
            ]
        );
    }

    #[test]
    fn finds_the_current_line() {
        let lines = parse_lrc("[00:05.00] a\n[00:10.00] b\n[00:15.00] c");
        assert_eq!(current_line(&lines, 1.0), None);
        assert_eq!(current_line(&lines, 5.0), Some(0));
        assert_eq!(current_line(&lines, 12.0), Some(1));
        assert_eq!(current_line(&lines, 99.0), Some(2));
        assert_eq!(current_line(&[], 5.0), None);
    }

    fn cand(duration: f64, synced: bool) -> Candidate {
        Candidate {
            duration: Some(duration),
            instrumental: false,
            plain_lyrics: Some("words".into()),
            synced_lyrics: synced.then(|| "[00:01.00] words".into()),
        }
    }

    #[test]
    fn picks_the_closest_version() {
        let best = pick_best(vec![cand(225.0, true), cand(321.0, false), cand(319.5, true)], Some(322.0)).unwrap();
        assert_eq!(best.duration, Some(321.0)); // closest length wins over timed lyrics
        let best = pick_best(vec![cand(320.0, false), cand(320.0, true)], Some(320.0)).unwrap();
        assert!(best.synced_lyrics.is_some()); // tie: timed lyrics win
        assert!(pick_best(vec![cand(225.0, true)], Some(322.0)).is_none()); // a different version
        assert!(pick_best(vec![cand(225.0, true)], None).is_some()); // our length unknown
    }

    /// Needs the network.
    #[tokio::test]
    #[ignore]
    async fn finds_lyrics_for_a_youtube_style_title() {
        let l = fetch("Daft Punk - One More Time (Official Video)".into(), Some("Daft Punk".into()), Some(322))
            .await
            .unwrap()
            .expect("lyrics for a famous song");
        assert!(!l.lines().is_empty(), "expected timed lyrics");
        let none = fetch("zzzz qqqq nothing here".into(), None, Some(100)).await.unwrap();
        assert!(none.is_none());
    }
}
