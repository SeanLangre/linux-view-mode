# Music mode

The third mode of CouchTube, next to YouTube and Twitch: search for a song, put it in a playlist, play the playlist. It is used like the other modes, from the couch with a keyboard, and it is modelled on the desktop music player (`~/mint_strg/projects/linux/music-player`). The first half of this file is the user guide; the second half, **Design notes**, records how it is built and why.

On the first screen (**What are we watching?**) choose **Music**, then pick a music profile.

## How it works

- **Search** uses `yt-dlp` on YouTube: 15 results at a time, **Load more** up to 150. Paste a YouTube **playlist link** to list its songs (up to 200), or a **video link** to get that one song.
- **Music never streams.** YouTube refuses direct streaming to mpv (it answers 403), so every song is **downloaded first and played from its file**. A song that is not downloaded yet shows *Downloading…* in the player bar and starts as soon as its file is there. While a song plays, the next one in the queue is downloaded in the background, so there is usually no wait.
- **Playback** is a separate, audio-only `mpv` that the backend starts on the first play and controls over its IPC socket. It keeps playing while you move between screens. It never runs together with the YouTube or Twitch players: switching service, or leaving the Music screens, stops it.
- **Downloads** keep the audio as downloaded (Opus where YouTube has it, no conversion). At most two run at once. Only the **newest 50** are kept (25, 50, 100 or 200 on the Settings screen); the song that is playing is never deleted. A deleted file never removes a song from a playlist: it just downloads again when played.
- A song that cannot be downloaded or played is **skipped** with a message. If three songs in a row fail, playback stops (so a lost connection does not race through a whole playlist).
- **Lyrics** come from [lrclib.net](https://lrclib.net) (only the title, artist and length are sent), are cached, and are shown timed when the service has them.

## Profiles

Music profiles are their own list, separate from the YouTube and Twitch ones, and need no sign-in. Each has its own playlists, favorites, play history, volume, shuffle and repeat. Downloaded songs are shared, so a song two people both have is stored once. **Guest** is always there. Removing a profile deletes its playlists, favorites and history, but never the downloaded files (the cache limit clears those in time).

## Screens

| Screen | What it does |
|---|---|
| **Who's listening?** | Pick a profile, **Add profile**, **D** removes one. |
| **Home** | Menu (Search, Favorites, Queue, Now playing, Settings, Phone remote, Profiles, Switch service), your **Playlists** with **+ New playlist**, and **Recently played**. |
| **Search** | Type and press Enter. Enter on a result plays it and continues with the results after it. A playlist link adds **Play all** and **Add all to a playlist…**. |
| **Playlist** | **Play**, **Shuffle**, **Rename**, **Delete**, and the songs. Enter plays from that song. |
| **Favorites** | Your ♥ songs, newest first, with **Play** and **Shuffle**. |
| **Queue** | The current song and what follows, in play order (so it reflects shuffle). Enter jumps to a song. |
| **Now playing** | Artwork, title and the lyrics. The current line is highlighted; Enter on a line jumps there. |
| **Settings** | How many downloads to keep, and **Clear downloads** (the playing song stays). |
| **Phone remote** | Turn the remote on and pair a phone (same as the other services). |

A **player bar** along the bottom of every screen shows the song, its progress, pause / shuffle / repeat / volume and the next song. Each song row shows **Offline** (downloaded), **Downloading 42 %**, **Not downloaded** or **Failed**, and ♥ for a favorite.

## Keys

Plain arrows move around the screen, so seeking uses Shift.

| Key | Action |
|---|---|
| ← ↑ → ↓ | Move |
| Enter | Open / play |
| Esc, Backspace | Back (from Home: to the profiles) |
| Space | Pause or resume |
| Shift+← / Shift+→ | Back / forward 10 seconds |
| N / B | Next song / previous song (**B** restarts a song that has played more than 3 seconds) |
| 9 / 0 | Volume down / up; **M** mutes |
| S | Shuffle on or off |
| E | Repeat: off → all → one |
| L | Now playing (lyrics) |
| Q | Queue |
| / | Search |
| P | Profiles |
| Home | Back to Music Home |
| A | Add the focused song (or the playing one) to a playlist |
| F | Favorite or unfavorite the focused song (or the playing one) |
| D | Remove the focused song from a playlist, or from Favorites |
| Shift+↑ / Shift+↓ | Move the focused song up or down in a playlist |
| R | Retry the download of the focused song |

**A** opens a small list of your playlists (the last one you used comes first) with **+ New playlist…**. Adding a whole imported playlist downloads the first 5 songs at once; the rest download as the playlist plays.

## Phone remote

With **Phone remote** turned on and a phone paired (see the main [README](../README.md)), the phone page switches to Music while you are in this mode: pause, back, next, ±10 s and ±1 min, volume, mute, shuffle and repeat, the **Up next** list (tap ▶ to jump to a song), and a box that takes a **song name or YouTube link**: **Play now** or **Add to queue**. The queue can only be read from the phone, not edited.

## Requirements

Arch Linux:

```bash
sudo pacman -S --needed mpv yt-dlp deno curl
```

- `mpv` plays the downloaded files.
- `yt-dlp` searches and downloads. `deno` lets it read YouTube's pages. Keep `yt-dlp` up to date: YouTube changes often, and an old version is the usual cause of search or download errors.
- `curl` fetches lyrics.

## Settings

| Environment variable | Default | Meaning |
|---|---|---|
| `COUCHTUBE_MUSIC_AO` | mpv's own choice | mpv audio output driver. `null` plays silently, which is useful for testing. |
| `COUCHTUBE_DATA_DIR` | `$XDG_DATA_HOME/couchtube` | Where the music folder lives (below). |

The cache size is set on the Settings screen and remembered.

## Where data is stored

Under `<data>/music/` (mode `0700`):

| Path | Contents |
|---|---|
| `cache/` | Downloaded songs, named `<video id>.<ext>`. |
| `tmp/` | Downloads in progress. Emptied on start. |

Everything else is in CouchTube's SQLite database (`couchtube.db` in the data folder): `music_profiles`, `music_tracks` (song details and which are downloaded), `music_playlists`, `music_playlist_tracks`, `music_favorites`, `music_history` and `music_lyrics`; per-profile settings are stored as `music:<profile>:<name>`. On start, the app forgets downloads whose files are missing and deletes cache files it does not know.

## HTTP API

All under `/api/music/`, on `127.0.0.1` like the rest of the backend.

| Route | Purpose |
|---|---|
| `GET/POST /profiles`, `DELETE /profiles/{id}`, `POST /profiles/{id}/select` | Music profiles |
| `GET /home` | Playlists, favorites count, recently played |
| `GET /search?q=&count=` | Songs, or the songs of a pasted playlist or video link. Results are saved, so they can be added by id |
| `GET/POST /playlists`, `GET/PUT/DELETE /playlists/{id}` | Playlists |
| `POST /playlists/{id}/tracks` `{video_ids, import?}` | Add songs and start their downloads |
| `DELETE /playlists/{id}/tracks/{position}`, `POST /playlists/{id}/move` `{from, to}` | Remove or move a song |
| `GET /favorites`, `PUT/DELETE /favorites/{video_id}` | Favorites |
| `POST /play` | One of `{playlist_id}`, `{favorites}`, `{recent}` or `{video_ids}`, with optional `start` and `shuffle` |
| `POST /control` `{action, value?, name?}` | `pause`, `resume`, `toggle`, `seek`, `seek_to`, `next`, `prev`, `jump`, `volume`, `volume_up`, `volume_down`, `mute`, `shuffle`, `repeat`, `stop` |
| `GET /state`, `GET /queue` | What the player bar and the Queue screen show |
| `GET /lyrics/{video_id}` | `{found, synced: [[seconds, line]], plain}` |
| `POST/DELETE /downloads/{video_id}`, `DELETE /downloads` | Retry or cancel a download; clear all |
| `GET/PUT /settings` | Cache size |

Music mode is chosen with `PUT /api/mode` `{"mode": "music"}`.

## Development

```bash
cargo test music_                 # the Music unit tests (database, queue, cache, player, yt-dlp parsing)
cargo test music_queue -- --ignored --nocapture   # the big queue monkey test
cd frontend && npx vitest run     # includes the arrow-key navigation and lyrics-line tests
```

The player tests use a fake `mpv` and a socket the test controls, so they need no audio and no network. The rest of the code is in `src/music*.rs` and `src/lyrics.rs`; the screens are `frontend/src/Music*.tsx`.

To try it without sound:

```bash
COUCHTUBE_MUSIC_AO=null cargo run
```

## Limits

- No media keys or system tray (there is no desktop session on the TV); the phone remote takes that role.
- No local music files yet. The desktop app can import a folder; this mode only plays YouTube downloads.
- The queue cannot be edited (only jumped in), and a song added from the phone goes to the end.
- Search is plain YouTube search, so results can include covers, lyric videos and live versions.
- Lyrics coverage depends on lrclib.net, and matching a YouTube title to a song is a best guess.
- The song that is playing is protected, but an upcoming song can still be removed from the cache by the size limit before it plays if the limit is small and the queue is long; it then downloads again when its turn comes.
- Checked so far: the backend against the real YouTube and mpv (search, download, play from file, seek, pause, next, lyrics), with the audio output turned off. The screens and the phone page are type-checked and built but have **not been looked at on a screen or a phone**, and nothing has been heard through real speakers yet.

## Legal

This is a personal tool. It cannot tell whether a download is permitted. Downloading copyrighted music from YouTube can violate copyright law or YouTube's terms, so only download content you own, have permission to download, or that is legally available to download.

---

# Design notes

## Where the build differs from the original plan

The mode was planned on 2026-11-12 and built the same day (steps 1–8; step 9 is not done). The plan's choices that changed:

- **No streaming at all.** The plan said "play the file, otherwise stream". Streaming fails here (YouTube answers mpv's direct requests with HTTP 403), and the desktop app's code in fact downloads first and plays when the file is ready (only its README still says "streams"). Music mode does the same: a song that is not downloaded yet *waits* while it downloads, then plays from its file. Song states are `offline`, `missing`, `downloading` and `failed` (there is no "streams").
- **Player state is `GET /api/music/state`**, not a `music:` id in `/api/status`. The YouTube and Twitch screens therefore needed no change.
- **Files:** the yt-dlp parts are in `src/music_yt.rs` (not `ytdlp.rs`) and the queries in `src/music_db.rs` (not `db.rs`, which only creates the tables). The mode list was not moved to a `src/modes.rs`; `MODES` in `src/twitch.rs` just gained `music`.
- **Added to the plan:** a `music_history` table (Recently played), `Queue::append` and "Add to queue" from the phone, retry and clear-all for downloads, spatial arrow-key navigation (`spatial` in `nav.ts`) so rows, grids and lists share one screen, and a shared `Music::control` used by both the screens and the phone remote.
- **Dropped from the plan:** a `MusicLyrics.tsx` file (lyrics are part of `MusicHome.tsx`), a "screensaver" Now playing screen, and a separate `PlayerBar` that polls `/api/status`.

## How it maps onto the desktop music player

The desktop app (Iced + libmpv in one process) is the reference for *behaviour*, so its logic is **ported, not linked**: the pure parts are copied into CouchTube's backend and the UI is rebuilt in React with CouchTube's focus model.

| Desktop music player | Music mode in CouchTube |
|---|---|
| Iced window, mouse and shortcuts | React screens in the Firefox kiosk, arrow keys / Enter / Esc |
| libmpv inside the app | An audio-only `mpv --idle` process driven over its IPC socket (`src/music_player.rs`) |
| `youtube.rs` search, playlist import, download | `src/music_yt.rs` (same yt-dlp arguments and error messages) |
| `cache.rs`: newest 50 downloads, protect the playing song, cleanup at start | `src/music_cache.rs`, same rules, files in `<data>/music/cache/` |
| `queue.rs`: order, shuffle, repeat, peek next | `src/music_queue.rs` (same code, tests and monkey test, plus `append`) |
| `lyrics.rs` (lrclib.net via `ureq`) | `src/lyrics.rs`, HTTP through `curl` like the rest of CouchTube (no new HTTP crate) |
| SQLite `library.db` | `music_*` tables in CouchTube's database |
| MPRIS and tray | Left out (no desktop on the TV); the phone remote takes that role |
| Local files via file picker and drag and drop | Not done (a folder set by `COUCHTUBE_MUSIC_DIR` is the idea, step 9) |

## Modes and profiles

- `ModeGate.tsx` has a third card, **Music**. `MODES` in `src/twitch.rs` is `["youtube", "twitch", "music"]` and `PUT /api/mode` accepts `music`. Switching service stops everything that plays, the music included. **Within** Music, playback continues while you move between screens (unlike video, where playback covers the screen).
- **Separate music profiles** (decided 2026-11-12), like Twitch: `music_profiles` with a fixed **Guest** and no sign-in, since music needs no account. Playlists, favorites, history and settings (volume, shuffle, repeat) are per profile. Song metadata and downloaded files are shared (one cache, keyed by video id). Removing a profile deletes its playlists, favorites, history and `music:<profile>:*` settings, never cached files directly; the size limit cleans those up later. Choosing or removing a profile stops the music, since the queue belongs to whoever started it.

## Data model

Created in `src/db.rs` (same `CREATE TABLE IF NOT EXISTS` style as the other tables), queried in `src/music_db.rs`:

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
-- The newest 50 songs each profile played (Recently played).
CREATE TABLE music_history (
    profile_id TEXT NOT NULL REFERENCES music_profiles(id) ON DELETE CASCADE,
    video_id   TEXT NOT NULL REFERENCES music_tracks(video_id),
    played_at  INTEGER NOT NULL DEFAULT (unixepoch()),
    PRIMARY KEY (profile_id, video_id)
);
CREATE TABLE music_lyrics (
    video_id TEXT PRIMARY KEY, synced TEXT, plain TEXT, fetched_at INTEGER NOT NULL
);
```

The same song may appear more than once in a playlist (as in the desktop app). Positions are rewritten as 0, 1, 2… on every removal or move, so they never clash. Search saves every result into `music_tracks`, so a song can be added, favorited or played by its id alone and the client never supplies titles.

## Playback design

A dedicated audio-only mpv, driven by the backend, because:
- the music must keep playing while the interface changes screens, and a backend-owned process does that naturally (the phone remote works the same way);
- mpv plays Opus and AAC files with seeking, which a Firefox `<audio>` tag would not do as well.

How it works (`src/music_player.rs`, separate from the video `Player`):
- One long-lived `mpv --no-config --no-video --idle=yes --no-terminal --force-window=no --audio-display=no --really-quiet --ytdl=no --input-ipc-server=<dir>/ipc`, started on the first play and kept while songs keep playing (no start-up delay between songs). Stopped on mode switch, profile change, leaving the Music screens and at exit. The socket is in a `0700` folder under `$XDG_RUNTIME_DIR`.
- The backend owns the **queue**. mpv reports `end-file`: `eof` plays the next song (`queue.next(auto = true)`), `error` skips with a message, `stop` (our own `loadfile replace`) is ignored. Three failures in a row stop playback.
- A song is loaded from its file. If it has none, the player **waits** (`waiting_id`): the cache downloads it, and a 500 ms tick (`Music::tick`) calls `resume_waiting` once the file exists, or `fail_waiting` (skip) if the download failed or was cancelled.
- When a song starts, the backend records it in history, protects it from eviction, and starts downloads for it and the next song in the queue (if they are not downloaded).
- The video player and the music mpv never run together: `POST /music/play` stops video playback first, and switching mode stops the music.
- `GET /api/music/state` is what the player bar polls (once a second, twice a second on the lyrics screen).

## Downloads and cache

Port of `cache.rs`, same rules:
1. Adding a song to a playlist saves it at once and starts a background download; playing a song downloads it if needed.
2. yt-dlp arguments as in the desktop app: `-f bestaudio[acodec=opus]/bestaudio --no-playlist --newline --progress-template … --print after_move:filepath`, into `<data>/music/tmp/`, then an atomic rename into `<data>/music/cache/` (both are in the same folder tree, so the rename is atomic).
3. At most **2 downloads at once** (a semaphore). Progress per song is kept in memory and shows in the lists.
4. Keep the **newest 50** downloads (25, 50, 100 or 200 on the Settings screen). Never evict the song that is playing (`pick_evictions(…, protected)`).
5. At start: forget database paths whose files are missing, delete unknown cache files, empty `tmp/`.
6. A failed or cancelled download leaves the song in its playlist; the row shows **Failed · R retries** or **Not downloaded**.
7. Importing a YouTube playlist downloads its first 5 songs at once; the rest are fetched as the playlist plays.

Disk use is bounded (about 50 × 5 MB). The `<data>/music/` folder is `0700`, like the other data folders.

## Backend modules

| Module | Contents |
|---|---|
| `src/music.rs` | Routes and handlers, `Music` (player + cache + the shared `control`, `start`, `enqueue`, `song_from_text`), the 500 ms `tick` |
| `src/music_player.rs` | Audio-only mpv session, queue driving, end-of-song handling, the waiting-for-download state |
| `src/music_queue.rs` | Order, shuffle, repeat, peek, upcoming, append (port of the desktop `queue.rs`) |
| `src/music_cache.rs` | Download queue, size limit, eviction, startup cleanup, per-song state |
| `src/music_db.rs` | Queries for all `music_*` tables |
| `src/music_yt.rs` | yt-dlp: search, playlist and video import, audio download, error messages |
| `src/lyrics.rs` | Title cleaning, LRC parsing, best match; `curl` for HTTP |

Input validation follows the existing rules: video ids go through `ytdlp::is_video_id`, playlist links must be on `youtube.com`, `music.youtube.com` or `youtu.be` with a `list=` id, names are length-limited, and yt-dlp is always called with arguments (no shell) and with `--` before user input.

## Frontend

- `ModeGate.tsx`: the third card; renders `MusicGate` for `music`.
- `MusicGate.tsx`: "Who's listening?" (modelled on `TwitchGate.tsx`).
- `MusicHome.tsx`: all the screens, the playlist picker and the prompts, using a back stack like the other modes. It owns the global music keys and polls `/api/music/state`. Arrow keys use `spatial` in `nav.ts`, which picks the nearest button in a direction, so rows, grids and lists of different sizes share a screen.
- `MusicBar.tsx`: the player bar. `MusicRemote.tsx`: the phone-remote screen.
- `api.ts`: `Mode` gains `'music'`, with typed calls for every route. The lyric line for a position is found in `format.ts` (`lyricLine`).
- Music has no player page (no `watch.html` equivalent): the audio comes from mpv, not the browser.

## Phone remote design

`/state` carries `mode: "music"`. The status block has the shape the phone page already reads (`playing`, `title`, `position`, `duration`, `paused`), plus `loading` and a `music` block (shuffle, repeat, volume, muted); the queue is what plays after the current song. Commands go through the same `Music::control` as the screens. `play` and `enqueue` take a song name or YouTube link (`Music::song_from_text`: a link, or else the first search result); `dequeue` and `clear` are refused.

## Left out on purpose

- MPRIS, system tray, desktop notifications (no desktop session on the TV).
- Signing in to YouTube Music, and account playlists and likes (YouTube profiles' cookies could be reused later).
- Converting audio, editing tags, albums and artists views.
- A file picker for local files.

## Security

- Nothing new listens on the network: Music uses the main `127.0.0.1` API (with its `Host` check); the phone remote port behaves as before.
- yt-dlp and mpv run without a shell, with validated inputs. The mpv socket lives in a `0700` run folder.
- lrclib.net receives only the title, artist and length. Lyrics are cached, so each song is looked up once.
- Downloads are capped by count, so a large imported playlist cannot fill the disk.

## Build order

1. [x] **Mode and profiles:** `music` in `MODES`, `music_profiles` and routes, `MusicGate.tsx`, the Music card.
2. [x] **Search and audio playback:** `/music/search`, the audio-only mpv session, `/music/play`, `/music/control`, the Search screen and the player bar.
3. [x] **Library:** songs, playlists, favorites; Home, Playlist and Favorites screens; add, remove, move, rename, delete.
4. [x] **Queue:** play a playlist or favorites in order or shuffled, repeat, next and previous, auto-advance, skip on error; Queue screen.
5. [x] **Download cache:** background downloads, progress and states in lists, retry and cancel, the size limit, startup cleanup, prefetch, Settings screen.
6. [x] **Playlist import:** playlist links in search, **Add all**, first 5 downloaded at once.
7. [x] **Lyrics:** lookup and cache, Now playing screen with timed highlighting and seek-to-line.
8. [x] **Phone remote** in Music mode.
9. [ ] Optional, not done: a local music folder (`COUCHTUBE_MUSIC_DIR`, tags read with `lofty`) and embedded cover art.

## Decisions

Decided 2026-11-12:
- **Profiles:** separate music profiles, like Twitch (not shared with YouTube).
- **Switching service stops the music**, like every other mode switch.
- **Search source:** plain YouTube search through yt-dlp (`ytsearch15:`, Load more up to 150), the same as the desktop app, whose notes say YouTube Music search was too slow (15 s) and returned no artist or length.
- **Seek keys:** `Shift+←` / `Shift+→` jump 10 s; plain arrows stay for moving around the screens.
- **No streaming:** every song is downloaded and played from its file.

Still open:
1. **Share the download cache with the desktop app?** Not proposed: they are different machines in practice, and two programs evicting the same folder would conflict.
2. **Name on the mode card:** "Music" (proposed).
