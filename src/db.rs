//! Small SQLite store. All database work runs off the async executor.
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
            CREATE TABLE IF NOT EXISTS settings (key TEXT PRIMARY KEY, value TEXT NOT NULL);
            INSERT OR IGNORE INTO profiles(id, name, color) VALUES ('guest', 'Guest', '#707887');
            INSERT OR IGNORE INTO settings(key, value) VALUES ('active_profile', 'guest');",
        )?;
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
                "SELECT id, name, color FROM profiles ORDER BY id != 'guest', created_at, rowid",
            )?;
            Ok(statement
                .query_map([], |r| {
                    Ok(Profile {
                        id: r.get(0)?,
                        name: r.get(1)?,
                        color: r.get(2)?,
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
                    "SELECT id, name, color FROM profiles WHERE id = ?1",
                    [id],
                    |r| {
                        Ok(Profile {
                            id: r.get(0)?,
                            name: r.get(1)?,
                            color: r.get(2)?,
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
            Ok(Profile { id, name, color })
        })
        .await
    }

    pub async fn active(&self) -> Result<Profile> {
        self.with(|db| Ok(db.query_row("SELECT p.id, p.name, p.color FROM profiles p JOIN settings s ON s.value = p.id WHERE s.key = 'active_profile'", [],
            |r| Ok(Profile { id: r.get(0)?, name: r.get(1)?, color: r.get(2)? }))?)).await
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
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn profiles_are_persistent_and_deletion_returns_to_guest() {
        let path = std::env::temp_dir().join(format!("ct-db-{}.sqlite", std::process::id()));
        let db = Database::open(&path).unwrap();
        assert_eq!(db.active().await.unwrap().id, "guest");
        let profile = db.create("A name ' with SQL", "#aabbcc").await.unwrap();
        assert_eq!(profile.id.len(), 32);
        db.select(&profile.id).await.unwrap();
        drop(db);
        let db = Database::open(&path).unwrap();
        assert_eq!(db.active().await.unwrap().name, profile.name);
        assert!(db.select("missing").await.is_err());
        db.remove(&profile.id).await.unwrap();
        assert_eq!(db.active().await.unwrap().id, "guest");
        assert!(db.remove("guest").await.is_err());
        assert_eq!(db.list().await.unwrap().len(), 1);
        drop(db);
        std::fs::remove_file(path).unwrap();
    }
}
