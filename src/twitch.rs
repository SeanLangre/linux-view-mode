//! Twitch mode (see `docs/twitch-mode.md`). So far: the saved mode and the separate Twitch
//! profiles. Twitch profiles are their own list: they share no ids, caches or settings with the
//! YouTube profiles, so removing one never touches the other.
use axum::{
    Json, Router,
    extract::{Path, State, rejection::JsonRejection},
    http::StatusCode,
    routing::{get, post},
};
use serde::Deserialize;
use serde_json::json;

use crate::{
    api::{ApiError, AppState, internal},
    db::TwitchProfile,
    profiles,
};

pub const MODES: [&str; 2] = ["youtube", "twitch"];

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/mode", get(get_mode).put(set_mode))
        .route("/twitch/profiles", get(list_profiles).post(create_profile))
        .route("/twitch/profiles/{id}", axum::routing::delete(remove_profile))
        .route("/twitch/profiles/{id}/select", post(select_profile))
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
    let profile = state.profiles.db.twitch_create(request.name.trim(), color).await.map_err(internal)?;
    Ok((StatusCode::CREATED, Json(profile)))
}

async fn select_profile(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<TwitchProfile>, ApiError> {
    let _transition = state.transition.lock().await;
    let profile = require_profile(&state, &id).await?;
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
    state.playback.stop().await.map_err(internal)?;
    state.profiles.db.twitch_remove(&id).await.map_err(internal)?;
    Ok(StatusCode::NO_CONTENT)
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
    async fn mode_is_empty_until_chosen_then_remembered() {
        let (db, path) = temp_db("mode");
        assert_eq!(db.mode().await.unwrap(), None);
        db.set_mode("twitch").await.unwrap();
        db.set_mode("youtube").await.unwrap();
        assert_eq!(db.mode().await.unwrap().as_deref(), Some("youtube"));
        drop(db);
        std::fs::remove_file(path).unwrap();
    }
}
