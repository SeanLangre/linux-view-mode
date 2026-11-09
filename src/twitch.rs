//! Twitch mode (see `docs/twitch-mode.md`): the saved mode, the separate Twitch profiles and
//! browsing. Twitch profiles are their own list: they share no ids, caches or settings with the
//! YouTube profiles, so removing one never touches the other.
use axum::{
    Json, Router,
    extract::{Path, Query, State, rejection::{JsonRejection, QueryRejection}},
    http::StatusCode,
    routing::{get, post},
};
use serde::Deserialize;
use serde_json::json;

use crate::{
    api::{ApiError, AppState, internal},
    db::TwitchProfile,
    gql::{self, Gql},
    helix::{self, Category, Channel, Helix, Page, Stream},
    profiles,
};

/// Where Twitch data comes from. The **unofficial** web API needs no registration and is the
/// default; setting `COUCHTUBE_TWITCH_CLIENT_ID` and `COUCHTUBE_TWITCH_CLIENT_SECRET` switches to
/// the **official** Helix API. The routes and the frontend are the same for both.
#[derive(Clone)]
pub enum Source {
    /// Browsing through Helix; the web API client is still kept for Following (the profile's login cookie).
    Official(Helix, Gql),
    Unofficial(Gql),
}

impl Source {
    pub fn from_env() -> Self {
        let official = Helix::from_env();
        if official.configured() { Self::Official(official, Gql::from_env()) } else { Self::Unofficial(Gql::from_env()) }
    }

    /// The web API client: Following and the sign-in check always use it, whichever source browses.
    pub fn web(&self) -> &Gql {
        match self {
            Self::Official(_, web) | Self::Unofficial(web) => web,
        }
    }

    fn name(&self) -> &'static str {
        match self {
            Self::Official(..) => "official",
            Self::Unofficial(_) => "unofficial",
        }
    }

    pub async fn top_categories(&self, cursor: Option<&str>, refresh: bool) -> anyhow::Result<Page<Category>> {
        match self {
            Self::Official(helix, _) => helix.top_categories(cursor, refresh).await,
            Self::Unofficial(gql) => gql.top_categories(cursor, refresh).await,
        }
    }

    pub async fn streams(&self, game_id: Option<&str>, cursor: Option<&str>, refresh: bool) -> anyhow::Result<Page<Stream>> {
        match self {
            Self::Official(helix, _) => helix.streams(game_id, cursor, refresh).await,
            Self::Unofficial(gql) => gql.streams(game_id, cursor, refresh).await,
        }
    }

    pub async fn search_channels(&self, text: &str, cursor: Option<&str>, refresh: bool) -> anyhow::Result<Page<Channel>> {
        match self {
            Self::Official(helix, _) => helix.search_channels(text, cursor, refresh).await,
            // The unofficial search has no paging.
            Self::Unofficial(gql) => gql.search_channels(text, refresh).await,
        }
    }
}

pub const MODES: [&str; 2] = ["youtube", "twitch"];

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/mode", get(get_mode).put(set_mode))
        .route("/twitch/profiles", get(list_profiles).post(create_profile))
        .route("/twitch/profiles/{id}", axum::routing::delete(remove_profile))
        .route("/twitch/profiles/{id}/select", post(select_profile))
        .route("/twitch/profiles/{id}/signin", post(signin))
        .route("/twitch/profiles/{id}/status", get(login_status))
        .route("/twitch/following", get(following))
        .route("/twitch/status", get(status))
        .route("/twitch/play", post(play))
        .route("/twitch/channel/{login}", get(channel))
        .route("/twitch/categories", get(categories))
        .route("/twitch/streams", get(streams))
        .route("/twitch/search", get(search))
}

/// `{source}`: `unofficial` (Twitch's web API, no registration) or `official` (Helix, with a
/// registered application).
async fn status(State(state): State<AppState>) -> Json<serde_json::Value> {
    Json(json!({ "source": state.twitch.name() }))
}

/// What to play: a live channel (`login`), a past broadcast (`video`, digits, with an optional
/// `start` in seconds; the active profile's saved position is used otherwise) or a `clip` slug.
#[derive(Deserialize)]
struct PlayRequest {
    login: Option<String>,
    video: Option<String>,
    clip: Option<String>,
    start: Option<f64>,
}

/// Starts a channel, broadcast or clip in the CouchTube window (the page named in `/api/status`).
async fn play(
    State(state): State<AppState>,
    body: Result<Json<PlayRequest>, JsonRejection>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let Json(request) = body.map_err(|_| ApiError::new(StatusCode::BAD_REQUEST, "expected {login}, {video} or {clip} JSON"))?;
    let bad = |what| ApiError::new(StatusCode::BAD_REQUEST, what);
    let _transition = state.transition.lock().await;
    let started = match (&request.login, &request.video, &request.clip) {
        (Some(login), None, None) => {
            if !helix::is_login(login) {
                return Err(bad("invalid channel name"));
            }
            state.playback.play_twitch(login).await
        }
        (None, Some(video), None) => {
            if !helix::is_id(video) {
                return Err(bad("invalid video id"));
            }
            if request.start.is_some_and(|s| !s.is_finite() || !(0.0..=1_000_000.0).contains(&s)) {
                return Err(bad("invalid start position"));
            }
            let profile = state.profiles.db.twitch_active().await.map_err(internal)?;
            state.playback.play_twitch_video(&profile.id, video, request.start).await
        }
        (None, None, Some(clip)) => {
            if !gql::is_slug(clip) {
                return Err(bad("invalid clip"));
            }
            state.playback.play_twitch_clip(clip).await
        }
        _ => return Err(bad("send exactly one of login, video or clip")),
    };
    started.map_err(|err| {
        tracing::error!("could not start Twitch playback: {err:#}");
        ApiError::new(StatusCode::BAD_GATEWAY, "could not start the player, check the server log")
    })?;
    Ok(Json(json!({ "status": "starting" })))
}

#[derive(Deserialize)]
struct ChannelQuery {
    /// `vods` (default) or `clips`.
    kind: Option<String>,
    cursor: Option<String>,
    #[serde(default)]
    refresh: bool,
}

/// A channel's live stream (if on), and its past broadcasts or clips. Uses Twitch's web API
/// whichever source browses, like Following. Broadcasts carry the active profile's resume state.
async fn channel(
    State(state): State<AppState>,
    Path(login): Path<String>,
    params: Result<Query<ChannelQuery>, QueryRejection>,
) -> Result<Json<gql::ChannelPage>, ApiError> {
    let Query(params) = params.map_err(|_| ApiError::new(StatusCode::BAD_REQUEST, "invalid query"))?;
    if !helix::is_login(&login) {
        return Err(ApiError::new(StatusCode::BAD_REQUEST, "invalid channel name"));
    }
    if params.cursor.as_deref().is_some_and(|c| !helix::valid_cursor(c)) {
        return Err(ApiError::new(StatusCode::BAD_REQUEST, "invalid cursor"));
    }
    let clips = match params.kind.as_deref() {
        None | Some("vods") => false,
        Some("clips") => true,
        Some(_) => return Err(ApiError::new(StatusCode::BAD_REQUEST, "kind is vods or clips")),
    };
    let mut page = state
        .twitch
        .web()
        .channel(&login, clips, params.cursor.as_deref(), params.refresh)
        .await
        .map_err(upstream("channel"))?
        .ok_or_else(|| ApiError::new(StatusCode::NOT_FOUND, "no such channel"))?;
    if !clips && !page.items.is_empty() {
        let profile = state.profiles.db.twitch_active().await.map_err(internal)?;
        let ids = page.items.iter().map(|v| v.id.clone()).collect();
        let saved = state.profiles.db.twitch_progress_for(&profile.id, ids).await.map_err(internal)?;
        for item in &mut page.items {
            if let Some(&(position, duration, completed)) = saved.get(&item.id) {
                item.completed = completed;
                if !completed && position >= 5.0 && duration.is_none_or(|d| position < d - 5.0) {
                    item.resume_position = Some(position);
                }
            }
        }
    }
    Ok(Json(page))
}

#[derive(Deserialize)]
struct Browse {
    game_id: Option<String>,
    cursor: Option<String>,
    q: Option<String>,
    #[serde(default)]
    refresh: bool,
}

fn browse(params: Result<Query<Browse>, QueryRejection>) -> Result<Browse, ApiError> {
    let Query(params) = params.map_err(|_| ApiError::new(StatusCode::BAD_REQUEST, "invalid query"))?;
    if params.cursor.as_deref().is_some_and(|c| !helix::valid_cursor(c)) {
        return Err(ApiError::new(StatusCode::BAD_REQUEST, "invalid cursor"));
    }
    if params.game_id.as_deref().is_some_and(|g| !helix::is_id(g)) {
        return Err(ApiError::new(StatusCode::BAD_REQUEST, "invalid category id"));
    }
    Ok(params)
}

fn upstream(what: &'static str) -> impl FnOnce(anyhow::Error) -> ApiError {
    move |err| {
        tracing::error!("Twitch {what} failed: {err:#}");
        ApiError::new(StatusCode::BAD_GATEWAY, "could not reach Twitch, try again")
    }
}

async fn categories(
    State(state): State<AppState>,
    params: Result<Query<Browse>, QueryRejection>,
) -> Result<Json<Page<Category>>, ApiError> {
    let params = browse(params)?;
    let page = state.twitch.top_categories(params.cursor.as_deref(), params.refresh).await.map_err(upstream("categories"))?;
    Ok(Json(page))
}

/// Live streams: of one category (`game_id`), or the most watched on all of Twitch.
async fn streams(
    State(state): State<AppState>,
    params: Result<Query<Browse>, QueryRejection>,
) -> Result<Json<Page<Stream>>, ApiError> {
    let params = browse(params)?;
    let page = state
        .twitch
        .streams(params.game_id.as_deref(), params.cursor.as_deref(), params.refresh)
        .await
        .map_err(upstream("streams"))?;
    Ok(Json(page))
}

async fn search(
    State(state): State<AppState>,
    params: Result<Query<Browse>, QueryRejection>,
) -> Result<Json<Page<Channel>>, ApiError> {
    let params = browse(params)?;
    let text = params
        .q
        .as_deref()
        .and_then(helix::clean_query)
        .ok_or_else(|| ApiError::new(StatusCode::BAD_REQUEST, "type 1-100 characters to search for"))?;
    let page = state.twitch.search_channels(&text, params.cursor.as_deref(), params.refresh).await.map_err(upstream("search"))?;
    Ok(Json(page))
}

async fn get_mode(State(state): State<AppState>) -> Result<Json<serde_json::Value>, ApiError> {
    let mode = state.profiles.db.mode().await.map_err(internal)?;
    Ok(Json(json!({ "mode": mode })))
}

#[derive(Deserialize)]
struct ModeRequest {
    mode: String,
}

/// Choosing a service stops whatever is playing: the two services use different player pages.
async fn set_mode(
    State(state): State<AppState>,
    body: Result<Json<ModeRequest>, JsonRejection>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let Json(request) = body.map_err(|_| ApiError::new(StatusCode::BAD_REQUEST, "expected {mode} JSON"))?;
    if !MODES.contains(&request.mode.as_str()) {
        return Err(ApiError::new(StatusCode::BAD_REQUEST, "mode must be youtube or twitch"));
    }
    let _transition = state.transition.lock().await;
    state.playback.stop().await.map_err(internal)?;
    state.profiles.db.set_mode(&request.mode).await.map_err(internal)?;
    Ok(Json(json!({ "mode": request.mode })))
}

async fn list_profiles(State(state): State<AppState>) -> Result<Json<serde_json::Value>, ApiError> {
    let _transition = state.transition.lock().await;
    let list = state.profiles.db.twitch_list().await.map_err(internal)?;
    let active = state.profiles.db.twitch_active().await.map_err(internal)?;
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
) -> Result<(StatusCode, Json<TwitchProfile>), ApiError> {
    let Json(request) =
        body.map_err(|_| ApiError::new(StatusCode::BAD_REQUEST, "expected {name, color?} JSON"))?;
    if !profiles::valid_name(&request.name) {
        return Err(ApiError::new(StatusCode::BAD_REQUEST, "name must contain 1–40 characters without control characters"));
    }
    let color = request.color.as_deref().unwrap_or("#9146ff");
    if !profiles::valid_color(color) {
        return Err(ApiError::new(StatusCode::BAD_REQUEST, "color must be a six-digit hex color"));
    }
    let _transition = state.transition.lock().await;
    let profile = state.profiles.twitch_create(request.name.trim(), color).await.map_err(internal)?;
    Ok((StatusCode::CREATED, Json(profile)))
}

async fn select_profile(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<TwitchProfile>, ApiError> {
    let _transition = state.transition.lock().await;
    let profile = require_profile(&state, &id).await?;
    if state.profiles.twitch_login_status(&id).await.busy() {
        return Err(ApiError::new(StatusCode::CONFLICT, "close the sign-in window and wait for the login check first"));
    }
    state.playback.stop().await.map_err(internal)?;
    state.profiles.db.twitch_select(&id).await.map_err(internal)?;
    Ok(Json(profile))
}

async fn remove_profile(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<StatusCode, ApiError> {
    if id == "guest" {
        return Err(ApiError::new(StatusCode::BAD_REQUEST, "Guest cannot be removed"));
    }
    let _transition = state.transition.lock().await;
    require_profile(&state, &id).await?;
    if state.profiles.twitch_login_status(&id).await.busy() {
        return Err(ApiError::new(StatusCode::CONFLICT, "close the sign-in window and wait for the login check first"));
    }
    state.playback.stop().await.map_err(internal)?;
    state.profiles.twitch_remove(&id, state.twitch.web()).await.map_err(internal)?;
    Ok(StatusCode::NO_CONTENT)
}

/// Opens Firefox on Twitch's login page for this profile. The phone keyboard works as on YouTube.
async fn signin(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<crate::profiles::LoginStatus>, ApiError> {
    let _transition = state.transition.lock().await;
    let profile = require_profile(&state, &id).await?;
    if id == "guest" {
        return Err(ApiError::new(StatusCode::BAD_REQUEST, "Guest stays signed out"));
    }
    if state.profiles.twitch_login_status(&id).await.busy() {
        return Err(ApiError::new(StatusCode::CONFLICT, "sign-in is already open or being checked"));
    }
    state.playback.stop().await.map_err(internal)?;
    state.twitch.web().forget(&id);
    state.profiles.twitch_signin(profile, state.twitch.web().clone()).await.map_err(|err| {
        tracing::error!("could not open the Twitch sign-in: {err:#}");
        ApiError::new(StatusCode::BAD_GATEWAY, "could not open Firefox, is it installed?")
    })?;
    Ok(Json(state.profiles.twitch_login_status(&id).await))
}

#[derive(Deserialize)]
struct LoginQuery {
    #[serde(default)]
    check: bool,
}

/// The sign-in state; `?check=true` asks Twitch who the saved login belongs to.
async fn login_status(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Query(query): Query<LoginQuery>,
) -> Result<Json<crate::profiles::LoginStatus>, ApiError> {
    let _transition = state.transition.lock().await;
    require_profile(&state, &id).await?;
    let status = state.profiles.twitch_login_status(&id).await;
    if query.check && id != "guest" && !status.busy() {
        state.profiles.twitch_start_check(&id, state.twitch.web().clone()).await;
    }
    Ok(Json(state.profiles.twitch_login_status(&id).await))
}

/// The channels the active profile follows that are live now. Needs a signed-in profile.
async fn following(
    State(state): State<AppState>,
    params: Result<Query<Browse>, QueryRejection>,
) -> Result<Json<Page<Stream>>, ApiError> {
    let Query(params) = params.map_err(|_| ApiError::new(StatusCode::BAD_REQUEST, "invalid query"))?;
    let profile = {
        let _transition = state.transition.lock().await;
        state.profiles.db.twitch_active().await.map_err(internal)?
    };
    let Some(token) = state.profiles.twitch_token(&profile.id).await else {
        return Err(ApiError::new(
            StatusCode::CONFLICT,
            if profile.id == "guest" {
                "Sign in to Twitch with a profile (not Guest) to see who you follow"
            } else {
                "This profile is not signed in to Twitch yet: press S on it in the profile list"
            },
        ));
    };
    match state.twitch.web().followed_streams(&profile.id, &token, params.refresh).await {
        Ok(items) => Ok(Json(Page { items, cursor: None })),
        Err(err) if err.downcast_ref::<gql::SignedOut>().is_some() => Err(ApiError::new(
            StatusCode::CONFLICT,
            "Twitch did not accept the saved sign-in. Sign in again from the profile screen",
        )),
        Err(err) => Err(upstream("following")(err)),
    }
}

async fn require_profile(state: &AppState, id: &str) -> Result<TwitchProfile, ApiError> {
    state
        .profiles
        .db
        .twitch_get(id)
        .await
        .map_err(internal)?
        .ok_or_else(|| ApiError::new(StatusCode::NOT_FOUND, "no such Twitch profile"))
}

#[cfg(test)]
mod tests {
    use crate::db::Database;

    fn temp_db(name: &str) -> (Database, std::path::PathBuf) {
        let path = std::env::temp_dir().join(format!("ct-twitch-{name}-{}.db", std::process::id()));
        let _ = std::fs::remove_file(&path);
        (Database::open(&path).unwrap(), path)
    }

    #[tokio::test]
    async fn twitch_profiles_are_a_separate_list_with_their_own_guest_and_active_profile() {
        let (db, path) = temp_db("profiles");
        assert_eq!(db.twitch_list().await.unwrap().len(), 1);
        assert_eq!(db.twitch_active().await.unwrap().id, "guest");
        let alice = db.twitch_create("Alice", "#aabbcc").await.unwrap();
        // Neither list sees the other's profiles.
        assert!(db.get(&alice.id).await.unwrap().is_none());
        let youtube = db.create("Bob", "#ddeeff").await.unwrap();
        assert!(db.twitch_get(&youtube.id).await.unwrap().is_none());
        // Each service remembers its own active profile.
        db.twitch_select(&alice.id).await.unwrap();
        assert_eq!(db.twitch_active().await.unwrap().id, alice.id);
        assert_eq!(db.active().await.unwrap().id, "guest");
        db.select(&youtube.id).await.unwrap();
        assert_eq!(db.twitch_active().await.unwrap().id, alice.id);
        // Removing the active Twitch profile falls back to its Guest and leaves YouTube alone.
        db.twitch_remove(&alice.id).await.unwrap();
        assert_eq!(db.twitch_active().await.unwrap().id, "guest");
        assert_eq!(db.active().await.unwrap().id, youtube.id);
        assert!(db.twitch_remove("guest").await.is_err());
        assert!(db.twitch_select("nope").await.is_err());
        drop(db);
        std::fs::remove_file(path).unwrap();
    }

    #[tokio::test]
    async fn broadcast_positions_are_kept_per_twitch_profile_and_removed_with_it() {
        let (db, path) = temp_db("progress");
        let alice = db.twitch_create("Alice", "#aabbcc").await.unwrap();
        db.twitch_save_progress(&alice.id, "111", 600.0, Some(7200.0), false).await.unwrap();
        assert_eq!(db.twitch_resume(&alice.id, "111").await.unwrap(), Some(600.0));
        assert_eq!(db.twitch_resume("guest", "111").await.unwrap(), None, "another profile has its own place");
        // Too early, nearly at the end and finished broadcasts start from the beginning.
        db.twitch_save_progress(&alice.id, "111", 3.0, Some(7200.0), false).await.unwrap();
        assert_eq!(db.twitch_resume(&alice.id, "111").await.unwrap(), None);
        db.twitch_save_progress(&alice.id, "111", 7198.0, Some(7200.0), false).await.unwrap();
        assert_eq!(db.twitch_resume(&alice.id, "111").await.unwrap(), None);
        db.twitch_save_progress(&alice.id, "111", 900.0, Some(7200.0), true).await.unwrap();
        assert_eq!(db.twitch_resume(&alice.id, "111").await.unwrap(), None);
        let saved = db.twitch_progress_for(&alice.id, vec!["111".into(), "222".into()]).await.unwrap();
        assert_eq!(saved.len(), 1);
        assert!(saved["111"].2);
        db.twitch_remove(&alice.id).await.unwrap();
        assert!(db.twitch_progress_for(&alice.id, vec!["111".into()]).await.unwrap().is_empty());
        drop(db);
        std::fs::remove_file(path).unwrap();
    }
}
