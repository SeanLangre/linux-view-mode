//! Owns the player for one video: Firefox showing YouTube's embedded player (reporting back
//! over HTTP) or, as the fallback and by choice, mpv observed over a persistent IPC connection.
use anyhow::{Context, Result, bail};
use serde::Serialize;
use serde_json::{Value, json};
use std::{
    collections::VecDeque,
    os::unix::fs::DirBuilderExt,
    path::{Path, PathBuf},
    process::Stdio,
    sync::{Arc, Mutex as StdMutex},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
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
    firefox: PathBuf,
    /// Port of our own server when videos play in Firefox; `None` plays everything in mpv.
    web_port: Option<u16>,
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
    /// Playing in Firefox rather than mpv.
    web: bool,
    started: Instant,
    launch: Launch,
    /// A throwaway Firefox profile (Guest has none); removed when the player ends.
    temp_profile: Option<PathBuf>,
}
/// What a video was started with, kept so mpv can take over if Firefox cannot play it.
#[derive(Clone)]
struct Launch {
    start: Option<f64>,
    firefox: Option<PathBuf>,
    subtitles: Option<String>,
}
/// How the Firefox page says playback ended.
#[derive(Clone, Debug, PartialEq)]
enum Outcome {
    Eof,
    Next,
    Quit,
    Error(i64),
}
/// A progress report from the player page.
#[derive(Debug, Default, serde::Deserialize)]
pub struct WebReport {
    pub session: u64,
    pub position: Option<f64>,
    pub duration: Option<f64>,
    pub paused: Option<bool>,
    pub title: Option<String>,
    pub outcome: Option<String>,
    pub code: Option<i64>,
}
/// The Firefox page gives up on a video that has not started playing after this long.
const WEB_START_TIMEOUT: Duration = Duration::from_secs(30);
/// Autoplay without a click, and no fullscreen prompts, for the Firefox that plays videos.
const PLAYER_PREFS: &str = "user_pref(\"media.autoplay.default\", 0);
user_pref(\"media.autoplay.blocking_policy\", 0);
user_pref(\"full-screen-api.warning.timeout\", 0);
user_pref(\"full-screen-api.approval-required\", false);
";
#[derive(Clone, Default)]
struct Details {
    title: Option<String>,
    position: Option<f64>,
    duration: Option<f64>,
    paused: bool,
    eof: bool,
    /// Firefox only: how the page ended playback, and commands waiting for its next report.
    outcome: Option<Outcome>,
    commands: Vec<Value>,
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
        if let Some(profile) = &self.temp_profile {
            let _ = std::fs::remove_dir_all(profile);
        }
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
            firefox: "firefox".into(),
            web_port: None,
            running: None,
            error: None,
            sequence: 0,
            ended: VecDeque::new(),
        }))))
    }
    /// Plays videos in Firefox (through our server on `port`) instead of mpv. mpv still takes over
    /// for a video that YouTube will not play in an embedded player.
    pub async fn use_firefox(&self, port: u16) {
        self.0.lock().await.web_port = Some(port);
    }
    pub async fn play(
        &self,
        id: &str,
        start: Option<f64>,
        firefox: Option<&Path>,
        subtitles: Option<&str>,
    ) -> Result<u64> {
        if !crate::ytdlp::is_video_id(id) || start.is_some_and(|v| !v.is_finite() || v < 0.0) {
            bail!("invalid video id or start position");
        }
        if subtitles.is_some_and(|language| !crate::profiles::valid_language(language)) {
            bail!("invalid subtitle language");
        }
        let mut inner = self.0.lock().await;
        inner.stop().await?;
        inner.error = None;
        let launch = Launch {
            start,
            firefox: firefox.map(Path::to_path_buf),
            subtitles: subtitles.map(str::to_owned),
        };
        let sequence = inner.sequence + 1;
        let running = match inner.web_port {
            Some(port) => match inner.launch_web(id, sequence, &launch, port).await {
                Ok(running) => running,
                Err(err) => {
                    tracing::warn!("could not open Firefox, using mpv: {err:#}");
                    inner.launch_mpv(id, sequence, &launch)?
                }
            },
            None => inner.launch_mpv(id, sequence, &launch)?,
        };
        inner.sequence = sequence;
        inner.running = Some(running);
        Ok(sequence)
    }
    /// A report from the Firefox player page; returns the commands waiting for it.
    pub async fn web_report(&self, report: WebReport) -> Vec<Value> {
        let inner = self.0.lock().await;
        let Some(running) = inner.running.as_ref().filter(|r| r.web && r.sequence == report.session) else {
            return vec![];
        };
        let mut details = running.details.lock().unwrap_or_else(|p| p.into_inner());
        if let Some(value) = report.position.filter(|v| v.is_finite() && *v >= 0.0) {
            details.position = Some(value);
        }
        if let Some(value) = report.duration.filter(|v| v.is_finite() && *v > 0.0) {
            details.duration = Some(value);
        }
        if let Some(paused) = report.paused {
            details.paused = paused;
        }
        if let Some(title) = report.title.filter(|t| !t.is_empty()) {
            details.title = Some(title);
        }
        match report.outcome.as_deref() {
            Some("eof") => { details.eof = true; details.outcome = Some(Outcome::Eof) }
            Some("next") => details.outcome = Some(Outcome::Next),
            Some("quit") => details.outcome = Some(Outcome::Quit),
            Some("error") => details.outcome = Some(Outcome::Error(report.code.unwrap_or(0))),
            _ => {}
        }
        std::mem::take(&mut details.commands)
    }
    pub async fn stop(&self) -> Result<()> {
        self.0.lock().await.stop().await
    }
    /// Send one command to the running mpv over a second IPC connection (mpv accepts several).
    pub async fn command(&self, command: &[Value]) -> Result<()> {
        let socket = {
            let inner = self.0.lock().await;
            let running = inner.running.as_ref().context("nothing is playing")?;
            if running.web {
                // The Firefox page picks these up with its next report.
                let mut details = running.details.lock().unwrap_or_else(|p| p.into_inner());
                if details.commands.len() < 50 {
                    details.commands.push(Value::Array(command.to_vec()));
                }
                return Ok(());
            }
            running.directory.join("ipc")
        };
        let line = format!("{}\n", json!({ "command": command }));
        let send = async {
            let mut stream = UnixStream::connect(&socket).await?;
            stream.write_all(line.as_bytes()).await?;
            stream.flush().await
        };
        tokio::time::timeout(Duration::from_secs(2), send)
            .await
            .context("mpv did not answer")?
            .context("could not send the command to mpv")
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
    fn runtime_directory(&self) -> Result<PathBuf> {
        let base = std::env::var_os("XDG_RUNTIME_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(std::env::temp_dir);
        let nonce = SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos();
        let directory = base.join(format!("couchtube-{}-{nonce:x}", std::process::id()));
        std::fs::DirBuilder::new().mode(0o700).create(&directory)?;
        Ok(directory)
    }

    fn launch_mpv(&self, id: &str, sequence: u64, launch: &Launch) -> Result<Running> {
        let directory = self.runtime_directory()?;
        let socket = directory.join("ipc");
        let mut command = Command::new(&self.executable);
        command
            .arg(format!("--config-dir={}", self.config.display()))
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
        if let Some(folder) = &launch.firefox {
            command.arg(format!(
                "--ytdl-raw-options-append=cookies-from-browser=firefox:{}",
                folder.display()
            ));
        }
        match launch.subtitles.as_deref() {
            Some(language) => {
                // yt-dlp fetches the track (manual, else automatic); mpv shows it.
                command
                    .arg("--ytdl-raw-options-append=write-subs=")
                    .arg("--ytdl-raw-options-append=write-auto-subs=")
                    .arg(format!("--ytdl-raw-options-append=sub-langs={language}.*,{language}"))
                    .arg(format!("--slang={language}"))
                    .arg("--sid=auto");
            }
            None => {
                command.arg("--sid=no");
            }
        }
        if let Some(start) = launch.start {
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
        let details = Arc::new(StdMutex::new(Details::default()));
        let observer = tokio::spawn(observe(socket, details.clone()));
        Ok(Running {
            child,
            directory,
            id: id.into(),
            sequence,
            details,
            observer,
            web: false,
            started: Instant::now(),
            launch: launch.clone(),
            temp_profile: None,
        })
    }

    /// Opens a kiosk Firefox on our player page, signed in as the profile when it has an account.
    async fn launch_web(&self, id: &str, sequence: u64, launch: &Launch, port: u16) -> Result<Running> {
        let directory = self.runtime_directory()?;
        let (profile, temp_profile) = match &launch.firefox {
            Some(folder) => (folder.clone(), None),
            None => {
                // Guest has no profile directory of its own, so use a throwaway one.
                let folder = directory.join("firefox");
                crate::profiles::private_directory(&folder)?;
                (folder.clone(), Some(folder))
            }
        };
        let prepared = std::fs::write(
            profile.join("user.js"),
            format!("{}\n{PLAYER_PREFS}", crate::profiles::USER_JS),
        );
        let mut url = format!("http://127.0.0.1:{port}/watch.html?v={id}&s={sequence}");
        if let Some(start) = launch.start {
            url.push_str(&format!("&start={start}"));
        }
        if let Some(language) = &launch.subtitles {
            url.push_str(&format!("&sub={language}"));
        }
        let spawned = match prepared {
            Ok(()) => Command::new(&self.firefox)
                .args(["--no-remote", "--new-instance", "--kiosk", "--profile"])
                .arg(&profile)
                .arg(&url)
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .kill_on_drop(true)
                .spawn()
                .context("could not start Firefox"),
            Err(err) => Err(err).context("could not prepare the Firefox profile"),
        };
        let child = match spawned {
            Ok(child) => child,
            Err(err) => {
                let _ = std::fs::remove_dir_all(&directory);
                return Err(err);
            }
        };
        Ok(Running {
            child,
            directory,
            id: id.into(),
            sequence,
            details: Arc::new(StdMutex::new(Details::default())),
            observer: tokio::spawn(async {}),
            web: true,
            started: Instant::now(),
            launch: launch.clone(),
            temp_profile,
        })
    }

    async fn reap(&mut self) -> Result<()> {
        let mut web_outcome = None;
        let exit = match self.running.as_mut() {
            Some(running) if running.web => {
                let (outcome, position) = {
                    let details = running.details.lock().unwrap_or_else(|p| p.into_inner());
                    (details.outcome.clone(), details.position)
                };
                let exited = running.child.try_wait()?.is_some();
                web_outcome = match outcome {
                    Some(outcome) => Some(outcome),
                    // The window was closed (Alt+F4) after playing, or Firefox died without ever playing.
                    None if exited && position.is_some() => Some(Outcome::Quit),
                    None if exited => Some(Outcome::Error(-2)),
                    None if position.is_none() && running.started.elapsed() > WEB_START_TIMEOUT => {
                        Some(Outcome::Error(-3))
                    }
                    None => None,
                };
                None
            }
            Some(running) => running.child.try_wait()?,
            None => None,
        };
        if let Some(outcome) = web_outcome {
            let mut running = self.running.take().expect("running player");
            close_firefox(&mut running.child).await;
            running.finish().await;
            if let Outcome::Error(code) = outcome {
                tracing::warn!(video_id=%running.id, code, "Firefox could not play this video; using mpv");
                let mut launch = running.launch.clone();
                let position = running.status().position.filter(|p| *p >= 5.0);
                launch.start = position.or(launch.start);
                match self.launch_mpv(&running.id, running.sequence, &launch) {
                    Ok(fallback) => {
                        self.running = Some(fallback);
                        return Ok(());
                    }
                    Err(err) => tracing::warn!("mpv fallback failed: {err:#}"),
                }
                self.error = Some("Playback failed. Check the server log and try again.".into());
                self.ended.push_back(Ended { status: running.status(), reason: EndReason::Failed });
                return Ok(());
            }
            let reason = match outcome {
                Outcome::Eof => EndReason::Finished,
                Outcome::Next => EndReason::Next,
                _ => EndReason::Stopped,
            };
            tracing::debug!(video_id=%running.id, ?reason, "Firefox session ended");
            self.ended.push_back(Ended { status: running.status(), reason });
            return Ok(());
        }
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
                if running.web {
                    close_firefox(&mut running.child).await;
                } else {
                    running.child.kill().await.context("could not stop mpv")?;
                }
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

/// Asks Firefox to quit so it saves its profile (cookies), and kills it if it does not.
async fn close_firefox(child: &mut Child) {
    if child.try_wait().ok().flatten().is_some() {
        return;
    }
    if let Some(pid) = child.id() {
        let _ = Command::new("kill").args(["-TERM", &pid.to_string()]).status().await;
    }
    if tokio::time::timeout(Duration::from_secs(5), child.wait()).await.is_err() {
        let _ = child.kill().await;
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
        player.play("aqz-KE-bpKQ", Some(12.0), None, None).await.unwrap();
        let (first_pid, first_socket_dir) = {
            let inner = player.0.lock().await;
            let running = inner.running.as_ref().unwrap();
            (running.child.id().unwrap(), running.directory.clone())
        };
        assert!(player.status().await.unwrap().playing);
        player.play("rFZHOHl-L8A", None, None, None).await.unwrap();
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
        player.play("aqz-KE-bpKQ", None, None, None).await.unwrap();
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

    fn fake(directory: &Path, name: &str, script: &str) -> PathBuf {
        use std::os::unix::fs::PermissionsExt;
        let executable = directory.join(name);
        std::fs::write(&executable, script).unwrap();
        std::fs::set_permissions(&executable, std::fs::Permissions::from_mode(0o700)).unwrap();
        executable
    }

    #[tokio::test]
    async fn firefox_reports_drive_status_commands_and_the_end() {
        let directory = std::env::temp_dir().join(format!("ct-web-test-{}", std::process::id()));
        std::fs::create_dir(&directory).unwrap();
        let player = Player::new("config/mpv".into()).unwrap();
        {
            let mut inner = player.0.lock().await;
            inner.firefox = fake(&directory, "fake-firefox", "#!/bin/sh\nexec sleep 60\n");
        }
        player.use_firefox(7878).await;
        let session = player.play("aqz-KE-bpKQ", None, None, None).await.unwrap();
        assert!(player.status().await.unwrap().playing);
        // A report for another session is ignored.
        assert!(player.web_report(WebReport { session: session + 1, position: Some(9.0), ..Default::default() }).await.is_empty());
        player.command(&[json!("cycle"), json!("pause")]).await.unwrap();
        let report = WebReport {
            session, position: Some(12.5), duration: Some(120.0), paused: Some(true), title: Some("Video".into()), ..Default::default()
        };
        assert_eq!(player.web_report(report).await, vec![json!(["cycle", "pause"])]);
        assert!(player.web_report(WebReport { session, ..Default::default() }).await.is_empty());
        let status = player.status().await.unwrap();
        assert_eq!((status.position, status.duration, status.paused), (Some(12.5), Some(120.0), true));
        assert_eq!(status.title.as_deref(), Some("Video"));
        player.web_report(WebReport { session, position: Some(119.0), outcome: Some("eof".into()), ..Default::default() }).await;
        assert!(!player.status().await.unwrap().playing);
        let ended = player.take_ended().await.unwrap();
        assert_eq!(ended.len(), 1);
        assert_eq!(ended[0].reason, EndReason::Finished);
        assert_eq!(ended[0].status.position, Some(119.0));
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[tokio::test]
    async fn an_embed_error_hands_the_video_to_mpv_with_the_same_session() {
        let directory = std::env::temp_dir().join(format!("ct-fallback-test-{}", std::process::id()));
        std::fs::create_dir(&directory).unwrap();
        let player = Player::new("config/mpv".into()).unwrap();
        {
            let mut inner = player.0.lock().await;
            inner.firefox = fake(&directory, "fake-firefox", "#!/bin/sh\nexec sleep 60\n");
            inner.executable = fake(&directory, "fake-mpv", "#!/bin/sh\nexec sleep 60\n");
        }
        player.use_firefox(7878).await;
        let session = player.play("aqz-KE-bpKQ", Some(30.0), None, None).await.unwrap();
        player.web_report(WebReport { session, outcome: Some("error".into()), code: Some(150), ..Default::default() }).await;
        let status = player.status().await.unwrap();
        assert!(status.playing);
        assert_eq!(status.session, Some(session));
        assert!(!player.0.lock().await.running.as_ref().unwrap().web);
        assert!(player.take_ended().await.unwrap().is_empty());
        // A missing Firefox also falls back to mpv.
        player.0.lock().await.firefox = directory.join("missing");
        player.play("rFZHOHl-L8A", None, None, None).await.unwrap();
        assert!(!player.0.lock().await.running.as_ref().unwrap().web);
        player.stop().await.unwrap();
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[tokio::test]
    async fn rejects_invalid_playback_before_spawning() {
        let player = Player::new("config/mpv".into()).unwrap();
        assert!(player.play("--bad", None, None, None).await.is_err());
        assert!(player.play("aqz-KE-bpKQ", Some(-1.0), None, None).await.is_err());
        assert!(
            player
                .play("aqz-KE-bpKQ", Some(f64::INFINITY), None, None)
                .await
                .is_err()
        );
        assert!(player.play("aqz-KE-bpKQ", None, None, Some("en;rm")).await.is_err());
        assert!(!player.status().await.unwrap().playing);
        player.stop().await.unwrap();
    }
}
