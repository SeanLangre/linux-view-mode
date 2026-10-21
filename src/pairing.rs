//! Phone pairing for sign-in: while the Firefox sign-in window is open, a QR code on the TV
//! opens a LAN page where the phone types into that window. Nothing typed is ever logged.
use anyhow::{Context, Result, bail};
use axum::{
    Json, Router,
    extract::{Path, State},
    http::StatusCode,
    response::{Html, IntoResponse, Response},
    routing::{get, post},
};
use serde::Deserialize;
use serde_json::json;
use std::{net::UdpSocket, process::Stdio, sync::Arc};
use tokio::{io::AsyncReadExt, process::Command, sync::Mutex};

const MAX_TEXT: usize = 256;
const PAGE: &str = include_str!("pairing.html");

struct Session {
    token: String,
    pid: Option<u32>,
}

/// At most one pairing session exists: the one for the open sign-in window.
#[derive(Clone)]
pub struct Pairing {
    port: u16,
    session: Arc<Mutex<Option<Session>>>,
}

impl Pairing {
    pub fn new(port: u16) -> Self {
        Self {
            port,
            session: Arc::new(Mutex::new(None)),
        }
    }

    pub async fn start(&self, pid: Option<u32>) -> Result<()> {
        let mut bytes = [0u8; 16];
        tokio::fs::File::open("/dev/urandom")
            .await?
            .read_exact(&mut bytes)
            .await?;
        let token = bytes.iter().map(|b| format!("{b:02x}")).collect();
        *self.session.lock().await = Some(Session { token, pid });
        Ok(())
    }

    pub async fn end(&self) {
        *self.session.lock().await = None;
    }

    /// Phone URL and its QR code as an SVG, if a sign-in window is open.
    pub async fn info(&self) -> Result<Option<(String, String)>> {
        let Some(token) = self.session.lock().await.as_ref().map(|s| s.token.clone()) else {
            return Ok(None);
        };
        let url = format!("http://{}:{}/p/{token}", lan_address()?, self.port);
        let svg = qrcode::QrCode::new(url.as_bytes())?
            .render::<qrcode::render::svg::Color>()
            .min_dimensions(320, 320)
            .quiet_zone(true)
            .build();
        Ok(Some((url, svg)))
    }

    async fn check(&self, token: &str) -> Option<Option<u32>> {
        let session = self.session.lock().await;
        session.as_ref().filter(|s| s.token == token).map(|s| s.pid)
    }

    /// Phone-side router, served on the LAN. Every route needs the live session token.
    pub fn router(&self) -> Router {
        Router::new()
            .route("/p/{token}", get(page))
            .route("/p/{token}/type", post(type_text))
            .route("/p/{token}/key", post(press_key))
            .route("/p/{token}/login", post(login))
            .with_state(self.clone())
    }
}

/// The address the phone should use: the one the OS routes outward from (no packets are sent).
fn lan_address() -> Result<std::net::IpAddr> {
    let socket = UdpSocket::bind("0.0.0.0:0")?;
    socket.connect("192.0.2.1:9").context("no network route")?;
    Ok(socket.local_addr()?.ip())
}

#[derive(Deserialize)]
struct TypeBody {
    text: String,
}
#[derive(Deserialize)]
struct LoginBody {
    email: String,
    password: String,
}
#[derive(Deserialize)]
struct KeyBody {
    key: String,
}

fn failure(status: StatusCode, message: &str) -> Response {
    (status, Json(json!({"error": message}))).into_response()
}

async fn page(State(pairing): State<Pairing>, Path(token): Path<String>) -> Response {
    match pairing.check(&token).await {
        Some(_) => Html(PAGE).into_response(),
        None => failure(StatusCode::NOT_FOUND, "this sign-in link has expired"),
    }
}

async fn type_text(
    State(pairing): State<Pairing>,
    Path(token): Path<String>,
    Json(body): Json<TypeBody>,
) -> Response {
    let Some(pid) = pairing.check(&token).await else {
        return failure(StatusCode::NOT_FOUND, "this sign-in link has expired");
    };
    if body.text.is_empty() || body.text.chars().count() > MAX_TEXT || body.text.contains('\0') {
        return failure(StatusCode::BAD_REQUEST, "text must be 1-256 characters");
    }
    xdotool(pid, &["type", "--clearmodifiers", "--delay", "12", "--", &body.text]).await
}

/// Google asks for the email first, then loads the password page.
const PASSWORD_PAGE_DELAY: std::time::Duration = std::time::Duration::from_secs(4);

async fn login(
    State(pairing): State<Pairing>,
    Path(token): Path<String>,
    Json(body): Json<LoginBody>,
) -> Response {
    let Some(pid) = pairing.check(&token).await else {
        return failure(StatusCode::NOT_FOUND, "this sign-in link has expired");
    };
    let valid = |text: &str| !text.is_empty() && text.chars().count() <= MAX_TEXT && !text.contains('\0');
    if !valid(&body.email) || !valid(&body.password) {
        return failure(StatusCode::BAD_REQUEST, "email and password must be 1-256 characters");
    }
    let steps = async {
        send_keys(pid, &["type", "--clearmodifiers", "--delay", "12", "--", &body.email]).await?;
        send_keys(pid, &["key", "--clearmodifiers", "--", "Return"]).await?;
        tokio::time::sleep(PASSWORD_PAGE_DELAY).await;
        send_keys(pid, &["type", "--clearmodifiers", "--delay", "12", "--", &body.password]).await?;
        send_keys(pid, &["key", "--clearmodifiers", "--", "Return"]).await
    };
    match steps.await {
        Ok(()) => Json(json!({"status": "ok"})).into_response(),
        Err(err) => {
            tracing::warn!("phone login failed: {err:#}");
            failure(StatusCode::BAD_GATEWAY, "the TV could not type (is xdotool installed?)")
        }
    }
}

async fn press_key(
    State(pairing): State<Pairing>,
    Path(token): Path<String>,
    Json(body): Json<KeyBody>,
) -> Response {
    let Some(pid) = pairing.check(&token).await else {
        return failure(StatusCode::NOT_FOUND, "this sign-in link has expired");
    };
    let key = match body.key.as_str() {
        "enter" => "Return",
        "tab" => "Tab",
        "backspace" => "BackSpace",
        "shift-tab" => "shift+Tab",
        _ => return failure(StatusCode::BAD_REQUEST, "unknown key"),
    };
    xdotool(pid, &["key", "--clearmodifiers", "--", key]).await
}

async fn xdotool(pid: Option<u32>, args: &[&str]) -> Response {
    match send_keys(pid, args).await {
        Ok(()) => Json(json!({"status": "ok"})).into_response(),
        Err(err) => {
            tracing::warn!("phone keyboard failed: {err:#}");
            failure(
                StatusCode::BAD_GATEWAY,
                "the TV could not type (is xdotool installed?)",
            )
        }
    }
}

async fn send_keys(pid: Option<u32>, args: &[&str]) -> Result<()> {
    // Best effort: bring the sign-in window to the front before typing.
    if let Some(pid) = pid {
        let found = Command::new("xdotool")
            .args(["search", "--onlyvisible", "--pid", &pid.to_string()])
            .stdin(Stdio::null())
            .stderr(Stdio::null())
            .output()
            .await;
        if let Some(window) = found
            .ok()
            .and_then(|out| String::from_utf8(out.stdout).ok())
            .and_then(|out| out.lines().next().map(str::to_owned))
        {
            let _ = Command::new("xdotool")
                .args(["windowfocus", "--sync", &window])
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status()
                .await;
        }
    }
    let status = Command::new("xdotool")
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .await
        .context("could not run xdotool")?;
    if !status.success() {
        bail!("xdotool exited with {status}");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn token_is_random_and_ends_with_the_session() {
        let pairing = Pairing::new(7879);
        assert!(pairing.check("x").await.is_none());
        pairing.start(Some(1)).await.unwrap();
        let first = pairing.session.lock().await.as_ref().unwrap().token.clone();
        assert_eq!(first.len(), 32);
        assert_eq!(pairing.check(&first).await, Some(Some(1)));
        assert!(pairing.check("wrong").await.is_none());
        pairing.start(None).await.unwrap();
        assert!(pairing.check(&first).await.is_none());
        pairing.end().await;
        assert!(pairing.info().await.unwrap().is_none());
    }
}
