# Music mode

The third mode of CouchTube, next to YouTube and Twitch: search for a song, put it in a playlist, play the playlist. It is used like the other modes, from the couch with a keyboard, and it is modelled on the desktop music player (`~/mint_strg/projects/linux/music-player`). The original design is in [`music-plan.md`](music-plan.md).

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
