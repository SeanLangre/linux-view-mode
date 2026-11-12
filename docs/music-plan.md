# Music mode

Status: **implemented 2026-11-12, steps 1–8** (step 9, local files, is not done). See [`README-music.md`](README-music.md) for how it works and what was checked; this file keeps the original design and the decisions behind it. **Where the build differs from this plan** is listed under *Changes made while building*. A third mode of CouchTube, next to YouTube and Twitch, that works like the separate desktop app in `~/mint_strg/projects/linux/music-player`: **search for a song → add it to a playlist → play the playlist**, with a rolling download cache, favorites, a queue and timed lyrics. It is used the same way as the other modes: keyboard only, fullscreen, from the couch.

The desktop app is the reference for *behaviour*. Its code is Iced + libmpv in one process, so it is **ported, not linked**: the pure logic (queue, cache eviction, lyrics parsing, yt-dlp parsing) is copied into CouchTube's backend, and the UI is rebuilt in React with CouchTube's cards and `nav.ts` focus model.

## Changes made while building

- **No streaming at all.** The plan said "play the file, otherwise stream". Streaming fails here (YouTube answers mpv's direct requests with HTTP 403), and the desktop app's code in fact downloads first and plays when the file is ready (only its README still says "streams"). Music mode does the same: a song that is not downloaded yet *waits* while it downloads, then plays from its file; the next song is downloaded while the current one plays. Song states are `offline`, `missing`, `downloading` and `failed` (there is no "Streams"). mpv runs with `--ytdl=no`.
- **Player state is `GET /api/music/state`**, not a `music:` id in `/api/status`. The YouTube and Twitch screens therefore needed no change.
- **Files:** the yt-dlp parts are in `src/music_yt.rs` (not `ytdlp.rs`) and the queries in `src/music_db.rs` (not `db.rs`, which only creates the tables). Modes were not moved to `src/modes.rs`; `MODES` in `src/twitch.rs` just gained `music`.
- **Added:** a `music_history` table (Recently played), `Queue::append` and "Add to queue" from the phone, `POST /music/downloads/{id}` (retry) and `DELETE /music/downloads` (clear), a spatial arrow-key navigation (`spatial` in `nav.ts`) so rows, grids and lists share one screen, and a shared `Music::control` used by both the screens and the phone remote.
- **Left out so far:** step 9 (a local music folder, embedded cover art); a Settings screen item for the lyrics source (none needed).

## Goal

Open CouchTube, choose **Music** on the first screen, pick a music profile, and:
- **search** YouTube for songs, or paste a YouTube playlist link to import it;
- add songs to **playlists** (any size), reorder and remove them;
- **play** a playlist in order or shuffled, with repeat off / all / one;
- mark **favorites** (♥) and play them as a list;
- see the **queue** (what plays next, in play order, so it reflects shuffle);
- see **timed lyrics** for the song that is playing, the current line highlighted;
- keep browsing while the music plays: a **player bar** at the bottom stays on every music screen.

## How it maps onto the desktop music player

| Desktop music player | Music mode in CouchTube |
|---|---|
| Iced window, mouse and shortcuts | React screens in the Firefox kiosk, arrow keys / Enter / Esc, same cards and `nav.ts` |
| libmpv inside the app | **A headless `mpv --no-video --idle` process** started by the backend and driven over its IPC socket (the code already exists for the legacy video fallback, see Playback) |
| `youtube.rs` search and playlist import | Reuse `src/ytdlp.rs` search; port `parse_playlist`, `is_playlist_url` and `friendly_error` |
| `cache.rs`: 50 newest downloads, protect the current song, cleanup at start | Ported as `src/music_cache.rs`, same rules, files in `<data>/music/cache/` |
| `queue.rs`: order, shuffle, repeat, peek next | Ported unchanged as `src/music_queue.rs` (with its tests and monkey test) |
| `lyrics.rs` (lrclib.net via `ureq`) | Ported as `src/lyrics.rs`; HTTP through `curl` like the rest of CouchTube (no new HTTP crate) |
| SQLite `library.db` | New `music_*` tables in CouchTube's database (below) |
| Favorites, playlists, queue screen, lyrics screen | Same screens, keyboard-driven |
| MPRIS / tray | **Left out** (no desktop on the TV); the **phone remote** takes that role |
| Local files via file picker / drag and drop | **Later, optional**: a folder set by `COUCHTUBE_MUSIC_DIR`, scanned on request (no file picker on a TV) |
| Remembers screen, volume, shuffle, repeat | Same, per music profile in `settings` |

## Start screen and modes

- `ModeGate.tsx` gets a third card: **Music** ("Your playlists, favorites and lyrics", a green or blurple colour). `MODES` in `src/twitch.rs` becomes `["youtube", "twitch", "music"]`; `PUT /api/mode` accepts `music`.
- The mode choice moves out of `twitch.rs` into a small `src/modes.rs` (it is no longer Twitch-specific). Optional cleanup, can be done in step 1.
- Switching service stops anything playing, as today. **Within** music mode, playback continues while moving between screens (unlike video, where playback covers the screen).
- **Switch service** goes in the Music Home menu; Esc on its "Who's listening?" screen returns to the mode choice.

## Profiles

**Decided 2026-11-12: separate music profiles**, like Twitch.
- Tables `music_profiles(id, name, color, created_at)` with a fixed **Guest**. No sign-in: music needs no account (search and streaming run as Guest through yt-dlp, like For You discovery).
- Playlists, favorites, play history and settings (volume, shuffle, repeat, last screen) are per music profile.
- The **downloaded files are shared** across profiles (one cache, keyed by video id), so a song two people both have is stored once.
- Removing a profile deletes its playlists and favorites, never cached files directly; eviction cleans them up later.

## Data model

New tables in `src/db.rs` (same `CREATE TABLE IF NOT EXISTS` migration style):

```sql
CREATE TABLE music_profiles (
    id TEXT PRIMARY KEY, name TEXT NOT NULL, color TEXT NOT NULL,
    created_at INTEGER NOT NULL DEFAULT (unixepoch())
);
-- Song metadata, shared by all profiles.
CREATE TABLE music_tracks (
    video_id      TEXT PRIMARY KEY,
    title         TEXT NOT NULL,
    artist        TEXT,
    duration_secs INTEGER,
    thumbnail_url TEXT,
    file_path     TEXT,     -- NULL when not downloaded
    downloaded_at INTEGER   -- NULL when not downloaded
);
CREATE TABLE music_playlists (
    id         INTEGER PRIMARY KEY AUTOINCREMENT,
    profile_id TEXT NOT NULL REFERENCES music_profiles(id) ON DELETE CASCADE,
    name       TEXT NOT NULL,
    created_at INTEGER NOT NULL DEFAULT (unixepoch())
);
CREATE TABLE music_playlist_tracks (
    playlist_id INTEGER NOT NULL REFERENCES music_playlists(id) ON DELETE CASCADE,
    position    INTEGER NOT NULL,
    video_id    TEXT NOT NULL REFERENCES music_tracks(video_id),
    PRIMARY KEY (playlist_id, position)
);
CREATE TABLE music_favorites (
    profile_id TEXT NOT NULL REFERENCES music_profiles(id) ON DELETE CASCADE,
    video_id   TEXT NOT NULL REFERENCES music_tracks(video_id),
    added_at   INTEGER NOT NULL DEFAULT (unixepoch()),
    PRIMARY KEY (profile_id, video_id)
);
CREATE TABLE music_lyrics (
    video_id TEXT PRIMARY KEY, synced TEXT, plain TEXT, fetched_at INTEGER NOT NULL
);
```

The same song may appear more than once in a playlist (as in the desktop app). Per-profile settings go in the existing `settings` table under `music:<profile>:<key>`.

## Playback

**Choice: a dedicated audio-only mpv, driven by the backend.** Reasons:
- mpv plays both cached files and YouTube streams (through yt-dlp) with Opus/AAC, gapless, which a Firefox `<audio>` tag cannot do for streams without extracting URLs ourselves.
- The music must keep playing while the interface changes screens; a backend-owned process does that naturally, and the phone remote works the same way.
- `src/player.rs` already launches mpv with an IPC socket, observes properties (`time-pos`, `duration`, `pause`, `eof`) and sends commands. Music reuses that machinery.

Design (`src/music_player.rs`, a thin layer over the existing mpv IPC code, or a new session kind in `Player`):
- One long-lived `mpv --no-video --idle=yes --no-terminal --input-ipc-server=<dir>/ipc --ytdl-format=bestaudio[acodec=opus]/bestaudio` started on the first play and kept running while in music mode (avoids start-up delay between songs). Stopped on mode switch, on `/api/stop` and at exit.
- Source per song, decided when it starts (as in the desktop app): the cached file if `file_path` exists, otherwise `https://www.youtube.com/watch?v=<id>` streamed.
- The backend owns the **queue** (`music_queue.rs`). On mpv `end-file` (reason `eof`) it calls `queue.next(auto=true)` and loads the next source; on error it shows the message and skips to the next song.
- **Prefetch:** when a song starts, the next song in the queue is downloaded in the background if the cache has room (the desktop behaviour).
- `/api/status` reports `id: "music:<video_id>"`, `page: null`, position, duration, paused, volume, plus a `music` block (title, artist, thumbnail, next song, shuffle, repeat). The YouTube and Twitch screens ignore ids with the `music:` prefix, and the music screens react only to them (same rule as `twitch:`).
- The video `Player` and the music mpv never run at once: starting one stops the other.

## Downloads and cache

Port of `cache.rs`, same rules:
1. Adding a song to a playlist saves its metadata and playlist entry at once and starts a background download.
2. yt-dlp args as in the desktop app: `-f bestaudio[acodec=opus]/bestaudio --no-playlist --newline --progress-template … --print after_move:filepath`, into `<data>/music/tmp/`, then an atomic rename into `<data>/music/cache/`.
3. At most **2 downloads at once** (a semaphore). Progress per song kept in memory and exposed in track lists.
4. Keep the **50 most recently downloaded** songs (setting, changeable on the Music settings screen). Never evict the song that is playing (`pick_evictions(..., protected)`).
5. At start: forget database paths whose files are missing, delete unknown cache files, empty `tmp/`.
6. A failed or cancelled download leaves the song in the playlist; it streams. Track rows show **Downloading 42 %**, **Offline**, **Streams** or **Failed – R to retry**.
7. Importing a YouTube playlist downloads the first 5 songs at once; the rest are fetched as the playlist plays (desktop behaviour).

Disk use is bounded (~50 × 5 MB). The `<data>/music/` folder is `0700`, like the other data folders.

## Screens

All screens share a **player bar** at the bottom: thumbnail, title – artist, a progress line, ▶/⏸, shuffle and repeat state, and "Next: …". It is not focusable; keys control it (below).

| Screen | Contents |
|---|---|
| **Who's listening?** | Music profiles + Guest + Add profile (name, colour). `D` removes a profile (with confirmation). |
| **Home** | Rows: **Playlists** (cards, plus **New playlist**), **Favorites**, **Recently played**. Menu: Search, Queue, Lyrics, Settings, Phone remote, Profiles, Switch service. |
| **Search** | Text box (phone keyboard or TV keyboard). Results as cards (15, **Load more** up to 150). Enter plays the song now; `A` adds it to a playlist (a small picker with the last used playlist first); `F` toggles favorite. A pasted playlist link shows its songs with **Add all to…**. |
| **Playlist** | Track list (rows, not cards: song, artist, length, state). Top buttons **Play**, **Shuffle**, **Rename**, **Delete**. Enter plays from that song; `D` removes it; `Shift+↑/↓` moves it; `F` favorite; `R` retries a failed download. |
| **Favorites** | Like a playlist, newest first, read-only order. |
| **Queue** | The current song and what follows, in play order. Enter jumps to a song. |
| **Lyrics / Now playing** | Big artwork, title, artist, and the lyrics; timed lyrics scroll with the current line highlighted, Enter on a line seeks to it. "No lyrics found" otherwise. This is also the screen shown when nothing else is focused for a while (screensaver-like). |
| **Settings** | Cache size (25 / 50 / 100 / 200), clear downloads, show disk use. |

Text entry (search, playlist names) uses the existing on-screen flow and the phone keyboard where it already exists.

## Keys

Consistent with the other modes where possible:

| Key | Action |
|---|---|
| ← ↑ → ↓ / Enter / Esc | Move / choose / back |
| Space | Pause or resume (anywhere in music mode) |
| `Shift+←` / `Shift+→` | Seek −/+ 10 s (decided 2026-11-12) |
| `N` / `B` | Next / previous song |
| `9` / `0` | Volume down / up; `M` mute |
| `S` | Shuffle on/off; `E` cycles repeat (off → all → one) |
| `L` | Open Lyrics / Now playing |
| `Q` | Open Queue |
| `A` / `F` / `D` | Add to playlist / favorite / remove (on a focused song) |
| `/` | Search; `P` profiles; Home key: Music Home |

Plain arrows stay for navigation (unlike the video player, music screens are always browsable), which is why seeking uses Shift+arrows.

## Backend

New modules, registered in `main.rs` and merged into the router like `twitch::router()`:

| Module | Contents |
|---|---|
| `src/music.rs` | Routes and handlers (below), music profile CRUD, per-profile settings |
| `src/music_player.rs` | Audio-only mpv session, queue driving, end-of-song handling, prefetch |
| `src/music_queue.rs` | Port of `queue.rs` (order, shuffle, repeat, peek, upcoming) + tests |
| `src/music_cache.rs` | Port of `cache.rs` (download queue, limit, eviction, reconcile) + tests |
| `src/lyrics.rs` | Port of `lyrics.rs` (title cleaning, LRC parsing, best match), `curl` for HTTP |
| `src/music_yt.rs` | yt-dlp: search, playlist and video import, audio download (built instead of extending `ytdlp.rs`) |
| `src/music_db.rs`, `src/db.rs` | The queries; `db.rs` creates the `music_*` tables |

Routes (all under `/api/music/`, same `127.0.0.1` / Host checks as the rest):

```
GET/POST   /music/profiles                 DELETE /music/profiles/{id}   POST /music/profiles/{id}/select
GET        /music/search?q=&page=          (song results, or a playlist import when q is a playlist link)
GET/POST   /music/playlists                GET/PUT/DELETE /music/playlists/{id}
POST       /music/playlists/{id}/tracks    {video_id | tracks[]}   (adds + starts downloads)
DELETE     /music/playlists/{id}/tracks/{position}
POST       /music/playlists/{id}/move      {from, to}
GET        /music/favorites                PUT/DELETE /music/favorites/{video_id}
POST       /music/play     {playlist_id?, favorites?, video_id?, start?, shuffle?}
POST       /music/control  {pause | seek | next | prev | volume | shuffle | repeat | jump}
GET        /music/queue
GET        /music/lyrics/{video_id}
POST       /music/downloads/{video_id}/retry   DELETE /music/downloads/{video_id}
GET/PUT    /music/settings
```

`/api/status` and `/api/stop` stay the shared entry points. Thumbnails reuse `/api/thumb/{id}` (YouTube ids) and the existing thumbnail cache.

Input validation follows the existing rules: video ids through `ytdlp::is_video_id`, playlist links only for `youtube.com`/`music.youtube.com` with a `list=` id, names length-limited, yt-dlp always called with arguments (no shell) and with `--` before user input.

## Frontend

- `ModeGate.tsx`: third card; renders `MusicGate` for `music`.
- `MusicGate.tsx`: "Who's listening?" (modelled on `TwitchGate.tsx`).
- `MusicHome.tsx`: the screens above, using the existing back stack and `nav.ts`; a `PlayerBar` component that polls `/api/status` (same interval as today) and owns the global music keys.
- `MusicLyrics.tsx`: lyrics view; `current_line` logic ported to TS (or the backend returns the index).
- `api.ts`: `Mode` gains `'music'`; typed calls for the routes above.
- No new player page: music has no `watch.html` equivalent.

## Phone remote

`/state` carries `mode: "music"`; the phone page then shows pause, previous/next, seek, volume, shuffle/repeat and the queue, and its box takes a song search or a YouTube link and plays it (or adds it to the queue). Done after the main screens, like Twitch step 6.

## Left out on purpose

- MPRIS, system tray, desktop notifications (no desktop session on the TV).
- Signing in to YouTube Music / account playlists and likes (could reuse the YouTube profiles' cookies later; see Open questions).
- Converting audio, editing tags, albums/artists views.
- A file picker for local files (only an optional configured folder, later).

## Security and limits

- Nothing new listens on the network: music uses the main `127.0.0.1` API; the phone remote port behaves as today.
- yt-dlp and mpv run without a shell, inputs validated as above. The mpv IPC socket lives in a `0700` run directory, as for the video player.
- lrclib.net receives only title, artist and length (as the desktop app). Lyrics are cached, so each song is looked up once.
- Downloads are capped by count; a large imported playlist cannot fill the disk.
- Same legal note as the desktop app: the box can't tell whether a download is permitted.

## Build order

1. [x] **Mode and profiles.** `music` in `MODES` (optionally move modes to `src/modes.rs`), `music_profiles` table and routes, `MusicGate.tsx`, Music card on the mode choice, a placeholder Music Home with **Switch service**. Test: unit tests + a smoke test in the style of `tests/twitch_smoke.py`.
2. [x] **Search and audio playback.** `/music/search`, the audio-only mpv session, `/music/play {video_id}`, `/music/control`, `music:` ids in `/api/status`, the Search screen and the player bar with Space/seek/volume. Test with a fake mpv (as `player.rs` tests do) and once by hand with real audio.
3. [x] **Library.** `music_tracks`, playlists, favorites; Home rows, Playlist and Favorites screens; add/remove/move/rename/delete.
4. [x] **Queue.** Port `queue.rs` with its tests; play a playlist/favorites in order or shuffled, repeat modes, next/previous, auto-advance on end of song, skip on error; Queue screen.
5. [x] **Download cache.** Port `cache.rs` with its tests; background downloads (max 2), progress and states in lists, retry/cancel, 50-song limit with the playing song protected, startup cleanup, prefetch of the next song, Settings screen for the limit.
6. [x] **Playlist import.** Playlist links in search, **Add all**, first 5 downloaded at once.
7. [x] **Lyrics.** Port `lyrics.rs` with its tests, `music_lyrics` cache, Lyrics / Now playing screen with timed highlighting and seek-to-line.
8. [x] **Phone remote** in music mode.
9. [ ] Optional (not done): local music folder (`COUCHTUBE_MUSIC_DIR`, tags read with `lofty`), embedded cover art.

The user-facing description lives in [`README-music.md`](README-music.md) (keys, settings, data folders, API, limits).

## Open questions

**Decided 2026-11-12:**
- **Profiles:** separate music profiles, like Twitch (not shared with YouTube).
- **Switching service stops the music**, the same rule as today.
- **Search source:** plain YouTube search through yt-dlp, the same as the desktop app (`ytsearch15:` with Load more up to 150). No YouTube Music search.
- **Seek keys:** `Shift+←` / `Shift+→` jump back/forward 10 s; plain arrows stay for moving around the screens.

**Still open:**
1. **Share the download cache with the desktop app?** (still open, still not proposed) Not proposed: different machines in practice, and two programs evicting the same folder would conflict.
2. **Name on the mode card:** "Music" (proposed).
