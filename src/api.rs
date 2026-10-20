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

use crate::player::{Player, Status};
use crate::ytdlp::{self, Channel, Video};

const DEFAULT_LIMIT: u32 = 24;
const MAX_LIMIT: u32 = 50;

pub fn router(player: Player) -> Router {
    Router::new()
        .route("/health", get(health))
        .route("/play", post(play))
        .route("/stop", post(stop))
        .route("/status", get(status))
        .route("/search", get(search))
        .route("/channel/{id}", get(channel))
        // Without this, unknown /api paths would fall through to the frontend's index.html.
        .fallback(|| async { ApiError::new(StatusCode::NOT_FOUND, "no such endpoint") })
        .with_state(player)
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

async fn search(Query(params): Query<SearchParams>) -> Result<Json<Vec<Video>>, ApiError> {
    let query = ytdlp::clean_query(&params.q)
        .ok_or_else(|| ApiError::new(StatusCode::BAD_REQUEST, "empty search query"))?;
    let limit = params.limit.unwrap_or(DEFAULT_LIMIT).clamp(1, MAX_LIMIT);

    ytdlp::search(&query, limit).await.map(Json).map_err(|err| {
        tracing::error!("search for {query:?} failed: {err:#}");
        ApiError::new(StatusCode::BAD_GATEWAY, "search failed, try again")
    })
}

async fn channel(Path(id): Path<String>) -> Result<Json<Channel>, ApiError> {
    if !ytdlp::is_channel_id(&id) {
        return Err(ApiError::new(StatusCode::BAD_REQUEST, "invalid channel id"));
    }

    ytdlp::channel(&id, DEFAULT_LIMIT)
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
    State(player): State<Player>,
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
    player
        .play(&request.id, request.start)
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

async fn stop(State(player): State<Player>) -> Result<Json<serde_json::Value>, ApiError> {
    player.stop().await.map_err(|err| {
        tracing::error!("could not stop mpv: {err:#}");
        ApiError::new(StatusCode::INTERNAL_SERVER_ERROR, "could not stop playback")
    })?;
    Ok(Json(json!({"status": "stopped"})))
}

async fn status(State(player): State<Player>) -> Result<Json<Status>, ApiError> {
    player.status().await.map(Json).map_err(|err| {
        tracing::error!("could not read mpv status: {err:#}");
        ApiError::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            "could not read playback status",
        )
    })
}
