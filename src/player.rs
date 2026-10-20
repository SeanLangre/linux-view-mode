//! Owns one mpv process. IPC lives in a private, per-playback directory.
use std::{
    os::unix::fs::DirBuilderExt,
    path::PathBuf,
    process::Stdio,
    sync::Arc,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use anyhow::{Context, Result, bail};
use serde::Serialize;
use serde_json::{Value, json};
use tokio::{
    io::{AsyncBufReadExt, AsyncWriteExt, BufReader},
    net::UnixStream,
    process::{Child, Command},
    sync::Mutex,
};

#[derive(Clone)]
pub struct Player(Arc<Mutex<Inner>>);

struct Inner {
    config: PathBuf,
    executable: PathBuf,
    running: Option<Running>,
    error: Option<String>,
}

struct Running {
    child: Child,
    directory: PathBuf,
    id: String,
}

impl Drop for Running {
    fn drop(&mut self) {
        // Child has kill_on_drop enabled; remove only our own socket directory.
        let _ = std::fs::remove_file(self.directory.join("ipc"));
        let _ = std::fs::remove_dir(&self.directory);
    }
}

#[derive(Debug, Default, Serialize)]
pub struct Status {
    pub playing: bool,
    pub id: Option<String>,
    pub title: Option<String>,
    pub position: Option<f64>,
    pub duration: Option<f64>,
    pub paused: bool,
    pub error: Option<String>,
}

impl Player {
    pub fn new(config: PathBuf) -> Result<Self> {
        let config = config
            .canonicalize()
            .context("could not find mpv config directory")?;
        Ok(Self(Arc::new(Mutex::new(Inner {
            config,
            executable: "mpv".into(),
            running: None,
            error: None,
        }))))
    }

    pub async fn play(&self, id: &str, start: Option<f64>) -> Result<()> {
        if !crate::ytdlp::is_video_id(id) || start.is_some_and(|v| !v.is_finite() || v < 0.0) {
            bail!("invalid video id or start position");
        }
        let mut inner = self.0.lock().await;
        inner.stop().await?;
        inner.error = None;
        let base = std::env::var_os("XDG_RUNTIME_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(std::env::temp_dir);
        let nonce = SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos();
        let directory = base.join(format!("couchtube-{}-{nonce:x}", std::process::id()));
        std::fs::DirBuilder::new().mode(0o700).create(&directory)?;
        let socket = directory.join("ipc");
        let mut command = Command::new(&inner.executable);
        command
            .arg(format!("--config-dir={}", inner.config.display()))
            .arg("--fullscreen")
            .arg("--idle=no")
            .arg("--keep-open=no")
            .arg("--input-terminal=no")
            .arg(format!("--input-ipc-server={}", socket.display()))
            .stdin(Stdio::null())
            .stdout(Stdio::inherit())
            .stderr(Stdio::inherit())
            .kill_on_drop(true);
        if let Some(start) = start {
            command.arg(format!("--start={start}"));
        }
        let child = command
            .arg("--")
            .arg(format!("https://www.youtube.com/watch?v={id}"))
            .spawn();
        match child {
            Ok(child) => {
                inner.running = Some(Running {
                    child,
                    directory,
                    id: id.into(),
                })
            }
            Err(err) => {
                let _ = std::fs::remove_dir(directory);
                return Err(err).context("could not start mpv (is it installed?)");
            }
        }
        Ok(())
    }

    pub async fn stop(&self) -> Result<()> {
        self.0.lock().await.stop().await
    }

    pub async fn status(&self) -> Result<Status> {
        let mut inner = self.0.lock().await;
        inner.reap()?;
        let mut status = Status {
            error: inner.error.clone(),
            ..Status::default()
        };
        if let Some(running) = &inner.running {
            status.playing = true;
            status.id = Some(running.id.clone());
            // The socket/properties may not exist yet while yt-dlp loads the video.
            if let Ok(Ok(properties)) = tokio::time::timeout(
                Duration::from_millis(750),
                properties(&running.directory.join("ipc")),
            )
            .await
            {
                status.title = properties[0].as_str().map(str::to_owned);
                status.position = properties[1].as_f64();
                status.duration = properties[2].as_f64();
                status.paused = properties[3].as_bool().unwrap_or(false);
            }
        }
        Ok(status)
    }

    /// Reap even when no browser is polling (Esc, EOF, or startup failure).
    pub fn monitor(&self) -> tokio::task::JoinHandle<()> {
        let player = self.clone();
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(Duration::from_millis(500));
            loop {
                interval.tick().await;
                if let Err(err) = player.0.lock().await.reap() {
                    tracing::error!("mpv wait failed: {err:#}");
                }
            }
        })
    }
}

impl Inner {
    fn reap(&mut self) -> Result<()> {
        if let Some(running) = &mut self.running
            && let Some(exit) = running.child.try_wait()?
        {
            if !exit.success() {
                tracing::warn!("mpv exited with {exit}");
                self.error = Some("Playback failed. Check the server log and try again.".into());
            }
            self.running = None;
        }
        Ok(())
    }

    async fn stop(&mut self) -> Result<()> {
        if let Some(mut running) = self.running.take()
            && running.child.try_wait()?.is_none()
        {
            running.child.kill().await.context("could not stop mpv")?;
        }
        self.error = None;
        Ok(())
    }
}

async fn properties(socket: &std::path::Path) -> Result<Vec<Value>> {
    let mut stream = BufReader::new(UnixStream::connect(socket).await?);
    let names = ["media-title", "time-pos", "duration", "pause"];
    for (id, name) in names.iter().enumerate() {
        let line = format!(
            "{}\n",
            json!({"command": ["get_property", name], "request_id": id})
        );
        stream.get_mut().write_all(line.as_bytes()).await?;
    }
    let mut values = vec![Value::Null; names.len()];
    let mut seen = [false; 4];
    let mut line = String::new();
    while seen.contains(&false) {
        line.clear();
        if stream.read_line(&mut line).await? == 0 {
            bail!("mpv IPC closed");
        }
        let reply: Value = serde_json::from_str(&line)?;
        // Events can arrive between replies; replies may arrive out of order.
        if let Some(id) = reply["request_id"].as_u64().filter(|id| *id < 4) {
            seen[id as usize] = true;
            if reply["error"] == "success" {
                values[id as usize] = reply["data"].clone();
            }
        }
    }
    Ok(values)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn ipc_handles_events_out_of_order_and_missing_properties() {
        let directory = std::env::temp_dir().join(format!("ct-ipc-test-{}", std::process::id()));
        std::fs::create_dir(&directory).unwrap();
        let socket = directory.join("ipc");
        let listener = tokio::net::UnixListener::bind(&socket).unwrap();
        let server = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            let mut stream = BufReader::new(stream);
            for _ in 0..4 {
                let mut line = String::new();
                stream.read_line(&mut line).await.unwrap();
            }
            stream.get_mut().write_all(b"{\"event\":\"idle\"}\n{\"request_id\":3,\"error\":\"success\",\"data\":true}\n{\"request_id\":1,\"error\":\"success\",\"data\":12.5}\n{\"request_id\":0,\"error\":\"success\",\"data\":\"Video\"}\n{\"request_id\":2,\"error\":\"property unavailable\"}\n").await.unwrap();
        });
        let values = properties(&socket).await.unwrap();
        assert_eq!(
            values,
            vec![json!("Video"), json!(12.5), Value::Null, json!(true)]
        );
        server.await.unwrap();
        std::fs::remove_file(socket).unwrap();
        std::fs::remove_dir(directory).unwrap();
    }

    #[tokio::test]
    async fn lifecycle_replaces_stops_and_reports_failed_processes() {
        use std::os::unix::fs::PermissionsExt;
        let directory =
            std::env::temp_dir().join(format!("ct-process-test-{}", std::process::id()));
        std::fs::create_dir(&directory).unwrap();
        let executable = directory.join("fake-mpv");
        std::fs::write(&executable, "#!/bin/sh\nexec sleep 60\n").unwrap();
        std::fs::set_permissions(&executable, std::fs::Permissions::from_mode(0o700)).unwrap();
        let player = Player::new("config/mpv".into()).unwrap();
        player.0.lock().await.executable = executable.clone();
        player.play("aqz-KE-bpKQ", Some(12.0)).await.unwrap();
        let (first_pid, first_socket_dir) = {
            let inner = player.0.lock().await;
            let running = inner.running.as_ref().unwrap();
            (running.child.id().unwrap(), running.directory.clone())
        };
        assert!(player.status().await.unwrap().playing);
        player.play("rFZHOHl-L8A", None).await.unwrap();
        assert!(!PathBuf::from(format!("/proc/{first_pid}")).exists());
        assert!(!first_socket_dir.exists());
        assert_eq!(
            player.status().await.unwrap().id.as_deref(),
            Some("rFZHOHl-L8A")
        );
        player.stop().await.unwrap();
        player.stop().await.unwrap();
        assert!(!player.status().await.unwrap().playing);
        std::fs::write(&executable, "#!/bin/sh\nexit 1\n").unwrap();
        player.play("aqz-KE-bpKQ", None).await.unwrap();
        tokio::time::timeout(Duration::from_secs(3), async {
            loop {
                let status = player.status().await.unwrap();
                if !status.playing {
                    assert!(status.error.is_some());
                    break;
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .unwrap();
        std::fs::remove_file(executable).unwrap();
        std::fs::remove_dir(directory).unwrap();
    }

    #[tokio::test]
    async fn rejects_invalid_playback_before_spawning() {
        let player = Player::new("config/mpv".into()).unwrap();
        assert!(player.play("--bad", None).await.is_err());
        assert!(player.play("aqz-KE-bpKQ", Some(-1.0)).await.is_err());
        assert!(
            player
                .play("aqz-KE-bpKQ", Some(f64::INFINITY))
                .await
                .is_err()
        );
        assert!(!player.status().await.unwrap().playing);
        player.stop().await.unwrap();
    }
}
