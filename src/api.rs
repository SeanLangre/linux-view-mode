//! HTTP handlers under `/api`.

use axum::{
    Json, Router,
    extract::Query,
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::get,
};
use serde::Deserialize;
use serde_json::json;

use crate::ytdlp::{self, Video};

const DEFAULT_LIMIT: u32 = 24;
const MAX_LIMIT: u32 = 50;

pub fn router() -> Router {
    Router::new()
        .route("/health", get(health))
        .route("/search", get(search))
        // Without this, unknown /api paths would fall through to the frontend's index.html.
        .fallback(|| async { ApiError::new(StatusCode::NOT_FOUND, "no such endpoint") })
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
