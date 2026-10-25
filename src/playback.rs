//! Playback progress belongs to the profile that started the video, never the current picker.
use crate::{
    db::Database,
    player::{EndReason, Player, Status},
    profiles::Account,
    ytdlp::{self, Video},
};
use anyhow::Result;
use std::{
    sync::Arc,
    time::{Duration, Instant},
};
use tokio::sync::Mutex;

#[derive(Clone)]
pub struct Playback {
    pub player: Player,
    db: Database,
    session: Arc<Mutex<Option<Session>>>,
}
struct Session {
    token: u64,
    account: Account,
    video: Video,
    last_tick: Instant,
    last_position: Option<f64>,
    pending_watched: f64,
    last_saved: Instant,
    mark_attempted: bool,
}
impl Playback {
    pub fn new(player: Player, db: Database) -> Self {
        Self {
            player,
            db,
            session: Default::default(),
        }
    }
    // Callers hold AppState.transition for every mutation/tick.
    pub async fn play(&self, account: Account, id: &str, start: Option<f64>) -> Result<()> {
        self.stop().await?;
        let video = self.db.video(&account.profile.id, id).await?;
        let start = match start {
            Some(start) => Some(start),
            None => self.db.resume(&account.profile.id, id).await?,
        };
        let token = self.player.play(id, start, account.cookies()).await?;
        self.db.begin_viewing(&account.profile.id, id).await?;
        self.db.dequeue(&account.profile.id, Some(id)).await?;
        *self.session.lock().await = Some(Session {
            token,
            account,
            video,
            last_tick: Instant::now(),
            last_position: None,
            pending_watched: 0.0,
            last_saved: Instant::now(),
            mark_attempted: false,
        });
        Ok(())
    }
    pub async fn stop(&self) -> Result<()> {
        let status = self.player.status().await?;
        self.save(&status, true, false).await?;
        self.player.stop().await?;
        for ended in self.player.take_ended().await? {
            self.save(&ended.status, true, ended.reason == EndReason::Finished)
                .await?;
        }
        *self.session.lock().await = None;
        Ok(())
    }
    pub async fn next(&self, account: Account) -> Result<bool> {
        self.stop().await?;
        let queue = self.db.queue(&account.profile.id).await?;
        if let Some(video) = queue.first() {
            self.play(account, &video.id, None).await?;
            return Ok(true);
        }
        Ok(false)
    }
    pub async fn tick(&self) -> Result<()> {
        let status = self.player.status().await?;
        self.save(&status, false, false).await?;
        for ended in self.player.take_ended().await? {
            self.save(&ended.status, true, ended.reason == EndReason::Finished)
                .await?;
            let account = {
                let mut session = self.session.lock().await;
                if session
                    .as_ref()
                    .is_some_and(|s| Some(s.token) == ended.status.session)
                {
                    session.take().map(|s| s.account)
                } else {
                    None
                }
            };
            if matches!(ended.reason, EndReason::Finished | EndReason::Next)
                && let Some(account) = account
            {
                self.next(account).await?;
            }
        }
        Ok(())
    }
    async fn save(&self, status: &Status, force: bool, completed: bool) -> Result<()> {
        let mut session = self.session.lock().await;
        let Some(session) = session.as_mut().filter(|s| Some(s.token) == status.session) else {
            return Ok(());
        };
        let now = Instant::now();
        let delta = now.duration_since(session.last_tick).as_secs_f64().min(2.0);
        session.last_tick = now;
        if let Some(position) = status.position {
            if !status.paused
                && session
                    .last_position
                    .is_some_and(|previous| position > previous)
            {
                // A seek advances position, but counts only the real time elapsed since the last sample.
                session.pending_watched += delta;
            }
            session.last_position = Some(position);
        }
        if let Some(title) = &status.title {
            session.video.title = title.clone();
        }
        if let Some(duration) = status.duration {
            session.video.duration = Some(duration);
        }
        let Some(position) = session.last_position else {
            return Ok(());
        };
        if force || session.last_saved.elapsed() >= Duration::from_secs(5) {
            self.db
                .save_progress(
                    &session.account.profile.id,
                    session.video.clone(),
                    position,
                    status.duration.or(session.video.duration),
                    session.pending_watched,
                    completed,
                )
                .await?;
            session.pending_watched = 0.0;
            session.last_saved = now;
            if !session.mark_attempted
                && session.account.cookies().is_some()
                && self
                    .db
                    .should_mark(&session.account.profile.id, &session.video.id)
                    .await?
            {
                session.mark_attempted = true;
                let account = session.account.clone();
                let video = session.video.id.clone();
                let db = self.db.clone();
                tokio::spawn(async move {
                    let _request = account.gate.lock().await;
                    if let Some(cookies) = account.cookies() {
                        if ytdlp::mark_watched(&video, cookies).await.is_ok() {
                            if let Err(err) = db.set_marked(&account.profile.id, &video).await {
                                tracing::error!("could not save watched status: {err:#}");
                            }
                        } else {
                            tracing::warn!(profile_id=%account.profile.id,video_id=%video,"YouTube watched-history sync failed; it can retry on a later viewing");
                        }
                    }
                });
            }
        }
        Ok(())
    }
}
