//! Phone pairing for sign-in: while the Firefox sign-in window is open, a QR code on the TV
//! opens a LAN page where the phone types into that window. Nothing typed is ever logged, and the
//! LAN port is only open while a sign-in window is.
//!
//! Everything the phone sends is sealed with a per-session key that travels only in the QR code's
//! `#fragment` (browsers never send it), so a passive sniffer sees ciphertext. The phone must also
//! show the same confirmation code as the TV before anything is typed. An attacker who can rewrite
//! the page in transit still wins; see `encryption-plan.md`.
use anyhow::{Context, Result, bail};
use axum::{
    Json, Router,
    extract::{Path, State},
    http::StatusCode,
    response::{Html, IntoResponse, Response},
    routing::{get, post},
};
use crypto_secretbox::{
    XSalsa20Poly1305,
    aead::{Aead, KeyInit, generic_array::GenericArray},
};
use serde::Deserialize;
use serde_json::{Value, json};
use std::{net::UdpSocket, process::Stdio, sync::{Arc, LazyLock}};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    process::Command,
    sync::Mutex,
    task::JoinHandle,
};

const MAX_TEXT: usize = 256;
/// Each step can be typed this many times, so one typo doesn't force the TV keyboard.
const MAX_TRIES: u8 = 3;
/// A session started before Firefox opens expires if the sign-in window never does.
const IDLE_LIFETIME: std::time::Duration = std::time::Duration::from_secs(15 * 60);
/// The phone page with tweetnacl inlined, so the phone makes no second request. `vendor/nacl.min.js`
/// is tweetnacl 1.0.3 straight from the npm tarball (sha256 973cc573…ddbbc).
static PAGE: LazyLock<String> = LazyLock::new(|| {
    include_str!("pairing.html").replace("/*NACL*/", include_str!("vendor/nacl.min.js"))
});

#[derive(Clone, Copy, PartialEq)]
enum Step {
    Email = 0,
    Password = 1,
    Code = 2,
}

impl Step {
    fn name(self) -> &'static str {
        match self {
            Step::Email => "email",
            Step::Password => "password",
            Step::Code => "code",
        }
    }
}

struct Session {
    token: String,
    key: [u8; 32],
    phone: Option<Phone>,
    pid: Option<u32>,
    port: u16,
    tries: [u8; 3],
    server: JoinHandle<()>,
}

impl Drop for Session {
    fn drop(&mut self) {
        self.server.abort();
    }
}

/// The phone bound to a session. Until it is confirmed on the TV, a newer phone replaces it.
struct Phone {
    id: String,
    code: String,
    confirmed: bool,
    last_seq: u64,
}

/// What the TV needs to show: the code of the bound phone and whether the user accepted it.
pub struct PhoneState {
    pub code: String,
    pub confirmed: bool,
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
        let token = hex(&random_bytes::<16>().await?);
        let key = random_bytes::<32>().await?;
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
            key,
            phone: None,
            pid,
            port,
            tries: [0; 3],
            server,
        });
        drop(session);
        if pid.is_none() {
            self.expire_later().await;
        }
        Ok(())
    }

    /// End a session that is still waiting for its window if that never opens. It acts only on
    /// the token it saw, so a later start or reset gets its own timer.
    async fn expire_later(&self) {
        let pairing = self.clone();
        let expected = self.session.lock().await.as_ref().map(|s| s.token.clone());
        tokio::spawn(async move {
            tokio::time::sleep(IDLE_LIFETIME).await;
            let mut session = pairing.session.lock().await;
            if session.as_ref().is_some_and(|s| s.pid.is_none() && Some(&s.token) == expected.as_ref()) {
                *session = None;
            }
        });
    }

    /// Tie the session to the sign-in window once it opens; starts one if none is waiting.
    pub async fn attach(&self, pid: Option<u32>) -> Result<()> {
        if let Some(session) = self.session.lock().await.as_mut() {
            // Asking again for a waiting session must not detach an open window.
            if pid.is_some() {
                session.pid = pid;
            }
            return Ok(());
        }
        self.start(pid).await
    }

    /// End a session that is still waiting for its window (a window that is open keeps it).
    pub async fn cancel(&self) {
        let mut session = self.session.lock().await;
        if session.as_ref().is_some_and(|s| s.pid.is_none()) {
            *session = None;
        }
    }

    /// End the session and close the LAN port.
    pub async fn end(&self) {
        *self.session.lock().await = None;
    }

    /// Start over with a new token and key (the old link is dead), keeping the port and window.
    pub async fn reset(&self) -> Result<()> {
        let token = hex(&random_bytes::<16>().await?);
        let key = random_bytes::<32>().await?;
        let waiting = {
            let mut guard = self.session.lock().await;
            let Some(session) = guard.as_mut() else { return Ok(()) };
            session.token = token;
            session.key = key;
            session.phone = None;
            session.tries = [0; 3];
            session.pid.is_none()
        };
        if waiting {
            self.expire_later().await;
        }
        Ok(())
    }

    /// The bound phone's code and whether it was confirmed, if any phone said hello.
    pub async fn phone(&self) -> Option<PhoneState> {
        let session = self.session.lock().await;
        let phone = session.as_ref()?.phone.as_ref()?;
        Some(PhoneState { code: phone.code.clone(), confirmed: phone.confirmed })
    }

    /// The user says the code on the TV matches. Only succeeds for the code currently bound, so a
    /// phone that replaced the first one just before the press can't be confirmed by mistake.
    pub async fn confirm(&self, code: &str) -> bool {
        let mut session = self.session.lock().await;
        match session.as_mut().and_then(|s| s.phone.as_mut()) {
            Some(phone) if phone.code == code => {
                phone.confirmed = true;
                true
            }
            _ => false,
        }
    }

    /// Phone URL and its QR code as an SVG, if a sign-in window is open.
    pub async fn info(&self) -> Result<Option<(String, String)>> {
        let Some((token, key, port)) = self
            .session
            .lock()
            .await
            .as_ref()
            .map(|s| (s.token.clone(), s.key, s.port))
        else {
            return Ok(None);
        };
        // The key goes after the `#`, which browsers never send. Never log this URL.
        let url = format!("http://{}:{port}/p/{token}#{}", lan_address()?, hex(&key));
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

    /// Open a sealed message for a typing step, running every check before a try is used up.
    async fn accept(&self, token: &str, step: Step, sealed: &Sealed) -> Result<Message, Response> {
        let mut guard = self.session.lock().await;
        let Some(session) = guard.as_mut().filter(|s| s.token == token) else {
            return Err(failure(StatusCode::NOT_FOUND, "this sign-in link has expired"));
        };
        let Some(message) = open(&session.key, sealed) else {
            return Err(failure(
                StatusCode::BAD_REQUEST,
                "this link is out of date: scan the QR code again",
            ));
        };
        match &mut session.phone {
            Some(phone) if phone.id == message.phone => {
                if !phone.confirmed {
                    return Err(failure(StatusCode::FORBIDDEN, "confirm the code on the TV first"));
                }
                if message.step != step.name() {
                    return Err(failure(StatusCode::BAD_REQUEST, "wrong step"));
                }
                if message.seq <= phone.last_seq {
                    return Err(failure(StatusCode::BAD_REQUEST, "repeated message"));
                }
                phone.last_seq = message.seq;
                Ok(message)
            }
            Some(_) => Err(failure(StatusCode::CONFLICT, "another phone is connected")),
            None => Err(failure(StatusCode::FORBIDDEN, "confirm the code on the TV first")),
        }
    }

    /// Bind the phone that scanned the link and give it its confirmation code. The same phone
    /// asking again gets the same code, so replaying a captured hello changes nothing.
    async fn hello(&self, token: &str, sealed: &Sealed) -> Response {
        let Ok(random) = random_bytes::<28>().await else {
            return failure(StatusCode::INTERNAL_SERVER_ERROR, "no randomness");
        };
        let mut guard = self.session.lock().await;
        let Some(session) = guard.as_mut().filter(|s| s.token == token) else {
            return failure(StatusCode::NOT_FOUND, "this sign-in link has expired");
        };
        let message = open(&session.key, sealed).filter(|m| {
            m.step == "hello" && m.seq == 0 && m.phone.len() == 32 && unhex(&m.phone).is_some()
        });
        let Some(message) = message else {
            return failure(StatusCode::BAD_REQUEST, "this link is out of date: scan the QR code again");
        };
        let code = match &session.phone {
            Some(phone) if phone.id == message.phone => phone.code.clone(),
            Some(phone) if phone.confirmed => {
                return failure(StatusCode::CONFLICT, "another phone is connected");
            }
            _ => {
                let number = u32::from_be_bytes(random[..4].try_into().unwrap()) % 1_000_000;
                let code = format!("{number:06}");
                session.phone = Some(Phone {
                    id: message.phone,
                    code: code.clone(),
                    confirmed: false,
                    last_seq: 0,
                });
                code
            }
        };
        Json(seal(&session.key, random[4..].try_into().unwrap(), &json!({"code": code}))).into_response()
    }

    /// Mark a step as used; says why not if the token is wrong or the step was already used.
    async fn claim(&self, token: &str, step: Step) -> Result<Option<u32>, Response> {
        let mut guard = self.session.lock().await;
        let Some(session) = guard.as_mut().filter(|s| s.token == token) else {
            return Err(failure(StatusCode::NOT_FOUND, "this sign-in link has expired"));
        };
        let tries = &mut session.tries[step as usize];
        if *tries >= MAX_TRIES {
            return Err(failure(StatusCode::CONFLICT, "too many tries; use the TV keyboard"));
        }
        *tries += 1;
        Ok(session.pid)
    }

    /// Give a step back after typing failed, so the phone can try again.
    async fn release(&self, token: &str, step: Step) {
        if let Some(session) = self.session.lock().await.as_mut().filter(|s| s.token == token) {
            let tries = &mut session.tries[step as usize];
            *tries = tries.saturating_sub(1);
        }
    }

    /// Phone-side router, served on the LAN. Every route needs the live session token.
    fn router(&self) -> Router {
        Router::new()
            .route("/p/{token}", get(page))
            .route("/p/{token}/hello", post(hello))
            .route("/p/{token}/status", get(status))
            .route("/p/{token}/email", post(|s, p, b| send(s, p, b, Step::Email)))
            .route("/p/{token}/password", post(|s, p, b| send(s, p, b, Step::Password)))
            .route("/p/{token}/code", post(|s, p, b| send(s, p, b, Step::Code)))
            .with_state(self.clone())
    }
}

/// The address the phone should use: the one the OS routes outward from (no packets are sent).
pub(crate) fn lan_address() -> Result<std::net::IpAddr> {
    let socket = UdpSocket::bind("0.0.0.0:0")?;
    socket.connect("192.0.2.1:9").context("no network route")?;
    Ok(socket.local_addr()?.ip())
}

/// A message on the wire: `n` is the 24-byte nonce and `c` the NaCl secretbox, both as hex.
#[derive(Deserialize)]
struct Sealed {
    n: String,
    c: String,
}

/// What the phone seals. `step` stops a captured password being replayed to the email route, and
/// `seq` stops any captured message being replayed at all.
#[derive(Deserialize)]
struct Message {
    phone: String,
    step: String,
    seq: u64,
    #[serde(default)]
    text: String,
}

pub(crate) fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn unhex(text: &str) -> Option<Vec<u8>> {
    if text.len() % 2 != 0 || !text.is_ascii() {
        return None;
    }
    (0..text.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&text[i..i + 2], 16).ok())
        .collect()
}

pub(crate) async fn random_bytes<const N: usize>() -> Result<[u8; N]> {
    let mut bytes = [0u8; N];
    tokio::fs::File::open("/dev/urandom")
        .await?
        .read_exact(&mut bytes)
        .await?;
    Ok(bytes)
}

/// Decrypt and parse; `None` for a wrong key, a changed byte or anything that isn't a message.
fn open(key: &[u8; 32], sealed: &Sealed) -> Option<Message> {
    let nonce: [u8; 24] = unhex(&sealed.n)?.try_into().ok()?;
    let plain = XSalsa20Poly1305::new(GenericArray::from_slice(key))
        .decrypt(GenericArray::from_slice(&nonce), unhex(&sealed.c)?.as_slice())
        .ok()?;
    serde_json::from_slice(&plain).ok()
}

fn seal(key: &[u8; 32], nonce: [u8; 24], value: &Value) -> Value {
    let cipher = XSalsa20Poly1305::new(GenericArray::from_slice(key));
    let boxed = cipher
        .encrypt(GenericArray::from_slice(&nonce), value.to_string().as_bytes())
        .expect("encrypting in memory cannot fail");
    json!({"n": hex(&nonce), "c": hex(&boxed)})
}

fn failure(status: StatusCode, message: &str) -> Response {
    (status, Json(json!({"error": message}))).into_response()
}

async fn page(State(pairing): State<Pairing>, Path(token): Path<String>) -> Response {
    match pairing.check(&token).await {
        Some(_) => Html(PAGE.as_str()).into_response(),
        None => failure(StatusCode::NOT_FOUND, "this sign-in link has expired"),
    }
}

async fn hello(State(pairing): State<Pairing>, Path(token): Path<String>, Json(body): Json<Sealed>) -> Response {
    pairing.hello(&token, &body).await
}

#[derive(Deserialize)]
struct StatusQuery {
    phone: Option<String>,
}

/// Carries no secrets, so the phone can poll it in plain text. A phone that isn't the bound one
/// sees `none`, which tells it another phone took over.
async fn status(
    State(pairing): State<Pairing>,
    Path(token): Path<String>,
    axum::extract::Query(query): axum::extract::Query<StatusQuery>,
) -> Response {
    let guard = pairing.session.lock().await;
    let Some(session) = guard.as_ref().filter(|s| s.token == token) else {
        return failure(StatusCode::NOT_FOUND, "this sign-in link has expired");
    };
    let state = match &session.phone {
        Some(phone) if query.phone.as_deref().is_none_or(|id| id == phone.id) => {
            if phone.confirmed { "confirmed" } else { "waiting" }
        }
        _ => "none",
    };
    Json(json!({"state": state})).into_response()
}

/// Type one field into the sign-in window, then press Enter.
async fn send(
    State(pairing): State<Pairing>,
    Path(token): Path<String>,
    Json(body): Json<Sealed>,
    step: Step,
) -> Response {
    let text = match pairing.accept(&token, step, &body).await {
        Ok(message) => message.text,
        Err(response) => return response,
    };
    if text.is_empty() || text.chars().count() > MAX_TEXT || text.chars().any(char::is_control) {
        return failure(StatusCode::BAD_REQUEST, "text must be 1-256 characters on one line");
    }
    let pid = match pairing.claim(&token, step).await {
        Ok(pid) => pid,
        Err(response) => return response,
    };
    if pid.is_none() {
        pairing.release(&token, step).await;
        return failure(
            StatusCode::TOO_EARLY,
            "the sign-in window isn't open yet: press Open Firefox sign-in on the TV, then try again",
        );
    }
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
    async fn steps_are_limited_and_can_be_given_back() {
        let pairing = Pairing::new(0);
        pairing.start(Some(1)).await.unwrap();
        let token = pairing.session.lock().await.as_ref().unwrap().token.clone();
        for step in [Step::Email, Step::Password, Step::Code] {
            for _ in 0..MAX_TRIES {
                assert!(pairing.claim(&token, step).await.is_ok());
            }
            assert!(pairing.claim(&token, step).await.is_err());
            pairing.release(&token, step).await;
            assert!(pairing.claim(&token, step).await.is_ok());
        }
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

    const PHONE: &str = "00112233445566778899aabbccddeeff";

    async fn session() -> (Pairing, String, [u8; 32]) {
        let pairing = Pairing::new(0);
        pairing.start(Some(1)).await.unwrap();
        let guard = pairing.session.lock().await;
        let session = guard.as_ref().unwrap();
        let (token, key) = (session.token.clone(), session.key);
        drop(guard);
        (pairing, token, key)
    }

    fn sealed(key: &[u8; 32], phone: &str, step: &str, seq: u64) -> Sealed {
        let value = seal(key, [7; 24], &json!({"phone": phone, "step": step, "seq": seq, "text": "x"}));
        Sealed { n: value["n"].as_str().unwrap().into(), c: value["c"].as_str().unwrap().into() }
    }

    async fn hello_code(pairing: &Pairing, token: &str, key: &[u8; 32], phone: &str) -> Option<String> {
        let response = pairing.hello(token, &sealed(key, phone, "hello", 0)).await;
        if response.status() != StatusCode::OK {
            return None;
        }
        let body = axum::body::to_bytes(response.into_body(), 4096).await.unwrap();
        let reply: Value = serde_json::from_slice(&body).unwrap();
        let sealed = Sealed { n: reply["n"].as_str()?.into(), c: reply["c"].as_str()?.into() };
        let nonce: [u8; 24] = unhex(&sealed.n)?.try_into().ok()?;
        let plain = XSalsa20Poly1305::new(GenericArray::from_slice(key))
            .decrypt(GenericArray::from_slice(&nonce), unhex(&sealed.c)?.as_slice())
            .ok()?;
        Some(serde_json::from_slice::<Value>(&plain).ok()?["code"].as_str()?.into())
    }

    async fn confirmed_phone() -> (Pairing, String, [u8; 32]) {
        let (pairing, token, key) = session().await;
        let code = hello_code(&pairing, &token, &key, PHONE).await.unwrap();
        assert!(pairing.confirm(&code).await);
        (pairing, token, key)
    }

    fn status_of(response: Result<Message, Response>) -> StatusCode {
        response.err().map_or(StatusCode::OK, |r| r.status())
    }

    #[test]
    fn rust_opens_what_tweetnacl_sealed() {
        // Made in Node: nacl.secretbox(msg, nonce, key) with key = 0..31 and nonce = 100..123.
        let key: [u8; 32] = std::array::from_fn(|i| i as u8);
        let sealed = Sealed {
            n: "6465666768696a6b6c6d6e6f707172737475767778797a7b".into(),
            c: "8eb31a0fbc0daa74e17f42ed5431edcc799be9a155d8abcb8adf13a406b5912a0d990c93d1937d2d913b7aac33b2a8e9ca0d977b2b53fe819370354e2260a900b1b2943a62e74a2c748e2160f99ebc1726fa6e57fb106afedcf76bd07ae185e63ae3e87595dd06".into(),
        };
        let message = open(&key, &sealed).unwrap();
        assert_eq!((message.phone.as_str(), message.step.as_str(), message.seq), (PHONE, "password", 3));
        assert_eq!(message.text, "hunter2");
    }

    #[tokio::test]
    async fn wrong_key_or_garbage_does_not_use_a_try() {
        let (pairing, token, key) = confirmed_phone().await;
        let wrong = sealed(&[9; 32], PHONE, "email", 1);
        assert_eq!(status_of(pairing.accept(&token, Step::Email, &wrong).await), StatusCode::BAD_REQUEST);
        let garbage = Sealed { n: "zz".into(), c: "00".into() };
        assert_eq!(status_of(pairing.accept(&token, Step::Email, &garbage).await), StatusCode::BAD_REQUEST);
        let mut flipped = sealed(&key, PHONE, "email", 1);
        flipped.c.replace_range(0..2, if flipped.c.starts_with("00") { "01" } else { "00" });
        assert_eq!(status_of(pairing.accept(&token, Step::Email, &flipped).await), StatusCode::BAD_REQUEST);
        let tries = pairing.session.lock().await.as_ref().unwrap().tries;
        assert_eq!(tries, [0; 3]);
        assert!(pairing.accept(&token, Step::Email, &sealed(&key, PHONE, "email", 1)).await.is_ok());
    }

    #[tokio::test]
    async fn replayed_seq_is_rejected() {
        let (pairing, token, key) = confirmed_phone().await;
        let first = sealed(&key, PHONE, "password", 5);
        assert!(pairing.accept(&token, Step::Password, &first).await.is_ok());
        assert_eq!(status_of(pairing.accept(&token, Step::Password, &first).await), StatusCode::BAD_REQUEST);
        let older = sealed(&key, PHONE, "password", 4);
        assert_eq!(status_of(pairing.accept(&token, Step::Password, &older).await), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn a_message_for_one_step_is_rejected_on_another() {
        let (pairing, token, key) = confirmed_phone().await;
        let password = sealed(&key, PHONE, "password", 1);
        assert_eq!(status_of(pairing.accept(&token, Step::Email, &password).await), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn steps_wait_for_confirmation() {
        let (pairing, token, key) = session().await;
        let message = sealed(&key, PHONE, "email", 1);
        assert_eq!(status_of(pairing.accept(&token, Step::Email, &message).await), StatusCode::FORBIDDEN);
        hello_code(&pairing, &token, &key, PHONE).await.unwrap();
        assert_eq!(status_of(pairing.accept(&token, Step::Email, &message).await), StatusCode::FORBIDDEN);
    }

    #[tokio::test]
    async fn an_old_code_cannot_confirm() {
        let (pairing, token, key) = session().await;
        let mut first = hello_code(&pairing, &token, &key, PHONE).await.unwrap();
        let other = "ffeeddccbbaa99887766554433221100";
        let second = hello_code(&pairing, &token, &key, other).await.unwrap();
        if first == second {
            // One chance in a million that two random codes match; make them differ.
            first = "x".into();
        }
        assert!(!pairing.confirm(&first).await);
        assert!(!pairing.phone().await.unwrap().confirmed);
        assert!(pairing.confirm(&second).await);
        // The replaced phone is no longer the bound one.
        let old = sealed(&key, PHONE, "email", 1);
        assert_eq!(status_of(pairing.accept(&token, Step::Email, &old).await), StatusCode::CONFLICT);
    }

    #[tokio::test]
    async fn a_second_phone_after_confirmation_is_refused() {
        let (pairing, token, key) = confirmed_phone().await;
        let other = "ffeeddccbbaa99887766554433221100";
        assert!(hello_code(&pairing, &token, &key, other).await.is_none());
        let response = pairing.hello(&token, &sealed(&key, other, "hello", 0)).await;
        assert_eq!(response.status(), StatusCode::CONFLICT);
        assert!(pairing.phone().await.unwrap().confirmed);
    }

    #[tokio::test]
    async fn a_reload_of_the_confirmed_phone_gets_the_same_code() {
        let (pairing, token, key) = session().await;
        let first = hello_code(&pairing, &token, &key, PHONE).await.unwrap();
        assert_eq!(hello_code(&pairing, &token, &key, PHONE).await.unwrap(), first);
        assert!(pairing.confirm(&first).await);
        assert_eq!(hello_code(&pairing, &token, &key, PHONE).await.unwrap(), first);
        assert!(pairing.phone().await.unwrap().confirmed);
    }

    #[tokio::test]
    async fn reset_changes_token_and_key() {
        let (pairing, token, key) = confirmed_phone().await;
        pairing.reset().await.unwrap();
        let guard = pairing.session.lock().await;
        let session = guard.as_ref().unwrap();
        assert_ne!(session.token, token);
        assert_ne!(session.key, key);
        assert!(session.phone.is_none());
        assert_eq!(session.pid, Some(1));
        drop(guard);
        assert!(pairing.check(&token).await.is_none());
    }

    #[tokio::test]
    async fn the_link_carries_the_key_after_the_fragment_only() {
        let (pairing, token, key) = session().await;
        let Ok(Some((url, _))) = pairing.info().await else { return };
        assert!(url.ends_with(&format!("/p/{token}#{}", hex(&key))));
    }
}
