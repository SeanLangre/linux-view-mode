//! Phone remote: play/pause, seek, volume, skip and the queue from a phone on the LAN.
//!
//! The LAN port is off until the TV user turns the remote on. A phone gets in only by scanning a
//! one-time QR code shown on the TV and then having its confirmation code accepted on the TV; it
//! receives a random device token that is kept (hashed) until the TV user forgets all phones.
//! Like the sign-in pairing, this is plain HTTP, so use it on a network you trust.
use crate::{
    api::AppState,
    pairing::{hex, lan_address, random_bytes},
    ytdlp,
};
use anyhow::{Context, Result};
use axum::{
    Json, Router,
    extract::State,
    http::{HeaderMap, StatusCode, header},
    response::{Html, IntoResponse, Response},
    routing::{get, post},
};
use serde::Deserialize;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    sync::Arc,
    time::{Duration, Instant},
};
use tokio::{sync::Mutex, task::JoinHandle};

/// A pairing link stops working after this long.
const PAIR_LIFETIME: Duration = Duration::from_secs(10 * 60);
const PAGE: &str = include_str!("remote.html");

#[derive(Clone)]
pub struct Remote {
    port: u16,
    db: crate::db::Database,
    inner: Arc<Mutex<Inner>>,
}

#[derive(Default)]
struct Inner {
    server: Option<JoinHandle<()>>,
    attempt: Option<Attempt>,
}

struct Attempt {
    token: String,
    expires: Instant,
    phone: Option<String>,
    code: String,
    confirmed: bool,
    /// The device token waiting to be collected by the phone, once.
    device: Option<String>,
}

#[derive(Clone)]
struct Lan {
    remote: Remote,
    app: AppState,
}

fn digest(token: &str) -> String {
    hex(&Sha256::digest(token.as_bytes()))
}

impl Remote {
    pub fn new(db: crate::db::Database, port: u16) -> Self {
        Self { port, db, inner: Default::default() }
    }

    /// Open the port if the remote is switched on (called at startup).
    pub async fn resume(&self, app: AppState) {
        if self.db.remote_enabled().await.unwrap_or(false)
            && let Err(err) = self.set_enabled(true, app).await
        {
            tracing::warn!("phone remote could not start: {err:#}");
        }
    }

    pub async fn set_enabled(&self, enabled: bool, app: AppState) -> Result<()> {
        let mut inner = self.inner.lock().await;
        if let Some(server) = inner.server.take() {
            server.abort();
        }
        inner.attempt = None;
        if enabled {
            let listener = tokio::net::TcpListener::bind(("0.0.0.0", self.port))
                .await
                .with_context(|| format!("could not listen on port {}", self.port))?;
            let router = Router::new()
                .route("/", get(page))
                .route("/r/{token}", get(page))
                .route("/pair/hello", post(hello))
                .route("/pair/poll", post(poll))
                .route("/state", get(state))
                .route("/cmd", post(command))
                .with_state(Lan { remote: self.clone(), app });
            inner.server = Some(tokio::spawn(async move {
                let _ = axum::serve(listener, router).await;
            }));
        }
        drop(inner);
        self.db.set_remote_enabled(enabled).await
    }

    pub async fn end(&self) {
        let mut inner = self.inner.lock().await;
        if let Some(server) = inner.server.take() {
            server.abort();
        }
    }

    /// Start a pairing attempt (replacing any earlier one) and return the phone URL and QR code.
    pub async fn start_pairing(&self) -> Result<Option<(String, String)>> {
        let token = hex(&random_bytes::<16>().await?);
        let mut inner = self.inner.lock().await;
        if inner.server.is_none() {
            return Ok(None);
        }
        inner.attempt = Some(Attempt {
            token,
            expires: Instant::now() + PAIR_LIFETIME,
            phone: None,
            code: String::new(),
            confirmed: false,
            device: None,
        });
        drop(inner);
        self.pairing_info().await
    }

    /// The URL, QR code, code from the connected phone, and whether the user accepted it.
    pub async fn pairing_info(&self) -> Result<Option<(String, String)>> {
        let inner = self.inner.lock().await;
        let Some(attempt) = inner.attempt.as_ref().filter(|a| a.expires > Instant::now()) else {
            return Ok(None);
        };
        // Never log this URL: whoever has it can pair.
        let url = format!("http://{}:{}/r/{}", lan_address()?, self.port, attempt.token);
        let svg = qrcode::QrCode::new(url.as_bytes())?
            .render::<qrcode::render::svg::Color>()
            .min_dimensions(320, 320)
            .quiet_zone(true)
            .build();
        Ok(Some((url, svg)))
    }

    pub async fn phone_code(&self) -> Option<(String, bool)> {
        let inner = self.inner.lock().await;
        let attempt = inner.attempt.as_ref().filter(|a| a.expires > Instant::now())?;
        attempt.phone.as_ref()?;
        Some((attempt.code.clone(), attempt.confirmed))
    }

    pub async fn running(&self) -> bool {
        self.inner.lock().await.server.is_some()
    }

    /// The user says the phone shows this code. Makes the device token the phone will collect.
    pub async fn confirm(&self, code: &str) -> Result<bool> {
        let device = hex(&random_bytes::<32>().await?);
        let mut inner = self.inner.lock().await;
        let Some(attempt) = inner
            .attempt
            .as_mut()
            .filter(|a| a.expires > Instant::now() && a.phone.is_some() && a.code == code && !a.confirmed)
        else {
            return Ok(false);
        };
        self.db.add_device(&digest(&device)).await?;
        attempt.confirmed = true;
        attempt.device = Some(device);
        Ok(true)
    }

    /// The code didn't match: kill the link.
    pub async fn reject(&self) {
        self.inner.lock().await.attempt = None;
    }

    pub async fn forget_devices(&self) -> Result<()> {
        self.inner.lock().await.attempt = None;
        self.db.clear_devices().await
    }

    pub async fn device_count(&self) -> u32 {
        self.db.device_count().await.unwrap_or(0)
    }

    async fn authorized(&self, headers: &HeaderMap) -> bool {
        let Some(token) = headers
            .get(header::AUTHORIZATION)
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.strip_prefix("Bearer "))
            .filter(|t| t.len() == 64 && t.bytes().all(|b| b.is_ascii_hexdigit()))
        else {
            return false;
        };
        self.db.has_device(&digest(token)).await.unwrap_or(false)
    }
}

fn failure(status: StatusCode, message: &str) -> Response {
    (status, Json(json!({"error": message}))).into_response()
}

async fn page() -> Html<&'static str> {
    Html(PAGE)
}

#[derive(Deserialize)]
struct Hello {
    token: String,
    phone: String,
}

async fn hello(State(lan): State<Lan>, Json(body): Json<Hello>) -> Response {
    let Ok(random) = random_bytes::<4>().await else {
        return failure(StatusCode::INTERNAL_SERVER_ERROR, "no randomness");
    };
    let mut inner = lan.remote.inner.lock().await;
    let Some(attempt) = inner
        .attempt
        .as_mut()
        .filter(|a| a.expires > Instant::now() && a.token == body.token)
    else {
        return failure(StatusCode::NOT_FOUND, "this link has expired: scan the QR code again");
    };
    if body.phone.len() != 32 || !body.phone.bytes().all(|b| b.is_ascii_hexdigit()) {
        return failure(StatusCode::BAD_REQUEST, "bad phone id");
    }
    match &attempt.phone {
        Some(phone) if *phone == body.phone => {}
        Some(_) if attempt.confirmed => {
            return failure(StatusCode::CONFLICT, "another phone is connected");
        }
        _ => {
            let number = u32::from_be_bytes(random) % 1_000_000;
            attempt.code = format!("{number:06}");
            attempt.phone = Some(body.phone);
        }
    }
    Json(json!({"code": attempt.code})).into_response()
}

/// The phone asks whether the TV accepted its code; the device token is handed over once.
async fn poll(State(lan): State<Lan>, Json(body): Json<Hello>) -> Response {
    let mut inner = lan.remote.inner.lock().await;
    let Some(attempt) = inner
        .attempt
        .as_mut()
        .filter(|a| a.expires > Instant::now() && a.token == body.token)
    else {
        return failure(StatusCode::NOT_FOUND, "this link has expired: scan the QR code again");
    };
    if attempt.phone.as_deref() != Some(body.phone.as_str()) {
        return failure(StatusCode::CONFLICT, "another phone is connected");
    }
    match attempt.device.take() {
        Some(device) => {
            inner.attempt = None;
            Json(json!({"device": device})).into_response()
        }
        None => Json(json!({"waiting": true})).into_response(),
    }
}

async fn state(State(lan): State<Lan>, headers: HeaderMap) -> Response {
    if !lan.remote.authorized(&headers).await {
        return failure(StatusCode::UNAUTHORIZED, "pair this phone again");
    }
    let status = match lan.app.playback.player.status().await {
        Ok(status) => status,
        Err(_) => return failure(StatusCode::INTERNAL_SERVER_ERROR, "could not read playback"),
    };
    let (profile, queue) = {
        let _transition = lan.app.transition.lock().await;
        let Ok(account) = lan.app.profiles.active().await else {
            return failure(StatusCode::INTERNAL_SERVER_ERROR, "could not read the profile");
        };
        let queue = lan.app.profiles.db.queue(&account.profile.id).await.unwrap_or_default();
        (account.profile.name, queue)
    };
    Json(json!({
        "status": status,
        "profile": profile,
        "queue": queue.iter().map(|v| json!({"id": v.id, "title": v.title})).collect::<Vec<_>>(),
    }))
    .into_response()
}

#[derive(Deserialize)]
struct Command {
    cmd: String,
    value: Option<Value>,
}

/// A video id from a bare id or a youtube.com / youtu.be link.
pub fn video_id(text: &str) -> Option<String> {
    let text = text.trim();
    if ytdlp::is_video_id(text) {
        return Some(text.into());
    }
    let url = text.strip_prefix("https://").or_else(|| text.strip_prefix("http://"))?;
    let (host, rest) = url.split_once('/')?;
    let host = host.strip_prefix("www.").or_else(|| host.strip_prefix("m.")).unwrap_or(host);
    let id = match host {
        "youtu.be" => rest.split(['?', '&', '#']).next()?,
        "youtube.com" => {
            if let Some(path) = rest.strip_prefix("shorts/").or_else(|| rest.strip_prefix("live/")) {
                path.split(['?', '&', '#']).next()?
            } else {
                let query = rest.split_once('?')?.1;
                query
                    .split(['&', '#'])
                    .find_map(|pair| pair.strip_prefix("v="))?
            }
        }
        _ => return None,
    };
    ytdlp::is_video_id(id).then(|| id.to_owned())
}

async fn command(State(lan): State<Lan>, headers: HeaderMap, Json(body): Json<Command>) -> Response {
    if !lan.remote.authorized(&headers).await {
        return failure(StatusCode::UNAUTHORIZED, "pair this phone again");
    }
    let number = body.value.as_ref().and_then(Value::as_f64).filter(|v| v.is_finite());
    let player = &lan.app.playback.player;
    let mpv = match body.cmd.as_str() {
        "pause" => Some(vec![json!("cycle"), json!("pause")]),
        "mute" => Some(vec![json!("cycle"), json!("mute")]),
        "subtitles" => Some(vec![json!("cycle"), json!("sub")]),
        "seek" => number.map(|n| vec![json!("seek"), json!(n.clamp(-600.0, 600.0))]),
        "volume" => number.map(|n| vec![json!("add"), json!("volume"), json!(n.clamp(-20.0, 20.0))]),
        _ => None,
    };
    if let Some(mpv) = mpv {
        return match player.command(&mpv).await {
            Ok(()) => Json(json!({"ok": true})).into_response(),
            Err(_) => failure(StatusCode::CONFLICT, "nothing is playing"),
        };
    }
    let _transition = lan.app.transition.lock().await;
    let Ok(account) = lan.app.profiles.active().await else {
        return failure(StatusCode::INTERNAL_SERVER_ERROR, "could not read the profile");
    };
    let db = &lan.app.profiles.db;
    let id = body.value.as_ref().and_then(Value::as_str).and_then(video_id);
    let result = match (body.cmd.as_str(), id) {
        ("stop", _) => lan.app.playback.stop().await,
        ("next", _) => lan.app.playback.next(account).await.map(|_| ()),
        ("play", Some(id)) => {
            if lan.app.profiles.login_status(&account.profile.id).await.busy() {
                return failure(StatusCode::CONFLICT, "a sign-in is in progress on the TV");
            }
            lan.app.playback.play(account, &id, None).await
        }
        ("enqueue", Some(id)) => db.enqueue(&account.profile.id, &id).await,
        ("dequeue", Some(id)) => db.dequeue(&account.profile.id, Some(&id)).await,
        ("clear", _) => db.dequeue(&account.profile.id, None).await,
        ("play" | "enqueue" | "dequeue", None) => {
            return failure(StatusCode::BAD_REQUEST, "that is not a YouTube link");
        }
        _ => return failure(StatusCode::BAD_REQUEST, "unknown command"),
    };
    match result {
        Ok(()) => Json(json!({"ok": true})).into_response(),
        Err(err) => {
            tracing::error!("remote command {} failed: {err:#}", body.cmd);
            failure(StatusCode::BAD_GATEWAY, "the TV could not do that")
        }
    }
}

#[cfg(test)]
mod tests {
    use super::video_id;

    #[test]
    fn finds_video_ids_in_links() {
        let id = Some("aqz-KE-bpKQ".to_owned());
        for text in [
            "aqz-KE-bpKQ",
            "https://www.youtube.com/watch?v=aqz-KE-bpKQ&t=3",
            "https://youtube.com/watch?list=x&v=aqz-KE-bpKQ",
            "https://m.youtube.com/shorts/aqz-KE-bpKQ?feature=share",
            "https://youtu.be/aqz-KE-bpKQ?si=x",
        ] {
            assert_eq!(video_id(text), id, "{text}");
        }
        for text in ["", "https://evil.example/watch?v=aqz-KE-bpKQ", "https://youtube.com/", "javascript:alert(1)", "short"] {
            assert_eq!(video_id(text), None, "{text}");
        }
    }
}
