//! Persistent per-profile cache. The account gate also coalesces concurrent requests.
//!
//! An answer is *fresh* for its time limit. After that it is still shown at once (for up to a
//! week, `db::STALE_SECS`) while one background request per list fetches a new copy, so opening a
//! screen never waits for yt-dlp unless nothing was ever saved. The pictures of every loaded list
//! are saved on disk by `Thumbs`.
use crate::{
    db::Database,
    discover,
    profiles::Account,
    thumbs::Thumbs,
    ytdlp::{self, Channel, Video},
};
use anyhow::{Context, Result};
use std::{
    collections::{HashMap, HashSet},
    sync::{Arc, Mutex as StdMutex},
    time::{Duration, Instant},
};

/// How long a list stays fresh, in seconds. Longer than the old limits, because an expired list is
/// now shown at once and refreshed in the background rather than waited for.
const FEED_FRESH: u32 = 1800;
const HISTORY_FRESH: u32 = 300;
const SEARCH_FRESH: u32 = 6 * 3600;
const CHANNEL_FRESH: u32 = 3600;
const FORYOU_FRESH: u32 = 1800;

/// How long after an old copy was shown the screen is told to ask again (see `served_stale`).
const STALE_WINDOW: Duration = Duration::from_secs(30);

/// The database, the picture store, the background refreshes in flight (`profile\nkey`) and when
/// each profile was last shown an old copy.
#[derive(Clone)]
pub struct Feeds(pub Database, Thumbs, Arc<StdMutex<HashSet<String>>>, Arc<StdMutex<HashMap<String, Instant>>>);
impl Feeds {
    pub fn new(db: Database, thumbs: Thumbs) -> Self {
        Self(db, thumbs, Arc::default(), Arc::default())
    }

    fn note_stale(&self, profile: &str) {
        self.3.lock().unwrap().insert(profile.to_owned(), Instant::now());
    }

    /// Runs `task` in the background unless the same list is already being refreshed.
    fn refresh_later(&self, profile: &str, key: &str, task: impl std::future::Future<Output = ()> + Send + 'static) {
        let id = format!("{profile}\n{key}");
        if !self.2.lock().unwrap().insert(id.clone()) {
            return;
        }
        let running = self.2.clone();
        tokio::spawn(async move {
            task.await;
            running.lock().unwrap().remove(&id);
        });
    }

    /// True if this profile was just shown an old copy of a list (within `STALE_WINDOW`), so a
    /// new one is being fetched. The API tells the screen, which asks again a few seconds later.
    /// Recorded when the old copy is shown, because the refresh can finish before the answer goes out.
    pub fn served_stale(&self, profile: &str) -> bool {
        self.3.lock().unwrap().get(profile).is_some_and(|at| at.elapsed() < STALE_WINDOW)
    }

    pub async fn search(
        &self,
        account: &Account,
        query: &str,
        limit: u32,
        refresh: bool,
    ) -> Result<Vec<Video>> {
        let key = format!("search:{limit}:{query}");
        if !refresh && let Some((json, fresh)) = self.0.cache_get_stale(&account.profile.id, &key).await? {
            if !fresh {
                self.note_stale(&account.profile.id);
                let (feeds, account, key, query) = (self.clone(), account.clone(), key.clone(), query.to_owned());
                self.refresh_later(&account.profile.id.clone(), &key.clone(), async move {
                    let _request = account.gate.lock().await;
                    if let Err(err) = feeds.load_search(&account, &key, &query, limit).await {
                        tracing::debug!("background search refresh failed: {err:#}");
                    }
                });
            }
            return Ok(serde_json::from_str(&json)?);
        }
        let _request = account.gate.lock().await;
        if !refresh && let Some(json) = self.0.cache_get(&account.profile.id, &key).await? {
            return Ok(serde_json::from_str(&json)?);
        }
        self.load_search(account, &key, query, limit).await
    }
    /// Asks yt-dlp and stores the answer; the caller holds the account's gate.
    async fn load_search(&self, account: &Account, key: &str, query: &str, limit: u32) -> Result<Vec<Video>> {
        let videos = ytdlp::search(query, limit, account.cookies()).await?;
        self.0
            .cache_put(&account.profile.id, key, serde_json::to_string(&videos)?, SEARCH_FRESH, videos.clone())
            .await?;
        self.1.prefetch(&videos);
        Ok(videos)
    }
    pub async fn channel(
        &self,
        account: &Account,
        id: &str,
        page: u32,
        refresh: bool,
    ) -> Result<Channel> {
        let key = format!("channel:{id}:{page}");
        if !refresh && let Some((json, fresh)) = self.0.cache_get_stale(&account.profile.id, &key).await? {
            if !fresh {
                self.note_stale(&account.profile.id);
                let (feeds, account, key, id) = (self.clone(), account.clone(), key.clone(), id.to_owned());
                self.refresh_later(&account.profile.id.clone(), &key.clone(), async move {
                    let _request = account.gate.lock().await;
                    if let Err(err) = feeds.load_channel(&account, &key, &id, page).await {
                        tracing::debug!("background channel refresh failed: {err:#}");
                    }
                });
            }
            return Ok(serde_json::from_str(&json)?);
        }
        let _request = account.gate.lock().await;
        if !refresh && let Some(json) = self.0.cache_get(&account.profile.id, &key).await? {
            return Ok(serde_json::from_str(&json)?);
        }
        self.load_channel(account, &key, id, page).await
    }
    async fn load_channel(&self, account: &Account, key: &str, id: &str, page: u32) -> Result<Channel> {
        let channel = ytdlp::channel(id, page, account.cookies()).await?;
        self.0
            .cache_put(&account.profile.id, key, serde_json::to_string(&channel)?, CHANNEL_FRESH, channel.videos.clone())
            .await?;
        self.1.prefetch(&channel.videos);
        Ok(channel)
    }
    pub async fn feed(
        &self,
        account: &Account,
        kind: &str,
        page: u32,
        refresh: bool,
    ) -> Result<Vec<Video>> {
        // Local lists (continue watching, guest history) are not paged.
        if page > 0 && (kind == "continue" || kind == "local" || account.cookies().is_none()) {
            return Ok(vec![]);
        }
        if kind == "continue" {
            return self.0.progress_list(&account.profile.id, true).await;
        }
        // CouchTube's own history of this profile, never YouTube's.
        if kind == "local" {
            return self.0.progress_list(&account.profile.id, false).await;
        }
        // Guest has local watch history; account feeds require a cookie database.
        if account.cookies().is_none() {
            return if kind == "history" {
                self.0.progress_list(&account.profile.id, false).await
            } else {
                Ok(vec![])
            };
        }
        let key = format!("feed:{kind}:{page}");
        if !refresh && let Some((json, fresh)) = self.0.cache_get_stale(&account.profile.id, &key).await? {
            if !fresh {
                self.note_stale(&account.profile.id);
                let (feeds, account, key, kind) = (self.clone(), account.clone(), key.clone(), kind.to_owned());
                self.refresh_later(&account.profile.id.clone(), &key.clone(), async move {
                    let _request = account.gate.lock().await;
                    if let Err(err) = feeds.load_feed(&account, &key, &kind, page).await {
                        tracing::debug!("background feed refresh failed: {err:#}");
                    }
                });
            }
            return Ok(serde_json::from_str(&json)?);
        }
        let _request = account.gate.lock().await;
        if !refresh && let Some(json) = self.0.cache_get(&account.profile.id, &key).await? {
            return Ok(serde_json::from_str(&json)?);
        }
        self.load_feed(account, &key, kind, page).await
    }
    async fn load_feed(&self, account: &Account, key: &str, kind: &str, page: u32) -> Result<Vec<Video>> {
        let videos = ytdlp::feed(kind, page, account.cookies().context("sign in to load this feed")?).await?;
        self.0
            .cache_put(
                &account.profile.id,
                key,
                serde_json::to_string(&videos)?,
                if kind == "history" { HISTORY_FRESH } else { FEED_FRESH },
                videos.clone(),
            )
            .await?;
        self.1.prefetch(&videos);
        Ok(videos)
    }
    /// Cache key of a profile's ranked "For You" list.
    pub const FORYOU_KEY: &'static str = "foryou:all";

    /// The profile's own recommendations, built from its seeds. `guest` runs every request, so
    /// no account cookies are used; its caches are shared between profiles.
    pub async fn foryou(
        &self,
        profile_id: &str,
        guest: &Account,
        page: u32,
        refresh: bool,
    ) -> Result<Vec<Video>> {
        let all = self.foryou_all(profile_id, guest, refresh).await?;
        let size = ytdlp::PAGE_SIZE as usize;
        Ok(all.into_iter().skip(page as usize * size).take(size).collect())
    }

    async fn foryou_all(&self, profile_id: &str, guest: &Account, refresh: bool) -> Result<Vec<Video>> {
        if !refresh && let Some((json, fresh)) = self.0.cache_get_stale(profile_id, Self::FORYOU_KEY).await? {
            if !fresh {
                self.note_stale(profile_id);
                // Rebuild from the (cached) sources in the background; each of those refreshes itself the same way.
                let (feeds, guest, profile) = (self.clone(), guest.clone(), profile_id.to_owned());
                self.refresh_later(profile_id, Self::FORYOU_KEY, async move {
                    if let Err(err) = feeds.build_foryou(&profile, &guest, false).await {
                        tracing::debug!("background For You refresh failed: {err:#}");
                    }
                });
            }
            return Ok(serde_json::from_str(&json)?);
        }
        self.build_foryou(profile_id, guest, refresh).await
    }

    /// Builds the ranked list from the seeds. `refresh` also forces every source search.
    async fn build_foryou(&self, profile_id: &str, guest: &Account, refresh: bool) -> Result<Vec<Video>> {
        let seeds = self.0.seeds(profile_id).await?;
        let hour = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| (d.as_secs() / 3600) as usize);
        let (themes, channels, videos) = discover::pick_sources(&seeds, hour);
        let mut sources = Vec::new();
        let mut failed = 0;
        let mut seed_videos = HashSet::new();
        for (label, query, needs) in discover::blends(&themes, hour) {
            match self.search(guest, &query, discover::BLEND_LIMIT, refresh).await {
                Ok(videos) => sources.push(discover::Source { label, videos, needs }),
                Err(err) => { failed += 1; tracing::warn!("For You blended search failed: {err:#}"); }
            }
        }
        for seed in themes {
            match self.search(guest, &seed.value, discover::THEME_LIMIT, refresh).await {
                Ok(videos) => sources.push(discover::Source { label: seed.label.clone(), videos, needs: vec![] }),
                Err(err) => { failed += 1; tracing::warn!("For You theme search failed: {err:#}"); }
            }
        }
        for seed in channels {
            if !ytdlp::is_channel_id(&seed.value) { continue; }
            match self.channel(guest, &seed.value, 0, refresh).await {
                Ok(channel) => sources.push(discover::Source {
                    label: seed.label.clone(),
                    videos: channel.videos.into_iter().take(discover::CHANNEL_LIMIT).collect(),
                    needs: vec![],
                }),
                Err(err) => { failed += 1; tracing::warn!("For You channel failed: {err:#}"); }
            }
        }
        // A video seed becomes a search for its title, so we find videos like it.
        for seed in videos {
            seed_videos.insert(seed.value.clone());
            let title = if seed.label != seed.value {
                seed.label.clone()
            } else {
                let info = {
                    let _request = guest.gate.lock().await;
                    ytdlp::video_info(&seed.value).await
                };
                match info {
                    Ok((title, _)) => {
                        self.0.set_seed_label(profile_id, seed.id, &title).await?;
                        title
                    }
                    Err(err) => { failed += 1; tracing::warn!("For You video lookup failed: {err:#}"); continue; }
                }
            };
            match self.search(guest, &discover::query_from_title(&title), discover::THEME_LIMIT, refresh).await {
                Ok(found) => sources.push(discover::Source { label: format!("like {title}"), videos: found, needs: vec![] }),
                Err(err) => { failed += 1; tracing::warn!("For You similar search failed: {err:#}"); }
            }
        }
        // Do not cache a list that is empty only because every request failed.
        if sources.is_empty() && failed > 0 {
            anyhow::bail!("every discovery request failed");
        }
        let seed_channels: HashSet<String> = seeds.iter().filter(|s| s.kind == "channel").map(|s| s.value.clone()).collect();
        self.fill_view_counts(guest, &mut sources, &seed_channels).await;
        let mut known = self.0.watched_ids(profile_id).await?;
        let (dismissed, dismissed_channels) = self.0.dismissed(profile_id).await?;
        known.extend(dismissed);
        known.extend(seed_videos);
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_secs() as i64);
        let excluded: Vec<String> = seeds.iter().filter(|s| s.kind == "exclude").map(|s| s.value.clone()).collect();
        let videos = discover::rank(sources, &known, &dismissed_channels, &seed_channels, &excluded, now);
        // The reason belongs to this list only, so the shared video table keeps none.
        let stored = videos.iter().cloned().map(|mut v| { v.reason = None; v }).collect();
        self.0
            .cache_put(profile_id, Self::FORYOU_KEY, serde_json::to_string(&videos)?, FORYOU_FRESH, stored)
            .await?;
        self.1.prefetch(&videos);
        Ok(videos)
    }

    /// Videos from channels that are not seeds need a view count to pass the minimum, but channel
    /// pages sometimes leave it out. Looks up a few of the missing ones (remembered for a week).
    async fn fill_view_counts(&self, guest: &Account, sources: &mut [discover::Source], seed_channels: &HashSet<String>) {
        let known: HashSet<String> = sources.iter().flat_map(|s| &s.videos).filter(|v| v.view_count.is_some()).map(|v| v.id.clone()).collect();
        let mut missing: Vec<String> = Vec::new();
        for video in sources.iter().flat_map(|s| &s.videos) {
            let seeded = video.channel_id.as_ref().is_some_and(|c| seed_channels.contains(c));
            if video.view_count.is_none() && !seeded && !known.contains(&video.id) && !missing.contains(&video.id) {
                missing.push(video.id.clone());
            }
        }
        let mut counts: HashMap<String, u64> = HashMap::new();
        let mut looked_up = 0;
        for id in missing {
            let key = format!("views:{id}");
            if let Ok(Some(json)) = self.0.cache_get(&guest.profile.id, &key).await {
                if let Ok(Some(count)) = serde_json::from_str::<Option<u64>>(&json) { counts.insert(id, count); }
                continue;
            }
            if looked_up >= discover::MAX_VIEW_LOOKUPS { continue; }
            looked_up += 1;
            let found = {
                let _request = guest.gate.lock().await;
                ytdlp::view_count(&id).await
            };
            match found {
                Ok(count) => {
                    let _ = self.0.cache_put(&guest.profile.id, &key, serde_json::to_string(&count).unwrap_or_default(), 7 * 86_400, vec![]).await;
                    if let Some(count) = count { counts.insert(id, count); }
                }
                Err(err) => tracing::debug!("For You view count lookup failed: {err:#}"),
            }
        }
        for video in sources.iter_mut().flat_map(|s| s.videos.iter_mut()) {
            if video.view_count.is_none() { video.view_count = counts.get(&video.id).copied(); }
        }
    }

    pub fn prefetch(&self, account: Account) {
        if account.cookies().is_none() {
            return;
        }
        let feeds = self.clone();
        tokio::spawn(async move {
            for kind in ["recommended", "subscriptions"] {
                if feeds.feed(&account, kind, 0, false).await.is_err() {
                    tracing::debug!(profile_id=%account.profile.id, kind, "feed prefetch failed");
                }
            }
        });
    }
}
