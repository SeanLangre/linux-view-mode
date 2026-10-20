//! HTTP handlers under `/api`.

use axum::{
    Json, Router,
    extract::{Path, Query, State, rejection::JsonRejection},
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::{get, post},
};
use serde::Deserialize;
use serde_json::json;

use crate::{
    db::Profile,
    profiles::{self, Profiles},
};
use std::sync::Arc;
use tokio::sync::Mutex;

#[derive(Clone)]
pub struct AppState {
    pub player: Player,
    pub profiles: Profiles,
    pub transition: Arc<Mutex<()>>,
}

use crate::player::{Player, Status};
use crate::ytdlp::{self, Channel, Video};

const DEFAULT_LIMIT: u32 = 24;
const MAX_LIMIT: u32 = 50;

pub fn router(state: AppState) -> Router {
    Router::new()
        .route("/health", get(health))
        .route("/profiles", get(list_profiles).post(create_profile))
        .route("/profiles/{id}", axum::routing::delete(remove_profile))
        .route("/profiles/{id}/select", post(select_profile))
        .route("/profiles/{id}/signin", post(signin))
        .route("/profiles/{id}/status", get(login_status))
        .route("/play", post(play))
        .route("/stop", post(stop))
        .route("/status", get(status))
        .route("/search", get(search))
        .route("/channel/{id}", get(channel))
        // Without this, unknown /api paths would fall through to the frontend's index.html.
        .fallback(|| async { ApiError::new(StatusCode::NOT_FOUND, "no such endpoint") })
        .with_state(state)
}

async fn health() -> Json<serde_json::Value> {
    Json(json!({ "status": "ok" }))
}

#[derive(Deserialize)]
struct SearchParams {
    #[serde(default)]
    q: String,
    limit: Option<u32>,
}

async fn search(
    State(state): State<AppState>,
    Query(params): Query<SearchParams>,
) -> Result<Json<Vec<Video>>, ApiError> {
    let query = ytdlp::clean_query(&params.q)
        .ok_or_else(|| ApiError::new(StatusCode::BAD_REQUEST, "empty search query"))?;
    let limit = params.limit.unwrap_or(DEFAULT_LIMIT).clamp(1, MAX_LIMIT);

    let account = {
        let _transition = state.transition.lock().await;
        state.profiles.active().await.map_err(internal)?
    };
    let _request = account.gate.lock().await;
    ytdlp::search(&query, limit, account.cookies())
        .await
        .map(Json)
        .map_err(|err| {
            tracing::error!("search for {query:?} failed: {err:#}");
            ApiError::new(StatusCode::BAD_GATEWAY, "search failed, try again")
        })
}

async fn channel(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<Channel>, ApiError> {
    if !ytdlp::is_channel_id(&id) {
        return Err(ApiError::new(StatusCode::BAD_REQUEST, "invalid channel id"));
    }

    let account = {
        let _transition = state.transition.lock().await;
        state.profiles.active().await.map_err(internal)?
    };
    let _request = account.gate.lock().await;
    ytdlp::channel(&id, DEFAULT_LIMIT, account.cookies())
        .await
        .map(Json)
        .map_err(|err| {
            tracing::error!("channel {id} failed: {err:#}");
            ApiError::new(StatusCode::BAD_GATEWAY, "could not load channel, try again")
        })
}

/// An error sent to the frontend as `{"error": "..."}`.
pub struct ApiError {
    status: StatusCode,
    message: &'static str,
}

impl ApiError {
    fn new(status: StatusCode, message: &'static str) -> Self {
        Self { status, message }
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        (self.status, Json(json!({ "error": self.message }))).into_response()
    }
}

#[derive(Deserialize)]
struct PlayRequest {
    id: String,
    start: Option<f64>,
}

async fn play(
    State(state): State<AppState>,
    body: Result<Json<PlayRequest>, JsonRejection>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let Json(request) =
        body.map_err(|_| ApiError::new(StatusCode::BAD_REQUEST, "expected {id, start?} JSON"))?;
    if !ytdlp::is_video_id(&request.id) {
        return Err(ApiError::new(StatusCode::BAD_REQUEST, "invalid video id"));
    }
    if request
        .start
        .is_some_and(|start| !start.is_finite() || start < 0.0)
    {
        return Err(ApiError::new(
            StatusCode::BAD_REQUEST,
            "start must be a finite, non-negative number",
        ));
    }
    let _transition = state.transition.lock().await;
    let account = state.profiles.active().await.map_err(internal)?;
    if state
        .profiles
        .login_status(&account.profile.id)
        .await
        .busy()
    {
        return Err(ApiError::new(
            StatusCode::CONFLICT,
            "close the sign-in window and wait for the login check first",
        ));
    }
    state
        .player
        .play(&request.id, request.start, account.cookies())
        .await
        .map_err(|err| {
            tracing::error!("could not start playback: {err:#}");
            ApiError::new(
                StatusCode::BAD_GATEWAY,
                "could not start mpv, check the server log",
            )
        })?;
    Ok(Json(json!({"status": "starting"})))
}

async fn stop(State(state): State<AppState>) -> Result<Json<serde_json::Value>, ApiError> {
    state.player.stop().await.map_err(|err| {
        tracing::error!("could not stop mpv: {err:#}");
        ApiError::new(StatusCode::INTERNAL_SERVER_ERROR, "could not stop playback")
    })?;
    Ok(Json(json!({"status": "stopped"})))
}

async fn status(State(state): State<AppState>) -> Result<Json<Status>, ApiError> {
    state.player.status().await.map(Json).map_err(|err| {
        tracing::error!("could not read mpv status: {err:#}");
        ApiError::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            "could not read playback status",
        )
    })
}

fn internal(err: anyhow::Error) -> ApiError {
    tracing::error!("profile operation failed: {err:#}");
    ApiError::new(
        StatusCode::INTERNAL_SERVER_ERROR,
        "could not update profiles, check the server log",
    )
}

async fn require_profile(state: &AppState, id: &str) -> Result<crate::profiles::Account, ApiError> {
    state
        .profiles
        .account(id)
        .await
        .map_err(internal)?
        .ok_or_else(|| ApiError::new(StatusCode::NOT_FOUND, "no such profile"))
}

async fn list_profiles(State(state): State<AppState>) -> Result<Json<serde_json::Value>, ApiError> {
    let _transition = state.transition.lock().await;
    let profiles = state.profiles.db.list().await.map_err(internal)?;
    let active = state.profiles.db.active().await.map_err(internal)?;
    Ok(Json(json!({"profiles": profiles, "active_id": active.id})))
}

#[derive(Deserialize)]
struct NewProfile {
    name: String,
    color: Option<String>,
}

async fn create_profile(
    State(state): State<AppState>,
    body: Result<Json<NewProfile>, JsonRejection>,
) -> Result<(StatusCode, Json<Profile>), ApiError> {
    let Json(request) =
        body.map_err(|_| ApiError::new(StatusCode::BAD_REQUEST, "expected {name, color?} JSON"))?;
    if !profiles::valid_name(&request.name) {
        return Err(ApiError::new(
            StatusCode::BAD_REQUEST,
            "name must contain 1–40 characters without control characters",
        ));
    }
    let color = request.color.as_deref().unwrap_or("#6288d9");
    if !profiles::valid_color(color) {
        return Err(ApiError::new(
            StatusCode::BAD_REQUEST,
            "color must be a six-digit hex color",
        ));
    }
    let _transition = state.transition.lock().await;
    let profile = state
        .profiles
        .create(request.name.trim(), color)
        .await
        .map_err(internal)?;
    Ok((StatusCode::CREATED, Json(profile)))
}

async fn select_profile(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<Profile>, ApiError> {
    let _transition = state.transition.lock().await;
    let account = require_profile(&state, &id).await?;
    if state.profiles.login_status(&id).await.busy() {
        return Err(ApiError::new(
            StatusCode::CONFLICT,
            "close the sign-in window and wait for the login check first",
        ));
    }
    state.player.stop().await.map_err(internal)?;
    state.profiles.db.select(&id).await.map_err(internal)?;
    Ok(Json(account.profile))
}

async fn remove_profile(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let _transition = state.transition.lock().await;
    if id == "guest" {
        return Err(ApiError::new(
            StatusCode::BAD_REQUEST,
            "Guest cannot be removed",
        ));
    }
    let account = require_profile(&state, &id).await?;
    if state.profiles.login_status(&id).await.busy() {
        return Err(ApiError::new(
            StatusCode::CONFLICT,
            "close the sign-in window and wait for the login check first",
        ));
    }
    if state.profiles.db.active().await.map_err(internal)?.id == id {
        state.player.stop().await.map_err(internal)?;
    }
    state.profiles.remove(&account).await.map_err(internal)?;
    Ok(Json(json!({"status": "removed"})))
}

async fn signin(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<crate::profiles::LoginStatus>, ApiError> {
    let _transition = state.transition.lock().await;
    let account = require_profile(&state, &id).await?;
    if id == "guest" {
        return Err(ApiError::new(
            StatusCode::BAD_REQUEST,
            "Guest stays signed out",
        ));
    }
    if state.profiles.login_status(&id).await.busy() {
        return Err(ApiError::new(
            StatusCode::CONFLICT,
            "sign-in is already open or being checked",
        ));
    }
    // Don't rotate cookies while this profile is playing a video.
    if state.profiles.db.active().await.map_err(internal)?.id == id {
        state.player.stop().await.map_err(internal)?;
    }
    state.profiles.signin(account).await.map_err(|_| {
        ApiError::new(
            StatusCode::BAD_GATEWAY,
            "could not open Firefox, is it installed?",
        )
    })?;
    Ok(Json(state.profiles.login_status(&id).await))
}

#[derive(Deserialize)]
struct LoginQuery {
    #[serde(default)]
    check: bool,
}

async fn login_status(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Query(query): Query<LoginQuery>,
) -> Result<Json<crate::profiles::LoginStatus>, ApiError> {
    let _transition = state.transition.lock().await;
    let account = require_profile(&state, &id).await?;
    let status = state.profiles.login_status(&id).await;
    if query.check && id != "guest" && !status.busy() {
        state.profiles.start_check(account).await;
    }
    Ok(Json(state.profiles.login_status(&id).await))
}
