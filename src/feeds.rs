//! Persistent per-profile cache. The account gate also coalesces concurrent requests.
use crate::{
    db::Database,
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
