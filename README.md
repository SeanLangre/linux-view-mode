# CouchTube

A fullscreen, keyboard-only app for a TV. It runs on an Arch Linux box connected to the TV over HDMI and is used from the couch with a wireless keyboard: browse with the arrow keys, press Enter, and it plays fullscreen with no ads. It has three modes, chosen on the first screen:

| Mode | What it is | Documentation |
|---|---|---|
| **YouTube** | Search, channels, your account's feeds, For You, resume and a queue | [`docs/README-youtube.md`](docs/README-youtube.md) |
| **Twitch** | Live streams, categories, channels, past broadcasts and clips, Following, chat | [`docs/README-twitch.md`](docs/README-twitch.md) (in progress) |
| **Music** | Search songs, playlists, favorites, queue and lyrics; songs are downloaded and played from their files | [`docs/README-music.md`](docs/README-music.md) |

- **Backend:** Rust + Axum on `127.0.0.1:7878`. Runs `yt-dlp` for search, channel pages and music downloads, tracks playback, and plays video in the Firefox window (YouTube's and Twitch's embedded players), with an audio-only `mpv` for music (and `mpv` as the legacy video fallback).
- **Frontend:** React + TypeScript + Vite, shown in Firefox kiosk mode.

Design notes live in [`docs/`](docs/): [`design.md`](docs/design.md) (decisions, architecture, security rules), [`accounts.md`](docs/accounts.md) (profiles, sign-in, feeds), [`phone-signin-encryption.md`](docs/phone-signin-encryption.md), [`for-you.md`](docs/for-you.md), [`server.md`](docs/server.md) (TV server setup and checks), [`roadmap.md`](docs/roadmap.md) and [`legacy-mpv.md`](docs/legacy-mpv.md) (mpv, no longer the main player). The working checklist is [`TODO.md`](TODO.md).

## Requirements

Arch Linux:

```bash
sudo pacman -S --needed rust nodejs npm yt-dlp firefox curl xdotool mpv deno
```

- `yt-dlp` (with `deno`, which lets it read YouTube's pages) does YouTube search and Music downloads; keep it up to date.
- `mpv` plays **Music** and is also the legacy fallback player for YouTube.
- `xdotool` is only needed for typing from a phone during sign-in (see the [YouTube](docs/README-youtube.md) and [Twitch](docs/README-twitch.md) docs).
- `curl` is used for Twitch, thumbnails and lyrics.

## Run it

```bash
# Build the frontend once (the backend serves frontend/dist)
cd frontend && npm install && npm run build && cd ..

# Start the backend
cargo run

# Open it like the TV will
firefox --kiosk http://127.0.0.1:7878
```

CouchTube opens on **What are we watching?**: choose **YouTube**, **Twitch** or **Music** (the one used last is highlighted). Each mode has its own **Who’s …?** screen and its own profiles, separate from the others. Esc on a profile screen, or **Switch service** in a mode's Home menu, returns to the choice, and switching stops whatever is playing.

Firefox in kiosk mode has no window controls: close it with **Alt+F4**.

### Frontend development (live reload)

Keep `cargo run` running, then in another terminal:

```bash
cd frontend && npm run dev
```

and open `http://127.0.0.1:5173`. Vite forwards `/api` calls to the backend (with `changeOrigin`, because the backend rejects any `Host` other than `127.0.0.1:<port>` or `localhost:<port>`).

### Settings

| Environment variable | Default | Meaning |
|---|---|---|
| `COUCHTUBE_PORT` | `7878` | Port to listen on (always `127.0.0.1` only) |
| `COUCHTUBE_PAIR_PORT` | `7879` | Port for the phone keyboard used during sign-in (listens on all interfaces, see below) |
| `COUCHTUBE_REMOTE_PORT` | `7880` | Port for the phone remote; opened on all interfaces only while the remote is switched on (see below) |
| `COUCHTUBE_TWITCH_CLIENT_ID`, `COUCHTUBE_TWITCH_CLIENT_SECRET` | unset | Optional. Twitch mode works without them, using Twitch's unofficial web API. Set both (from a free application at `dev.twitch.tv/console`) to switch to the official API; see [`docs/README-twitch.md`](docs/README-twitch.md). Never commit the secret |
| `COUCHTUBE_CEC_DEVICE` | `/dev/cec0` | HDMI-CEC device used by `cec-ctl` for **Turn TV off** |
| `COUCHTUBE_FRONTEND` | `frontend/dist` | Folder with the built frontend |
| `COUCHTUBE_PLAYER` | `browser` | Legacy: `mpv` plays every video in mpv; otherwise videos play in the CouchTube window |
| `COUCHTUBE_MUSIC_AO` | mpv's own choice | Music mode: mpv audio output driver; `null` plays silently (for tests) |
| `COUCHTUBE_MPV_CONFIG` | `config/mpv` | Legacy: mpv config directory, including scripts (relative to the backend working directory) |
| `COUCHTUBE_DATA_DIR` | `$XDG_DATA_HOME/couchtube` or `~/.local/share/couchtube` | Private SQLite database and account Firefox profiles |
| `RUST_LOG` | `couchtube=info` | Log level, e.g. `couchtube=debug,tower_http=debug` |

## Turn the TV off and the phone remote

- **Turn TV off**: shown in the YouTube Home menu only when `cec-client` (libcec) or `cec-ctl` (v4l-utils) and a `/dev/cec*` device exist. Most PC graphics cards have no CEC; a Pulse-Eight USB adapter does.
- **Phone remote**: **Phone remote** in a mode's Home menu → **Turn on**, then **Pair a phone**. Scan the QR code, check the phone shows the same code as the TV, and choose **Yes**. The phone page then follows the mode you are in: in YouTube it controls pause, seek, volume, mute, subtitles, next and stop, shows the queue, and can play or queue a pasted YouTube link; in Twitch it has pause, volume, mute, chat and stop, and takes a channel name or twitch.tv link; in Music it has pause, back, next, seek, volume, shuffle and repeat, shows what plays next, and takes a song name or YouTube link. The port (`7880`) is closed until you turn the remote on. Each phone keeps a random token; the server stores only its hash, and **Forget phones** revokes all of them. It is plain HTTP, so anyone on your Wi-Fi who sniffs the traffic can copy a phone's token. Use it only on a network you trust.

## API

These endpoints are shared by all modes. Each mode's own endpoints are listed in its documentation: [YouTube](docs/README-youtube.md#api), [Twitch](docs/README-twitch.md#api) and [Music](docs/README-music.md#http-api) (`/api/music/…`).

| Method | Path | Returns |
|---|---|---|
| GET/POST/DELETE | `/api/pairing` | Phone URL and QR code (SVG) while a sign-in is in progress, otherwise 404; POST starts it (`?service=twitch` for a Twitch sign-in) before Firefox opens (the login page does this when it opens), DELETE cancels it if no window is open |
| POST | `/api/system/exit` | Stops the kiosk service (only when it runs as the systemd unit) |
| GET/POST | `/api/system/tv` | GET `{available}` (a CEC tool and device exist); POST `{on}` switches the TV on or to standby over HDMI-CEC |
| GET/PUT | `/api/remote` | Phone remote state `{enabled, devices, url, svg, code, confirmed}`; PUT `{enabled}` opens or closes its LAN port |
| POST | `/api/remote/pair`, `/confirm`, `/reject` | Start a pairing link (10 minutes, one use), accept the phone's code `{code}`, or refuse it |
| DELETE | `/api/remote/devices` | Forget every paired phone |
| GET/PUT | `/api/mode` | GET `{mode}` (`youtube`, `twitch`, `music` or `null` before the first choice); PUT `{mode}` saves the choice and stops playback |
| GET | `/api/health` | `{"status":"ok"}` |
| GET | `/api/version` | `{version, commit}`: the package version and the git commit the build came from (`-dirty` if tracked files were modified, `unknown` outside a git checkout); shown bottom-right on the service and profile screens |
| GET | `/api/thumb/<id>` | A YouTube thumbnail (11-character video id), saved in `<data>/thumbs/` on first request; only `i.ytimg.com` is contacted |
| POST | `/api/stop` | Stops playback, returns `{"status":"stopped"}` (safe when idle) |
| GET | `/api/status` | `{playing, page, id, session, title, position, duration, paused, error}`; `page` is the player page the CouchTube window must show (browser playback) |
| POST | `/api/player/report` | Used by the player page: `{session, position?, duration?, paused?, title?, outcome?, code?}`; answers `{commands}` (mpv-style, from the phone remote) |

Playback positions are seconds; `start` must be finite and non-negative. Status fields can be `null` while loading or for live streams. `playing` means the mpv process is running, including while loading or paused. Starting is asynchronous; a later playback failure appears in status. mpv output goes to the backend log. The backend stops mpv on Ctrl+C or SIGTERM.

Errors are `{"error": "..."}`.

## Tests and checks

```bash
cargo test && cargo clippy          # backend, including profiles, player lifecycle and IPC
cargo test music_                   # only Music mode's tests
cd frontend && npx vitest run       # navigation, API caching, recent searches
npm run lint && npm run build       # lint + type-check
```

The smoke tests start the real backend with fake `firefox`, `yt-dlp` and Twitch servers (no network, no real accounts) and need permission to bind loopback:

```bash
cargo build
python3 tests/profiles_smoke.py     # simulated YouTube accounts and sign-in
python3 tests/twitch_smoke.py       # Twitch browsing, watching, sign-in and Following (unofficial and official sources)
python3 tests/feeds_smoke.py        # simulated feeds, queue, resume and watched sync, driving the Firefox player's reports
lua tests/sponsorblock_test.lua     # needs Lua; tests SponsorBlock without network access
```

## Project layout

```
src/            Rust backend (main.rs, api.rs, ytdlp.rs, player.rs, playback.rs, profiles.rs, pairing.rs, remote.rs, cec.rs, feeds.rs, db.rs, discover.rs for For You, twitch.rs, gql.rs and helix.rs for Twitch mode, music*.rs and lyrics.rs for Music mode)
frontend/       React app (App.tsx, ModeGate.tsx, ProfileGate.tsx, TwitchGate.tsx, TwitchHome.tsx, MusicGate.tsx, MusicHome.tsx, nav.ts, api.ts, styles.css); public/watch.html and twitch.html are the player pages
config/mpv/     Legacy mpv settings, keys, SponsorBlock script
config/firefox/ Account preferences and extension policy
docs/            Design notes, server setup, roadmap
deploy/         install script and systemd units
packaging/      AppImage build
tests/          Smoke tests
TODO.md         Checklist
```

## Running as a TV appliance

On the server, from a checkout: `./deploy/install.sh`. It builds the backend and frontend, installs them to `~/.local/share/couchtube/app`, installs the systemd user units (`couchtube`, `couchtube-kiosk`, and a daily `couchtube-refresh` timer that re-checks each profile's sign-in), copies `config/firefox/policies.json` to `/etc/firefox/policies/` with `sudo` (it never overwrites a different existing file), and restarts the services. It needs `cargo npm firefox mpv yt-dlp xdotool curl`.

Set these in Plasma by hand (CouchTube does not change them):
- System Settings → Users / Login Screen: automatic login for the TV user.
- Screen Locking: off. Energy Saving: no screen blanking, no sleep.
- Optional: bind a KDE shortcut (System Settings → Shortcuts) to the **CouchTube kiosk** launcher, which runs `systemctl --user start couchtube-kiosk`.

**Exit to desktop** (last item in the Home menu) stops the kiosk service; the launcher or `systemctl --user start couchtube-kiosk` starts it again. Firefox restarts by itself if it crashes. The services need the desktop session's `DISPLAY` and `WAYLAND_DISPLAY`; if mpv or the phone keyboard can't open a window, run `systemctl --user import-environment DISPLAY WAYLAND_DISPLAY` once and restart `couchtube`.

## AppImage

A single-file build for running CouchTube without installing it. It contains the backend, the built frontend and the mpv config; `mpv`, `firefox` and `yt-dlp` still come from the system (plus `xdotool` for the phone keyboard and `curl`).

Build it from a checkout:

```bash
sudo pacman -S --needed rust nodejs npm librsvg   # librsvg provides rsvg-convert
# appimagetool: from the AUR (appimagetool-bin) or https://github.com/AppImage/appimagetool/releases, on PATH
./packaging/build-appimage.sh
```

The script runs `npm ci && npm run build` in `frontend/`, builds the backend with `cargo build --release` (with the home and project paths removed from the binary), assembles `target/AppDir` from `packaging/AppRun`, `packaging/couchtube.desktop` and an icon made from `frontend/public/favicon.svg`, and writes `CouchTube-<arch>.AppImage` (for example `CouchTube-x86_64.AppImage`) to the repository root. The file is in `.gitignore`.

Run it:

```bash
chmod +x CouchTube-x86_64.AppImage
./CouchTube-x86_64.AppImage
```

It starts the backend, waits for `/api/health`, and opens `firefox --kiosk` on it; closing Firefox (**Alt+F4**) also stops the backend. The settings above (`COUCHTUBE_PORT`, `COUCHTUBE_DATA_DIR`, …) work the same, and profiles are kept in the same data folder as a normal install.

Differences from `deploy/install.sh`:
- No systemd units, so no auto-start, crash restart, or daily sign-in refresh.
- The Firefox extension policy (uBlock Origin, SponsorBlock) isn't installed; copy `config/firefox/policies.json` to `/etc/firefox/policies/` by hand if you want it.
- AppImages need FUSE 2 (`sudo pacman -S fuse2`). Without it, run `./CouchTube-x86_64.AppImage --appimage-extract-and-run`.
