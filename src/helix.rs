//! Minimal read-only Twitch Helix client (see `docs/README-twitch.md`).
//!
//! HTTP goes through `curl`, like SponsorBlock and yt-dlp elsewhere, so no TLS crate is needed. The
//! client secret and the access token are written to curl's config on stdin: they never appear in
//! `ps` or a log. This uses an **app access token** (client-credentials flow), which is enough for
//! public data: top games, streams and channel search.
use anyhow::{Context, Result, bail};
use serde::Serialize;
use serde_json::Value;
use std::{
    collections::HashMap,
    process::Stdio,
    sync::{Arc, Mutex as StdMutex},
    time::{Duration, Instant},
};
use tokio::{io::AsyncWriteExt, process::Command, sync::Mutex};

const DEFAULT_API: &str = "https://api.twitch.tv/helix";
const DEFAULT_TOKEN_URL: &str = "https://id.twitch.tv/oauth2/token";
pub const PAGE_SIZE: u32 = 24;
const CURL_TIMEOUT: Duration = Duration::from_secs(20);
pub const TTL_CATEGORIES: Duration = Duration::from_secs(600);
pub const TTL_LIVE: Duration = Duration::from_secs(60);
const CACHE_LIMIT: usize = 200;

/// A small in-memory cache of answers, shared by the official and unofficial clients.
pub struct Cache(StdMutex<HashMap<String, (Instant, Value)>>);

impl Cache {
    pub fn new() -> Self {
        Self(StdMutex::new(HashMap::new()))
    }

    pub fn get(&self, key: &str, ttl: Duration) -> Option<Value> {
        let map = self.0.lock().unwrap();
        map.get(key).filter(|(stored, _)| stored.elapsed() < ttl).map(|(_, value)| value.clone())
    }

    /// Forget every answer whose key starts with `prefix` (one profile's personal answers).
    pub fn remove_prefix(&self, prefix: &str) {
        self.0.lock().unwrap().retain(|key, _| !key.starts_with(prefix));
    }

    pub fn put(&self, key: String, value: Value) {
        let mut map = self.0.lock().unwrap();
        if map.len() >= CACHE_LIMIT {
            map.retain(|_, (stored, _)| stored.elapsed() < TTL_CATEGORIES);
            if map.len() >= CACHE_LIMIT {
                map.clear();
            }
        }
        map.insert(key, (Instant::now(), value));
    }
}

struct Credentials {
    id: String,
    secret: String,
}

struct Token {
    value: String,
    expires: Instant,
}

struct Inner {
    credentials: Option<Credentials>,
    api: String,
    token_url: String,
    token: Mutex<Option<Token>>,
    /// One Twitch request at a time, like the one-yt-dlp-at-a-time rule on the YouTube side.
    gate: Mutex<()>,
    cache: Cache,
}

#[derive(Clone)]
pub struct Helix(Arc<Inner>);

/// Twitch ids and secrets are plain alphanumerics. Anything else is refused, so a value can never
/// break out of the curl config it is written into.
pub(crate) fn valid_credential(value: &str) -> bool {
    (10..=64).contains(&value.len()) && value.bytes().all(|b| b.is_ascii_alphanumeric())
}

/// A base URL for tests and mirrors: scheme and host only use characters that are safe in a curl config.
pub(crate) fn valid_url(value: &str) -> bool {
    (value.starts_with("https://") || value.starts_with("http://127.0.0.1:"))
        && value.bytes().all(|b| b.is_ascii_alphanumeric() || b"-._~:/".contains(&b))
}

impl Helix {
    /// Reads `COUCHTUBE_TWITCH_CLIENT_ID` and `COUCHTUBE_TWITCH_CLIENT_SECRET`. Without both,
    /// Twitch mode shows how to set them up instead of failing.
    pub fn from_env() -> Self {
        let id = std::env::var("COUCHTUBE_TWITCH_CLIENT_ID").ok();
        let secret = std::env::var("COUCHTUBE_TWITCH_CLIENT_SECRET").ok();
        let credentials = match (id, secret) {
            (Some(id), Some(secret)) if valid_credential(&id) && valid_credential(&secret) => Some(Credentials { id, secret }),
            (None, None) => None,
            _ => {
                tracing::warn!("Twitch client id/secret: one of the pair is missing or not 10-64 letters and digits; using the unofficial Twitch API instead");
                None
            }
        };
        let url = |name: &str, default: &str| {
            std::env::var(name).ok().filter(|value| valid_url(value)).unwrap_or_else(|| default.to_owned())
        };
        Self::new(credentials, url("COUCHTUBE_TWITCH_API_URL", DEFAULT_API), url("COUCHTUBE_TWITCH_TOKEN_URL", DEFAULT_TOKEN_URL))
    }

    fn new(credentials: Option<Credentials>, api: String, token_url: String) -> Self {
        Self(Arc::new(Inner {
            credentials,
            api,
            token_url,
            token: Mutex::new(None),
            gate: Mutex::new(()),
            cache: Cache::new(),
        }))
    }

    pub fn configured(&self) -> bool {
        self.0.credentials.is_some()
    }

    /// GET a Helix path with a query, cached for `ttl` (skipped when `refresh` is set).
    pub async fn get(&self, path: &str, query: &[(&str, &str)], ttl: Duration, refresh: bool) -> Result<Value> {
        let credentials = self.0.credentials.as_ref().context("Twitch is not set up")?;
        let url = format!("{}/{}{}", self.0.api, path, query_string(query));
        if !refresh {
            if let Some(value) = self.0.cache.get(&url, ttl) {
                return Ok(value);
            }
        }
        let _gate = self.0.gate.lock().await;
        let mut value = None;
        for attempt in 0..2 {
            let token = self.token(credentials, attempt == 1).await?;
            let (status, body) = curl(get_config(&url, &credentials.id, &token)).await?;
            match status {
                200 => {
                    value = Some(serde_json::from_str::<Value>(&body).context("Twitch sent something that is not JSON")?);
                    break;
                }
                // The token expired or was revoked: get a new one and try once more.
                401 if attempt == 0 => continue,
                other => bail!("Twitch answered HTTP {other}: {}", message(&body)),
            }
        }
        let value = value.context("Twitch refused the access token")?;
        self.0.cache.put(url, value.clone());
        Ok(value)
    }

    async fn token(&self, credentials: &Credentials, renew: bool) -> Result<String> {
        let mut slot = self.0.token.lock().await;
        if !renew {
            if let Some(token) = slot.as_ref().filter(|t| t.expires > Instant::now()) {
                return Ok(token.value.clone());
            }
        }
        let (status, body) = curl(token_config(&self.0.token_url, credentials)).await?;
        if status != 200 {
            bail!("Twitch refused the client id/secret (HTTP {status}): {}", message(&body));
        }
        let json: Value = serde_json::from_str(&body).context("Twitch sent something that is not JSON")?;
        let value = json["access_token"].as_str().context("no access token in Twitch's answer")?.to_owned();
        if !value.bytes().all(|b| b.is_ascii_alphanumeric()) {
            bail!("unexpected characters in Twitch's access token");
        }
        let seconds = json["expires_in"].as_u64().unwrap_or(3600).saturating_sub(60).max(60);
        *slot = Some(Token { value: value.clone(), expires: Instant::now() + Duration::from_secs(seconds) });
        Ok(value)
    }

    pub async fn top_categories(&self, cursor: Option<&str>, refresh: bool) -> Result<Page<Category>> {
        let first = PAGE_SIZE.to_string();
        let mut query = vec![("first", first.as_str())];
        query.extend(cursor.map(|c| ("after", c)));
        Ok(parse_categories(&self.get("games/top", &query, TTL_CATEGORIES, refresh).await?))
    }

    /// Live streams, all of Twitch or one category.
    pub async fn streams(&self, game_id: Option<&str>, cursor: Option<&str>, refresh: bool) -> Result<Page<Stream>> {
        let first = PAGE_SIZE.to_string();
        let mut query = vec![("first", first.as_str())];
        query.extend(game_id.map(|g| ("game_id", g)));
        query.extend(cursor.map(|c| ("after", c)));
        Ok(parse_streams(&self.get("streams", &query, TTL_LIVE, refresh).await?))
    }

    pub async fn search_channels(&self, text: &str, cursor: Option<&str>, refresh: bool) -> Result<Page<Channel>> {
        let first = PAGE_SIZE.to_string();
        let mut query = vec![("query", text), ("first", first.as_str())];
        query.extend(cursor.map(|c| ("after", c)));
        Ok(parse_channels(&self.get("search/channels", &query, TTL_LIVE, refresh).await?))
    }
}

// ---- requests -------------------------------------------------------------------------------

fn percent_encode(text: &str) -> String {
    let mut out = String::new();
    for byte in text.bytes() {
        if byte.is_ascii_alphanumeric() || b"-._~".contains(&byte) {
            out.push(byte as char);
        } else {
            out.push_str(&format!("%{byte:02X}"));
        }
    }
    out
}

fn query_string(query: &[(&str, &str)]) -> String {
    if query.is_empty() {
        return String::new();
    }
    let parts: Vec<String> = query.iter().map(|(k, v)| format!("{}={}", percent_encode(k), percent_encode(v))).collect();
    format!("?{}", parts.join("&"))
}

fn get_config(url: &str, client_id: &str, token: &str) -> String {
    format!(
        "silent\nshow-error\nmax-time = 15\nurl = \"{url}\"\nheader = \"Client-Id: {client_id}\"\nheader = \"Authorization: Bearer {token}\"\nwrite-out = \"\\n%{{http_code}}\"\n"
    )
}

fn token_config(url: &str, credentials: &Credentials) -> String {
    format!(
        "silent\nshow-error\nmax-time = 15\nurl = \"{url}\"\nrequest = \"POST\"\ndata-urlencode = \"client_id={}\"\ndata-urlencode = \"client_secret={}\"\ndata-urlencode = \"grant_type=client_credentials\"\nwrite-out = \"\\n%{{http_code}}\"\n",
        credentials.id, credentials.secret
    )
}

/// Runs curl with its config on stdin; returns the HTTP status and the body.
pub(crate) async fn curl(config: String) -> Result<(u16, String)> {
    // `-q` first: ignore any ~/.curlrc.
    let mut child = Command::new("curl")
        .args(["-q", "-K", "-"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .context("could not start curl (is it installed?)")?;
    let mut stdin = child.stdin.take().context("curl has no stdin")?;
    stdin.write_all(config.as_bytes()).await?;
    drop(stdin);
    let output = tokio::time::timeout(CURL_TIMEOUT, child.wait_with_output())
        .await
        .context("Twitch did not answer in time")??;
    if !output.status.success() {
        // curl's message names the host and the problem, never the config.
        bail!("curl failed: {}", String::from_utf8_lossy(&output.stderr).trim());
    }
    let text = String::from_utf8_lossy(&output.stdout).into_owned();
    let (body, status) = text.rsplit_once('\n').context("no HTTP status from curl")?;
    Ok((status.trim().parse().context("bad HTTP status from curl")?, body.to_owned()))
}

/// Twitch's error text, for the log only.
pub(crate) fn message(body: &str) -> String {
    serde_json::from_str::<Value>(body)
        .ok()
        .and_then(|v| v["message"].as_str().map(str::to_owned))
        .unwrap_or_default()
}

// ---- answers --------------------------------------------------------------------------------

#[derive(Serialize, Debug, PartialEq)]
pub struct Page<T> {
    pub items: Vec<T>,
    pub cursor: Option<String>,
}

#[derive(Serialize, Debug, PartialEq)]
pub struct Category {
    pub id: String,
    pub name: String,
    pub box_art: String,
}

#[derive(Serialize, Debug, PartialEq)]
pub struct Stream {
    pub login: String,
    pub name: String,
    pub title: String,
    pub game_id: String,
    pub game_name: String,
    pub viewers: u64,
    pub thumb: String,
    pub started_at: Option<String>,
}

#[derive(Serialize, Debug, PartialEq)]
pub struct Channel {
    pub login: String,
    pub name: String,
    pub live: bool,
    pub title: String,
    pub game_name: String,
    pub thumb: String,
}

pub(crate) fn text(value: &Value, key: &str) -> String {
    value[key].as_str().unwrap_or_default().to_owned()
}

/// Helix image URLs carry `{width}x{height}` placeholders.
pub(crate) fn sized(url: &str, width: u32, height: u32) -> String {
    url.replace("{width}", &width.to_string()).replace("{height}", &height.to_string())
}

/// Only images from Twitch's own host are passed to the page.
pub(crate) fn image(url: String) -> String {
    if url.starts_with("https://static-cdn.jtvnw.net/") || url.starts_with("https://static-cdn.jtvnw.net:") { url } else { String::new() }
}

fn cursor(body: &Value) -> Option<String> {
    body["pagination"]["cursor"].as_str().filter(|c| valid_cursor(c)).map(str::to_owned)
}

fn items(body: &Value) -> &[Value] {
    body["data"].as_array().map_or(&[], Vec::as_slice)
}

pub fn parse_categories(body: &Value) -> Page<Category> {
    Page {
        items: items(body)
            .iter()
            .filter(|g| is_id(g["id"].as_str().unwrap_or_default()))
            .map(|g| Category { id: text(g, "id"), name: text(g, "name"), box_art: image(sized(&text(g, "box_art_url"), 285, 380)) })
            .collect(),
        cursor: cursor(body),
    }
}

pub fn parse_streams(body: &Value) -> Page<Stream> {
    Page {
        items: items(body)
            .iter()
            .filter(|s| is_login(s["user_login"].as_str().unwrap_or_default()))
            .map(|s| Stream {
                login: text(s, "user_login"),
                name: text(s, "user_name"),
                title: text(s, "title"),
                game_id: text(s, "game_id"),
                game_name: text(s, "game_name"),
                viewers: s["viewer_count"].as_u64().unwrap_or(0),
                thumb: image(sized(&text(s, "thumbnail_url"), 440, 248)),
                started_at: s["started_at"].as_str().map(str::to_owned),
            })
            .collect(),
        cursor: cursor(body),
    }
}

pub fn parse_channels(body: &Value) -> Page<Channel> {
    Page {
        items: items(body)
            .iter()
            .filter(|c| is_login(c["broadcaster_login"].as_str().unwrap_or_default()))
            .map(|c| Channel {
                login: text(c, "broadcaster_login"),
                name: text(c, "display_name"),
                live: c["is_live"].as_bool().unwrap_or(false),
                title: text(c, "title"),
                game_name: text(c, "game_name"),
                thumb: image(text(c, "thumbnail_url")),
            })
            .collect(),
        cursor: cursor(body),
    }
}

// ---- validation -----------------------------------------------------------------------------

/// Category (game) and video ids are digits.
pub fn is_id(id: &str) -> bool {
    (1..=20).contains(&id.len()) && id.bytes().all(|b| b.is_ascii_digit())
}

/// Twitch logins are 1-25 letters, digits and underscores.
pub fn is_login(login: &str) -> bool {
    (1..=25).contains(&login.len()) && login.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_')
}

/// Pagination cursors are opaque base64-like strings.
pub fn valid_cursor(cursor: &str) -> bool {
    (1..=300).contains(&cursor.len()) && cursor.bytes().all(|b| b.is_ascii_alphanumeric() || b"=_-.".contains(&b))
}

/// A search text: 1-100 characters without control characters.
pub fn clean_query(text: &str) -> Option<String> {
    let text = text.trim();
    ((1..=100).contains(&text.chars().count()) && !text.chars().any(char::is_control)).then(|| text.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn credentials_and_urls_cannot_break_out_of_the_curl_config() {
        assert!(valid_credential("abcdefghij0123456789"));
        for bad in ["short", "has space 1234567890", "quote\"1234567890", "new\nline1234567890", &"a".repeat(65)] {
            assert!(!valid_credential(bad), "{bad:?}");
        }
        assert!(valid_url("https://api.twitch.tv/helix"));
        assert!(valid_url("http://127.0.0.1:9999/helix"));
        for bad in ["http://evil.example/helix", "https://x\"y", "https://a b", "ftp://x"] {
            assert!(!valid_url(bad), "{bad:?}");
        }
    }

    #[test]
    fn parses_categories_streams_and_channels() {
        let games = json!({"data": [
            {"id": "509658", "name": "Just Chatting", "box_art_url": "https://static-cdn.jtvnw.net/ttv-boxart/509658-{width}x{height}.jpg"},
            {"id": "not-an-id", "name": "Bad", "box_art_url": ""}],
            "pagination": {"cursor": "abc123"}});
        let page = parse_categories(&games);
        assert_eq!(page.items.len(), 1);
        assert_eq!(page.items[0].box_art, "https://static-cdn.jtvnw.net/ttv-boxart/509658-285x380.jpg");
        assert_eq!(page.cursor.as_deref(), Some("abc123"));

        let streams = json!({"data": [
            {"user_login": "shroud", "user_name": "shroud", "title": "hi", "game_id": "32982", "game_name": "GTA V",
             "viewer_count": 4200, "started_at": "2026-11-08T10:00:00Z",
             "thumbnail_url": "https://static-cdn.jtvnw.net/previews-ttv/live_user_shroud-{width}x{height}.jpg"},
            {"user_login": "bad login!", "user_name": "x", "title": "", "viewer_count": 1, "thumbnail_url": ""},
            {"user_login": "evil", "user_name": "evil", "title": "t", "viewer_count": 2, "thumbnail_url": "https://evil.example/x.jpg"}],
            "pagination": {}});
        let page = parse_streams(&streams);
        assert_eq!(page.items.len(), 2);
        assert_eq!((page.items[0].viewers, page.items[0].game_name.as_str()), (4200, "GTA V"));
        assert_eq!(page.items[0].thumb, "https://static-cdn.jtvnw.net/previews-ttv/live_user_shroud-440x248.jpg");
        // A thumbnail from another host is dropped rather than passed to the page.
        assert_eq!(page.items[1].thumb, "");
        assert_eq!(page.cursor, None);

        let channels = json!({"data": [{"broadcaster_login": "lirik", "display_name": "LIRIK", "is_live": true,
            "title": "t", "game_name": "g", "thumbnail_url": "https://static-cdn.jtvnw.net/jtv_user_pictures/x.png"}]});
        let page = parse_channels(&channels);
        assert_eq!((page.items[0].login.as_str(), page.items[0].live), ("lirik", true));
        assert!(parse_channels(&json!({})).items.is_empty());
    }
}
