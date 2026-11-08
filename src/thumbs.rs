//! YouTube thumbnails saved on disk, so a list never waits for pictures and they survive a restart.
//!
//! The pages ask for `/api/thumb/<video id>`. The first request (or a prefetch, when a list was just
//! loaded) downloads the picture from YouTube's image host through `curl`; later ones come from
//! `<data>/thumbs/`. Only `i.ytimg.com` is ever contacted, the id is checked before it becomes a
//! file name, and what is saved must really be an image. The folder is trimmed to the newest files.
use crate::{db::Database, profiles::private_directory, ytdlp::{self, Video}};
use anyhow::{Context, Result, bail};
use std::{
    collections::HashMap,
    path::PathBuf,
    process::Stdio,
    sync::{Arc, Mutex as StdMutex},
    time::Duration,
};
use tokio::{io::AsyncWriteExt, process::Command, sync::{Mutex, Semaphore}};

const MAX_BYTES: usize = 2_000_000;
/// The folder is trimmed to `KEEP_FILES` newest pictures once it holds more than `MAX_FILES`.
const MAX_FILES: usize = 2000;
const KEEP_FILES: usize = 1500;
const DOWNLOADS_AT_ONCE: usize = 4;

struct Inner {
    dir: PathBuf,
    db: Database,
    /// One download per picture at a time.
    locks: StdMutex<HashMap<String, Arc<Mutex<()>>>>,
    limit: Semaphore,
}

#[derive(Clone)]
pub struct Thumbs(Arc<Inner>);

impl Thumbs {
    pub fn new(dir: PathBuf, db: Database) -> Result<Self> {
        private_directory(&dir)?;
        Ok(Self(Arc::new(Inner { dir, db, locks: StdMutex::new(HashMap::new()), limit: Semaphore::new(DOWNLOADS_AT_ONCE) })))
    }

    fn path(&self, id: &str) -> PathBuf {
        // `id` is a checked video id: letters, digits, `-` and `_`.
        self.0.dir.join(id)
    }

    /// The picture's bytes and its content type, downloading it first if it is not on disk yet.
    pub async fn get(&self, id: &str) -> Result<(Vec<u8>, &'static str)> {
        if !ytdlp::is_video_id(id) {
            bail!("invalid video id");
        }
        if let Some(found) = self.read(id).await {
            return Ok(found);
        }
        let url = self.0.db.thumb_url(id).await?.filter(|u| valid_url(u)).unwrap_or_else(|| default_url(id));
        self.download(id, &url).await?;
        self.read(id).await.context("the thumbnail was not saved")
    }

    async fn read(&self, id: &str) -> Option<(Vec<u8>, &'static str)> {
        let bytes = tokio::fs::read(self.path(id)).await.ok()?;
        let kind = sniff(&bytes)?;
        Some((bytes, kind))
    }

    /// Starts downloading the pictures of a list that was just loaded, a few at a time.
    pub fn prefetch(&self, videos: &[Video]) {
        for video in videos {
            if !ytdlp::is_video_id(&video.id) || !valid_url(&video.thumb) || self.path(&video.id).exists() {
                continue;
            }
            let (me, id, url) = (self.clone(), video.id.clone(), video.thumb.clone());
            tokio::spawn(async move {
                if let Err(err) = me.download(&id, &url).await {
                    tracing::debug!(video_id=%id, "thumbnail prefetch failed: {err:#}");
                }
            });
        }
    }

    async fn download(&self, id: &str, url: &str) -> Result<()> {
        let lock = self.0.locks.lock().unwrap().entry(id.to_owned()).or_default().clone();
        let _one = lock.lock().await;
        if self.path(id).exists() {
            return Ok(());
        }
        let _slot = self.0.limit.acquire().await?;
        let bytes = fetch(url).await?;
        let temporary = self.0.dir.join(format!("{id}.tmp"));
        tokio::fs::write(&temporary, &bytes).await?;
        tokio::fs::rename(&temporary, self.path(id)).await?;
        self.0.locks.lock().unwrap().remove(id);
        let dir = self.0.dir.clone();
        tokio::task::spawn_blocking(move || trim(&dir));
        Ok(())
    }
}

/// Only YouTube's own image host, over https, with characters that are safe in a curl config.
fn valid_url(url: &str) -> bool {
    (url.starts_with("https://i.ytimg.com/vi/") || url.starts_with("https://i.ytimg.com/vi_webp/"))
        && url.len() <= 300
        && url.bytes().all(|b| b.is_ascii_alphanumeric() || b"-._~:/?=&%".contains(&b))
}

fn default_url(id: &str) -> String {
    format!("https://i.ytimg.com/vi/{id}/mqdefault.jpg")
}

/// The content type if these bytes are a JPEG, PNG or WebP picture.
fn sniff(bytes: &[u8]) -> Option<&'static str> {
    if bytes.len() < 12 || bytes.len() > MAX_BYTES {
        return None;
    }
    if bytes.starts_with(&[0xFF, 0xD8, 0xFF]) {
        Some("image/jpeg")
    } else if bytes.starts_with(&[0x89, b'P', b'N', b'G']) {
        Some("image/png")
    } else if &bytes[..4] == b"RIFF" && &bytes[8..12] == b"WEBP" {
        Some("image/webp")
    } else {
        None
    }
}

/// Downloads one picture with curl (config on stdin, no redirects, size-limited) and checks it.
async fn fetch(url: &str) -> Result<Vec<u8>> {
    let config = format!("silent\nshow-error\nfail\nmax-time = 15\nmax-filesize = {MAX_BYTES}\nurl = \"{url}\"\n");
    let mut child = Command::new("curl")
        .args(["-q", "-K", "-"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .kill_on_drop(true)
        .spawn()
        .context("could not start curl")?;
    child.stdin.take().context("no stdin")?.write_all(config.as_bytes()).await?;
    let output = tokio::time::timeout(Duration::from_secs(20), child.wait_with_output())
        .await
        .context("the thumbnail download timed out")??;
    if !output.status.success() {
        bail!("curl could not fetch the thumbnail ({})", output.status);
    }
    sniff(&output.stdout).context("that is not a picture")?;
    Ok(output.stdout)
}

/// Keeps the newest `KEEP_FILES` pictures once there are more than `MAX_FILES`; removes leftovers.
fn trim(dir: &std::path::Path) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    let mut files: Vec<(std::time::SystemTime, PathBuf)> = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        let modified = entry.metadata().and_then(|m| m.modified()).unwrap_or(std::time::UNIX_EPOCH);
        if path.extension().is_some_and(|e| e == "tmp") {
            if modified.elapsed().is_ok_and(|age| age > Duration::from_secs(3600)) {
                let _ = std::fs::remove_file(path);
            }
        } else {
            files.push((modified, path));
        }
    }
    if files.len() > MAX_FILES {
        files.sort();
        for (_, path) in &files[..files.len() - KEEP_FILES] {
            let _ = std::fs::remove_file(path);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_youtubes_image_host_and_real_pictures_are_accepted() {
        assert!(valid_url("https://i.ytimg.com/vi/aqz-KE-bpKQ/hq720.jpg?sqp=-oaym&rs=AOn4"));
        for bad in ["http://i.ytimg.com/vi/x/a.jpg", "https://evil.example/vi/x.jpg", "https://i.ytimg.com.evil.example/vi/x.jpg",
                    "https://i.ytimg.com/other/x.jpg", "https://i.ytimg.com/vi/x\"y.jpg", "https://i.ytimg.com/vi/x y.jpg", ""] {
            assert!(!valid_url(bad), "{bad:?}");
        }
        let jpeg = [&[0xFF, 0xD8, 0xFF, 0xE0][..], &[0; 20]].concat();
        let webp = [&b"RIFF"[..], &[0; 4], b"WEBP", &[0; 8]].concat();
        assert_eq!((sniff(&jpeg), sniff(&webp)), (Some("image/jpeg"), Some("image/webp")));
        assert_eq!(sniff(b"<html>not a picture at all</html>"), None);
        assert_eq!(sniff(&[0xFF, 0xD8]), None, "too short");
    }
}
