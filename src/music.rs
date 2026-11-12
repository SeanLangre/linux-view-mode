//! Music mode (see `docs/music-plan.md`): profiles, search, playlists, favorites, playback control,
//! the download cache and lyrics. Everything under `/api/music/`. Playback itself is the
//! audio-only mpv in `music_player`; this module owns the orchestration and the HTTP surface.
use std::collections::HashSet;

use axum::{
    Json, Router,
    extract::{Path, Query, State, rejection::{JsonRejection, QueryRejection}},
    http::StatusCode,
    routing::{delete, get, post, put},
};
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::{
    api::{ApiError, AppState, internal},
    db::Database,
    lyrics,
    music_cache::{DEFAULT_LIMIT, DownloadState, LIMITS, MusicCache},
    music_db::{MusicProfile, MusicTrack, StoredLyrics},
    music_player::{Loaded, MusicPlayer, PlayOptions, VOLUME_STEP, parse_repeat, repeat_name},
    music_queue::Repeat,
    music_yt, profiles, remote, ytdlp,
};

/// How many songs one request may add or play.
const MAX_SONGS: usize = 200;
const SEARCH_STEP: usize = 15;
/// The first songs of an imported playlist are downloaded at once; the rest as the playlist plays.
const IMPORT_DOWNLOADS: usize = 5;
const MAX_PLAYLIST_NAME: usize = 60;

/// Everything Music mode keeps in memory: the player, the cache and a database handle.
#[derive(Clone)]
pub struct Music {
    pub player: MusicPlayer,
    pub cache: MusicCache,
    db: Database,
}

impl Music {
    pub async fn new(db: Database, data_dir: &std::path::Path) -> anyhow::Result<Self> {
        let limit = db
            .music_global_setting("cache_limit")
            .await?
            .and_then(|v| v.parse().ok())
            .filter(|n| LIMITS.contains(n))
            .unwrap_or(DEFAULT_LIMIT);
        let cache = MusicCache::new(db.clone(), data_dir, limit);
        cache.reconcile().await?;
        Ok(Self { player: MusicPlayer::new(db.clone())?, cache, db })
    }

    /// Stops the music and ends mpv (switching service, or leaving).
    pub async fn stop(&self) {
        self.player.stop().await;
        self.cache.set_protected(None);
    }

    /// Called twice a second: song changes, the cache's protected song, and follow-ups.
    pub async fn tick(&self) {
        match self.player.tick().await {
            Ok(Some(loaded)) => self.started(&loaded).await,
            Ok(None) => {}
            Err(err) => tracing::error!("music playback failed: {err:#}"),
        }
        self.cache.set_protected(self.player.current_id().await);
        self.resume_when_downloaded().await;
    }

    /// A song that waited for its download plays as soon as the file is there; if the download
    /// failed or was cancelled, the song is skipped.
    async fn resume_when_downloaded(&self) {
        let Some(id) = self.player.waiting_id().await else { return };
        let track = match self.db.music_track(&id).await {
            Ok(Some(track)) => track,
            _ => return,
        };
        if track.local_file().is_some() {
            if let Err(err) = self.player.resume_waiting().await {
                tracing::error!("could not start the downloaded song: {err:#}");
            }
            return;
        }
        let reason = match self.cache.state(&track) {
            DownloadState::Downloading(_) => return,
            DownloadState::Failed(message) => message,
            _ => "the download was cancelled".to_owned(),
        };
        match self.player.fail_waiting(&reason).await {
            Ok(Some(loaded)) => self.started(&loaded).await,
            Ok(None) => {}
            Err(err) => tracing::error!("music playback failed: {err:#}"),
        }
    }

    /// What follows a song starting: history, the cache's protected song and the downloads of this
    /// song (if it is not here yet) and the next one. Music never streams.
    async fn started(&self, loaded: &Loaded) {
        self.cache.set_protected(Some(loaded.track.video_id.clone()));
        match self.db.music_active().await {
            Ok(profile) => {
                if let Err(err) = self.db.music_record_play(&profile.id, &loaded.track.video_id).await {
                    tracing::warn!("could not save play history: {err:#}");
                }
            }
            Err(err) => tracing::warn!("could not read the active music profile: {err:#}"),
        }
        for track in std::iter::once(&loaded.track).chain(loaded.next.as_ref()) {
            if track.local_file().is_none() {
                self.cache.start(track.video_id.clone());
            }
        }
    }

    async fn views(&self, favorites: &HashSet<String>, tracks: Vec<MusicTrack>) -> anyhow::Result<Vec<TrackView>> {
        let ids: Vec<String> = tracks.iter().map(|t| t.video_id.clone()).collect();
        let fresh = self.db.music_tracks_by_ids(&ids).await?;
        Ok(tracks
            .into_iter()
            .map(|t| self.view(favorites, fresh.get(&t.video_id).cloned().unwrap_or(t)))
            .collect())
    }

    fn view(&self, favorites: &HashSet<String>, track: MusicTrack) -> TrackView {
        let (state, progress, error) = match self.cache.state(&track) {
            DownloadState::Offline => ("offline", None, None),
            DownloadState::Missing => ("missing", None, None),
            DownloadState::Downloading(progress) => ("downloading", progress, None),
            DownloadState::Failed(message) => ("failed", None, Some(message)),
        };
        TrackView {
            thumb: format!("/api/thumb/{}", track.video_id),
            favorite: favorites.contains(&track.video_id),
            id: track.video_id,
            title: track.title,
            artist: track.artist,
            duration: track.duration_secs,
            state,
            progress,
            error,
        }
    }

    /// Starts playing `tracks` from `start`. `shuffle` turns shuffle on or off (and remembers it);
    /// left out, the profile's saved setting applies. Downloads begin at once: music never streams.
    pub async fn start(&self, tracks: Vec<MusicTrack>, start: usize, shuffle: Option<bool>) -> Result<(), ApiError> {
        let profile = self.db.music_active().await.map_err(internal)?;
        let (saved_shuffle, repeat, volume) = self.prefs(&profile.id).await.map_err(internal)?;
        let shuffle = shuffle.unwrap_or(saved_shuffle);
        if shuffle != saved_shuffle {
            self.db.set_music_setting(&profile.id, "shuffle", if shuffle { "1" } else { "0" }).await.map_err(internal)?;
        }
        let loaded = self.player.play(tracks, start, PlayOptions { shuffle, repeat, volume }).await.map_err(|err| {
            tracing::error!("could not start music: {err:#}");
            ApiError::new(StatusCode::BAD_GATEWAY, "could not start the audio player, is mpv installed?")
        })?;
        self.started(&loaded).await;
        Ok(())
    }

    /// One playback command: `pause`, `resume`, `toggle`, `seek` (value: seconds, negative goes
    /// back), `seek_to`, `next`, `prev`, `jump` (value: places ahead in the queue), `volume`
    /// (value: 0–100), `volume_up`, `volume_down`, `volume_by` (value: points), `mute`, `shuffle`,
    /// `repeat` (name: off, all or one; left out it cycles) and `stop`.
    pub async fn control(&self, action: &str, value: Option<f64>, name: Option<&str>) -> Result<(), ApiError> {
        let profile = self.db.music_active().await.map_err(internal)?;
        let number = || value.filter(|v| v.is_finite()).ok_or_else(|| bad("this action needs a value"));
        let failed = |err: anyhow::Error| {
            tracing::error!("music control failed: {err:#}");
            ApiError::new(StatusCode::BAD_GATEWAY, "could not control the audio player")
        };
        let player = &self.player;
        let started = match action {
            "pause" => player.set_paused(true).await.map(|_| None),
            "resume" => player.set_paused(false).await.map(|_| None),
            "toggle" => player.toggle_pause().await.map(|_| None),
            "seek" => player.seek_by(number()?.clamp(-3600.0, 3600.0)).await.map(|_| None),
            "seek_to" => player.seek_to(number()?.clamp(0.0, 1_000_000.0)).await.map(|_| None),
            "next" => player.next().await,
            "prev" => player.prev().await,
            "jump" => {
                let offset = number()?;
                if !(0.0..=1000.0).contains(&offset) {
                    return Err(bad("invalid queue position"));
                }
                player.jump(offset as usize).await
            }
            "volume" | "volume_up" | "volume_down" | "volume_by" => {
                let volume = match action {
                    "volume" => player.set_volume(number()?).await,
                    "volume_up" => player.volume_by(VOLUME_STEP).await,
                    "volume_down" => player.volume_by(-VOLUME_STEP).await,
                    _ => player.volume_by(number()?.clamp(-20.0, 20.0)).await,
                }
                .map_err(failed)?;
                self.db.set_music_setting(&profile.id, "volume", &volume.to_string()).await.map_err(internal)?;
                Ok(None)
            }
            "mute" => player.toggle_mute().await.map(|_| None),
            "shuffle" => {
                let on = value.map(|v| v != 0.0).unwrap_or(!player.shuffle().await);
                player.set_shuffle(on).await;
                self.db.set_music_setting(&profile.id, "shuffle", if on { "1" } else { "0" }).await.map_err(internal)?;
                Ok(None)
            }
            "repeat" => {
                let repeat = match name {
                    Some(name) => parse_repeat(name).ok_or_else(|| bad("repeat must be off, all or one"))?,
                    None => player.repeat().await.cycle(),
                };
                player.set_repeat(repeat).await;
                self.db.set_music_setting(&profile.id, "repeat", repeat_name(repeat)).await.map_err(internal)?;
                Ok(None)
            }
            "stop" => {
                self.stop().await;
                Ok(None)
            }
            _ => return Err(bad("unknown action")),
        }
        .map_err(failed)?;
        if let Some(loaded) = started {
            self.started(&loaded).await;
        }
        Ok(())
    }

    /// A song from what someone typed: a YouTube link, or else the first search result.
    pub async fn song_from_text(&self, text: &str) -> Result<MusicTrack, ApiError> {
        let upstream = |message: String| ApiError::new(StatusCode::BAD_GATEWAY, message);
        let text = music_yt::clean_query(text).ok_or_else(|| bad("type a song name or paste a YouTube link"))?;
        let track = match remote::video_id(&text).filter(|_| text.contains('/')) {
            Some(id) => music_yt::fetch_video(&id).await.map_err(upstream)?,
            None => music_yt::search(&text, 1).await.map_err(upstream)?.into_iter().next().ok_or_else(|| not_found("no songs found"))?,
        };
        self.db.music_upsert_tracks(vec![track.clone()]).await.map_err(internal)?;
        Ok(self.db.music_track(&track.video_id).await.map_err(internal)?.unwrap_or(track))
    }

    /// Adds a song to the end of the queue and starts downloading it. `false` when nothing is
    /// playing, so there is no queue to add to.
    pub async fn enqueue(&self, track: MusicTrack) -> bool {
        if track.local_file().is_none() {
            self.cache.start(track.video_id.clone());
        }
        self.player.enqueue(track).await
    }

    async fn prefs(&self, profile: &str) -> anyhow::Result<(bool, Repeat, f64)> {
        let shuffle = self.db.music_setting(profile, "shuffle").await?.as_deref() == Some("1");
        let repeat = self.db.music_setting(profile, "repeat").await?.and_then(|v| parse_repeat(&v)).unwrap_or(Repeat::Off);
        let volume = self.db.music_setting(profile, "volume").await?.and_then(|v| v.parse().ok()).unwrap_or(70.0);
        Ok((shuffle, repeat, volume))
    }
}

/// A song as the screens see it.
#[derive(Serialize, Debug)]
struct TrackView {
    id: String,
    title: String,
    artist: Option<String>,
    /// Seconds.
    duration: Option<u32>,
    thumb: String,
    /// `offline` (downloaded), `missing` (not downloaded), `downloading` or `failed`.
    state: &'static str,
    progress: Option<f32>,
    error: Option<String>,
    favorite: bool,
}

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/music/profiles", get(list_profiles).post(create_profile))
        .route("/music/profiles/{id}", delete(remove_profile))
        .route("/music/profiles/{id}/select", post(select_profile))
        .route("/music/home", get(home))
        .route("/music/search", get(search))
        .route("/music/playlists", get(list_playlists).post(create_playlist))
        .route("/music/playlists/{id}", get(get_playlist).put(rename_playlist).delete(delete_playlist))
        .route("/music/playlists/{id}/tracks", post(add_tracks))
        .route("/music/playlists/{id}/tracks/{position}", delete(remove_track))
        .route("/music/playlists/{id}/move", post(move_track))
        .route("/music/favorites", get(favorites))
        .route("/music/favorites/{video_id}", put(add_favorite).delete(remove_favorite))
        .route("/music/play", post(play))
        .route("/music/control", post(control))
        .route("/music/state", get(state))
        .route("/music/queue", get(queue))
        .route("/music/lyrics/{video_id}", get(get_lyrics))
        .route("/music/downloads", delete(clear_downloads))
        .route("/music/downloads/{video_id}", post(start_download).delete(cancel_download))
        .route("/music/settings", get(get_settings).put(set_settings))
}

fn bad(message: &'static str) -> ApiError {
    ApiError::new(StatusCode::BAD_REQUEST, message)
}

fn not_found(message: &'static str) -> ApiError {
    ApiError::new(StatusCode::NOT_FOUND, message)
}

fn json_body<T>(body: Result<Json<T>, JsonRejection>, expected: &'static str) -> Result<T, ApiError> {
    body.map(|Json(value)| value).map_err(|_| bad(expected))
}

async fn active(state: &AppState) -> Result<MusicProfile, ApiError> {
    state.profiles.db.music_active().await.map_err(internal)
}

fn video_id(id: &str) -> Result<(), ApiError> {
    if ytdlp::is_video_id(id) { Ok(()) } else { Err(bad("invalid song id")) }
}

/// A song that is already in the database (search saves every result it shows).
async fn known_track(state: &AppState, id: &str) -> Result<MusicTrack, ApiError> {
    video_id(id)?;
    state.profiles.db.music_track(id).await.map_err(internal)?.ok_or_else(|| not_found("unknown song, search for it again"))
}

// ---- profiles --------------------------------------------------------------------------------

async fn list_profiles(State(state): State<AppState>) -> Result<Json<serde_json::Value>, ApiError> {
    let list = state.profiles.db.music_profiles().await.map_err(internal)?;
    let active = active(&state).await?;
    Ok(Json(json!({ "profiles": list, "active_id": active.id })))
}

#[derive(Deserialize)]
struct NewProfile {
    name: String,
    color: Option<String>,
}

async fn create_profile(
    State(state): State<AppState>,
    body: Result<Json<NewProfile>, JsonRejection>,
) -> Result<(StatusCode, Json<MusicProfile>), ApiError> {
    let request = json_body(body, "expected {name, color?} JSON")?;
    if !profiles::valid_name(&request.name) {
        return Err(bad("name must contain 1–40 characters without control characters"));
    }
    let color = request.color.as_deref().unwrap_or("#53a888");
    if !profiles::valid_color(color) {
        return Err(bad("color must be a six-digit hex color"));
    }
    let profile = state.profiles.db.music_create_profile(request.name.trim(), color).await.map_err(internal)?;
    Ok((StatusCode::CREATED, Json(profile)))
}

async fn select_profile(State(state): State<AppState>, Path(id): Path<String>) -> Result<Json<MusicProfile>, ApiError> {
    let profile = state.profiles.db.music_profile(&id).await.map_err(internal)?.ok_or_else(|| not_found("no such music profile"))?;
    // The queue belongs to whoever started it.
    state.music.stop().await;
    state.profiles.db.music_select(&id).await.map_err(internal)?;
    Ok(Json(profile))
}

async fn remove_profile(State(state): State<AppState>, Path(id): Path<String>) -> Result<StatusCode, ApiError> {
    if id == "guest" {
        return Err(bad("Guest cannot be removed"));
    }
    state.profiles.db.music_profile(&id).await.map_err(internal)?.ok_or_else(|| not_found("no such music profile"))?;
    state.music.stop().await;
    state.profiles.db.music_remove_profile(&id).await.map_err(internal)?;
    Ok(StatusCode::NO_CONTENT)
}

// ---- home and search -------------------------------------------------------------------------

async fn home(State(state): State<AppState>) -> Result<Json<serde_json::Value>, ApiError> {
    let db = &state.profiles.db;
    let profile = active(&state).await?;
    let playlists = db.music_playlists(&profile.id).await.map_err(internal)?;
    let favorite_ids = db.music_favorite_ids(&profile.id).await.map_err(internal)?;
    let recent = db.music_recent(&profile.id, 20).await.map_err(internal)?;
    let recent = state.music.views(&favorite_ids, recent).await.map_err(internal)?;
    Ok(Json(json!({ "profile": profile, "playlists": playlists, "favorites": favorite_ids.len(), "recent": recent })))
}

#[derive(Deserialize)]
struct SearchParams {
    #[serde(default)]
    q: String,
    /// How many results to show (15, 30, … up to 150).
    count: Option<usize>,
}

/// Songs for a search, or the songs of a pasted YouTube playlist (or video) link. Every result is
/// saved as song metadata, so it can be added to a playlist by its id alone.
async fn search(
    State(state): State<AppState>,
    params: Result<Query<SearchParams>, QueryRejection>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let Query(params) = params.map_err(|_| bad("invalid query"))?;
    let text = music_yt::clean_query(&params.q).ok_or_else(|| bad("type something to search for"))?;
    let count = params.count.unwrap_or(SEARCH_STEP).clamp(1, music_yt::SEARCH_MAX);
    let upstream = |message: String| ApiError::new(StatusCode::BAD_GATEWAY, message);
    let (kind, title, tracks, more) = if music_yt::is_playlist_url(&text) {
        let (title, tracks) = music_yt::fetch_playlist(&text).await.map_err(upstream)?;
        ("playlist", Some(title), tracks, false)
    } else if let Some(id) = remote::video_id(&text).filter(|_| text.contains('/')) {
        let track = music_yt::fetch_video(&id).await.map_err(upstream)?;
        ("songs", None, vec![track], false)
    } else {
        let tracks = music_yt::search(&text, count).await.map_err(upstream)?;
        let more = count < music_yt::SEARCH_MAX && tracks.len() >= count;
        ("songs", None, tracks, more)
    };
    let db = &state.profiles.db;
    db.music_upsert_tracks(tracks.clone()).await.map_err(internal)?;
    let profile = active(&state).await?;
    let favorite_ids = db.music_favorite_ids(&profile.id).await.map_err(internal)?;
    let items = state.music.views(&favorite_ids, tracks).await.map_err(internal)?;
    Ok(Json(json!({ "kind": kind, "title": title, "items": items, "more": more })))
}

// ---- playlists -------------------------------------------------------------------------------

fn valid_playlist_name(name: &str) -> bool {
    let length = name.trim().chars().count();
    (1..=MAX_PLAYLIST_NAME).contains(&length) && !name.chars().any(char::is_control)
}

async fn owned_playlist(state: &AppState, id: i64) -> Result<crate::music_db::MusicPlaylist, ApiError> {
    let profile = active(state).await?;
    state.profiles.db.music_playlist(&profile.id, id).await.map_err(internal)?.ok_or_else(|| not_found("no such playlist"))
}

async fn list_playlists(State(state): State<AppState>) -> Result<Json<serde_json::Value>, ApiError> {
    let profile = active(&state).await?;
    let playlists = state.profiles.db.music_playlists(&profile.id).await.map_err(internal)?;
    Ok(Json(json!({ "playlists": playlists })))
}

#[derive(Deserialize)]
struct PlaylistName {
    name: String,
}

async fn create_playlist(
    State(state): State<AppState>,
    body: Result<Json<PlaylistName>, JsonRejection>,
) -> Result<(StatusCode, Json<crate::music_db::MusicPlaylist>), ApiError> {
    let request = json_body(body, "expected {name} JSON")?;
    if !valid_playlist_name(&request.name) {
        return Err(bad("a playlist name has 1–60 characters"));
    }
    let profile = active(&state).await?;
    let playlist = state.profiles.db.music_create_playlist(&profile.id, request.name.trim()).await.map_err(internal)?;
    Ok((StatusCode::CREATED, Json(playlist)))
}

async fn get_playlist(State(state): State<AppState>, Path(id): Path<i64>) -> Result<Json<serde_json::Value>, ApiError> {
    let playlist = owned_playlist(&state, id).await?;
    let profile = active(&state).await?;
    let tracks = state.profiles.db.music_playlist_tracks(id).await.map_err(internal)?;
    let favorite_ids = state.profiles.db.music_favorite_ids(&profile.id).await.map_err(internal)?;
    let items = state.music.views(&favorite_ids, tracks).await.map_err(internal)?;
    Ok(Json(json!({ "id": playlist.id, "name": playlist.name, "count": items.len(), "items": items })))
}

async fn rename_playlist(
    State(state): State<AppState>,
    Path(id): Path<i64>,
    body: Result<Json<PlaylistName>, JsonRejection>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let request = json_body(body, "expected {name} JSON")?;
    if !valid_playlist_name(&request.name) {
        return Err(bad("a playlist name has 1–60 characters"));
    }
    owned_playlist(&state, id).await?;
    state.profiles.db.music_rename_playlist(id, request.name.trim()).await.map_err(internal)?;
    Ok(Json(json!({ "id": id, "name": request.name.trim() })))
}

async fn delete_playlist(State(state): State<AppState>, Path(id): Path<i64>) -> Result<StatusCode, ApiError> {
    owned_playlist(&state, id).await?;
    state.profiles.db.music_delete_playlist(id).await.map_err(internal)?;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Deserialize)]
struct AddTracks {
    video_ids: Vec<String>,
    /// Start downloading the first songs at once (used when importing a whole playlist).
    #[serde(default)]
    import: bool,
}

/// Appends songs (they must come from a search) and starts their downloads in the background.
async fn add_tracks(
    State(state): State<AppState>,
    Path(id): Path<i64>,
    body: Result<Json<AddTracks>, JsonRejection>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let request = json_body(body, "expected {video_ids} JSON")?;
    if request.video_ids.is_empty() || request.video_ids.len() > MAX_SONGS {
        return Err(bad("add between 1 and 200 songs at a time"));
    }
    owned_playlist(&state, id).await?;
    let mut tracks = Vec::with_capacity(request.video_ids.len());
    for song in &request.video_ids {
        tracks.push(known_track(&state, song).await?);
    }
    state.profiles.db.music_playlist_add(id, tracks.clone()).await.map_err(internal)?;
    // One song downloads as soon as it is added; an imported list starts with its first few and
    // fetches the rest as it plays.
    let eager = if request.import { IMPORT_DOWNLOADS } else { tracks.len() };
    for track in tracks.iter().take(eager) {
        if track.local_file().is_none() {
            state.music.cache.start(track.video_id.clone());
        }
    }
    Ok(Json(json!({ "added": tracks.len() })))
}

async fn remove_track(State(state): State<AppState>, Path((id, position)): Path<(i64, usize)>) -> Result<StatusCode, ApiError> {
    owned_playlist(&state, id).await?;
    if state.profiles.db.music_playlist_remove(id, position).await.map_err(internal)? {
        Ok(StatusCode::NO_CONTENT)
    } else {
        Err(not_found("no song at that position"))
    }
}

#[derive(Deserialize)]
struct MoveTrack {
    from: usize,
    to: usize,
}

async fn move_track(
    State(state): State<AppState>,
    Path(id): Path<i64>,
    body: Result<Json<MoveTrack>, JsonRejection>,
) -> Result<StatusCode, ApiError> {
    let request = json_body(body, "expected {from, to} JSON")?;
    owned_playlist(&state, id).await?;
    if state.profiles.db.music_playlist_move(id, request.from, request.to).await.map_err(internal)? {
        Ok(StatusCode::NO_CONTENT)
    } else {
        Err(bad("position out of range"))
    }
}

// ---- favorites -------------------------------------------------------------------------------

async fn favorites(State(state): State<AppState>) -> Result<Json<serde_json::Value>, ApiError> {
    let profile = active(&state).await?;
    let tracks = state.profiles.db.music_favorites(&profile.id).await.map_err(internal)?;
    let ids = tracks.iter().map(|t| t.video_id.clone()).collect();
    let items = state.music.views(&ids, tracks).await.map_err(internal)?;
    Ok(Json(json!({ "items": items })))
}

async fn add_favorite(State(state): State<AppState>, Path(id): Path<String>) -> Result<StatusCode, ApiError> {
    let track = known_track(&state, &id).await?;
    let profile = active(&state).await?;
    state.profiles.db.music_set_favorite(&profile.id, track, true).await.map_err(internal)?;
    Ok(StatusCode::NO_CONTENT)
}

async fn remove_favorite(State(state): State<AppState>, Path(id): Path<String>) -> Result<StatusCode, ApiError> {
    video_id(&id)?;
    let profile = active(&state).await?;
    let track = state.profiles.db.music_track(&id).await.map_err(internal)?.ok_or_else(|| not_found("unknown song"))?;
    state.profiles.db.music_set_favorite(&profile.id, track, false).await.map_err(internal)?;
    Ok(StatusCode::NO_CONTENT)
}

// ---- playback --------------------------------------------------------------------------------

#[derive(Deserialize)]
struct PlayRequest {
    /// Play this playlist from `start` (a position in it).
    playlist_id: Option<i64>,
    #[serde(default)]
    favorites: bool,
    #[serde(default)]
    recent: bool,
    /// Play these songs (search results) from `start`.
    video_ids: Option<Vec<String>>,
    start: Option<usize>,
    /// Turns shuffle on or off (and remembers it); left out, the saved setting applies.
    shuffle: Option<bool>,
}

async fn play(
    State(state): State<AppState>,
    body: Result<Json<PlayRequest>, JsonRejection>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let request = json_body(body, "expected a playlist_id, favorites, recent or video_ids JSON")?;
    let db = &state.profiles.db;
    let profile = active(&state).await?;
    let tracks = match (request.playlist_id, request.favorites, request.recent, &request.video_ids) {
        (Some(id), false, false, None) => {
            owned_playlist(&state, id).await?;
            db.music_playlist_tracks(id).await.map_err(internal)?
        }
        (None, true, false, None) => db.music_favorites(&profile.id).await.map_err(internal)?,
        (None, false, true, None) => db.music_recent(&profile.id, 20).await.map_err(internal)?,
        (None, false, false, Some(ids)) => {
            if ids.is_empty() || ids.len() > MAX_SONGS {
                return Err(bad("play between 1 and 200 songs"));
            }
            let mut tracks = Vec::with_capacity(ids.len());
            for id in ids {
                tracks.push(known_track(&state, id).await?);
            }
            tracks
        }
        _ => return Err(bad("send exactly one of playlist_id, favorites, recent or video_ids")),
    };
    if tracks.is_empty() {
        return Err(bad("there is nothing to play yet"));
    }
    let start = request.start.unwrap_or(0);
    if start >= tracks.len() {
        return Err(bad("start is past the end of the list"));
    }
    let _transition = state.transition.lock().await;
    // A video from the other services must not play under the music.
    state.playback.stop().await.map_err(internal)?;
    state.music.start(tracks, start, request.shuffle).await?;
    drop(_transition);
    current_state(&state).await
}

#[derive(Deserialize)]
struct ControlRequest {
    action: String,
    value: Option<f64>,
    name: Option<String>,
}

/// One playback command, see `Music::control`.
async fn control(
    State(state): State<AppState>,
    body: Result<Json<ControlRequest>, JsonRejection>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let request = json_body(body, "expected {action, value?, name?} JSON")?;
    state.music.control(&request.action, request.value, request.name.as_deref()).await?;
    current_state(&state).await
}

async fn state(State(state): State<AppState>) -> Result<Json<serde_json::Value>, ApiError> {
    current_state(&state).await
}

/// The player bar's data: the current song, its progress and the player's settings.
pub async fn current_state(state: &AppState) -> Result<Json<serde_json::Value>, ApiError> {
    let snapshot = state.music.player.snapshot().await;
    let profile = active(state).await?;
    let favorite_ids = state.profiles.db.music_favorite_ids(&profile.id).await.map_err(internal)?;
    let track = snapshot.current.clone().map(|t| state.music.view(&favorite_ids, t));
    let next = snapshot.next.clone().map(|t| state.music.view(&favorite_ids, t));
    // The current song's download state can change while it plays.
    let track = match (track, &snapshot.current) {
        (Some(_), Some(current)) => {
            let fresh = state.profiles.db.music_track(&current.video_id).await.map_err(internal)?.unwrap_or_else(|| current.clone());
            Some(state.music.view(&favorite_ids, fresh))
        }
        (track, _) => track,
    };
    Ok(Json(json!({
        "playing": track.is_some(),
        "track": track,
        "next": next,
        "position": snapshot.position,
        "duration": snapshot.duration,
        "paused": snapshot.paused,
        "loaded": snapshot.loaded,
        "volume": snapshot.volume,
        "muted": snapshot.muted,
        "shuffle": snapshot.shuffle,
        "repeat": repeat_name(snapshot.repeat),
        "error": snapshot.error,
    })))
}

/// The current song and what follows, in play order.
async fn queue(State(state): State<AppState>) -> Result<Json<serde_json::Value>, ApiError> {
    let profile = active(&state).await?;
    let favorite_ids = state.profiles.db.music_favorite_ids(&profile.id).await.map_err(internal)?;
    let upcoming = state.music.player.upcoming().await;
    let items = state.music.views(&favorite_ids, upcoming).await.map_err(internal)?;
    Ok(Json(json!({ "items": items })))
}

// ---- lyrics ----------------------------------------------------------------------------------

/// Lyrics for a song: the saved copy, or a lookup (only title, artist and length are sent).
/// `synced` is a list of `[seconds, line]`.
async fn get_lyrics(State(state): State<AppState>, Path(id): Path<String>) -> Result<Json<serde_json::Value>, ApiError> {
    let track = known_track(&state, &id).await?;
    let db = &state.profiles.db;
    let stored = match db.music_lyrics(&id).await.map_err(internal)? {
        Some(stored) => stored,
        None => {
            let found = lyrics::fetch(track.title.clone(), track.artist.clone(), track.duration_secs).await.map_err(|message| {
                tracing::warn!("lyrics lookup failed: {message}");
                ApiError::new(StatusCode::BAD_GATEWAY, message)
            })?;
            let stored = found.map(|l| StoredLyrics { synced: l.synced, plain: l.plain }).unwrap_or_default();
            db.music_save_lyrics(&id, stored.clone()).await.map_err(internal)?;
            stored
        }
    };
    let lines = stored.synced.as_deref().map(lyrics::parse_lrc).unwrap_or_default();
    let found = !lines.is_empty() || stored.plain.is_some();
    Ok(Json(json!({
        "found": found,
        "synced": (!lines.is_empty()).then_some(lines),
        "plain": stored.plain,
    })))
}

// ---- downloads and settings ------------------------------------------------------------------

async fn start_download(State(state): State<AppState>, Path(id): Path<String>) -> Result<StatusCode, ApiError> {
    let track = known_track(&state, &id).await?;
    if track.local_file().is_none() {
        state.music.cache.start(id);
    }
    Ok(StatusCode::ACCEPTED)
}

async fn cancel_download(State(state): State<AppState>, Path(id): Path<String>) -> Result<StatusCode, ApiError> {
    video_id(&id)?;
    state.music.cache.cancel(&id);
    Ok(StatusCode::NO_CONTENT)
}

async fn clear_downloads(State(state): State<AppState>) -> Result<Json<serde_json::Value>, ApiError> {
    let removed = state.music.cache.clear().await.map_err(internal)?;
    Ok(Json(json!({ "removed": removed })))
}

async fn get_settings(State(state): State<AppState>) -> Result<Json<serde_json::Value>, ApiError> {
    let (bytes, files) = state.music.cache.disk_use();
    Ok(Json(json!({ "cache_limit": state.music.cache.limit(), "limits": LIMITS, "files": files, "bytes": bytes })))
}

#[derive(Deserialize)]
struct SettingsRequest {
    cache_limit: usize,
}

async fn set_settings(
    State(state): State<AppState>,
    body: Result<Json<SettingsRequest>, JsonRejection>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let request = json_body(body, "expected {cache_limit} JSON")?;
    if !LIMITS.contains(&request.cache_limit) {
        return Err(bad("cache_limit must be 25, 50, 100 or 200"));
    }
    state.music.cache.set_limit(request.cache_limit);
    state.profiles.db.set_music_global_setting("cache_limit", &request.cache_limit.to_string()).await.map_err(internal)?;
    state.music.cache.enforce_limit().await;
    get_settings(State(state)).await
}
