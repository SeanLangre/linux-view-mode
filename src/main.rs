mod api;
mod ytdlp;

use std::net::{Ipv4Addr, SocketAddr};
use std::path::PathBuf;

use anyhow::{Context, Result};
use axum::Router;
use tower_http::services::{ServeDir, ServeFile};
use tower_http::trace::TraceLayer;
use tracing_subscriber::EnvFilter;

const DEFAULT_PORT: u16 = 7878;

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
    let frontend = ServeDir::new(&frontend_dir)
        .fallback(ServeFile::new(frontend_dir.join("index.html")));

    let app = Router::new()
        .nest("/api", api::router())
        .fallback_service(frontend)
        .layer(TraceLayer::new_for_http());

    // Loopback only: the API starts processes and will hold account logins.
    let addr = SocketAddr::from((Ipv4Addr::LOCALHOST, port));
    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .with_context(|| format!("could not listen on {addr}"))?;
    tracing::info!("listening on http://{addr}, frontend from {}", frontend_dir.display());

    axum::serve(listener, app).await?;
    Ok(())
}
