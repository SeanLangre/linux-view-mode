//! Music's rolling download cache (ported from the desktop music player): songs download in the
//! background and only the most recently downloaded N are kept. Files are shared by all music
//! profiles and live in `<data>/music/cache/`.
use std::{
    collections::{HashMap, HashSet},
    os::unix::fs::DirBuilderExt,
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
};

use anyhow::{Context, Result};
use tokio::{sync::Semaphore, task::AbortHandle};

use crate::{music_db::MusicTrack, music_yt, db::Database};

pub const DEFAULT_LIMIT: usize = 50;
/// The sizes the Settings screen offers.
pub const LIMITS: [usize; 4] = [25, 50, 100, 200];
const MAX_CONCURRENT_DOWNLOADS: usize = 2;

/// What the screens show for a song.
#[derive(Clone, Debug, PartialEq)]
pub enum DownloadState {
    /// In the cache: plays from the file.
    Offline,
    /// Not downloaded and not downloading.
    Missing,
    /// Queued or running; the fraction is known once yt-dlp has reported it.
    Downloading(Option<f32>),
    /// The last attempt failed (the message is for the log and the screen).
    Failed(String),
}

#[derive(Clone)]
pub struct MusicCache {
    db: Database,
    dir: PathBuf,
    tmp: PathBuf,
    limit: Arc<AtomicUsize>,
    protected: Arc<Mutex<Option<String>>>,
    permits: Arc<Semaphore>,
    /// Songs being downloaded (or waiting for a slot), with a handle to cancel them.
    in_flight: Arc<Mutex<HashMap<String, AbortHandle>>>,
    /// Download progress (0.0 to 1.0) of running downloads.
    progress: Arc<Mutex<HashMap<String, f32>>>,
    failed: Arc<Mutex<HashMap<String, String>>>,
}

/// Which songs to delete. `ids_oldest_first` is the download order; the protected id (currently
/// playing) is never chosen.
pub fn pick_evictions(ids_oldest_first: &[String], limit: usize, protected: Option<&str>) -> Vec<String> {
    let excess = ids_oldest_first.len().saturating_sub(limit);
    ids_oldest_first.iter().filter(|id| Some(id.as_str()) != protected).take(excess).cloned().collect()
}

/// Clears a download's bookkeeping when it finishes, fails or is cancelled.
struct InFlightGuard {
    in_flight: Arc<Mutex<HashMap<String, AbortHandle>>>,
    progress: Arc<Mutex<HashMap<String, f32>>>,
    id: String,
}

impl Drop for InFlightGuard {
    fn drop(&mut self) {
        lock(&self.in_flight).remove(&self.id);
        lock(&self.progress).remove(&self.id);
    }
}

fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(|e| e.into_inner())
}

fn private_dir(path: &Path) -> std::io::Result<()> {
    std::fs::DirBuilder::new().recursive(true).mode(0o700).create(path)
}

impl MusicCache {
    pub fn new(db: Database, data_dir: &Path, limit: usize) -> Self {
        let root = data_dir.join("music");
        Self {
            db,
            dir: root.join("cache"),
            tmp: root.join("tmp"),
            limit: Arc::new(AtomicUsize::new(limit)),
            protected: Arc::default(),
            permits: Arc::new(Semaphore::new(MAX_CONCURRENT_DOWNLOADS)),
            in_flight: Arc::default(),
            progress: Arc::default(),
            failed: Arc::default(),
        }
    }

    pub fn limit(&self) -> usize {
        self.limit.load(Ordering::Relaxed)
    }

    pub fn set_limit(&self, n: usize) {
        self.limit.store(n, Ordering::Relaxed);
    }

    /// The song that is playing; it won't be deleted by the limit.
    pub fn set_protected(&self, video_id: Option<String>) {
        *lock(&self.protected) = video_id;
    }

    /// The state to show for a song (`track` is its current database row).
    pub fn state(&self, track: &MusicTrack) -> DownloadState {
        if track.local_file().is_some() {
            return DownloadState::Offline;
        }
        if lock(&self.in_flight).contains_key(&track.video_id) {
            return DownloadState::Downloading(lock(&self.progress).get(&track.video_id).copied());
        }
        match lock(&self.failed).get(&track.video_id) {
            Some(message) => DownloadState::Failed(message.clone()),
            None => DownloadState::Missing,
        }
    }

    /// Delete leftovers of an unfinished download of this song.
    fn discard_partial(&self, video_id: &str) {
        let Ok(entries) = std::fs::read_dir(&self.tmp) else { return };
        for entry in entries.flatten() {
            if entry.file_name().to_string_lossy().starts_with(video_id) {
                let _ = std::fs::remove_file(entry.path());
            }
        }
    }

    /// Startup cleanup: forget missing files, delete unknown files and temp leftovers.
    pub async fn reconcile(&self) -> Result<()> {
        private_dir(&self.dir).context("could not create the music cache folder")?;
        let _ = std::fs::remove_dir_all(&self.tmp);
        private_dir(&self.tmp).context("could not create the music temp folder")?;

        let mut known = HashSet::new();
        for t in self.db.music_downloaded_oldest_first().await? {
            match t.file_path.as_deref().map(Path::new) {
                Some(p) if p.is_file() => {
                    known.insert(p.to_path_buf());
                }
                _ => self.db.music_clear_download(&t.video_id).await?,
            }
        }
        for entry in std::fs::read_dir(&self.dir)?.flatten() {
            let p = entry.path();
            if p.is_file() && !known.contains(&p) {
                let _ = std::fs::remove_file(p);
            }
        }
        self.enforce_limit().await;
        Ok(())
    }

    pub async fn enforce_limit(&self) {
        let Ok(downloaded) = self.db.music_downloaded_oldest_first().await else { return };
        let ids: Vec<String> = downloaded.iter().map(|t| t.video_id.clone()).collect();
        let protected = lock(&self.protected).clone();
        for id in pick_evictions(&ids, self.limit(), protected.as_deref()) {
            if let Some(path) = downloaded.iter().find(|t| t.video_id == id).and_then(|t| t.file_path.as_deref()) {
                let _ = std::fs::remove_file(path);
            }
            let _ = self.db.music_clear_download(&id).await;
        }
    }

    /// Deletes every downloaded file (songs stay in their playlists and stream). The playing song's
    /// file is kept.
    pub async fn clear(&self) -> Result<usize> {
        let protected = lock(&self.protected).clone();
        let mut removed = 0;
        for t in self.db.music_downloaded_oldest_first().await? {
            if Some(&t.video_id) == protected.as_ref() {
                continue;
            }
            if let Some(path) = &t.file_path {
                let _ = std::fs::remove_file(path);
            }
            self.db.music_clear_download(&t.video_id).await?;
            removed += 1;
        }
        Ok(removed)
    }

    /// Bytes used and number of files in the cache folder.
    pub fn disk_use(&self) -> (u64, usize) {
        let Ok(entries) = std::fs::read_dir(&self.dir) else { return (0, 0) };
        entries.flatten().filter_map(|e| e.metadata().ok()).filter(|m| m.is_file()).fold((0, 0), |(bytes, n), m| (bytes + m.len(), n + 1))
    }

    /// Starts a background download (does nothing if the song is cached or already downloading).
    pub fn start(&self, video_id: String) {
        if lock(&self.in_flight).contains_key(&video_id) {
            return;
        }
        lock(&self.failed).remove(&video_id);
        let cache = self.clone();
        let id = video_id.clone();
        // Hold the map while spawning, so the task's guard cannot run before its handle is stored.
        let mut in_flight = lock(&self.in_flight);
        let handle = tokio::spawn(async move {
            if let Err(message) = cache.download(id.clone()).await {
                tracing::warn!(video_id=%id, "music download failed: {message}");
                lock(&cache.failed).insert(id, message);
            }
        });
        in_flight.insert(video_id, handle.abort_handle());
    }

    /// Cancels a running download and removes its partial files. The song streams instead.
    pub fn cancel(&self, video_id: &str) {
        if let Some(handle) = lock(&self.in_flight).remove(video_id) {
            handle.abort();
        }
        lock(&self.progress).remove(video_id);
        lock(&self.failed).remove(video_id);
        self.discard_partial(video_id);
    }

    /// Downloads a song into the cache and records it. Waits for one of the two download slots.
    pub async fn download(&self, video_id: String) -> Result<(), String> {
        if let Ok(Some(t)) = self.db.music_track(&video_id).await {
            if t.local_file().is_some() {
                return Ok(());
            }
        }
        let _guard = InFlightGuard { in_flight: self.in_flight.clone(), progress: self.progress.clone(), id: video_id.clone() };
        let _permit = self.permits.acquire().await.map_err(|e| e.to_string())?;

        let progress = self.progress.clone();
        let id = video_id.clone();
        let tmp_file = music_yt::download(&video_id, &self.tmp, move |p| {
            lock(&progress).insert(id.clone(), p);
        })
        .await;
        let tmp_file = match tmp_file {
            Ok(path) => path,
            Err(message) => {
                self.discard_partial(&video_id);
                return Err(message);
            }
        };
        let ext = tmp_file.extension().and_then(|e| e.to_str()).filter(|e| e.bytes().all(|b| b.is_ascii_alphanumeric())).unwrap_or("audio").to_string();
        let dest = self.dir.join(format!("{video_id}.{ext}"));
        // tmp and cache share a filesystem (both under `<data>/music`), so this rename is atomic.
        tokio::fs::rename(&tmp_file, &dest).await.map_err(|e| format!("Couldn't save the download: {e}"))?;
        self.db
            .music_set_downloaded(&video_id, &dest.to_string_lossy(), now_secs())
            .await
            .map_err(|e| format!("Couldn't record the download: {e}"))?;
        self.enforce_limit().await;
        Ok(())
    }
}

pub fn now_secs() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ids(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn evicts_oldest_beyond_limit() {
        assert_eq!(pick_evictions(&ids(&["a", "b", "c", "d"]), 2, None), ids(&["a", "b"]));
        assert!(pick_evictions(&ids(&["a", "b"]), 2, None).is_empty());
    }

    #[test]
    fn never_evicts_protected_song() {
        assert_eq!(pick_evictions(&ids(&["a", "b", "c", "d"]), 2, Some("a")), ids(&["b", "c"]));
    }

    fn track(id: &str) -> MusicTrack {
        MusicTrack {
            video_id: id.into(),
            title: id.into(),
            artist: None,
            duration_secs: None,
            thumbnail_url: None,
            file_path: None,
            downloaded_at: None,
        }
    }

    async fn setup(name: &str) -> (MusicCache, Database, PathBuf) {
        let root = std::env::temp_dir().join(format!("ct-music-cache-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        let db = Database::open(&root.join("db.sqlite")).unwrap();
        let cache = MusicCache::new(db.clone(), &root, 2);
        cache.reconcile().await.unwrap();
        (cache, db, root)
    }

    async fn add_cached(cache: &MusicCache, db: &Database, id: &str, at: i64) -> PathBuf {
        db.music_upsert_tracks(vec![track(id)]).await.unwrap();
        let path = cache.dir.join(format!("{id}.opus"));
        std::fs::write(&path, b"audio").unwrap();
        db.music_set_downloaded(id, &path.to_string_lossy(), at).await.unwrap();
        path
    }

    #[tokio::test]
    async fn enforce_limit_deletes_files_and_keeps_metadata() {
        let (cache, db, root) = setup("limit").await;
        let a = add_cached(&cache, &db, "a", 1).await;
        let b = add_cached(&cache, &db, "b", 2).await;
        let c = add_cached(&cache, &db, "c", 3).await;
        cache.set_protected(Some("a".into()));
        cache.enforce_limit().await;
        assert!(a.exists(), "the playing song is kept");
        assert!(!b.exists());
        assert!(c.exists());
        assert!(db.music_track("b").await.unwrap().is_some(), "metadata stays");
        assert!(db.music_track("b").await.unwrap().unwrap().file_path.is_none());
        let _ = std::fs::remove_dir_all(root);
    }

    #[tokio::test]
    async fn reconcile_cleans_orphans_and_missing() {
        let (cache, db, root) = setup("reconcile").await;
        let a = add_cached(&cache, &db, "a", 1).await;
        let orphan = cache.dir.join("orphan.opus");
        std::fs::write(&orphan, b"x").unwrap();
        std::fs::write(cache.tmp.join("leftover.part"), b"x").unwrap();
        std::fs::remove_file(&a).unwrap(); // the database still lists it
        add_cached(&cache, &db, "b", 2).await;
        cache.reconcile().await.unwrap();
        assert!(!orphan.exists());
        assert!(!cache.tmp.join("leftover.part").exists());
        assert!(db.music_track("a").await.unwrap().unwrap().file_path.is_none());
        assert!(db.music_track("b").await.unwrap().unwrap().file_path.is_some());
        let _ = std::fs::remove_dir_all(root);
    }

    #[tokio::test]
    async fn state_reports_offline_missing_and_failed() {
        let (cache, db, root) = setup("state").await;
        add_cached(&cache, &db, "a", 1).await;
        let a = db.music_track("a").await.unwrap().unwrap();
        assert_eq!(cache.state(&a), DownloadState::Offline);
        assert_eq!(cache.state(&track("z")), DownloadState::Missing);
        lock(&cache.failed).insert("z".into(), "No internet connection.".into());
        assert_eq!(cache.state(&track("z")), DownloadState::Failed("No internet connection.".into()));
        let _ = std::fs::remove_dir_all(root);
    }

    #[tokio::test]
    async fn clear_removes_all_but_the_playing_song() {
        let (cache, db, root) = setup("clear").await;
        add_cached(&cache, &db, "a", 1).await;
        let b = add_cached(&cache, &db, "b", 2).await;
        cache.set_protected(Some("b".into()));
        assert_eq!(cache.clear().await.unwrap(), 1);
        assert!(b.exists());
        assert_eq!(db.music_downloaded_oldest_first().await.unwrap().len(), 1);
        let _ = std::fs::remove_dir_all(root);
    }

    #[tokio::test]
    async fn cancelling_a_download_cleans_up() {
        let (cache, _db, root) = setup("cancel").await;
        std::fs::write(cache.tmp.join("abcdefghijk.webm.part"), b"x").unwrap();
        cache.cancel("abcdefghijk");
        assert!(!cache.tmp.join("abcdefghijk.webm.part").exists());
        assert_eq!(cache.state(&track("abcdefghijk")), DownloadState::Missing);
        let _ = std::fs::remove_dir_all(root);
    }
}
