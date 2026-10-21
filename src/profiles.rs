//! Account directories and the Firefox sign-in lifecycle. Cookie contents never enter logs.
use crate::{
    db::{Database, Profile},
    pairing::Pairing,
    ytdlp,
};
use anyhow::{Context, Result, bail};
use serde::Serialize;
use std::{
    collections::HashMap,
    os::unix::fs::{DirBuilderExt, PermissionsExt},
    path::{Path, PathBuf},
    process::Stdio,
    sync::Arc,
};
use tokio::{process::Command, sync::Mutex};

const USER_JS: &str = include_str!("../config/firefox/user.js");

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum LoginState {
    Guest,
    Unknown,
    SigningIn,
    Checking,
    SignedIn,
    SignInAgain,
    CheckFailed,
}

#[derive(Clone, Serialize)]
pub struct LoginStatus {
    pub state: LoginState,
    pub message: String,
}
impl LoginStatus {
    fn new(state: LoginState, message: &str) -> Self {
        Self {
            state,
            message: message.into(),
        }
    }
    pub fn busy(&self) -> bool {
        matches!(self.state, LoginState::SigningIn | LoginState::Checking)
    }
}

#[derive(Clone)]
pub struct Account {
    pub profile: Profile,
    pub firefox: Option<PathBuf>,
    pub gate: Arc<Mutex<()>>,
}

impl Account {
    pub fn cookies(&self) -> Option<&Path> {
        self.firefox
            .as_deref()
            .filter(|folder| folder.join("cookies.sqlite").exists())
    }
}

#[derive(Clone)]
pub struct Profiles {
    pub db: Database,
    pub pairing: Pairing,
    signin_url: String,
    root: PathBuf,
    runtime: Arc<Mutex<HashMap<String, Runtime>>>,
}
struct Runtime {
    gate: Arc<Mutex<()>>,
    login: LoginStatus,
}
impl Runtime {
    fn new() -> Self {
        Self {
            gate: Arc::new(Mutex::new(())),
            login: LoginStatus::new(LoginState::Unknown, "Sign-in has not been checked"),
        }
    }
}

/// True once the sign-in window has stored YouTube's login cookie. Firefox keeps the cookie
/// database open, so read a copy of it and its write-ahead log instead of the live files.
fn logged_in(folder: &Path) -> bool {
    let Ok(copy) = tempfile_dir() else {
        return false;
    };
    let check = || -> Result<bool> {
        for name in ["cookies.sqlite", "cookies.sqlite-wal"] {
            let source = folder.join(name);
            if source.exists() {
                std::fs::copy(&source, copy.join(name))?;
            }
        }
        let db = rusqlite::Connection::open_with_flags(
            copy.join("cookies.sqlite"),
            rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
        )?;
        let found = db.query_row(
            "SELECT COUNT(*) FROM moz_cookies WHERE name = 'LOGIN_INFO' AND host LIKE '%youtube.com'",
            [],
            |row| row.get::<_, i64>(0),
        )?;
        Ok(found > 0)
    };
    let found = check().unwrap_or(false);
    let _ = std::fs::remove_dir_all(&copy);
    found
}

fn tempfile_dir() -> Result<PathBuf> {
    let path = std::env::temp_dir().join(format!("couchtube-cookies-{}", std::process::id()));
    private_directory(&path)?;
    Ok(path)
}

/// Ask Firefox to quit so it flushes its profile; kill it if it doesn't.
async fn close_firefox(child: &mut tokio::process::Child) {
    if let Some(pid) = child.id() {
        let _ = Command::new("kill").args(["-TERM", &pid.to_string()]).status().await;
    }
    if tokio::time::timeout(std::time::Duration::from_secs(10), child.wait())
        .await
        .is_err()
    {
        let _ = child.kill().await;
    }
}

pub fn private_directory(path: &Path) -> Result<()> {
    std::fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(path)?;
    let metadata = std::fs::symlink_metadata(path)?;
    if !metadata.is_dir() || metadata.file_type().is_symlink() {
        bail!("profile directory must be a real directory");
    }
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700))?;
    Ok(())
}

impl Profiles {
    pub fn open(root: PathBuf, pairing: Pairing, port: u16) -> Result<Self> {
        private_directory(&root)?;
        let root = root.canonicalize()?;
        private_directory(&root.join("profiles"))?;
        let db = Database::open(&root.join("couchtube.db"))?;
        std::fs::set_permissions(
            root.join("couchtube.db"),
            std::fs::Permissions::from_mode(0o600),
        )?;
        Ok(Self {
            db,
            pairing,
            signin_url: format!("http://127.0.0.1:{port}/signin"),
            root,
            runtime: Arc::new(Mutex::new(HashMap::new())),
        })
    }

    fn directory(&self, id: &str) -> PathBuf {
        // Only database-generated IDs can form paths; callers verify the profile exists.
        self.root.join("profiles").join(id).join("firefox")
    }

    pub async fn account(&self, id: &str) -> Result<Option<Account>> {
        let Some(profile) = self.db.get(id).await? else {
            return Ok(None);
        };
        let mut runtime = self.runtime.lock().await;
        let gate = runtime
            .entry(id.into())
            .or_insert_with(Runtime::new)
            .gate
            .clone();
        Ok(Some(Account {
            firefox: (id != "guest").then(|| self.directory(id)),
            profile,
            gate,
        }))
    }

    pub async fn active(&self) -> Result<Account> {
        let profile = self.db.active().await?;
        self.account(&profile.id)
            .await?
            .context("active profile missing")
    }

    pub async fn create(&self, name: &str, color: &str) -> Result<Profile> {
        let profile = self.db.create(name, color).await?;
        let folder = self.directory(&profile.id);
        let result = (|| -> Result<()> {
            private_directory(folder.parent().context("profile parent missing")?)?;
            private_directory(&folder)?;
            std::fs::write(folder.join("user.js"), USER_JS)?;
            Ok(())
        })();
        if let Err(err) = result {
            self.db.remove(&profile.id).await?;
            return Err(err);
        }
        Ok(profile)
    }

    pub async fn login_status(&self, id: &str) -> LoginStatus {
        if id == "guest" {
            return LoginStatus::new(LoginState::Guest, "Guest browses signed out");
        }
        self.runtime
            .lock()
            .await
            .entry(id.into())
            .or_insert_with(Runtime::new)
            .login
            .clone()
    }

    async fn set_status(&self, id: &str, status: LoginStatus) {
        self.runtime
            .lock()
            .await
            .entry(id.into())
            .or_insert_with(Runtime::new)
            .login = status;
    }

    pub async fn signin(&self, account: Account) -> Result<()> {
        let folder = account.firefox.clone().context("Guest cannot sign in")?;
        std::fs::write(folder.join("user.js"), USER_JS)?;
        let mut child = Command::new("firefox")
            .args(["--new-instance", "--no-remote", "--profile"])
            .arg(&folder)
            .arg(&self.signin_url)
            // X11 mode, so the phone keyboard can type into the window with xdotool.
            .env("MOZ_ENABLE_WAYLAND", "0")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .kill_on_drop(true)
            .spawn()
            .context("could not open Firefox")?;
        if let Err(err) = self.pairing.attach(child.id()).await {
            tracing::warn!("phone pairing unavailable: {err:#}");
        }
        self.set_status(
            &account.profile.id,
            LoginStatus::new(
                LoginState::SigningIn,
                "Sign in in Firefox. It closes by itself when you are signed in",
            ),
        )
        .await;
        let profiles = self.clone();
        tokio::spawn(async move {
            let mut ticker = tokio::time::interval(std::time::Duration::from_secs(2));
            let mut seen = 0u32;
            let exit = loop {
                tokio::select! {
                    exit = child.wait() => break exit.map(|status| status.success()),
                    _ = ticker.tick() => {
                        let path = folder.clone();
                        let found = tokio::task::spawn_blocking(move || logged_in(&path)).await.unwrap_or(false);
                        // Give Firefox a few more seconds to finish writing its cookies.
                        seen = if found { seen + 1 } else { 0 };
                        if seen >= 3 {
                            close_firefox(&mut child).await;
                            break Ok(true);
                        }
                    }
                }
            };
            profiles.pairing.end().await;
            match exit {
                Ok(true) => { profiles.check(account).await; }
                _ => profiles.set_status(&account.profile.id, LoginStatus::new(LoginState::CheckFailed, "Firefox did not close normally. Open sign-in again or check the login.")).await,
            }
        });
        Ok(())
    }

    pub async fn start_check(&self, account: Account) {
        self.set_status(
            &account.profile.id,
            LoginStatus::new(LoginState::Checking, "Checking YouTube sign-in…"),
        )
        .await;
        let profiles = self.clone();
        tokio::spawn(async move {
            profiles.check(account).await;
        });
    }

    async fn check(&self, account: Account) {
        self.set_status(
            &account.profile.id,
            LoginStatus::new(LoginState::Checking, "Checking YouTube sign-in…"),
        )
        .await;
        let _request = account.gate.lock().await;
        let folder = account
            .firefox
            .as_deref()
            .expect("only account profiles are checked");
        let status = if !folder.join("cookies.sqlite").exists() {
            LoginStatus::new(
                LoginState::SignInAgain,
                "No sign-in cookies yet. Open sign-in to connect YouTube.",
            )
        } else {
            match ytdlp::check_login(folder).await {
                Ok(()) => LoginStatus::new(LoginState::SignedIn, "YouTube sign-in works"),
                Err(_) => {
                    // yt-dlp output may contain account information: don't log it.
                    tracing::warn!(profile_id = %account.profile.id, "YouTube sign-in check failed");
                    LoginStatus::new(
                        LoginState::CheckFailed,
                        "Could not verify YouTube sign-in. Check your connection, update yt-dlp, or sign in again.",
                    )
                }
            }
        };
        if let Err(err) = self.db.invalidate(&account.profile.id).await {
            tracing::error!("could not invalidate account cache: {err:#}");
        }
        self.set_status(&account.profile.id, status).await;
    }

    pub async fn remove(&self, account: &Account) -> Result<()> {
        // Wait for searches using these cookie files before removing them.
        let _request = account.gate.lock().await;
        let directory = self.root.join("profiles").join(&account.profile.id);
        if directory.exists() {
            tokio::fs::remove_dir_all(directory).await?;
        }
        self.db.remove(&account.profile.id).await?;
        self.runtime.lock().await.remove(&account.profile.id);
        Ok(())
    }
}

pub fn valid_name(name: &str) -> bool {
    let length = name.trim().chars().count();
    (1..=40).contains(&length) && !name.chars().any(char::is_control)
}
pub fn valid_color(color: &str) -> bool {
    color.len() == 7 && color.starts_with('#') && color[1..].bytes().all(|b| b.is_ascii_hexdigit())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn detects_the_youtube_login_cookie() {
        let dir = std::env::temp_dir().join(format!("couchtube-test-{}", std::process::id()));
        private_directory(&dir).unwrap();
        let db = rusqlite::Connection::open(dir.join("cookies.sqlite")).unwrap();
        db.execute("CREATE TABLE moz_cookies (name TEXT, host TEXT)", []).unwrap();
        assert!(!logged_in(&dir));
        db.execute("INSERT INTO moz_cookies VALUES ('LOGIN_INFO', '.youtube.com')", []).unwrap();
        assert!(logged_in(&dir));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[tokio::test]
    async fn profiles_use_private_isolated_directories_and_guest_has_no_cookies() {
        let root = std::env::temp_dir().join(format!("ct-profiles-{}", std::process::id()));
        let profiles = Profiles::open(root.clone(), Pairing::new(0), 0).unwrap();
        let first = profiles.create("Alice", "#aabbcc").await.unwrap();
        let second = profiles.create("Bob", "#ddeeff").await.unwrap();
        let first = profiles.account(&first.id).await.unwrap().unwrap();
        let second = profiles.account(&second.id).await.unwrap().unwrap();
        assert_ne!(first.firefox, second.firefox);
        for folder in [
            &root,
            &root.join("profiles"),
            first.firefox.as_ref().unwrap(),
        ] {
            assert_eq!(
                std::fs::metadata(folder).unwrap().permissions().mode() & 0o777,
                0o700
            );
        }
        assert!(profiles.active().await.unwrap().firefox.is_none());
        assert!(first.firefox.as_ref().unwrap().join("user.js").exists());
        assert!(profiles.account("../../escape").await.unwrap().is_none());
        profiles.db.select(&first.profile.id).await.unwrap();
        profiles.remove(&first).await.unwrap();
        assert_eq!(profiles.active().await.unwrap().profile.id, "guest");
        assert!(second.firefox.as_ref().unwrap().exists());
        drop(profiles);
        std::fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn validates_display_fields() {
        assert!(valid_name(" Alice "));
        assert!(!valid_name(" \n"));
        assert!(!valid_name(&"x".repeat(41)));
        assert!(valid_color("#aaBB09"));
        assert!(!valid_color("red; position:fixed"));
        assert!(!valid_color("#ééé"));
    }
}
