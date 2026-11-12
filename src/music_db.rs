//! Music mode's tables (see `docs/README-music.md`): profiles, shared song metadata, playlists, favorites,
//! play history and cached lyrics. Song metadata and downloaded files are shared by all music
//! profiles; everything else belongs to one profile.
use crate::db::Database;
use anyhow::{Result, bail};
use rusqlite::{OptionalExtension, Row, Transaction, params};
use serde::Serialize;
use std::{collections::HashSet, path::Path};

#[derive(Clone, Debug, Serialize, PartialEq)]
pub struct MusicProfile {
    pub id: String,
    pub name: String,
    pub color: String,
}

/// One song. `file_path` is set while the song is in the download cache.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MusicTrack {
    pub video_id: String,
    pub title: String,
    pub artist: Option<String>,
    pub duration_secs: Option<u32>,
    pub thumbnail_url: Option<String>,
    pub file_path: Option<String>,
    pub downloaded_at: Option<i64>,
}

impl MusicTrack {
    /// The downloaded file, if the song is cached and the file still exists.
    pub fn local_file(&self) -> Option<&str> {
        self.file_path.as_deref().filter(|p| Path::new(p).is_file())
    }
}

#[derive(Clone, Debug, Serialize, PartialEq)]
pub struct MusicPlaylist {
    pub id: i64,
    pub name: String,
    pub count: i64,
}

#[derive(Clone, Debug, PartialEq, Default)]
pub struct StoredLyrics {
    pub synced: Option<String>,
    pub plain: Option<String>,
}

const TRACK_COLS: &str = "t.video_id, t.title, t.artist, t.duration_secs, t.thumbnail_url, t.file_path, t.downloaded_at";

fn track_from_row(r: &Row) -> rusqlite::Result<MusicTrack> {
    Ok(MusicTrack {
        video_id: r.get(0)?,
        title: r.get(1)?,
        artist: r.get(2)?,
        duration_secs: r.get::<_, Option<i64>>(3)?.map(|d| d.max(0) as u32),
        thumbnail_url: r.get(4)?,
        file_path: r.get(5)?,
        downloaded_at: r.get(6)?,
    })
}

fn profile_from_row(r: &Row) -> rusqlite::Result<MusicProfile> {
    Ok(MusicProfile { id: r.get(0)?, name: r.get(1)?, color: r.get(2)? })
}

/// Replaces a playlist's songs, numbering them 0, 1, 2… (used after a removal or a move).
fn rewrite(tx: &Transaction, playlist: i64, ids: &[String]) -> Result<()> {
    tx.execute("DELETE FROM music_playlist_tracks WHERE playlist_id = ?1", [playlist])?;
    let mut insert = tx.prepare("INSERT INTO music_playlist_tracks(playlist_id, position, video_id) VALUES (?1, ?2, ?3)")?;
    for (position, id) in ids.iter().enumerate() {
        insert.execute(params![playlist, position as i64, id])?;
    }
    Ok(())
}

fn playlist_ids(tx: &Transaction, playlist: i64) -> Result<Vec<String>> {
    let mut statement = tx.prepare("SELECT video_id FROM music_playlist_tracks WHERE playlist_id = ?1 ORDER BY position")?;
    Ok(statement.query_map([playlist], |r| r.get(0))?.collect::<rusqlite::Result<_>>()?)
}

impl Database {
    // ---- profiles ---------------------------------------------------------------------------

    pub async fn music_profiles(&self) -> Result<Vec<MusicProfile>> {
        self.with(|db| {
            let mut statement =
                db.prepare("SELECT id, name, color FROM music_profiles ORDER BY id != 'guest', created_at, rowid")?;
            Ok(statement.query_map([], profile_from_row)?.collect::<rusqlite::Result<_>>()?)
        })
        .await
    }

    pub async fn music_profile(&self, id: &str) -> Result<Option<MusicProfile>> {
        let id = id.to_owned();
        self.with(move |db| {
            Ok(db.query_row("SELECT id, name, color FROM music_profiles WHERE id = ?1", [id], profile_from_row).optional()?)
        })
        .await
    }

    pub async fn music_create_profile(&self, name: &str, color: &str) -> Result<MusicProfile> {
        let (name, color) = (name.to_owned(), color.to_owned());
        self.with(move |db| {
            let id: String = db.query_row("SELECT lower(hex(randomblob(16)))", [], |r| r.get(0))?;
            db.execute("INSERT INTO music_profiles(id, name, color) VALUES (?1, ?2, ?3)", params![id, name, color])?;
            Ok(MusicProfile { id, name, color })
        })
        .await
    }

    pub async fn music_active(&self) -> Result<MusicProfile> {
        self.with(|db| {
            Ok(db.query_row(
                "SELECT p.id, p.name, p.color FROM music_profiles p JOIN settings s ON s.value = p.id WHERE s.key = 'music_active_profile'",
                [],
                profile_from_row,
            )?)
        })
        .await
    }

    pub async fn music_select(&self, id: &str) -> Result<()> {
        let id = id.to_owned();
        self.with(move |db| {
            let exists: bool = db.query_row("SELECT EXISTS(SELECT 1 FROM music_profiles WHERE id = ?1)", [&id], |r| r.get(0))?;
            if !exists {
                bail!("no such music profile");
            }
            db.execute("UPDATE settings SET value = ?1 WHERE key = 'music_active_profile'", [id])?;
            Ok(())
        })
        .await
    }

    /// Deletes the profile with its playlists, favorites and history. Songs and downloaded files stay;
    /// the cache's limit removes files nobody needs any more.
    pub async fn music_remove_profile(&self, id: &str) -> Result<()> {
        let id = id.to_owned();
        self.with(move |db| {
            if id == "guest" {
                bail!("Guest cannot be removed");
            }
            let tx = db.transaction()?;
            tx.execute("UPDATE settings SET value = 'guest' WHERE key = 'music_active_profile' AND value = ?1", [&id])?;
            tx.execute("DELETE FROM settings WHERE key LIKE ?1 ESCAPE '\\'", [format!("music:{id}:%")])?;
            tx.execute("DELETE FROM music_profiles WHERE id = ?1", [id])?;
            tx.commit()?;
            Ok(())
        })
        .await
    }

    // ---- per-profile settings ---------------------------------------------------------------

    pub async fn music_setting(&self, profile: &str, key: &str) -> Result<Option<String>> {
        let name = format!("music:{profile}:{key}");
        self.with(move |db| Ok(db.query_row("SELECT value FROM settings WHERE key = ?1", [name], |r| r.get(0)).optional()?)).await
    }

    pub async fn set_music_setting(&self, profile: &str, key: &str, value: &str) -> Result<()> {
        let (name, value) = (format!("music:{profile}:{key}"), value.to_owned());
        self.with(move |db| {
            db.execute(
                "INSERT INTO settings(key, value) VALUES (?1, ?2) ON CONFLICT(key) DO UPDATE SET value = ?2",
                params![name, value],
            )?;
            Ok(())
        })
        .await
    }

    /// A setting shared by all profiles (the cache size).
    pub async fn music_global_setting(&self, key: &str) -> Result<Option<String>> {
        let name = format!("music:{key}");
        self.with(move |db| Ok(db.query_row("SELECT value FROM settings WHERE key = ?1", [name], |r| r.get(0)).optional()?)).await
    }

    pub async fn set_music_global_setting(&self, key: &str, value: &str) -> Result<()> {
        let (name, value) = (format!("music:{key}"), value.to_owned());
        self.with(move |db| {
            db.execute(
                "INSERT INTO settings(key, value) VALUES (?1, ?2) ON CONFLICT(key) DO UPDATE SET value = ?2",
                params![name, value],
            )?;
            Ok(())
        })
        .await
    }

    // ---- songs ------------------------------------------------------------------------------

    /// Saves song metadata (the downloaded file, if any, is left alone).
    pub async fn music_upsert_tracks(&self, tracks: Vec<MusicTrack>) -> Result<()> {
        self.with(move |db| {
            let tx = db.transaction()?;
            {
                let mut statement = tx.prepare(
                    "INSERT INTO music_tracks(video_id, title, artist, duration_secs, thumbnail_url) VALUES (?1, ?2, ?3, ?4, ?5)
                     ON CONFLICT(video_id) DO UPDATE SET title = ?2, artist = ?3,
                       duration_secs = COALESCE(?4, duration_secs), thumbnail_url = COALESCE(?5, thumbnail_url)",
                )?;
                for t in &tracks {
                    statement.execute(params![t.video_id, t.title, t.artist, t.duration_secs, t.thumbnail_url])?;
                }
            }
            tx.commit()?;
            Ok(())
        })
        .await
    }

    pub async fn music_track(&self, id: &str) -> Result<Option<MusicTrack>> {
        let id = id.to_owned();
        self.with(move |db| {
            Ok(db
                .query_row(&format!("SELECT {TRACK_COLS} FROM music_tracks t WHERE t.video_id = ?1"), [id], track_from_row)
                .optional()?)
        })
        .await
    }

    /// The current rows of these songs (unknown ids are left out).
    pub async fn music_tracks_by_ids(&self, ids: &[String]) -> Result<std::collections::HashMap<String, MusicTrack>> {
        let json = serde_json::to_string(ids)?;
        self.with(move |db| {
            let mut statement = db.prepare(&format!(
                "SELECT {TRACK_COLS} FROM music_tracks t WHERE t.video_id IN (SELECT value FROM json_each(?1))"
            ))?;
            Ok(statement
                .query_map([json], track_from_row)?
                .collect::<rusqlite::Result<Vec<_>>>()?
                .into_iter()
                .map(|t| (t.video_id.clone(), t))
                .collect())
        })
        .await
    }

    pub async fn music_downloaded_oldest_first(&self) -> Result<Vec<MusicTrack>> {
        self.with(|db| {
            let mut statement = db.prepare(&format!(
                "SELECT {TRACK_COLS} FROM music_tracks t WHERE t.file_path IS NOT NULL ORDER BY t.downloaded_at, t.rowid"
            ))?;
            Ok(statement.query_map([], track_from_row)?.collect::<rusqlite::Result<_>>()?)
        })
        .await
    }

    pub async fn music_set_downloaded(&self, id: &str, path: &str, at: i64) -> Result<()> {
        let (id, path) = (id.to_owned(), path.to_owned());
        self.with(move |db| {
            db.execute("UPDATE music_tracks SET file_path = ?2, downloaded_at = ?3 WHERE video_id = ?1", params![id, path, at])?;
            Ok(())
        })
        .await
    }

    pub async fn music_clear_download(&self, id: &str) -> Result<()> {
        let id = id.to_owned();
        self.with(move |db| {
            db.execute("UPDATE music_tracks SET file_path = NULL, downloaded_at = NULL WHERE video_id = ?1", [id])?;
            Ok(())
        })
        .await
    }

    // ---- playlists --------------------------------------------------------------------------

    pub async fn music_playlists(&self, profile: &str) -> Result<Vec<MusicPlaylist>> {
        let profile = profile.to_owned();
        self.with(move |db| {
            let mut statement = db.prepare(
                "SELECT p.id, p.name, (SELECT COUNT(*) FROM music_playlist_tracks x WHERE x.playlist_id = p.id)
                 FROM music_playlists p WHERE p.profile_id = ?1 ORDER BY p.created_at, p.id",
            )?;
            Ok(statement
                .query_map([profile], |r| Ok(MusicPlaylist { id: r.get(0)?, name: r.get(1)?, count: r.get(2)? }))?
                .collect::<rusqlite::Result<_>>()?)
        })
        .await
    }

    /// The playlist, if it belongs to this profile.
    pub async fn music_playlist(&self, profile: &str, id: i64) -> Result<Option<MusicPlaylist>> {
        let profile = profile.to_owned();
        self.with(move |db| {
            Ok(db
                .query_row(
                    "SELECT p.id, p.name, (SELECT COUNT(*) FROM music_playlist_tracks x WHERE x.playlist_id = p.id)
                     FROM music_playlists p WHERE p.id = ?1 AND p.profile_id = ?2",
                    params![id, profile],
                    |r| Ok(MusicPlaylist { id: r.get(0)?, name: r.get(1)?, count: r.get(2)? }),
                )
                .optional()?)
        })
        .await
    }

    pub async fn music_create_playlist(&self, profile: &str, name: &str) -> Result<MusicPlaylist> {
        let (profile, name) = (profile.to_owned(), name.to_owned());
        self.with(move |db| {
            db.execute("INSERT INTO music_playlists(profile_id, name) VALUES (?1, ?2)", params![profile, name])?;
            Ok(MusicPlaylist { id: db.last_insert_rowid(), name, count: 0 })
        })
        .await
    }

    pub async fn music_rename_playlist(&self, id: i64, name: &str) -> Result<()> {
        let name = name.to_owned();
        self.with(move |db| {
            db.execute("UPDATE music_playlists SET name = ?2 WHERE id = ?1", params![id, name])?;
            Ok(())
        })
        .await
    }

    pub async fn music_delete_playlist(&self, id: i64) -> Result<()> {
        self.with(move |db| {
            db.execute("DELETE FROM music_playlists WHERE id = ?1", [id])?;
            Ok(())
        })
        .await
    }

    pub async fn music_playlist_tracks(&self, id: i64) -> Result<Vec<MusicTrack>> {
        self.with(move |db| {
            let mut statement = db.prepare(&format!(
                "SELECT {TRACK_COLS} FROM music_playlist_tracks p JOIN music_tracks t ON t.video_id = p.video_id
                 WHERE p.playlist_id = ?1 ORDER BY p.position"
            ))?;
            Ok(statement.query_map([id], track_from_row)?.collect::<rusqlite::Result<_>>()?)
        })
        .await
    }

    /// Appends songs (their metadata is saved first). The same song may appear again.
    pub async fn music_playlist_add(&self, id: i64, tracks: Vec<MusicTrack>) -> Result<()> {
        self.music_upsert_tracks(tracks.clone()).await?;
        self.with(move |db| {
            let tx = db.transaction()?;
            let mut ids = playlist_ids(&tx, id)?;
            ids.extend(tracks.into_iter().map(|t| t.video_id));
            rewrite(&tx, id, &ids)?;
            tx.commit()?;
            Ok(())
        })
        .await
    }

    /// Removes the song at `position`; later songs move up. `false` when there is none.
    pub async fn music_playlist_remove(&self, id: i64, position: usize) -> Result<bool> {
        self.with(move |db| {
            let tx = db.transaction()?;
            let mut ids = playlist_ids(&tx, id)?;
            if position >= ids.len() {
                return Ok(false);
            }
            ids.remove(position);
            rewrite(&tx, id, &ids)?;
            tx.commit()?;
            Ok(true)
        })
        .await
    }

    /// Moves a song to another position. `false` when either position is out of range.
    pub async fn music_playlist_move(&self, id: i64, from: usize, to: usize) -> Result<bool> {
        self.with(move |db| {
            let tx = db.transaction()?;
            let mut ids = playlist_ids(&tx, id)?;
            if from >= ids.len() || to >= ids.len() {
                return Ok(false);
            }
            let moved = ids.remove(from);
            ids.insert(to, moved);
            rewrite(&tx, id, &ids)?;
            tx.commit()?;
            Ok(true)
        })
        .await
    }

    // ---- favorites and history --------------------------------------------------------------

    /// Newest first.
    pub async fn music_favorites(&self, profile: &str) -> Result<Vec<MusicTrack>> {
        let profile = profile.to_owned();
        self.with(move |db| {
            let mut statement = db.prepare(&format!(
                "SELECT {TRACK_COLS} FROM music_favorites f JOIN music_tracks t ON t.video_id = f.video_id
                 WHERE f.profile_id = ?1 ORDER BY f.added_at DESC, f.rowid DESC"
            ))?;
            Ok(statement.query_map([profile], track_from_row)?.collect::<rusqlite::Result<_>>()?)
        })
        .await
    }

    pub async fn music_favorite_ids(&self, profile: &str) -> Result<HashSet<String>> {
        let profile = profile.to_owned();
        self.with(move |db| {
            let mut statement = db.prepare("SELECT video_id FROM music_favorites WHERE profile_id = ?1")?;
            Ok(statement.query_map([profile], |r| r.get(0))?.collect::<rusqlite::Result<_>>()?)
        })
        .await
    }

    pub async fn music_set_favorite(&self, profile: &str, track: MusicTrack, on: bool) -> Result<()> {
        let profile = profile.to_owned();
        if on {
            self.music_upsert_tracks(vec![track.clone()]).await?;
        }
        self.with(move |db| {
            if on {
                db.execute(
                    "INSERT OR IGNORE INTO music_favorites(profile_id, video_id) VALUES (?1, ?2)",
                    params![profile, track.video_id],
                )?;
            } else {
                db.execute("DELETE FROM music_favorites WHERE profile_id = ?1 AND video_id = ?2", params![profile, track.video_id])?;
            }
            Ok(())
        })
        .await
    }

    pub async fn music_record_play(&self, profile: &str, id: &str) -> Result<()> {
        let (profile, id) = (profile.to_owned(), id.to_owned());
        self.with(move |db| {
            let tx = db.transaction()?;
            // Delete and insert again, so a repeat play gets a newer row even within the same second.
            tx.execute("DELETE FROM music_history WHERE profile_id = ?1 AND video_id = ?2", params![profile, id])?;
            tx.execute("INSERT INTO music_history(profile_id, video_id) VALUES (?1, ?2)", params![profile, id])?;
            // Keep the list short.
            tx.execute(
                "DELETE FROM music_history WHERE profile_id = ?1 AND video_id NOT IN
                 (SELECT video_id FROM music_history WHERE profile_id = ?1 ORDER BY played_at DESC, rowid DESC LIMIT 50)",
                [profile],
            )?;
            tx.commit()?;
            Ok(())
        })
        .await
    }

    pub async fn music_recent(&self, profile: &str, limit: u32) -> Result<Vec<MusicTrack>> {
        let profile = profile.to_owned();
        self.with(move |db| {
            let mut statement = db.prepare(&format!(
                "SELECT {TRACK_COLS} FROM music_history h JOIN music_tracks t ON t.video_id = h.video_id
                 WHERE h.profile_id = ?1 ORDER BY h.played_at DESC, h.rowid DESC LIMIT ?2"
            ))?;
            Ok(statement.query_map(params![profile, limit], track_from_row)?.collect::<rusqlite::Result<_>>()?)
        })
        .await
    }

    // ---- lyrics -----------------------------------------------------------------------------

    /// `Some` once looked up, even when the service had none (both parts empty).
    pub async fn music_lyrics(&self, id: &str) -> Result<Option<StoredLyrics>> {
        let id = id.to_owned();
        self.with(move |db| {
            Ok(db
                .query_row("SELECT synced, plain FROM music_lyrics WHERE video_id = ?1", [id], |r| {
                    Ok(StoredLyrics { synced: r.get(0)?, plain: r.get(1)? })
                })
                .optional()?)
        })
        .await
    }

    pub async fn music_save_lyrics(&self, id: &str, lyrics: StoredLyrics) -> Result<()> {
        let id = id.to_owned();
        self.with(move |db| {
            db.execute(
                "INSERT INTO music_lyrics(video_id, synced, plain, fetched_at) VALUES (?1, ?2, ?3, unixepoch())
                 ON CONFLICT(video_id) DO UPDATE SET synced = ?2, plain = ?3, fetched_at = unixepoch()",
                params![id, lyrics.synced, lyrics.plain],
            )?;
            Ok(())
        })
        .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    pub fn track(id: &str) -> MusicTrack {
        MusicTrack {
            video_id: id.into(),
            title: format!("Song {id}"),
            artist: Some("Artist".into()),
            duration_secs: Some(200),
            thumbnail_url: None,
            file_path: None,
            downloaded_at: None,
        }
    }

    fn open(name: &str) -> Database {
        let path = std::env::temp_dir().join(format!("ct-music-db-{name}-{}.sqlite", std::process::id()));
        let _ = std::fs::remove_file(&path);
        Database::open(&path).unwrap()
    }

    #[tokio::test]
    async fn profiles_are_separate_and_guest_stays() {
        let db = open("profiles");
        assert_eq!(db.music_active().await.unwrap().id, "guest");
        let ann = db.music_create_profile("Ann", "#53a888").await.unwrap();
        db.music_select(&ann.id).await.unwrap();
        assert_eq!(db.music_active().await.unwrap().name, "Ann");
        assert!(db.music_select("nobody").await.is_err());
        assert!(db.music_remove_profile("guest").await.is_err());
        db.music_remove_profile(&ann.id).await.unwrap();
        assert_eq!(db.music_active().await.unwrap().id, "guest");
        assert_eq!(db.music_profiles().await.unwrap().len(), 1);
    }

    #[tokio::test]
    async fn playlists_add_remove_and_move_keep_the_order() {
        let db = open("playlists");
        let list = db.music_create_playlist("guest", "Road trip").await.unwrap();
        db.music_playlist_add(list.id, vec![track("a"), track("b"), track("c"), track("a")]).await.unwrap();
        let ids = |tracks: Vec<MusicTrack>| tracks.into_iter().map(|t| t.video_id).collect::<Vec<_>>();
        assert_eq!(ids(db.music_playlist_tracks(list.id).await.unwrap()), ["a", "b", "c", "a"]);
        assert!(db.music_playlist_move(list.id, 3, 0).await.unwrap());
        assert_eq!(ids(db.music_playlist_tracks(list.id).await.unwrap()), ["a", "a", "b", "c"]);
        assert!(db.music_playlist_remove(list.id, 1).await.unwrap());
        assert_eq!(ids(db.music_playlist_tracks(list.id).await.unwrap()), ["a", "b", "c"]);
        assert!(!db.music_playlist_remove(list.id, 9).await.unwrap());
        assert!(!db.music_playlist_move(list.id, 0, 9).await.unwrap());
        assert_eq!(db.music_playlists("guest").await.unwrap()[0].count, 3);
        // Another profile cannot see it.
        assert!(db.music_playlist("someone-else", list.id).await.unwrap().is_none());
        db.music_delete_playlist(list.id).await.unwrap();
        assert!(db.music_playlists("guest").await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn removing_a_profile_deletes_its_library_but_keeps_downloads() {
        let db = open("remove");
        let ann = db.music_create_profile("Ann", "#53a888").await.unwrap();
        let list = db.music_create_playlist(&ann.id, "Mine").await.unwrap();
        db.music_playlist_add(list.id, vec![track("a")]).await.unwrap();
        db.music_set_favorite(&ann.id, track("a"), true).await.unwrap();
        db.music_set_music_setting_for_test(&ann.id).await;
        db.music_set_downloaded("a", "/tmp/a.opus", 5).await.unwrap();
        db.music_remove_profile(&ann.id).await.unwrap();
        assert!(db.music_favorites(&ann.id).await.unwrap().is_empty());
        assert!(db.music_playlists(&ann.id).await.unwrap().is_empty());
        assert!(db.music_setting(&ann.id, "volume").await.unwrap().is_none());
        assert_eq!(db.music_downloaded_oldest_first().await.unwrap().len(), 1);
    }

    #[tokio::test]
    async fn favorites_are_newest_first_and_history_is_recent_first() {
        let db = open("favorites");
        db.music_set_favorite("guest", track("a"), true).await.unwrap();
        db.music_set_favorite("guest", track("b"), true).await.unwrap();
        let ids: Vec<_> = db.music_favorites("guest").await.unwrap().into_iter().map(|t| t.video_id).collect();
        assert_eq!(ids, ["b", "a"]);
        db.music_set_favorite("guest", track("a"), false).await.unwrap();
        assert_eq!(db.music_favorite_ids("guest").await.unwrap().len(), 1);
        db.music_record_play("guest", "a").await.unwrap();
        db.music_record_play("guest", "b").await.unwrap();
        db.music_record_play("guest", "a").await.unwrap();
        let ids: Vec<_> = db.music_recent("guest", 10).await.unwrap().into_iter().map(|t| t.video_id).collect();
        assert_eq!(ids, ["a", "b"]);
    }

    #[tokio::test]
    async fn lyrics_round_trip_including_none_found() {
        let db = open("lyrics");
        assert!(db.music_lyrics("a").await.unwrap().is_none());
        db.music_save_lyrics("a", StoredLyrics::default()).await.unwrap();
        assert_eq!(db.music_lyrics("a").await.unwrap(), Some(StoredLyrics::default()));
        db.music_save_lyrics("a", StoredLyrics { synced: Some("[00:01.00] hi".into()), plain: None }).await.unwrap();
        assert!(db.music_lyrics("a").await.unwrap().unwrap().synced.is_some());
    }

    impl Database {
        async fn music_set_music_setting_for_test(&self, profile: &str) {
            self.set_music_setting(profile, "volume", "40").await.unwrap();
        }
    }
}
