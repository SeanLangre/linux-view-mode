//! Twitch's own web API (GraphQL), used **unofficially** and without any registration. It is what
//! twitch.tv itself calls, with the public client id the website ships. See `docs/twitch-mode.md`:
//! this is the default until a Twitch application is registered, after which `helix.rs` (the
//! official API) takes over just by setting `COUCHTUBE_TWITCH_CLIENT_ID` and `_CLIENT_SECRET`.
//!
//! What this means in practice:
//! * It is not a supported API. Twitch may change it or block it, like yt-dlp on the YouTube side.
//! * The client id is the website's. If Twitch rotates it, the new one is read from twitch.tv.
//! * Any `after:` cursor trips Twitch's integrity check (verified 2026-11-08), so "load more" asks
//!   for a bigger first page instead (24, 48, … up to Twitch's limits), and the page skips cards
//!   it already has.
//! * Read-only, public data, no login. Following needs the official API (a user token).
use anyhow::{Context, Result, bail};
use serde_json::{Value, json};
use std::{sync::Arc, time::Duration};
use tokio::sync::Mutex;

use crate::helix::{
    Cache, Category, Channel, Page, Stream, TTL_CATEGORIES, TTL_LIVE, curl, image, is_id, is_login, message, sized, text,
    valid_credential, valid_url,
};

const DEFAULT_URL: &str = "https://gql.twitch.tv/gql";
const DEFAULT_SITE: &str = "https://www.twitch.tv/";
/// twitch.tv's own client id (read from its page on 2026-11-08).
const DEFAULT_CLIENT_ID: &str = "kimne78kx3ncx6brgo4mv6wki5h1ko";
const BROWSER: &str = "Mozilla/5.0 (X11; Linux x86_64; rv:130.0) Gecko/20100101 Firefox/130.0";

/// Cards per first page, and how much each "load more" adds.
pub const STEP: usize = 24;
/// Twitch's limits for `first` (checked 2026-11-08): categories 100, all streams 30, a category's streams ~100.
pub const MAX_CATEGORIES: usize = 100;
pub const MAX_TOP_STREAMS: usize = 30;
pub const MAX_CATEGORY_STREAMS: usize = 100;

const CATEGORIES: &str = "query($first:Int!){games(first:$first){edges{node{id name boxArtURL(width:285,height:380)}}}}";
const STREAM_FIELDS: &str = "id title viewersCount createdAt previewImageURL(width:440,height:248) broadcaster{login displayName} game{id name}";
const GAME_STREAMS: &str = "query($id:ID!,$first:Int!){game(id:$id){streams(first:$first){edges{node{FIELDS}}}}}";
const TOP_STREAMS: &str = "query($first:Int!){streams(first:$first){edges{node{FIELDS}}}}";
const SEARCH: &str = "query($q:String!){searchFor(userQuery:$q,platform:\"web\",options:{targets:[{index:CHANNEL}]}){channels{edges{item{... on User{login displayName profileImageURL(width:150) broadcastSettings{title game{name}} stream{id}}}}}}}";

struct Inner {
    url: String,
    site: String,
    client_id: Mutex<String>,
    /// One request at a time, like the other Twitch and yt-dlp paths.
    gate: Mutex<()>,
    cache: Cache,
}

#[derive(Clone)]
pub struct Gql(Arc<Inner>);

impl Gql {
    pub fn from_env() -> Self {
        let url = |name: &str, default: &str| std::env::var(name).ok().filter(|v| valid_url(v)).unwrap_or_else(|| default.to_owned());
        let client_id = std::env::var("COUCHTUBE_TWITCH_WEB_CLIENT_ID")
            .ok()
            .filter(|v| valid_credential(v))
            .unwrap_or_else(|| DEFAULT_CLIENT_ID.to_owned());
        Self(Arc::new(Inner {
            url: url("COUCHTUBE_TWITCH_GQL_URL", DEFAULT_URL),
            site: url("COUCHTUBE_TWITCH_SITE_URL", DEFAULT_SITE),
            client_id: Mutex::new(client_id),
            gate: Mutex::new(()),
            cache: Cache::new(),
        }))
    }

    /// Runs one GraphQL query (user text only ever travels in `variables`) and returns its `data`.
    async fn query(&self, query: &str, variables: Value, ttl: Duration, refresh: bool) -> Result<Value> {
        let body = json!({ "query": query, "variables": variables }).to_string();
        if !refresh {
            if let Some(hit) = self.0.cache.get(&body, ttl) {
                return Ok(hit);
            }
        }
        let _gate = self.0.gate.lock().await;
        let mut answer = None;
        for attempt in 0..2 {
            let id = self.0.client_id.lock().await.clone();
            let (status, text) = curl(post_config(&self.0.url, &id, &body)).await?;
            // Twitch rotated the website's client id: read the current one from twitch.tv, once.
            if status == 400 && attempt == 0 && text.contains("Client-ID") {
                let fresh = self.discover_client_id().await?;
                tracing::warn!("Twitch's web client id changed; using the one from twitch.tv");
                *self.0.client_id.lock().await = fresh;
                continue;
            }
            if status != 200 {
                bail!("Twitch answered HTTP {status}: {}", message(&text));
            }
            answer = Some(serde_json::from_str::<Value>(&text).context("Twitch sent something that is not JSON")?);
            break;
        }
        let answer = answer.context("Twitch refused the client id")?;
        if answer["data"].is_null() || answer["data"].as_object().is_some_and(|o| o.values().all(Value::is_null) && answer["errors"].is_array()) {
            let reason = answer["errors"][0]["message"].as_str().unwrap_or("no data");
            bail!("Twitch refused the query: {reason}");
        }
        let data = answer["data"].clone();
        self.0.cache.put(body, data.clone());
        Ok(data)
    }

    async fn discover_client_id(&self) -> Result<String> {
        let config = format!("silent\nshow-error\nmax-time = 15\nurl = \"{}\"\nuser-agent = \"{BROWSER}\"\nlocation\nwrite-out = \"\\n%{{http_code}}\"\n", self.0.site);
        let (status, page) = curl(config).await?;
        if status != 200 {
            bail!("twitch.tv answered HTTP {status}");
        }
        client_id_from_page(&page).context("no client id found on twitch.tv")
    }

    pub async fn top_categories(&self, cursor: Option<&str>, refresh: bool) -> Result<Page<Category>> {
        let first = size(cursor, MAX_CATEGORIES);
        let data = self.query(CATEGORIES, json!({ "first": first }), TTL_CATEGORIES, refresh).await?;
        let items = parse_categories(&data);
        Ok(Page { cursor: next(first, items.len(), MAX_CATEGORIES), items })
    }

    /// Live streams of one category, or the most watched on all of Twitch (at most 30).
    pub async fn streams(&self, game_id: Option<&str>, cursor: Option<&str>, refresh: bool) -> Result<Page<Stream>> {
        let (query, variables, max) = match game_id {
            Some(id) => {
                let first = size(cursor, MAX_CATEGORY_STREAMS);
                (
                    GAME_STREAMS.replace("FIELDS", STREAM_FIELDS),
                    json!({ "id": id, "first": first }),
                    MAX_CATEGORY_STREAMS,
                )
            }
            None => {
                let first = size(cursor, MAX_TOP_STREAMS);
                (TOP_STREAMS.replace("FIELDS", STREAM_FIELDS), json!({ "first": first }), MAX_TOP_STREAMS)
            }
        };
        let first = variables["first"].as_u64().unwrap_or(STEP as u64) as usize;
        let data = self.query(&query, variables, TTL_LIVE, refresh).await?;
        let items = parse_streams(&data);
        Ok(Page { cursor: next(first, items.len(), max), items })
    }

    /// Twitch returns about 15 channels and no paging for a search.
    pub async fn search_channels(&self, text: &str, refresh: bool) -> Result<Page<Channel>> {
        let data = self.query(SEARCH, json!({ "q": text }), TTL_LIVE, refresh).await?;
        Ok(Page { items: parse_channels(&data), cursor: None })
    }
}

/// A cursor here is just the page size to ask for next: `n48`.
fn size(cursor: Option<&str>, max: usize) -> usize {
    cursor
        .and_then(|c| c.strip_prefix('n'))
        .and_then(|n| n.parse::<usize>().ok())
        .map_or(STEP, |n| n.clamp(1, max))
        .min(max)
}

/// The cursor for the next, bigger page: only when this one came back (nearly) full and there is
/// room left. Twitch often returns a few fewer than asked (23 of 24, 92 of 96), so 80% counts as full.
fn next(asked: usize, got: usize, max: usize) -> Option<String> {
    (got * 10 >= asked * 8 && asked < max).then(|| format!("n{}", (asked + STEP).min(max)))
}

fn post_config(url: &str, client_id: &str, body: &str) -> String {
    format!(
        "silent\nshow-error\nmax-time = 15\nurl = \"{url}\"\nrequest = \"POST\"\nheader = \"Client-ID: {client_id}\"\nheader = \"Content-Type: application/json\"\ndata = \"{}\"\nwrite-out = \"\\n%{{http_code}}\"\n",
        quote(body)
    )
}

/// Escapes a value for a double-quoted curl config string.
fn quote(text: &str) -> String {
    text.replace('\\', "\\\\").replace('"', "\\\"").replace('\n', "\\n").replace('\r', "\\r").replace('\t', "\\t")
}

/// Finds `clientId="…"` in twitch.tv's page.
fn client_id_from_page(page: &str) -> Option<String> {
    let rest = page.split("clientId=\"").nth(1)?;
    let id = rest.split('"').next()?;
    ((20..=40).contains(&id.len()) && id.bytes().all(|b| b.is_ascii_alphanumeric())).then(|| id.to_owned())
}

fn nodes(edges: &Value, key: &str) -> Vec<Value> {
    edges["edges"].as_array().map_or_else(Vec::new, |list| list.iter().map(|e| e[key].clone()).collect())
}

pub fn parse_categories(data: &Value) -> Vec<Category> {
    nodes(&data["games"], "node")
        .iter()
        .filter(|g| is_id(g["id"].as_str().unwrap_or_default()))
        .map(|g| Category { id: text(g, "id"), name: text(g, "name"), box_art: image(sized(&text(g, "boxArtURL"), 285, 380)) })
        .collect()
}

pub fn parse_streams(data: &Value) -> Vec<Stream> {
    // All of Twitch is `data.streams`; one category is `data.game.streams` (null if the id is unknown).
    let list = if data["streams"].is_object() { &data["streams"] } else { &data["game"]["streams"] };
    nodes(list, "node")
        .iter()
        .filter(|s| is_login(s["broadcaster"]["login"].as_str().unwrap_or_default()))
        .map(|s| Stream {
            login: text(&s["broadcaster"], "login"),
            name: text(&s["broadcaster"], "displayName"),
            title: text(s, "title"),
            game_id: text(&s["game"], "id"),
            game_name: text(&s["game"], "name"),
            viewers: s["viewersCount"].as_u64().unwrap_or(0),
            thumb: image(text(s, "previewImageURL")),
            started_at: s["createdAt"].as_str().map(str::to_owned),
        })
        .collect()
}

pub fn parse_channels(data: &Value) -> Vec<Channel> {
    nodes(&data["searchFor"]["channels"], "item")
        .iter()
        // Search can also return other kinds of results as empty objects.
        .filter(|c| is_login(c["login"].as_str().unwrap_or_default()))
        .map(|c| Channel {
            login: text(c, "login"),
            name: text(c, "displayName"),
            live: c["stream"].is_object(),
            title: text(&c["broadcastSettings"], "title"),
            game_name: text(&c["broadcastSettings"]["game"], "name"),
            thumb: image(text(c, "profileImageURL")),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn page_sizes_grow_by_a_step_up_to_twitchs_limits() {
        assert_eq!(size(None, 100), 24);
        assert_eq!(size(Some("n48"), 100), 48);
        assert_eq!(size(Some("n500"), 100), 100);
        assert_eq!(size(Some("n0"), 100), 1);
        assert_eq!(size(Some("garbage"), 100), 24);
        assert_eq!(size(None, 30), 24);
        assert_eq!(size(Some("n48"), 30), 30);
        assert_eq!(next(24, 24, 100).as_deref(), Some("n48"));
        assert_eq!(next(96, 96, 100).as_deref(), Some("n100"));
        assert_eq!(next(100, 100, 100), None, "at the limit");
        assert_eq!(next(24, 23, 100).as_deref(), Some("n48"), "Twitch often returns a few fewer");
        assert_eq!(next(24, 17, 100), None, "a short page is the last one");
        assert_eq!(next(24, 24, 30).as_deref(), Some("n30"));
        assert_eq!(next(30, 30, 30), None);
    }

    /// What curl does with a quoted config value: `\\`, `\"`, `\n`, `\r`, `\t` become the real character.
    fn curl_unquote(value: &str) -> String {
        let mut out = String::new();
        let mut chars = value.chars();
        while let Some(c) = chars.next() {
            if c != '\\' {
                out.push(c);
                continue;
            }
            match chars.next() {
                Some('n') => out.push('\n'),
                Some('r') => out.push('\r'),
                Some('t') => out.push('\t'),
                Some(other) => out.push(other),
                None => out.push('\\'),
            }
        }
        out
    }

    #[test]
    fn user_text_survives_the_curl_config_unchanged() {
        for text in ["plain", "a \"b\" \\ c\nd\te", "%{http_code} \" \n data = \"x\"", "ünï 🔴"] {
            let body = json!({ "query": "q", "variables": { "q": text } }).to_string();
            let config = post_config("https://gql.twitch.tv/gql", "id0123456789", &body);
            let lines: Vec<&str> = config.lines().filter(|l| l.starts_with("data = ")).collect();
            // Exactly one `data` line, so the text cannot add options of its own.
            assert_eq!(lines.len(), 1, "{text:?}");
            assert_eq!(config.lines().count(), 9, "{text:?}");
            let value = lines[0].strip_prefix("data = \"").unwrap().strip_suffix('"').unwrap();
            assert_eq!(curl_unquote(value), body, "{text:?}");
        }
    }

    #[test]
    fn reads_the_client_id_from_the_website() {
        assert_eq!(client_id_from_page("x clientId=\"kimne78kx3ncx6brgo4mv6wki5h1ko\" y").as_deref(), Some("kimne78kx3ncx6brgo4mv6wki5h1ko"));
        assert_eq!(client_id_from_page("clientId=\"short\""), None);
        assert_eq!(client_id_from_page("clientId=\"has space in it 0123456789012\""), None);
        assert_eq!(client_id_from_page("nothing here"), None);
    }

    // Shapes captured from the live endpoint on 2026-11-08.
    #[test]
    fn parses_what_twitch_returns() {
        let games = json!({"games": {"edges": [
            {"cursor": "x", "node": {"id": "21779", "name": "League of Legends", "boxArtURL": "https://static-cdn.jtvnw.net/ttv-boxart/21779-285x380.jpg"}},
            {"node": {"id": "bad id", "name": "x", "boxArtURL": ""}}]}});
        let categories = parse_categories(&games);
        assert_eq!(categories.len(), 1);
        assert_eq!(categories[0].box_art, "https://static-cdn.jtvnw.net/ttv-boxart/21779-285x380.jpg");

        let node = json!({"id": "1", "title": "🔴 hi", "viewersCount": 274889, "createdAt": "2026-11-08T17:03:57Z",
            "previewImageURL": "https://static-cdn.jtvnw.net/previews-ttv/live_user_caedrel-440x248.jpg",
            "broadcaster": {"login": "caedrel", "displayName": "Caedrel"}, "game": {"id": "21779", "name": "League of Legends"}});
        for data in [json!({"streams": {"edges": [{"node": node}]}}), json!({"game": {"streams": {"edges": [{"node": node}]}}})] {
            let streams = parse_streams(&data);
            assert_eq!(streams.len(), 1);
            assert_eq!((streams[0].login.as_str(), streams[0].viewers, streams[0].game_name.as_str()), ("caedrel", 274889, "League of Legends"));
            assert!(streams[0].thumb.ends_with("live_user_caedrel-440x248.jpg"));
        }
        assert!(parse_streams(&json!({"game": null})).is_empty(), "an unknown category id is an empty list");

        let search = json!({"searchFor": {"channels": {"edges": [
            {"item": {"login": "lirik", "displayName": "LIRIK", "stream": {"id": "9"}, "profileImageURL": "https://static-cdn.jtvnw.net/jtv_user_pictures/x.png",
                      "broadcastSettings": {"title": "t", "game": {"name": "g"}}}},
            {"item": {"login": "twitchdev", "displayName": "TwitchDev", "stream": null, "broadcastSettings": {"title": "old", "game": null}}},
            {"item": {}}]}}});
        let channels = parse_channels(&search);
        assert_eq!(channels.len(), 2, "empty results are skipped");
        assert!(channels[0].live && !channels[1].live);
        assert_eq!(channels[1].game_name, "");
    }
}
