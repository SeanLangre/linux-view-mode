//! Twitch sign-in, the same way as YouTube's (see `docs/README-twitch.md`, step 4): each Twitch
//! profile has its own Firefox folder, a sign-in window opens on twitch.tv, the phone can type into
//! it, and once the login cookie appears the window closes itself. The login is Twitch's
//! `auth-token` cookie; the backend reads it from a copy of `cookies.sqlite` when it needs it and
//! sends it to Twitch's web API. The token is never stored elsewhere, logged or put in an argument.
//!
//! **Not yet verified with a real login** (2026-11-08): that the cookie is called `auth-token`, that
//! Twitch's login form takes the phone's Tab/Enter typing, and that the web API accepts the cookie.
//! Each of those is one constant or one function here, so a failed first try is a small change.
use super::*;
use crate::{
    db::TwitchProfile,
    gql::{self, Gql, SignedOut},
    pairing::Flow,
};

/// Opens straight on Twitch's login form.
const LOGIN_URL: &str = "https://www.twitch.tv/login";
/// The cookie Twitch's website keeps its login token in, and the hosts it is set for.
const COOKIE_NAME: &str = "auth-token";
const COOKIE_HOST: &str = "%twitch.tv";

fn key(id: &str) -> String {
    format!("twitch:{id}")
}

/// The login token in a profile's cookie database, if it is there and looks like one.
fn read_token(folder: &Path) -> Option<String> {
    with_cookies(folder, |db| {
        Ok(db
            .prepare("SELECT value FROM moz_cookies WHERE name = ?1 AND host LIKE ?2 ORDER BY lastAccessed DESC")?
            .query_map([COOKIE_NAME, COOKIE_HOST], |row| row.get::<_, String>(0))?
            .filter_map(Result::ok)
            .find(|value| gql::valid_token(value)))
    })
    .flatten()
}

impl Profiles {
    fn twitch_directory(&self, id: &str) -> PathBuf {
        // Only database-generated ids reach this; callers check the profile exists first.
        self.root.join("twitch").join(id).join("firefox")
    }

    /// A Twitch profile and its Firefox folder (`0700`, with the same preferences as YouTube's).
    pub async fn twitch_create(&self, name: &str, color: &str) -> Result<TwitchProfile> {
        let profile = self.db.twitch_create(name, color).await?;
        let folder = self.twitch_directory(&profile.id);
        let made = (|| -> Result<()> {
            private_directory(&self.root.join("twitch"))?;
            private_directory(folder.parent().context("profile parent missing")?)?;
            private_directory(&folder)?;
            std::fs::write(folder.join("user.js"), USER_JS)?;
            Ok(())
        })();
        if let Err(err) = made {
            self.db.twitch_remove(&profile.id).await?;
            return Err(err);
        }
        Ok(profile)
    }

    /// Removes the profile and its sign-in files. Guest cannot be removed.
    pub async fn twitch_remove(&self, id: &str, web: &Gql) -> Result<()> {
        let directory = self.root.join("twitch").join(id);
        if id != "guest" && directory.exists() {
            tokio::fs::remove_dir_all(directory).await?;
        }
        self.db.twitch_remove(id).await?;
        self.runtime.lock().await.remove(&key(id));
        web.forget(id);
        Ok(())
    }

    pub async fn twitch_login_status(&self, id: &str) -> LoginStatus {
        if id == "guest" {
            return LoginStatus::new(LoginState::Guest, "Guest watches without a Twitch account");
        }
        self.runtime.lock().await.entry(key(id)).or_insert_with(Runtime::new).login.clone()
    }

    async fn twitch_set_status(&self, id: &str, status: LoginStatus) {
        self.runtime.lock().await.entry(key(id)).or_insert_with(Runtime::new).login = status;
    }

    /// The profile's login token, if it has signed in. Read fresh each time and never kept.
    pub async fn twitch_token(&self, id: &str) -> Option<String> {
        if id == "guest" {
            return None;
        }
        let folder = self.twitch_directory(id);
        tokio::task::spawn_blocking(move || read_token(&folder)).await.ok().flatten()
    }

    /// Opens Firefox on Twitch's login page for this profile and tells the phone page it is Twitch.
    pub async fn twitch_signin(&self, profile: TwitchProfile, web: Gql) -> Result<()> {
        if profile.id == "guest" {
            bail!("Guest stays signed out");
        }
        let id = profile.id;
        let folder = self.twitch_directory(&id);
        std::fs::write(folder.join("user.js"), USER_JS)?;
        let mut child = Command::new("firefox")
            .args(["--new-instance", "--no-remote", "--profile"])
            .arg(&folder)
            .arg(LOGIN_URL)
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
        self.pairing.set_flow(Flow::Twitch).await;
        self.twitch_set_status(
            &id,
            LoginStatus::new(LoginState::SigningIn, "Sign in to Twitch in Firefox. It closes by itself when you are signed in"),
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
                        let found = tokio::task::spawn_blocking(move || read_token(&path).is_some()).await.unwrap_or(false);
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
                Ok(true) => profiles.twitch_check(&id, &web).await,
                _ => profiles.twitch_set_status(&id, LoginStatus::new(LoginState::CheckFailed, "Firefox did not close normally. Open sign-in again or check the login.")).await,
            }
        });
        Ok(())
    }

    pub async fn twitch_start_check(&self, id: &str, web: Gql) {
        self.twitch_set_status(id, LoginStatus::new(LoginState::Checking, "Checking Twitch sign-in…")).await;
        let profiles = self.clone();
        let id = id.to_owned();
        tokio::spawn(async move { profiles.twitch_check(&id, &web).await });
    }

    /// Asks Twitch who the token belongs to.
    async fn twitch_check(&self, id: &str, web: &Gql) {
        self.twitch_set_status(id, LoginStatus::new(LoginState::Checking, "Checking Twitch sign-in…")).await;
        let status = match self.twitch_token(id).await {
            None => LoginStatus::new(LoginState::SignInAgain, "No Twitch sign-in yet. Open sign-in to connect Twitch."),
            Some(token) => match web.current_user(&token).await {
                Ok(Some(name)) => LoginStatus::new(LoginState::SignedIn, &format!("Signed in to Twitch as {name}")),
                Ok(None) => LoginStatus::new(LoginState::SignInAgain, "Twitch does not see you as signed in. Open sign-in again."),
                Err(err) if err.downcast_ref::<SignedOut>().is_some() => {
                    LoginStatus::new(LoginState::SignInAgain, "Twitch did not accept the sign-in. Open sign-in again.")
                }
                Err(err) => {
                    tracing::warn!(profile_id = %id, "Twitch sign-in check failed: {err:#}");
                    LoginStatus::new(LoginState::CheckFailed, "Could not verify the Twitch sign-in. Check your connection or sign in again.")
                }
            },
        };
        web.forget(id);
        self.twitch_set_status(id, status).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A Firefox-shaped cookie database with these cookies.
    fn cookie_folder(name: &str, cookies: &[(&str, &str, &str)]) -> PathBuf {
        let folder = std::env::temp_dir().join(format!("ct-twitch-login-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&folder);
        std::fs::create_dir_all(&folder).unwrap();
        let db = rusqlite::Connection::open(folder.join("cookies.sqlite")).unwrap();
        db.execute_batch("CREATE TABLE moz_cookies (name TEXT, value TEXT, host TEXT, lastAccessed INTEGER)").unwrap();
        for (i, (host, name, value)) in cookies.iter().enumerate() {
            db.execute("INSERT INTO moz_cookies VALUES (?1, ?2, ?3, ?4)", rusqlite::params![name, value, host, i as i64]).unwrap();
        }
        folder
    }

    #[test]
    fn reads_the_login_token_only_when_it_looks_like_one() {
        let token = "abcdefghijklmnopqrstuvwxyz0123";
        let folder = cookie_folder("ok", &[(".twitch.tv", "unique_id", "xyz"), (".twitch.tv", COOKIE_NAME, token)]);
        assert_eq!(read_token(&folder).as_deref(), Some(token));
        // The same name on another site, or no cookie at all, is not a login.
        let elsewhere = cookie_folder("elsewhere", &[(".example.com", COOKIE_NAME, token)]);
        assert_eq!(read_token(&elsewhere), None);
        let none = cookie_folder("none", &[(".twitch.tv", "unique_id", "xyz")]);
        assert_eq!(read_token(&none), None);
        // Values that could not be a token (and could not be safely written into a curl config) are ignored.
        for bad in ["short", "has space has space has space", "quote\"quote\"quote\"quote\"quote", ""] {
            let folder = cookie_folder("bad", &[(".twitch.tv", COOKIE_NAME, bad)]);
            assert_eq!(read_token(&folder), None, "{bad:?}");
        }
        // No database yet (the window has not made one): nothing, not an error.
        assert_eq!(read_token(&std::env::temp_dir().join("ct-twitch-login-missing")), None);
        for name in ["ok", "elsewhere", "none", "bad"] {
            let _ = std::fs::remove_dir_all(std::env::temp_dir().join(format!("ct-twitch-login-{name}-{}", std::process::id())));
        }
    }

    #[tokio::test]
    async fn twitch_profiles_get_private_folders_apart_from_youtube_and_removal_cleans_up() {
        let root = std::env::temp_dir().join(format!("ct-twitch-profiles-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let profiles = Profiles::open(root.clone(), Pairing::new(0)).unwrap();
        let web = Gql::from_env();
        let alice = profiles.twitch_create("Alice", "#9146ff").await.unwrap();
        let youtube = profiles.create("Bob", "#ddeeff").await.unwrap();
        let folder = profiles.twitch_directory(&alice.id);
        assert!(folder.join("user.js").exists());
        assert_eq!(std::fs::metadata(&folder).unwrap().permissions().mode() & 0o777, 0o700);
        assert_eq!(std::fs::metadata(root.join("twitch")).unwrap().permissions().mode() & 0o777, 0o700);
        assert_ne!(folder, profiles.directory(&youtube.id), "no shared folders");
        // Nothing is signed in yet; Guest never is.
        assert_eq!(profiles.twitch_token(&alice.id).await, None);
        assert_eq!(profiles.twitch_token("guest").await, None);
        assert!(matches!(profiles.twitch_login_status("guest").await.state, LoginState::Guest));
        assert!(matches!(profiles.twitch_login_status(&alice.id).await.state, LoginState::Unknown));
        profiles.twitch_remove(&alice.id, &web).await.unwrap();
        assert!(!root.join("twitch").join(&alice.id).exists());
        assert!(profiles.directory(&youtube.id).exists(), "YouTube profiles are untouched");
        assert!(profiles.twitch_remove("guest", &web).await.is_err());
        drop(profiles);
        std::fs::remove_dir_all(root).unwrap();
    }
}
