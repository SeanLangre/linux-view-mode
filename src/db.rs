//! Small SQLite store. All database work runs off the async executor.
use crate::ytdlp::Video;
use anyhow::{Context, Result, bail};
use rusqlite::{Connection, OptionalExtension, params};
use serde::Serialize;
use std::{
    path::Path,
    sync::{Arc, Mutex},
};

#[derive(Clone, Debug, Serialize)]
pub struct Profile {
    pub id: String,
    pub name: String,
    pub color: String,
    /// Preferred subtitle language (for example `en`); `None` keeps subtitles off.
    pub subtitles: Option<String>,
}

#[derive(Clone)]
pub struct Database(Arc<Mutex<Connection>>);

impl Database {
    pub fn open(path: &Path) -> Result<Self> {
        let connection = Connection::open(path)?;
        connection.busy_timeout(std::time::Duration::from_secs(5))?;
        connection.execute_batch(
            "PRAGMA foreign_keys = ON;
            CREATE TABLE IF NOT EXISTS profiles (
                id TEXT PRIMARY KEY, name TEXT NOT NULL, color TEXT NOT NULL,
                created_at INTEGER NOT NULL DEFAULT (unixepoch())
            );
            CREATE TABLE IF NOT EXISTS videos (
                profile_id TEXT NOT NULL REFERENCES profiles(id) ON DELETE CASCADE,
                video_id TEXT NOT NULL, json TEXT NOT NULL, PRIMARY KEY(profile_id, video_id)
            );
            CREATE TABLE IF NOT EXISTS progress (
                profile_id TEXT NOT NULL REFERENCES profiles(id) ON DELETE CASCADE,
                video_id TEXT NOT NULL, position REAL NOT NULL, duration REAL,
                watched REAL NOT NULL DEFAULT 0, marked INTEGER NOT NULL DEFAULT 0,
                completed INTEGER NOT NULL DEFAULT 0, updated_at INTEGER NOT NULL DEFAULT (unixepoch()),
                PRIMARY KEY(profile_id, video_id)
            );
            CREATE TABLE IF NOT EXISTS queue (
                position INTEGER PRIMARY KEY AUTOINCREMENT,
                profile_id TEXT NOT NULL REFERENCES profiles(id) ON DELETE CASCADE,
                video_id TEXT NOT NULL, UNIQUE(profile_id, video_id)
            );
            CREATE TABLE IF NOT EXISTS feed_cache (
                profile_id TEXT NOT NULL REFERENCES profiles(id) ON DELETE CASCADE,
                cache_key TEXT NOT NULL, json TEXT NOT NULL, expires_at INTEGER NOT NULL,
                PRIMARY KEY(profile_id, cache_key)
            );
            CREATE TABLE IF NOT EXISTS settings (key TEXT PRIMARY KEY, value TEXT NOT NULL);
            CREATE TABLE IF NOT EXISTS remote_devices (
                token_hash TEXT PRIMARY KEY, created_at INTEGER NOT NULL DEFAULT (unixepoch())
            );
            INSERT OR IGNORE INTO profiles(id, name, color) VALUES ('guest', 'Guest', '#707887');
            INSERT OR IGNORE INTO settings(key, value) VALUES ('active_profile', 'guest');",
        )?;
        let has_subtitles = connection
            .prepare("SELECT 1 FROM pragma_table_info('profiles') WHERE name = 'subtitles'")?
            .exists([])?;
        if !has_subtitles {
            connection.execute("ALTER TABLE profiles ADD COLUMN subtitles TEXT", [])?;
        }
        Ok(Self(Arc::new(Mutex::new(connection))))
    }

    async fn with<T, F>(&self, operation: F) -> Result<T>
    where
        T: Send + 'static,
        F: FnOnce(&mut Connection) -> Result<T> + Send + 'static,
    {
        let database = self.clone();
        tokio::task::spawn_blocking(move || {
            let mut connection = database
                .0
                .lock()
                .map_err(|_| anyhow::anyhow!("database lock poisoned"))?;
            operation(&mut connection)
        })
        .await
        .context("database task failed")?
    }

    pub async fn list(&self) -> Result<Vec<Profile>> {
        self.with(|db| {
            let mut statement = db.prepare(
                "SELECT id, name, color, subtitles FROM profiles ORDER BY id != 'guest', created_at, rowid",
            )?;
            Ok(statement
                .query_map([], |r| {
                    Ok(Profile {
                        id: r.get(0)?,
                        name: r.get(1)?,
                        color: r.get(2)?,
                        subtitles: r.get(3)?,
                    })
                })?
                .collect::<rusqlite::Result<_>>()?)
        })
        .await
    }

    pub async fn get(&self, id: &str) -> Result<Option<Profile>> {
        let id = id.to_owned();
        self.with(move |db| {
            Ok(db
                .query_row(
                    "SELECT id, name, color, subtitles FROM profiles WHERE id = ?1",
                    [id],
                    |r| {
                        Ok(Profile {
                            id: r.get(0)?,
                            name: r.get(1)?,
                            color: r.get(2)?,
                            subtitles: r.get(3)?,
                        })
                    },
                )
                .optional()?)
        })
        .await
    }

    pub async fn create(&self, name: &str, color: &str) -> Result<Profile> {
        let name = name.to_owned();
        let color = color.to_owned();
        self.with(move |db| {
            let id: String = db.query_row("SELECT lower(hex(randomblob(16)))", [], |r| r.get(0))?;
            db.execute(
                "INSERT INTO profiles(id, name, color) VALUES (?1, ?2, ?3)",
                params![id, name, color],
            )?;
            Ok(Profile { id, name, color, subtitles: None })
        })
        .await
    }

    pub async fn set_subtitles(&self, id: &str, language: Option<&str>) -> Result<()> {
        let id = id.to_owned();
        let language = language.map(str::to_owned);
        self.with(move |db| {
            db.execute("UPDATE profiles SET subtitles = ?2 WHERE id = ?1", params![id, language])?;
            Ok(())
        })
        .await
    }

    pub async fn remote_enabled(&self) -> Result<bool> {
        self.with(|db| {
            Ok(db
                .query_row("SELECT value FROM settings WHERE key = 'remote_enabled'", [], |r| r.get::<_, String>(0))
                .optional()?
                .is_some_and(|v| v == "1"))
        })
        .await
    }

    pub async fn set_remote_enabled(&self, enabled: bool) -> Result<()> {
        self.with(move |db| {
            db.execute(
                "INSERT INTO settings(key, value) VALUES ('remote_enabled', ?1) ON CONFLICT(key) DO UPDATE SET value = ?1",
                [if enabled { "1" } else { "0" }],
            )?;
            Ok(())
        })
        .await
    }

    pub async fn add_device(&self, token_hash: &str) -> Result<()> {
        let hash = token_hash.to_owned();
        self.with(move |db| { db.execute("INSERT OR IGNORE INTO remote_devices(token_hash) VALUES (?1)", [hash])?; Ok(()) }).await
    }

    pub async fn has_device(&self, token_hash: &str) -> Result<bool> {
        let hash = token_hash.to_owned();
        self.with(move |db| Ok(db.query_row("SELECT EXISTS(SELECT 1 FROM remote_devices WHERE token_hash = ?1)", [hash], |r| r.get(0))?)).await
    }

    pub async fn device_count(&self) -> Result<u32> {
        self.with(|db| Ok(db.query_row("SELECT COUNT(*) FROM remote_devices", [], |r| r.get(0))?)).await
    }

    pub async fn clear_devices(&self) -> Result<()> {
        self.with(|db| { db.execute("DELETE FROM remote_devices", [])?; Ok(()) }).await
    }

    pub async fn active(&self) -> Result<Profile> {
        self.with(|db| Ok(db.query_row("SELECT p.id, p.name, p.color, p.subtitles FROM profiles p JOIN settings s ON s.value = p.id WHERE s.key = 'active_profile'", [],
            |r| Ok(Profile { id: r.get(0)?, name: r.get(1)?, color: r.get(2)?, subtitles: r.get(3)? }))?)).await
    }

    pub async fn select(&self, id: &str) -> Result<()> {
        let id = id.to_owned();
        self.with(move |db| {
            let exists: bool = db.query_row(
                "SELECT EXISTS(SELECT 1 FROM profiles WHERE id = ?1)",
                [&id],
                |r| r.get(0),
            )?;
            if !exists {
                bail!("no such profile");
            }
            db.execute(
                "UPDATE settings SET value = ?1 WHERE key = 'active_profile'",
                [id],
            )?;
            Ok(())
        })
        .await
    }

    pub async fn remove(&self, id: &str) -> Result<()> {
        let id = id.to_owned();
        self.with(move |db| {
            if id == "guest" {
                bail!("Guest cannot be removed");
            }
            let transaction = db.transaction()?;
            transaction.execute(
                "UPDATE settings SET value = 'guest' WHERE key = 'active_profile' AND value = ?1",
                [&id],
            )?;
            transaction.execute("DELETE FROM profiles WHERE id = ?1", [id])?;
            transaction.commit()?;
            Ok(())
        })
        .await
    }
    pub async fn cache_get(&self, profile: &str, key: &str) -> Result<Option<String>> {
        let profile = profile.to_owned();
        let key = key.to_owned();
        self.with(move |db| Ok(db.query_row("SELECT json FROM feed_cache WHERE profile_id=?1 AND cache_key=?2 AND expires_at > unixepoch()", params![profile, key], |r| r.get(0)).optional()?)).await
    }

    pub async fn cache_put(
        &self,
        profile: &str,
        key: &str,
        json: String,
        seconds: u32,
        videos: Vec<Video>,
    ) -> Result<()> {
        let profile = profile.to_owned();
        let key = key.to_owned();
        self.with(move |db| {
            let tx = db.transaction()?;
            for video in videos {
                tx.execute("INSERT INTO videos VALUES (?1,?2,?3) ON CONFLICT(profile_id,video_id) DO UPDATE SET json=excluded.json", params![profile, video.id, serde_json::to_string(&video)?])?;
            }
            tx.execute("INSERT INTO feed_cache VALUES (?1,?2,?3,unixepoch()+?4) ON CONFLICT(profile_id,cache_key) DO UPDATE SET json=excluded.json,expires_at=excluded.expires_at", params![profile,key,json,seconds])?;
            // Bound search/channel cache growth, without retaining expired private feeds.
            tx.execute("DELETE FROM feed_cache WHERE profile_id=?1 AND (expires_at <= unixepoch() OR (cache_key NOT LIKE 'feed:%' AND cache_key NOT IN (SELECT cache_key FROM feed_cache WHERE profile_id=?1 AND cache_key NOT LIKE 'feed:%' ORDER BY expires_at DESC LIMIT 100)))", [&profile])?;
            tx.commit()?;
            Ok(())
        }).await
    }

    pub async fn invalidate(&self, profile: &str) -> Result<()> {
        let profile = profile.to_owned();
        self.with(move |db| {
            db.execute("DELETE FROM feed_cache WHERE profile_id=?1", [profile])?;
            Ok(())
        })
        .await
    }

    pub async fn video(&self, profile: &str, id: &str) -> Result<Video> {
        let profile = profile.to_owned();
        let id = id.to_owned();
        self.with(move |db| {
            let json: Option<String> = db
                .query_row(
                    "SELECT json FROM videos WHERE profile_id=?1 AND video_id=?2",
                    params![profile, id],
                    |r| r.get(0),
                )
                .optional()?;
            Ok(match json {
                Some(json) => serde_json::from_str(&json)?,
                None => Video::placeholder(&id),
            })
        })
        .await
    }

    pub async fn save_progress(
        &self,
        profile: &str,
        video: Video,
        position: f64,
        duration: Option<f64>,
        watched: f64,
        completed: bool,
    ) -> Result<()> {
        let profile = profile.to_owned();
        self.with(move |db| {
            let tx = db.transaction()?;
            tx.execute("INSERT INTO videos VALUES (?1,?2,?3) ON CONFLICT(profile_id,video_id) DO UPDATE SET json=excluded.json", params![profile,video.id,serde_json::to_string(&video)?])?;
            tx.execute("INSERT INTO progress(profile_id,video_id,position,duration,watched,completed) VALUES (?1,?2,?3,?4,?5,?6) ON CONFLICT(profile_id,video_id) DO UPDATE SET position=excluded.position,duration=excluded.duration,watched=progress.watched+excluded.watched,completed=excluded.completed,updated_at=unixepoch()", params![profile,video.id,position,duration,watched,completed])?;
            tx.commit()?; Ok(())
        }).await
    }

    pub async fn resume(&self, profile: &str, id: &str) -> Result<Option<f64>> {
        let profile = profile.to_owned();
        let id = id.to_owned();
        self.with(move |db| Ok(db.query_row("SELECT position FROM progress WHERE profile_id=?1 AND video_id=?2 AND completed=0 AND position>=5 AND (duration IS NULL OR position < duration-5)", params![profile,id], |r| r.get(0)).optional()?)).await
    }

    pub async fn progress_list(&self, profile: &str, continuing: bool) -> Result<Vec<Video>> {
        let profile = profile.to_owned();
        self.with(move |db| {
            let mut query = db.prepare("SELECT v.json,p.position,p.completed FROM progress p JOIN videos v ON v.profile_id=p.profile_id AND v.video_id=p.video_id WHERE p.profile_id=?1 AND (NOT ?2 OR (p.completed=0 AND p.position>=5 AND (p.duration IS NULL OR p.position < p.duration-5))) ORDER BY p.updated_at DESC,p.rowid DESC LIMIT 24")?;
            let rows = query.query_map(params![profile,continuing], |r| Ok((r.get::<_,String>(0)?,r.get::<_,f64>(1)?,r.get::<_,bool>(2)?)))?;
            rows.map(|row| { let (json,pos,completed) = row?; let mut video: Video = serde_json::from_str(&json)?; video.resume_position = (!completed && pos>=5.0).then_some(pos); Ok(video) }).collect()
        }).await
    }

    /// A new viewing counts from zero, so a rewatch of 30 s or more is sent to YouTube again.
    pub async fn begin_viewing(&self, profile: &str, id: &str) -> Result<()> {
        let profile = profile.to_owned();
        let id = id.to_owned();
        self.with(move |db| { db.execute("UPDATE progress SET watched=0, marked=0 WHERE profile_id=?1 AND video_id=?2", params![profile,id])?; Ok(()) }).await
    }

    pub async fn should_mark(&self, profile: &str, id: &str) -> Result<bool> {
        let profile = profile.to_owned();
        let id = id.to_owned();
        self.with(move |db| Ok(db.query_row("SELECT watched>=30 AND marked=0 FROM progress WHERE profile_id=?1 AND video_id=?2", params![profile,id], |r| r.get(0)).optional()?.unwrap_or(false))).await
    }

    pub async fn set_marked(&self, profile: &str, id: &str) -> Result<()> {
        let profile = profile.to_owned();
        let id = id.to_owned();
        self.with(move |db| { db.execute("UPDATE progress SET marked=1 WHERE profile_id=?1 AND video_id=?2", params![profile,id])?; db.execute("DELETE FROM feed_cache WHERE profile_id=?1 AND (cache_key LIKE 'feed:recommended%' OR cache_key LIKE 'feed:history%')", [profile])?; Ok(()) }).await
    }

    pub async fn queue(&self, profile: &str) -> Result<Vec<Video>> {
        let profile = profile.to_owned();
        self.with(move |db| {
            let mut statement = db.prepare("SELECT q.video_id,v.json FROM queue q LEFT JOIN videos v ON q.profile_id=v.profile_id AND q.video_id=v.video_id WHERE q.profile_id=?1 ORDER BY q.position")?;
            let rows = statement.query_map([profile], |r| Ok((r.get::<_,String>(0)?,r.get::<_,Option<String>>(1)?)))?;
            rows.map(|row| { let (id,json) = row?; Ok(match json {Some(json)=>serde_json::from_str(&json)?,None=>Video::placeholder(&id)}) }).collect()
        }).await
    }

    pub async fn enqueue(&self, profile: &str, id: &str) -> Result<()> {
        let profile = profile.to_owned();
        let id = id.to_owned();
        self.with(move |db| {
            db.execute(
                "INSERT OR IGNORE INTO queue(profile_id,video_id) VALUES (?1,?2)",
                params![profile, id],
            )?;
            Ok(())
        })
        .await
    }

    pub async fn dequeue(&self, profile: &str, id: Option<&str>) -> Result<()> {
        let profile = profile.to_owned();
        let id = id.map(str::to_owned);
        self.with(move |db| {
            db.execute(
                "DELETE FROM queue WHERE profile_id=?1 AND (?2 IS NULL OR video_id=?2)",
                params![profile, id],
            )?;
            Ok(())
        })
        .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn watch_data_queue_and_cache_are_scoped_and_cascade_on_profile_removal() {
        let db = Database::open(Path::new(":memory:")).unwrap();
        let alice = db.create("Alice", "#aabbcc").await.unwrap();
        let bob = db.create("Bob", "#aabbcc").await.unwrap();
        let mut video = Video::placeholder("aqz-KE-bpKQ");
        video.title = "Alice's video".into();
        db.cache_put(
            &alice.id,
            "feed:recommended",
            "[]".into(),
            600,
            vec![video.clone()],
        )
        .await
        .unwrap();
        assert!(
            db.cache_get(&bob.id, "feed:recommended")
                .await
                .unwrap()
                .is_none()
        );
        assert_ne!(
            db.video(&bob.id, &video.id).await.unwrap().title,
            video.title
        );
        db.save_progress(&alice.id, video.clone(), 42.0, Some(100.0), 15.0, false)
            .await
            .unwrap();
        assert_eq!(db.resume(&alice.id, &video.id).await.unwrap(), Some(42.0));
        assert!(db.resume(&bob.id, &video.id).await.unwrap().is_none());
        assert!(!db.should_mark(&alice.id, &video.id).await.unwrap());
        db.save_progress(&alice.id, video.clone(), 55.0, Some(100.0), 16.0, false)
            .await
            .unwrap();
        assert!(db.should_mark(&alice.id, &video.id).await.unwrap());
        db.set_marked(&alice.id, &video.id).await.unwrap();
        assert!(!db.should_mark(&alice.id, &video.id).await.unwrap());
        // A rewatch starts from zero and is sent to YouTube again after 30 s.
        db.begin_viewing(&alice.id, &video.id).await.unwrap();
        assert!(!db.should_mark(&alice.id, &video.id).await.unwrap());
        db.save_progress(&alice.id, video.clone(), 40.0, Some(100.0), 31.0, false)
            .await
            .unwrap();
        assert!(db.should_mark(&alice.id, &video.id).await.unwrap());
        assert!(
            db.cache_get(&alice.id, "feed:recommended")
                .await
                .unwrap()
                .is_none()
        );
        db.enqueue(&alice.id, &video.id).await.unwrap();
        db.enqueue(&alice.id, &video.id).await.unwrap();
        db.enqueue(&alice.id, "rFZHOHl-L8A").await.unwrap();
        assert_eq!(db.queue(&alice.id).await.unwrap().len(), 2);
        assert!(db.queue(&bob.id).await.unwrap().is_empty());
        db.dequeue(&alice.id, Some(&video.id)).await.unwrap();
        assert_eq!(db.queue(&alice.id).await.unwrap()[0].id, "rFZHOHl-L8A");
        db.save_progress(&alice.id, video.clone(), 100.0, Some(100.0), 0.0, true)
            .await
            .unwrap();
        assert!(db.progress_list(&alice.id, true).await.unwrap().is_empty());
        assert!(db.resume(&alice.id, &video.id).await.unwrap().is_none());
        db.cache_put(&alice.id, "expired", "[]".into(), 0, vec![])
            .await
            .unwrap();
        assert!(db.cache_get(&alice.id, "expired").await.unwrap().is_none());
        for index in 0..101 {
            db.cache_put(
                &alice.id,
                &format!("search:{index}"),
                "[]".into(),
                1800,
                vec![],
            )
            .await
            .unwrap();
        }
        db.cache_put(&alice.id, "feed:history", "[]".into(), 120, vec![])
            .await
            .unwrap();
        assert!(
            db.cache_get(&alice.id, "feed:history")
                .await
                .unwrap()
                .is_some()
        );
        let alice_id = alice.id.clone();
        let count:i64=db.with(move |db|Ok(db.query_row("SELECT count(*) FROM feed_cache WHERE profile_id=?1 AND cache_key LIKE 'search:%'",[alice_id],|r|r.get(0))?)).await.unwrap();
        assert_eq!(count, 100);
        db.remove(&alice.id).await.unwrap();
        assert!(db.queue(&alice.id).await.unwrap().is_empty());
        assert!(db.progress_list(&alice.id, false).await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn profiles_are_persistent_and_deletion_returns_to_guest() {
        let path = std::env::temp_dir().join(format!("ct-db-{}.sqlite", std::process::id()));
        let db = Database::open(&path).unwrap();
        assert_eq!(db.active().await.unwrap().id, "guest");
        let profile = db.create("A name ' with SQL", "#aabbcc").await.unwrap();
        assert_eq!(profile.id.len(), 32);
        db.select(&profile.id).await.unwrap();
        assert_eq!(db.active().await.unwrap().subtitles, None);
        db.set_subtitles(&profile.id, Some("de")).await.unwrap();
        drop(db);
        let db = Database::open(&path).unwrap();
        assert_eq!(db.active().await.unwrap().name, profile.name);
        assert_eq!(db.active().await.unwrap().subtitles.as_deref(), Some("de"));
        assert!(db.select("missing").await.is_err());
        db.remove(&profile.id).await.unwrap();
        assert_eq!(db.active().await.unwrap().id, "guest");
        assert!(db.remove("guest").await.is_err());
        assert_eq!(db.list().await.unwrap().len(), 1);
        drop(db);
        std::fs::remove_file(path).unwrap();
    }
}
