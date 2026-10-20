//! Owns one mpv process and observes playback over a persistent IPC connection.
use anyhow::{Context, Result, bail};
use serde::Serialize;
use serde_json::{Value, json};
use std::{
    collections::VecDeque,
    os::unix::fs::DirBuilderExt,
    path::{Path, PathBuf},
    process::Stdio,
    sync::{Arc, Mutex as StdMutex},
    time::{Duration, SystemTime, UNIX_EPOCH},
};
use tokio::{
    io::{AsyncBufReadExt, AsyncWriteExt, BufReader},
    net::UnixStream,
    process::{Child, Command},
    sync::Mutex,
    task::JoinHandle,
};

#[derive(Clone)]
pub struct Player(Arc<Mutex<Inner>>);
struct Inner {
    config: PathBuf,
    executable: PathBuf,
    running: Option<Running>,
    error: Option<String>,
    sequence: u64,
    ended: VecDeque<Ended>,
}
struct Running {
    child: Child,
    directory: PathBuf,
    id: String,
    sequence: u64,
    details: Arc<StdMutex<Details>>,
    observer: JoinHandle<()>,
}
#[derive(Clone, Default)]
struct Details {
    title: Option<String>,
    position: Option<f64>,
    duration: Option<f64>,
    paused: bool,
    eof: bool,
}
impl Running {
    fn status(&self) -> Status {
        let details = self
            .details
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .clone();
        Status {
            playing: true,
            id: Some(self.id.clone()),
            session: Some(self.sequence),
            title: details.title,
            position: details.position,
            duration: details.duration,
            paused: details.paused,
            error: None,
        }
    }
    async fn finish(&mut self) {
        // Drain final end-file/property events before disposing of the socket.
        if tokio::time::timeout(Duration::from_millis(200), &mut self.observer)
            .await
            .is_err()
        {
            self.observer.abort();
        }
    }
}
impl Drop for Running {
    fn drop(&mut self) {
        self.observer.abort();
        let _ = std::fs::remove_file(self.directory.join("ipc"));
        let _ = std::fs::remove_dir(&self.directory);
    }
}
#[derive(Debug, Clone, Default, Serialize)]
pub struct Status {
    pub playing: bool,
    pub id: Option<String>,
    pub session: Option<u64>,
    pub title: Option<String>,
    pub position: Option<f64>,
    pub duration: Option<f64>,
    pub paused: bool,
    pub error: Option<String>,
}
#[derive(Debug, PartialEq)]
pub enum EndReason {
    Finished,
    Next,
    Stopped,
    Failed,
}
pub struct Ended {
    pub status: Status,
    pub reason: EndReason,
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
            sequence: 0,
            ended: VecDeque::new(),
        }))))
    }
    pub async fn play(&self, id: &str, start: Option<f64>, firefox: Option<&Path>) -> Result<u64> {
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
        command.arg("--ytdl-raw-options=ignore-config=");
        if let Some(folder) = firefox {
            command.arg(format!(
                "--ytdl-raw-options-append=cookies-from-browser=firefox:{}",
                folder.display()
            ));
        }
        if let Some(start) = start {
            command.arg(format!("--start={start}"));
        }
        let child = match command
            .arg("--")
            .arg(format!("https://www.youtube.com/watch?v={id}"))
            .spawn()
        {
            Ok(child) => child,
            Err(err) => {
                let _ = std::fs::remove_dir(directory);
                return Err(err).context("could not start mpv (is it installed?)");
            }
        };
        inner.sequence += 1;
        let sequence = inner.sequence;
        let details = Arc::new(StdMutex::new(Details::default()));
        let observer = tokio::spawn(observe(socket, details.clone()));
        inner.running = Some(Running {
            child,
            directory,
            id: id.into(),
            sequence,
            details,
            observer,
        });
        Ok(sequence)
    }
    pub async fn stop(&self) -> Result<()> {
        self.0.lock().await.stop().await
    }
    pub async fn status(&self) -> Result<Status> {
        let mut inner = self.0.lock().await;
        inner.reap().await?;
        Ok(inner
            .running
            .as_ref()
            .map(Running::status)
            .unwrap_or_else(|| Status {
                error: inner.error.clone(),
                ..Status::default()
            }))
    }
    pub async fn take_ended(&self) -> Result<Vec<Ended>> {
        let mut inner = self.0.lock().await;
        inner.reap().await?;
        Ok(inner.ended.drain(..).collect())
    }
}
impl Inner {
    async fn reap(&mut self) -> Result<()> {
        let exit = match self.running.as_mut() {
            Some(running) => running.child.try_wait()?,
            None => None,
        };
        if let Some(exit) = exit {
            let mut running = self.running.take().expect("running process");
            running.finish().await;
            let eof = running
                .details
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .eof;
            let reason = if exit.code() == Some(5) {
                EndReason::Next
            } else if !exit.success() {
                EndReason::Failed
            } else if eof {
                EndReason::Finished
            } else {
                EndReason::Stopped
            };
            if reason == EndReason::Failed {
                tracing::warn!("mpv exited with {exit}");
                self.error = Some("Playback failed. Check the server log and try again.".into());
            }
            tracing::debug!(video_id=%running.id, ?reason, "mpv session ended");
            self.ended.push_back(Ended {
                status: running.status(),
                reason,
            });
        }
        Ok(())
    }
    async fn stop(&mut self) -> Result<()> {
        self.reap().await?;
        if let Some(mut running) = self.running.take() {
            if running.child.try_wait()?.is_none() {
                running.child.kill().await.context("could not stop mpv")?;
            }
            running.finish().await;
            self.ended.push_back(Ended {
                status: running.status(),
                reason: EndReason::Stopped,
            });
        }
        self.error = None;
        Ok(())
    }
}

async fn observe(socket: PathBuf, details: Arc<StdMutex<Details>>) {
    // mpv starts the IPC server asynchronously, before or while resolving yt-dlp.
    let stream = loop {
        match UnixStream::connect(&socket).await {
            Ok(stream) => break stream,
            Err(_) => tokio::time::sleep(Duration::from_millis(50)).await,
        }
    };
    let mut stream = BufReader::new(stream);
    for (id, name) in ["media-title", "time-pos", "duration", "pause"]
        .iter()
        .enumerate()
    {
        let line = format!("{}\n", json!({"command":["observe_property",id,name]}));
        if stream.get_mut().write_all(line.as_bytes()).await.is_err() {
            return;
        }
    }
    let mut line = String::new();
    loop {
        line.clear();
        match stream.read_line(&mut line).await {
            Ok(0) | Err(_) => return,
            Ok(_) => {}
        }
        let Ok(event) = serde_json::from_str::<Value>(&line) else {
            continue;
        };
        let mut details = details.lock().unwrap_or_else(|p| p.into_inner());
        if event["event"] == "end-file" && event["reason"] == "eof" {
            details.eof = true;
        }
        if event["event"] == "property-change" {
            match event["name"].as_str() {
                Some("media-title") => details.title = event["data"].as_str().map(str::to_owned),
                Some("time-pos") => {
                    if let Some(value) = event["data"]
                        .as_f64()
                        .filter(|v| v.is_finite() && *v >= 0.0)
                    {
                        details.position = Some(value);
                    }
                }
                Some("duration") => {
                    if let Some(value) =
                        event["data"].as_f64().filter(|v| v.is_finite() && *v > 0.0)
                    {
                        details.duration = Some(value);
                    }
                }
                Some("pause") => details.paused = event["data"].as_bool().unwrap_or(false),
                _ => {}
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn ipc_observer_retains_final_properties_and_records_eof() {
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
                let request: Value = serde_json::from_str(&line).unwrap();
                assert_eq!(request["command"][0], "observe_property");
            }
            for event in [
                json!({"event":"property-change","name":"pause","data":true}),
                json!({"event":"property-change","name":"time-pos","data":12.5}),
                json!({"event":"property-change","name":"duration","data":120}),
                json!({"event":"property-change","name":"media-title","data":"Video"}),
                json!({"event":"end-file","reason":"eof"}),
                json!({"event":"property-change","name":"time-pos","data":null}),
                json!({"event":"property-change","name":"duration","data":null}),
            ] {
                stream
                    .get_mut()
                    .write_all(format!("{event}\n").as_bytes())
                    .await
                    .unwrap();
            }
        });
        let details = Arc::new(StdMutex::new(Details::default()));
        tokio::time::timeout(
            Duration::from_secs(3),
            observe(socket.clone(), details.clone()),
        )
        .await
        .unwrap();
        server.await.unwrap();
        let details = details.lock().unwrap().clone();
        assert_eq!(details.position, Some(12.5));
        assert_eq!(details.duration, Some(120.0));
        assert_eq!(details.title.as_deref(), Some("Video"));
        assert!(details.paused && details.eof);
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
        player.play("aqz-KE-bpKQ", Some(12.0), None).await.unwrap();
        let (first_pid, first_socket_dir) = {
            let inner = player.0.lock().await;
            let running = inner.running.as_ref().unwrap();
            (running.child.id().unwrap(), running.directory.clone())
        };
        assert!(player.status().await.unwrap().playing);
        player.play("rFZHOHl-L8A", None, None).await.unwrap();
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
        player.play("aqz-KE-bpKQ", None, None).await.unwrap();
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
        assert!(player.play("--bad", None, None).await.is_err());
        assert!(player.play("aqz-KE-bpKQ", Some(-1.0), None).await.is_err());
        assert!(
            player
                .play("aqz-KE-bpKQ", Some(f64::INFINITY), None)
                .await
                .is_err()
        );
        assert!(!player.status().await.unwrap().playing);
        player.stop().await.unwrap();
    }
}
