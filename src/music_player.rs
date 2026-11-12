//! The audio-only mpv for Music mode. One long-lived `mpv --idle --no-video` is started on the first
//! play and driven over its IPC socket; the backend owns the queue (`music_queue`), loads each song
//! from its downloaded file and moves on when mpv reports the end of a file. Music never streams:
//! a song that is not downloaded yet *waits* (`waiting_id`) while the cache downloads it, and
//! `resume_waiting` loads it once the file is there. It is separate from the video `Player`:
//! switching service stops it (`stop`).
use std::{
    os::unix::fs::DirBuilderExt,
    path::PathBuf,
    process::Stdio,
    sync::{Arc, Mutex as StdMutex},
    time::{Duration, Instant},
};

use anyhow::{Context, Result, bail};
use serde_json::{Value, json};
use tokio::{
    io::{AsyncBufReadExt, AsyncWriteExt, BufReader},
    net::{
        UnixStream,
        unix::{OwnedReadHalf, OwnedWriteHalf},
    },
    process::{Child, Command},
    sync::Mutex,
    task::JoinHandle,
};

use crate::{
    db::Database,
    music_db::MusicTrack,
    music_queue::{Queue, Repeat},
};

/// An error message stays in the state this long, so it can be read.
const ERROR_SHOWN: Duration = Duration::from_secs(8);
/// After this many songs in a row that fail to play, playback stops.
const MAX_FAILURES: u32 = 3;
pub const VOLUME_STEP: f64 = 5.0;

pub struct MusicPlayer(Arc<Mutex<Inner>>);

impl Clone for MusicPlayer {
    fn clone(&self) -> Self {
        Self(self.0.clone())
    }
}

struct Inner {
    db: Database,
    executable: PathBuf,
    /// `--ao=` for mpv (set `COUCHTUBE_MUSIC_AO=null` to play silently, for tests).
    audio_output: Option<String>,
    run_dir: PathBuf,
    proc: Option<Proc>,
    queue: Queue,
    current: Option<MusicTrack>,
    /// The current song is not downloaded yet: nothing plays until its file arrives.
    waiting: bool,
    error: Option<(String, Instant)>,
    volume: f64,
    muted: bool,
    failures: u32,
    live: Arc<StdMutex<Live>>,
}

struct Proc {
    child: Child,
    writer: OwnedWriteHalf,
    reader: JoinHandle<()>,
}

/// What mpv last reported about the file it is playing.
#[derive(Default)]
struct Live {
    position: Option<f64>,
    duration: Option<f64>,
    paused: bool,
    loaded: bool,
    ended: Option<End>,
}

#[derive(Clone, Debug, PartialEq)]
enum End {
    Eof,
    Failed(String),
}

/// A song that has just started, for the caller's follow-up (history, downloads).
#[derive(Clone, Debug)]
pub struct Loaded {
    pub track: MusicTrack,
    pub next: Option<MusicTrack>,
}

pub struct PlayOptions {
    pub shuffle: bool,
    pub repeat: Repeat,
    pub volume: f64,
}

#[derive(Clone, Debug)]
pub struct Snapshot {
    pub current: Option<MusicTrack>,
    pub next: Option<MusicTrack>,
    pub position: Option<f64>,
    pub duration: Option<f64>,
    pub paused: bool,
    /// mpv has opened the song (before that it is still connecting or buffering).
    pub loaded: bool,
    pub volume: f64,
    pub muted: bool,
    pub shuffle: bool,
    pub repeat: Repeat,
    pub error: Option<String>,
}

pub fn repeat_name(repeat: Repeat) -> &'static str {
    match repeat {
        Repeat::Off => "off",
        Repeat::All => "all",
        Repeat::One => "one",
    }
}

pub fn parse_repeat(name: &str) -> Option<Repeat> {
    match name {
        "off" => Some(Repeat::Off),
        "all" => Some(Repeat::All),
        "one" => Some(Repeat::One),
        _ => None,
    }
}

fn lock<T>(mutex: &StdMutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(|e| e.into_inner())
}

impl MusicPlayer {
    pub fn new(db: Database) -> Result<Self> {
        let base = std::env::var_os("XDG_RUNTIME_DIR").map(PathBuf::from).unwrap_or_else(std::env::temp_dir);
        let audio_output = std::env::var("COUCHTUBE_MUSIC_AO").ok().filter(|v| v.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_'));
        Self::with_executable(db, "mpv".into(), base.join(format!("couchtube-music-{}", std::process::id())), audio_output)
    }

    fn with_executable(db: Database, executable: PathBuf, run_dir: PathBuf, audio_output: Option<String>) -> Result<Self> {
        let _ = std::fs::remove_dir_all(&run_dir);
        std::fs::DirBuilder::new().recursive(true).mode(0o700).create(&run_dir).context("could not create the music run folder")?;
        Ok(Self(Arc::new(Mutex::new(Inner {
            db,
            executable,
            audio_output,
            run_dir,
            proc: None,
            queue: Queue::default(),
            current: None,
            waiting: false,
            error: None,
            volume: 70.0,
            muted: false,
            failures: 0,
            live: Default::default(),
        }))))
    }

    /// Replaces the queue with `tracks` and starts the song at `start`.
    pub async fn play(&self, tracks: Vec<MusicTrack>, start: usize, options: PlayOptions) -> Result<Loaded> {
        if tracks.is_empty() {
            bail!("there is nothing to play");
        }
        let mut inner = self.0.lock().await;
        inner.queue.shuffle = options.shuffle;
        inner.queue.repeat = options.repeat;
        inner.queue.start(tracks, start);
        inner.failures = 0;
        inner.error = None;
        inner.volume = options.volume.clamp(0.0, 100.0);
        let loaded = inner.load_current().await;
        if loaded.is_err() {
            inner.finish().await;
        }
        loaded
    }

    pub async fn set_paused(&self, paused: bool) -> Result<()> {
        let mut inner = self.0.lock().await;
        if inner.current.is_none() {
            return Ok(());
        }
        lock(&inner.live).paused = paused;
        inner.send(json!(["set_property", "pause", paused])).await
    }

    pub async fn toggle_pause(&self) -> Result<()> {
        let paused = lock(&self.0.lock().await.live).paused;
        self.set_paused(!paused).await
    }

    /// Jumps by `delta` seconds (negative goes back).
    pub async fn seek_by(&self, delta: f64) -> Result<()> {
        if !delta.is_finite() {
            bail!("invalid seek");
        }
        let mut inner = self.0.lock().await;
        if inner.current.is_none() {
            return Ok(());
        }
        inner.send(json!(["seek", delta, "relative"])).await
    }

    pub async fn seek_to(&self, seconds: f64) -> Result<()> {
        if !seconds.is_finite() || seconds < 0.0 {
            bail!("invalid seek");
        }
        let mut inner = self.0.lock().await;
        if inner.current.is_none() {
            return Ok(());
        }
        inner.send(json!(["seek", seconds, "absolute"])).await
    }

    /// The next song; at the end of the queue playback stops (`None`).
    pub async fn next(&self) -> Result<Option<Loaded>> {
        let mut inner = self.0.lock().await;
        inner.failures = 0;
        inner.advance(false).await
    }

    /// The previous song, or the start of this one when it has played for more than 3 seconds.
    pub async fn prev(&self) -> Result<Option<Loaded>> {
        let mut inner = self.0.lock().await;
        if inner.current.is_none() {
            return Ok(None);
        }
        inner.failures = 0;
        if lock(&inner.live).position.is_some_and(|p| p > 3.0) {
            inner.send(json!(["seek", 0, "absolute"])).await?;
            return Ok(None);
        }
        inner.queue.prev();
        inner.load_current().await.map(Some)
    }

    /// Plays the song `offset` places ahead in the queue (0 restarts the current one).
    pub async fn jump(&self, offset: usize) -> Result<Option<Loaded>> {
        let mut inner = self.0.lock().await;
        if inner.current.is_none() {
            return Ok(None);
        }
        inner.failures = 0;
        inner.queue.skip_forward(offset);
        inner.load_current().await.map(Some)
    }

    pub async fn set_volume(&self, volume: f64) -> Result<f64> {
        let mut inner = self.0.lock().await;
        inner.volume = volume.clamp(0.0, 100.0);
        inner.muted = false;
        let (volume, muted) = (inner.volume, false);
        if inner.proc.is_some() {
            inner.send(json!(["set_property", "volume", volume])).await?;
            inner.send(json!(["set_property", "mute", muted])).await?;
        }
        Ok(volume)
    }

    pub async fn volume_by(&self, delta: f64) -> Result<f64> {
        let volume = self.0.lock().await.volume;
        self.set_volume(volume + delta).await
    }

    pub async fn toggle_mute(&self) -> Result<()> {
        let mut inner = self.0.lock().await;
        inner.muted = !inner.muted;
        let muted = inner.muted;
        if inner.proc.is_some() {
            inner.send(json!(["set_property", "mute", muted])).await?;
        }
        Ok(())
    }

    /// Adds a song to the end of the queue. `false` when nothing is playing (so there is no queue).
    pub async fn enqueue(&self, track: MusicTrack) -> bool {
        let mut inner = self.0.lock().await;
        if inner.current.is_none() {
            return false;
        }
        inner.queue.append(track);
        true
    }

    pub async fn set_shuffle(&self, on: bool) {
        self.0.lock().await.queue.set_shuffle(on);
    }

    pub async fn set_repeat(&self, repeat: Repeat) {
        self.0.lock().await.queue.repeat = repeat;
    }

    pub async fn shuffle(&self) -> bool {
        self.0.lock().await.queue.shuffle
    }

    pub async fn repeat(&self) -> Repeat {
        self.0.lock().await.queue.repeat
    }

    /// The current song followed by the ones coming up, in play order.
    pub async fn upcoming(&self) -> Vec<MusicTrack> {
        self.0.lock().await.queue.upcoming().into_iter().cloned().collect()
    }

    pub async fn current_id(&self) -> Option<String> {
        self.0.lock().await.current.as_ref().map(|t| t.video_id.clone())
    }

    pub async fn snapshot(&self) -> Snapshot {
        let inner = self.0.lock().await;
        let live = lock(&inner.live);
        let playing = inner.current.is_some();
        Snapshot {
            current: inner.current.clone(),
            next: inner.queue.peek_next().cloned(),
            position: live.position.filter(|_| playing),
            duration: live.duration.or_else(|| inner.current.as_ref().and_then(|t| t.duration_secs).map(f64::from)),
            paused: live.paused,
            loaded: live.loaded,
            volume: inner.volume,
            muted: inner.muted,
            shuffle: inner.queue.shuffle,
            repeat: inner.queue.repeat,
            error: inner.error.as_ref().filter(|(_, at)| at.elapsed() < ERROR_SHOWN).map(|(m, _)| m.clone()),
        }
    }

    /// Called about twice a second: moves on when a song has ended or failed, and notices a dead mpv.
    /// Returns the song that started, if one did.
    pub async fn tick(&self) -> Result<Option<Loaded>> {
        let mut inner = self.0.lock().await;
        if let Some(proc) = inner.proc.as_mut() {
            if matches!(proc.child.try_wait(), Ok(Some(_))) {
                inner.kill().await;
                if inner.current.take().is_some() {
                    inner.queue.clear();
                    inner.set_error("The audio player stopped. Is mpv working?");
                }
                return Ok(None);
            }
        }
        let (ended, loaded) = {
            let mut live = lock(&inner.live);
            (live.ended.take(), live.loaded)
        };
        if loaded {
            inner.failures = 0;
        }
        if inner.current.is_none() {
            return Ok(None);
        }
        match ended {
            None => Ok(None),
            Some(End::Eof) => inner.advance(true).await,
            Some(End::Failed(reason)) => inner.failed(&reason).await,
        }
    }

    /// The song waiting for its download, if any.
    pub async fn waiting_id(&self) -> Option<String> {
        let inner = self.0.lock().await;
        if inner.waiting { inner.current.as_ref().map(|t| t.video_id.clone()) } else { None }
    }

    /// The waiting song's file has arrived: plays it. Does nothing if it is still missing.
    pub async fn resume_waiting(&self) -> Result<()> {
        let mut inner = self.0.lock().await;
        if !inner.waiting {
            return Ok(());
        }
        let Some(current) = inner.current.clone() else { return Ok(()) };
        let track = inner.db.music_track(&current.video_id).await?.unwrap_or(current);
        let Some(file) = track.local_file().map(str::to_owned) else { return Ok(()) };
        inner.waiting = false;
        inner.current = Some(track);
        inner.send(json!(["loadfile", file, "replace"])).await?;
        inner.send(json!(["set_property", "pause", false])).await
    }

    /// The waiting song could not be downloaded: counts as a failed song and moves on.
    pub async fn fail_waiting(&self, reason: &str) -> Result<Option<Loaded>> {
        let mut inner = self.0.lock().await;
        if !inner.waiting {
            return Ok(None);
        }
        inner.waiting = false;
        inner.failed(reason).await
    }

    /// Stops playback and ends the mpv process (used when switching service).
    pub async fn stop(&self) {
        let mut inner = self.0.lock().await;
        inner.finish().await;
        inner.error = None;
        inner.kill().await;
    }
}

impl Inner {
    fn set_error(&mut self, message: &str) {
        self.error = Some((message.to_owned(), Instant::now()));
    }

    /// Writes one command to mpv.
    async fn send(&mut self, command: Value) -> Result<()> {
        let proc = self.proc.as_mut().context("the audio player is not running")?;
        let line = format!("{}\n", json!({ "command": command }));
        proc.writer.write_all(line.as_bytes()).await.context("could not talk to mpv")
    }

    async fn kill(&mut self) {
        if let Some(mut proc) = self.proc.take() {
            let _ = proc.child.kill().await;
            proc.reader.abort();
        }
        let _ = std::fs::remove_file(self.run_dir.join("ipc"));
    }

    /// Nothing is playing any more.
    async fn finish(&mut self) {
        self.queue.clear();
        self.current = None;
        self.waiting = false;
        self.failures = 0;
        if self.proc.is_some() {
            let _ = self.send(json!(["stop"])).await;
        }
        *lock(&self.live) = Live::default();
    }

    /// The current song failed (mpv could not play it, or it could not be downloaded): skip it, and
    /// stop if too many fail in a row.
    async fn failed(&mut self, reason: &str) -> Result<Option<Loaded>> {
        tracing::warn!("the song could not be played: {reason}");
        self.failures += 1;
        self.set_error("Couldn't play that song, so it was skipped.");
        if self.failures >= MAX_FAILURES {
            self.set_error("Several songs in a row failed. Check the network and yt-dlp.");
            self.finish().await;
            return Ok(None);
        }
        self.advance(false).await
    }

    async fn advance(&mut self, auto: bool) -> Result<Option<Loaded>> {
        if self.queue.next(auto).is_some() {
            self.load_current().await.map(Some)
        } else {
            self.finish().await;
            Ok(None)
        }
    }

    /// Starts mpv if it is not running, and connects to its socket.
    async fn ensure_started(&mut self) -> Result<()> {
        if let Some(proc) = self.proc.as_mut() {
            if matches!(proc.child.try_wait(), Ok(None)) && !proc.reader.is_finished() {
                return Ok(());
            }
            self.kill().await;
        }
        let socket = self.run_dir.join("ipc");
        let mut command = Command::new(&self.executable);
        command
            .args([
                "--no-config",
                "--no-video",
                "--idle=yes",
                "--no-terminal",
                "--force-window=no",
                "--audio-display=no",
                "--really-quiet",
                "--ytdl=no",
            ])
            .arg(format!("--input-ipc-server={}", socket.display()))
            .arg(format!("--volume={}", self.volume))
            .arg(format!("--mute={}", if self.muted { "yes" } else { "no" }));
        if let Some(output) = &self.audio_output {
            command.arg(format!("--ao={output}"));
        }
        let mut child = command
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .kill_on_drop(true)
            .spawn()
            .context("could not start mpv (is it installed?)")?;
        // mpv creates the socket a moment after it starts.
        let mut stream = None;
        for _ in 0..200 {
            if let Ok(Some(status)) = child.try_wait() {
                bail!("mpv exited right away ({status})");
            }
            if let Ok(connected) = UnixStream::connect(&socket).await {
                stream = Some(connected);
                break;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        let Some(stream) = stream else {
            let _ = child.kill().await;
            bail!("mpv did not open its control socket");
        };
        let (read, mut writer) = stream.into_split();
        for (id, name) in ["time-pos", "duration", "pause"].iter().enumerate() {
            let line = format!("{}\n", json!({ "command": ["observe_property", id + 1, name] }));
            writer.write_all(line.as_bytes()).await.context("could not talk to mpv")?;
        }
        let live = self.live.clone();
        let reader = tokio::spawn(read_events(read, live));
        self.proc = Some(Proc { child, writer, reader });
        Ok(())
    }

    /// Starts the queue's current song: from its file, or, when it is not downloaded yet, waits for
    /// the download (nothing plays meanwhile).
    async fn load_current(&mut self) -> Result<Loaded> {
        let queued = self.queue.current().cloned().context("there is nothing to play")?;
        // The queue holds a copy from when it was built; the download may have finished (or been
        // evicted) since.
        let track = self.db.music_track(&queued.video_id).await?.unwrap_or(queued);
        self.ensure_started().await?;
        *lock(&self.live) = Live::default();
        match track.local_file().map(str::to_owned) {
            Some(file) => {
                self.waiting = false;
                self.send(json!(["loadfile", file, "replace"])).await?;
                self.send(json!(["set_property", "pause", false])).await?;
            }
            None => {
                self.waiting = true;
                self.send(json!(["stop"])).await?;
            }
        }
        self.current = Some(track.clone());
        let next = self.queue.peek_next().cloned();
        Ok(Loaded { track, next })
    }
}

/// Follows mpv's events and keeps `live` up to date.
async fn read_events(read: OwnedReadHalf, live: Arc<StdMutex<Live>>) {
    let mut lines = BufReader::new(read).lines();
    while let Ok(Some(line)) = lines.next_line().await {
        let Ok(event) = serde_json::from_str::<Value>(&line) else { continue };
        let mut live = lock(&live);
        match event["event"].as_str() {
            Some("start-file") => {
                live.position = None;
                live.duration = None;
                live.loaded = false;
                live.ended = None;
            }
            Some("file-loaded") => live.loaded = true,
            Some("end-file") => match event["reason"].as_str() {
                Some("eof") => live.ended = Some(End::Eof),
                Some("error") => live.ended = Some(End::Failed(event["file_error"].as_str().unwrap_or("unknown error").to_owned())),
                // "stop" is our own `loadfile replace` or `stop`; "quit" is mpv closing.
                _ => {}
            },
            Some("property-change") => match event["name"].as_str() {
                Some("time-pos") => {
                    if let Some(value) = event["data"].as_f64().filter(|v| v.is_finite() && *v >= 0.0) {
                        live.position = Some(value);
                    }
                }
                Some("duration") => {
                    if let Some(value) = event["data"].as_f64().filter(|v| v.is_finite() && *v > 0.0) {
                        live.duration = Some(value);
                    }
                }
                Some("pause") => live.paused = event["data"].as_bool().unwrap_or(false),
                _ => {}
            },
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;
    use tokio::{
        io::Lines,
        net::{UnixListener, unix::OwnedReadHalf as ReadHalf},
    };

    fn track(id: &str) -> MusicTrack {
        MusicTrack {
            video_id: id.into(),
            title: format!("Song {id}"),
            artist: None,
            duration_secs: Some(100),
            thumbnail_url: None,
            file_path: None,
            downloaded_at: None,
        }
    }

    const IDS: [&str; 3] = ["aaaaaaaaaaa", "bbbbbbbbbbb", "ccccccccccc"];

    fn options() -> PlayOptions {
        PlayOptions { shuffle: false, repeat: Repeat::Off, volume: 40.0 }
    }

    /// A player whose "mpv" is `sleep` and whose socket is a listener the test controls.
    struct Fake {
        player: MusicPlayer,
        db: Database,
        dir: PathBuf,
        listener: UnixListener,
    }

    fn fake(name: &str) -> Fake {
        let dir = std::env::temp_dir().join(format!("ct-music-player-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let executable = dir.join("fake-mpv");
        std::fs::write(&executable, "#!/bin/sh\nexec sleep 30\n").unwrap();
        std::fs::set_permissions(&executable, std::fs::Permissions::from_mode(0o755)).unwrap();
        let db = Database::open(&dir.join("db.sqlite")).unwrap();
        let run = dir.join("run");
        let player = MusicPlayer::with_executable(db.clone(), executable, run.clone(), None).unwrap();
        let listener = UnixListener::bind(run.join("ipc")).unwrap();
        Fake { player, db, dir, listener }
    }

    impl Fake {
        /// Puts these songs in the download cache (a small file each), so they can play.
        async fn cache(&self, ids: &[&str]) -> Vec<PathBuf> {
            let mut paths = vec![];
            for (n, id) in ids.iter().enumerate() {
                let path = self.dir.join(format!("{id}.opus"));
                std::fs::write(&path, b"audio").unwrap();
                self.db.music_upsert_tracks(vec![track(id)]).await.unwrap();
                self.db.music_set_downloaded(id, &path.to_string_lossy(), n as i64).await.unwrap();
                paths.push(path);
            }
            paths
        }
    }

    /// The commands the player sends, read from its end of the socket.
    struct Mpv {
        lines: Lines<BufReader<ReadHalf>>,
        writer: OwnedWriteHalf,
    }

    impl Mpv {
        async fn accept(listener: &UnixListener) -> Self {
            let (stream, _) = tokio::time::timeout(Duration::from_secs(5), listener.accept()).await.unwrap().unwrap();
            let (read, writer) = stream.into_split();
            Self { lines: BufReader::new(read).lines(), writer }
        }

        async fn command(&mut self) -> Vec<Value> {
            let line = tokio::time::timeout(Duration::from_secs(5), self.lines.next_line()).await.unwrap().unwrap().unwrap();
            serde_json::from_str::<Value>(&line).unwrap()["command"].as_array().unwrap().clone()
        }

        /// The next command that is not part of the start-up (observing properties).
        async fn command_after_observers(&mut self) -> Vec<Value> {
            loop {
                let command = self.command().await;
                if command[0] != "observe_property" {
                    return command;
                }
            }
        }

        /// The next `loadfile` command's source, skipping the property setup around it.
        async fn loaded(&mut self) -> String {
            loop {
                let command = self.command().await;
                if command[0] == "loadfile" {
                    return command[1].as_str().unwrap().to_owned();
                }
            }
        }

        async fn event(&mut self, event: Value) {
            self.writer.write_all(format!("{event}\n").as_bytes()).await.unwrap();
        }
    }

    async fn tick_until_loaded(player: &MusicPlayer) -> Loaded {
        for _ in 0..100 {
            if let Some(loaded) = player.tick().await.unwrap() {
                return loaded;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        panic!("no song started");
    }

    #[tokio::test]
    async fn a_song_that_is_not_downloaded_waits_and_plays_when_its_file_arrives() {
        let fake = fake("waits");
        let files = fake.cache(&[IDS[1]]).await;
        fake.db.music_upsert_tracks(IDS.map(track).to_vec()).await.unwrap();

        let loaded = fake.player.play(IDS.map(track).to_vec(), 0, options()).await.unwrap();
        assert_eq!(loaded.track.video_id, IDS[0]);
        assert_eq!(loaded.next.unwrap().video_id, IDS[1]);
        assert_eq!(fake.player.waiting_id().await.as_deref(), Some(IDS[0]));
        assert!(!fake.player.snapshot().await.loaded);
        let mut mpv = Mpv::accept(&fake.listener).await;
        // Nothing is loaded while it waits (never a stream), and resuming too early does nothing.
        fake.player.resume_waiting().await.unwrap();
        assert_eq!(mpv.command_after_observers().await, vec![json!("stop")]);

        // The download finishes: the song starts from its file.
        let file = fake.dir.join("aaaaaaaaaaa.opus");
        std::fs::write(&file, b"audio").unwrap();
        fake.db.music_set_downloaded(IDS[0], &file.to_string_lossy(), 9).await.unwrap();
        fake.player.resume_waiting().await.unwrap();
        assert!(fake.player.waiting_id().await.is_none());
        assert_eq!(mpv.loaded().await, file.to_string_lossy());

        // A downloaded song plays at once.
        let loaded = fake.player.next().await.unwrap().unwrap();
        assert_eq!(loaded.track.video_id, IDS[1]);
        assert!(fake.player.waiting_id().await.is_none());
        assert_eq!(mpv.loaded().await, files[0].to_string_lossy());
        fake.player.stop().await;
        let _ = std::fs::remove_dir_all(&fake.dir);
    }

    #[tokio::test]
    async fn a_song_whose_download_failed_is_skipped() {
        let fake = fake("download-failed");
        fake.cache(&[IDS[1]]).await;
        fake.db.music_upsert_tracks(IDS.map(track).to_vec()).await.unwrap();
        fake.player.play(IDS.map(track).to_vec(), 0, options()).await.unwrap();
        let mut mpv = Mpv::accept(&fake.listener).await;
        let loaded = fake.player.fail_waiting("No internet connection.").await.unwrap().unwrap();
        assert_eq!(loaded.track.video_id, IDS[1]);
        assert!(fake.player.snapshot().await.error.unwrap().contains("skipped"));
        mpv.loaded().await;
        // Nothing waits any more, so a second call does nothing.
        assert!(fake.player.fail_waiting("again").await.unwrap().is_none());
        fake.player.stop().await;
        let _ = std::fs::remove_dir_all(&fake.dir);
    }

    #[tokio::test]
    async fn moves_on_when_a_song_ends_and_stops_at_the_end() {
        let fake = fake("advance");
        fake.cache(&IDS).await;
        fake.player.play(IDS.map(track)[..2].to_vec(), 0, options()).await.unwrap();
        let mut mpv = Mpv::accept(&fake.listener).await;
        mpv.loaded().await;
        mpv.event(json!({"event": "file-loaded"})).await;
        mpv.event(json!({"event": "end-file", "reason": "eof"})).await;
        let loaded = tick_until_loaded(&fake.player).await;
        assert_eq!(loaded.track.video_id, IDS[1]);
        assert!(mpv.loaded().await.ends_with("bbbbbbbbbbb.opus"));
        // Our own `loadfile replace` makes mpv report "stop", which must not skip a song.
        mpv.event(json!({"event": "end-file", "reason": "stop"})).await;
        tokio::time::sleep(Duration::from_millis(100)).await;
        assert!(fake.player.tick().await.unwrap().is_none());
        assert_eq!(fake.player.current_id().await.as_deref(), Some(IDS[1]));
        // The last song ending empties the queue.
        mpv.event(json!({"event": "end-file", "reason": "eof"})).await;
        for _ in 0..100 {
            fake.player.tick().await.unwrap();
            if fake.player.current_id().await.is_none() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        assert!(fake.player.current_id().await.is_none());
        assert!(!fake.player.snapshot().await.loaded);
        fake.player.stop().await;
        let _ = std::fs::remove_dir_all(&fake.dir);
    }

    #[tokio::test]
    async fn a_failing_song_is_skipped_and_repeated_failures_stop_playback() {
        let fake = fake("failure");
        fake.cache(&["aaaaaaaaaaa", "bbbbbbbbbbb", "ccccccccccc", "ddddddddddd", "eeeeeeeeeee"]).await;
        let tracks: Vec<_> = ["aaaaaaaaaaa", "bbbbbbbbbbb", "ccccccccccc", "ddddddddddd", "eeeeeeeeeee"].map(track).to_vec();
        fake.player.play(tracks, 0, options()).await.unwrap();
        let mut mpv = Mpv::accept(&fake.listener).await;
        mpv.loaded().await;
        mpv.event(json!({"event": "end-file", "reason": "error", "file_error": "loading failed"})).await;
        let loaded = tick_until_loaded(&fake.player).await;
        assert_eq!(loaded.track.video_id, "bbbbbbbbbbb");
        assert!(fake.player.snapshot().await.error.unwrap().contains("skipped"));
        // Two more failures in a row: the third stops playback instead of working through the queue.
        for _ in 0..2 {
            mpv.event(json!({"event": "end-file", "reason": "error", "file_error": "loading failed"})).await;
            for _ in 0..100 {
                fake.player.tick().await.unwrap();
                tokio::time::sleep(Duration::from_millis(20)).await;
                if fake.player.current_id().await.is_none() {
                    break;
                }
            }
        }
        assert!(fake.player.current_id().await.is_none());
        assert!(fake.player.snapshot().await.error.unwrap().contains("Several songs"));
        fake.player.stop().await;
        let _ = std::fs::remove_dir_all(&fake.dir);
    }

    #[tokio::test]
    async fn controls_send_the_right_commands_and_progress_is_reported() {
        let fake = fake("controls");
        fake.cache(&IDS).await;
        fake.player.play(IDS.map(track).to_vec(), 0, options()).await.unwrap();
        let mut mpv = Mpv::accept(&fake.listener).await;
        mpv.loaded().await;
        fake.player.seek_by(-10.0).await.unwrap();
        fake.player.set_paused(true).await.unwrap();
        fake.player.volume_by(VOLUME_STEP).await.unwrap();
        fake.player.toggle_mute().await.unwrap();
        let mut seen = vec![];
        // The unpause after the load, the seek, the pause, volume and unmute, and the mute.
        for _ in 0..6 {
            seen.push(mpv.command().await);
        }
        assert!(seen.contains(&vec![json!("seek"), json!(-10.0), json!("relative")]));
        assert!(seen.contains(&vec![json!("set_property"), json!("pause"), json!(true)]));
        assert!(seen.contains(&vec![json!("set_property"), json!("volume"), json!(45.0)]));
        assert!(seen.contains(&vec![json!("set_property"), json!("mute"), json!(true)]));

        mpv.event(json!({"event": "property-change", "name": "time-pos", "data": 12.5})).await;
        mpv.event(json!({"event": "property-change", "name": "duration", "data": 200.0})).await;
        for _ in 0..50 {
            if fake.player.snapshot().await.position.is_some() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        let snapshot = fake.player.snapshot().await;
        assert_eq!(snapshot.position, Some(12.5));
        assert_eq!(snapshot.duration, Some(200.0));
        assert!(snapshot.paused && snapshot.muted);
        assert!(fake.player.seek_to(f64::NAN).await.is_err());
        fake.player.stop().await;
        let _ = std::fs::remove_dir_all(&fake.dir);
    }

    #[tokio::test]
    async fn previous_restarts_a_song_that_has_played_a_while() {
        let fake = fake("prev");
        fake.cache(&IDS).await;
        fake.player.play(IDS.map(track).to_vec(), 1, options()).await.unwrap();
        let mut mpv = Mpv::accept(&fake.listener).await;
        mpv.loaded().await;
        mpv.event(json!({"event": "property-change", "name": "time-pos", "data": 30.0})).await;
        for _ in 0..50 {
            if fake.player.snapshot().await.position.is_some() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        assert!(fake.player.prev().await.unwrap().is_none(), "past 3 s it only restarts the song");
        assert_eq!(fake.player.current_id().await.as_deref(), Some(IDS[1]));
        // Reset the position the way a new file would, then go back for real.
        mpv.event(json!({"event": "start-file"})).await;
        for _ in 0..50 {
            if fake.player.snapshot().await.position.is_none() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        let loaded = fake.player.prev().await.unwrap().unwrap();
        assert_eq!(loaded.track.video_id, IDS[0]);
        fake.player.stop().await;
        let _ = std::fs::remove_dir_all(&fake.dir);
    }
}
