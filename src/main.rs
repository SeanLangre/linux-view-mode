mod api;
mod db;
mod feeds;
mod pairing;
mod playback;
mod player;
mod profiles;
mod ytdlp;

use std::net::{Ipv4Addr, SocketAddr};
use std::path::PathBuf;

use anyhow::{Context, Result};
use axum::Router;
use tower_http::services::{ServeDir, ServeFile};
use tower_http::trace::TraceLayer;
use tracing_subscriber::EnvFilter;

const DEFAULT_PORT: u16 = 7878;
const DEFAULT_PAIR_PORT: u16 = 7879;

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| "couchtube=info".into()),
        )
        .init();

    let port = match std::env::var("COUCHTUBE_PORT") {
        Ok(p) => p.parse().context("COUCHTUBE_PORT must be a port number")?,
        Err(_) => DEFAULT_PORT,
    };
    let frontend_dir: PathBuf = std::env::var_os("COUCHTUBE_FRONTEND")
        .map(Into::into)
        .unwrap_or_else(|| "frontend/dist".into());

    // Unknown paths get index.html so the frontend can handle its own routes.
    let frontend =
        ServeDir::new(&frontend_dir).fallback(ServeFile::new(frontend_dir.join("index.html")));

    let player_config = std::env::var_os("COUCHTUBE_MPV_CONFIG")
        .map(PathBuf::from)
        .unwrap_or_else(|| "config/mpv".into());
    let player = player::Player::new(player_config)?;
    let data_dir = match std::env::var_os("COUCHTUBE_DATA_DIR") {
        Some(path) => PathBuf::from(path),
        None => std::env::var_os("XDG_DATA_HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|| {
                PathBuf::from(std::env::var_os("HOME").expect("HOME must be set"))
                    .join(".local/share")
            })
            .join("couchtube"),
    };
    let pair_port = match std::env::var("COUCHTUBE_PAIR_PORT") {
        Ok(p) => p.parse().context("COUCHTUBE_PAIR_PORT must be a port number")?,
        Err(_) => DEFAULT_PAIR_PORT,
    };
    let pairing = pairing::Pairing::new(pair_port);
    let profiles = profiles::Profiles::open(data_dir, pairing.clone(), port)?;
    let playback = playback::Playback::new(player.clone(), profiles.db.clone());
    let state = api::AppState {
        playback: playback.clone(),
        feeds: feeds::Feeds(profiles.db.clone()),
        profiles,
        transition: Default::default(),
    };
    let tracked = state.clone();
    let monitor = tokio::spawn(async move {
        let mut interval = tokio::time::interval(std::time::Duration::from_millis(500));
        loop {
            interval.tick().await;
            let _transition = tracked.transition.lock().await;
            if let Err(err) = tracked.playback.tick().await {
                tracing::error!("playback tracking failed: {err:#}");
            }
        }
    });
    let app = Router::new()
        .route(
            "/signin",
            axum::routing::get(|| async { axum::response::Html(include_str!("signin.html")) }),
        )
        .nest("/api", api::router(state))
        .fallback_service(frontend)
        .layer(axum::middleware::from_fn_with_state(port, api::require_local_host))
        .layer(TraceLayer::new_for_http());

    // Loopback only: the API starts processes and will hold account logins.
    let addr = SocketAddr::from((Ipv4Addr::LOCALHOST, port));
    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .with_context(|| format!("could not listen on {addr}"))?;
    tracing::info!(
        "listening on http://{addr}, frontend from {}",
        frontend_dir.display()
    );

    let result = axum::serve(listener, app)
        .with_graceful_shutdown(async {
            let ctrl_c = tokio::signal::ctrl_c();
            let mut terminate =
                tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
                    .expect("could not install SIGTERM handler");
            tokio::select! { _ = ctrl_c => {}, _ = terminate.recv() => {} }
        })
        .await;
    monitor.abort();
    pairing.end().await;
    playback.stop().await?;
    result?;
    Ok(())
}
