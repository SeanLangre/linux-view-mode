//! Persistent per-profile cache. The account gate also coalesces concurrent requests.
use crate::{
    db::Database,
    discover,
    profiles::Account,
    ytdlp::{self, Channel, Video},
};
use anyhow::{Context, Result};

#[derive(Clone)]
pub struct Feeds(pub Database);
impl Feeds {
    pub async fn search(
        &self,
        account: &Account,
        query: &str,
        limit: u32,
        refresh: bool,
    ) -> Result<Vec<Video>> {
        let key = format!("search:{limit}:{query}");
        if !refresh && let Some(json) = self.0.cache_get(&account.profile.id, &key).await? {
            return Ok(serde_json::from_str(&json)?);
        }
        let _request = account.gate.lock().await;
        if !refresh && let Some(json) = self.0.cache_get(&account.profile.id, &key).await? {
            return Ok(serde_json::from_str(&json)?);
        }
        let videos = ytdlp::search(query, limit, account.cookies()).await?;
        self.0
            .cache_put(
                &account.profile.id,
                &key,
                serde_json::to_string(&videos)?,
                1800,
                videos.clone(),
            )
            .await?;
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
        if !refresh && let Some(json) = self.0.cache_get(&account.profile.id, &key).await? {
            return Ok(serde_json::from_str(&json)?);
        }
        let _request = account.gate.lock().await;
        if !refresh && let Some(json) = self.0.cache_get(&account.profile.id, &key).await? {
            return Ok(serde_json::from_str(&json)?);
        }
        let channel = ytdlp::channel(id, page, account.cookies()).await?;
        self.0
            .cache_put(
                &account.profile.id,
                &key,
                serde_json::to_string(&channel)?,
                600,
                channel.videos.clone(),
            )
            .await?;
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
        if page > 0 && (kind == "continue" || account.cookies().is_none()) {
            return Ok(vec![]);
        }
        if kind == "continue" {
            return self.0.progress_list(&account.profile.id, true).await;
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
        if !refresh && let Some(json) = self.0.cache_get(&account.profile.id, &key).await? {
            return Ok(serde_json::from_str(&json)?);
        }
        let _request = account.gate.lock().await;
        if !refresh && let Some(json) = self.0.cache_get(&account.profile.id, &key).await? {
            return Ok(serde_json::from_str(&json)?);
        }
        let videos = ytdlp::feed(
            kind,
            page,
            account.cookies().context("sign in to load this feed")?,
        )
        .await?;
        self.0
            .cache_put(
                &account.profile.id,
                &key,
                serde_json::to_string(&videos)?,
                if kind == "history" { 120 } else { 600 },
                videos.clone(),
            )
            .await?;
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
        if !refresh && let Some(json) = self.0.cache_get(profile_id, Self::FORYOU_KEY).await? {
            return Ok(serde_json::from_str(&json)?);
        }
        let seeds = self.0.seeds(profile_id).await?;
        let (themes, channels) = discover::pick_sources(&seeds);
        let mut sources = Vec::new();
        let mut failed = 0;
        for seed in themes {
            match self.search(guest, &seed.value, discover::THEME_LIMIT, refresh).await {
                Ok(videos) => sources.push(videos),
                Err(err) => { failed += 1; tracing::warn!("For You theme search failed: {err:#}"); }
            }
        }
        for seed in channels {
            if !ytdlp::is_channel_id(&seed.value) { continue; }
            match self.channel(guest, &seed.value, 0, refresh).await {
                Ok(channel) => sources.push(channel.videos.into_iter().take(discover::CHANNEL_LIMIT).collect()),
                Err(err) => { failed += 1; tracing::warn!("For You channel failed: {err:#}"); }
            }
        }
        // Do not cache a list that is empty only because every request failed.
        if sources.is_empty() && failed > 0 {
            anyhow::bail!("every discovery request failed");
        }
        let known = self.0.watched_ids(profile_id).await?;
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_secs() as i64);
        let videos = discover::rank(sources, &known, now);
        self.0
            .cache_put(profile_id, Self::FORYOU_KEY, serde_json::to_string(&videos)?, 600, videos.clone())
            .await?;
        Ok(videos)
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
