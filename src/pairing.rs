//! Phone pairing for sign-in: while the Firefox sign-in window is open, a QR code on the TV
//! opens a LAN page where the phone types into that window. Nothing typed is ever logged, and the
//! LAN port is only open while a sign-in window is.
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
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    process::Command,
    sync::Mutex,
    task::JoinHandle,
};

const MAX_TEXT: usize = 256;
const MAX_CODES: u8 = 3;
const PAGE: &str = include_str!("pairing.html");

#[derive(Clone, Copy, PartialEq)]
enum Step {
    Email,
    Password,
    Code,
}

struct Session {
    token: String,
    pid: Option<u32>,
    port: u16,
    email_sent: bool,
    password_sent: bool,
    codes_sent: u8,
    server: JoinHandle<()>,
}

impl Drop for Session {
    fn drop(&mut self) {
        self.server.abort();
    }
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

    /// Open the LAN port and make a fresh token for the sign-in window with this process id.
    pub async fn start(&self, pid: Option<u32>) -> Result<()> {
        let mut bytes = [0u8; 16];
        tokio::fs::File::open("/dev/urandom")
            .await?
            .read_exact(&mut bytes)
            .await?;
        let token = bytes.iter().map(|b| format!("{b:02x}")).collect();
        let mut session = self.session.lock().await;
        // Drop any earlier session first so its port is free again.
        *session = None;
        let listener = tokio::net::TcpListener::bind(("0.0.0.0", self.port))
            .await
            .with_context(|| format!("could not listen on port {}", self.port))?;
        let port = listener.local_addr()?.port();
        let app = self.router();
        let server = tokio::spawn(async move {
            let _ = axum::serve(listener, app).await;
        });
        *session = Some(Session {
            token,
            pid,
            port,
            email_sent: false,
            password_sent: false,
            codes_sent: 0,
            server,
        });
        Ok(())
    }

    /// End the session and close the LAN port.
    pub async fn end(&self) {
        *self.session.lock().await = None;
    }

    /// Phone URL and its QR code as an SVG, if a sign-in window is open.
    pub async fn info(&self) -> Result<Option<(String, String)>> {
        let Some((token, port)) = self
            .session
            .lock()
            .await
            .as_ref()
            .map(|s| (s.token.clone(), s.port))
        else {
            return Ok(None);
        };
        let url = format!("http://{}:{port}/p/{token}", lan_address()?);
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

    /// Mark a step as used; says why not if the token is wrong or the step was already used.
    async fn claim(&self, token: &str, step: Step) -> Result<Option<u32>, Response> {
        let mut guard = self.session.lock().await;
        let Some(session) = guard.as_mut().filter(|s| s.token == token) else {
            return Err(failure(StatusCode::NOT_FOUND, "this sign-in link has expired"));
        };
        let used = match step {
            Step::Email => &mut session.email_sent,
            Step::Password => &mut session.password_sent,
            Step::Code if session.codes_sent < MAX_CODES => {
                session.codes_sent += 1;
                return Ok(session.pid);
            }
            Step::Code => return Err(failure(StatusCode::CONFLICT, "too many codes sent; use the TV keyboard")),
        };
        if std::mem::replace(used, true) {
            return Err(failure(StatusCode::CONFLICT, "already sent; use the TV keyboard"));
        }
        Ok(session.pid)
    }

    /// Give a step back after typing failed, so the phone can try again.
    async fn release(&self, token: &str, step: Step) {
        if let Some(session) = self.session.lock().await.as_mut().filter(|s| s.token == token) {
            match step {
                Step::Email => session.email_sent = false,
                Step::Password => session.password_sent = false,
                Step::Code => session.codes_sent = session.codes_sent.saturating_sub(1),
            }
        }
    }

    /// Phone-side router, served on the LAN. Every route needs the live session token.
    fn router(&self) -> Router {
        Router::new()
            .route("/p/{token}", get(page))
            .route("/p/{token}/email", post(|s, p, b| send(s, p, b, Step::Email)))
            .route("/p/{token}/password", post(|s, p, b| send(s, p, b, Step::Password)))
            .route("/p/{token}/code", post(|s, p, b| send(s, p, b, Step::Code)))
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
struct TextBody {
    text: String,
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

/// Type one field into the sign-in window, then press Enter.
async fn send(
    State(pairing): State<Pairing>,
    Path(token): Path<String>,
    Json(body): Json<TextBody>,
    step: Step,
) -> Response {
    let text = body.text;
    if text.is_empty() || text.chars().count() > MAX_TEXT || text.chars().any(char::is_control) {
        return failure(StatusCode::BAD_REQUEST, "text must be 1-256 characters on one line");
    }
    let pid = match pairing.claim(&token, step).await {
        Ok(pid) => pid,
        Err(response) => return response,
    };
    match type_and_enter(pid, &text).await {
        Ok(()) => Json(json!({"status": "ok"})).into_response(),
        Err(err) => {
            pairing.release(&token, step).await;
            tracing::warn!("phone typing failed: {err:#}");
            failure(
                StatusCode::BAD_GATEWAY,
                "the TV could not type (is xdotool installed and the sign-in window showing?)",
            )
        }
    }
}

/// The visible window of the sign-in Firefox, brought to the front. Fails if it isn't found or
/// doesn't take focus, so text never goes to whatever else has focus.
async fn focus_window(pid: Option<u32>) -> Result<String> {
    let pid = pid.context("sign-in window has no process id")?;
    let found = xdotool(&["search", "--onlyvisible", "--pid", &pid.to_string()]).await?;
    let window = found
        .lines()
        .next()
        .context("sign-in window not found")?
        .to_owned();
    xdotool(&["windowfocus", "--sync", &window]).await?;
    let focused = xdotool(&["getwindowfocus"]).await?;
    if focused.trim() != window {
        bail!("sign-in window did not take focus");
    }
    Ok(window)
}

async fn type_and_enter(pid: Option<u32>, text: &str) -> Result<()> {
    focus_window(pid).await?;
    // The text goes in on stdin, never as an argument, so `ps` can't show it. xdotool runs
    // without `--window` on purpose: that mode sends synthetic events Firefox ignores.
    let mut child = Command::new("xdotool")
        .args(["type", "--clearmodifiers", "--delay", "12", "--file", "-"])
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .kill_on_drop(true)
        .spawn()
        .context("could not run xdotool")?;
    let mut stdin = child.stdin.take().context("no stdin")?;
    stdin.write_all(text.as_bytes()).await?;
    drop(stdin);
    let status = child.wait().await?;
    if !status.success() {
        bail!("xdotool type exited with {status}");
    }
    // Re-check focus so Enter doesn't land in another window if focus moved while typing.
    focus_window(pid).await?;
    xdotool(&["key", "--clearmodifiers", "Return"]).await?;
    Ok(())
}

async fn xdotool(args: &[&str]) -> Result<String> {
    let out = Command::new("xdotool")
        .args(args)
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .await
        .context("could not run xdotool")?;
    if !out.status.success() {
        bail!("xdotool {} exited with {}", args[0], out.status);
    }
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn token_is_random_and_ends_with_the_session() {
        let pairing = Pairing::new(0);
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

    #[tokio::test]
    async fn steps_are_single_use_and_can_be_given_back() {
        let pairing = Pairing::new(0);
        pairing.start(Some(1)).await.unwrap();
        let token = pairing.session.lock().await.as_ref().unwrap().token.clone();
        assert!(pairing.claim(&token, Step::Email).await.is_ok());
        assert!(pairing.claim(&token, Step::Email).await.is_err());
        pairing.release(&token, Step::Email).await;
        assert!(pairing.claim(&token, Step::Email).await.is_ok());
        assert!(pairing.claim(&token, Step::Password).await.is_ok());
        assert!(pairing.claim(&token, Step::Password).await.is_err());
        for _ in 0..MAX_CODES {
            assert!(pairing.claim(&token, Step::Code).await.is_ok());
        }
        assert!(pairing.claim(&token, Step::Code).await.is_err());
        assert!(pairing.claim("wrong", Step::Email).await.is_err());
    }

    #[tokio::test]
    async fn port_is_only_open_during_a_session() {
        let probe = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = probe.local_addr().unwrap().port();
        drop(probe);
        let pairing = Pairing::new(port);
        assert!(tokio::net::TcpStream::connect(("127.0.0.1", port)).await.is_err());
        pairing.start(None).await.unwrap();
        assert!(tokio::net::TcpStream::connect(("127.0.0.1", port)).await.is_ok());
        pairing.end().await;
        tokio::task::yield_now().await;
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        assert!(tokio::net::TcpStream::connect(("127.0.0.1", port)).await.is_err());
    }
}
