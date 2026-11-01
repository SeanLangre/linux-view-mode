//! Persistent per-profile cache. The account gate also coalesces concurrent requests.
use crate::{
    db::Database,
    discover,
    profiles::Account,
    ytdlp::{self, Channel, Video},
};
use anyhow::{Context, Result};
use std::collections::{HashMap, HashSet};

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
            .cache_put(profile_id, Self::FORYOU_KEY, serde_json::to_string(&videos)?, 600, stored)
            .await?;
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
