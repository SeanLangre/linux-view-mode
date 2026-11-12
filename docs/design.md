# Design

A fullscreen, keyboard-only YouTube interface for the Arch Linux server connected to the TV over HDMI. It should feel like an appliance: it boots into a "Who's watching?" screen, you pick your Google account, browse your own recommendations and subscriptions with the arrow keys, and videos play fullscreen at up to 1080p with **no ads**. You never need a mouse, a terminal, or a browser tab.

How to run the code is in [`README.md`](../README.md); the working checklist is [`TODO.md`](../TODO.md). Related docs: [`legacy-mpv.md`](legacy-mpv.md), [`accounts.md`](accounts.md), [`server.md`](server.md), [`roadmap.md`](roadmap.md), [`for-you.md`](for-you.md), [`phone-signin-encryption.md`](phone-signin-encryption.md).

## Key decisions

| Decision | Choice | Why |
|---|---|---|
| UI | React + TypeScript + Vite in **Firefox kiosk mode** (`firefox --kiosk`) | Polished 10-foot interface; Firefox also hosts the sign-in windows and the fallback tile |
| Browser | **Firefox** (not Chromium) | Runs the full **uBlock Origin**; Chromium no longer supports it. yt-dlp reads Firefox cookies directly, with no keyring setup |
| Backend | **Rust + Axum** on `127.0.0.1` | Handles accounts, yt-dlp, starting mpv, storage |
| Accounts | **Several Google accounts**, each signed in once in its **own Firefox profile**; yt-dlp reuses that login (`--cookies-from-browser firefox:<dir>`) | Gives real recommendations, subscriptions, history, Watch Later and Liked videos for each person |
| Playback | Default (since 2026-11-01): YouTube's embedded player on `frontend/public/watch.html`, shown by the kiosk window, reporting to `POST /api/player/report`. **mpv** is the **legacy** fallback (see [`legacy-mpv.md`](legacy-mpv.md)) | The embed is not signed in, so it needs uBlock Origin for ads, and some videos refuse to embed (then mpv plays them) |
| Ad blocking | **uBlock Origin** + SponsorBlock in Firefox for the YouTube TV fallback (the player page also skips sponsor segments) | No YouTube Premium, so every path is ad-free |
| Quality | Capped at **1080p**: `bv*[height<=1080]+ba/b[height<=1080]` | Your choice. Iris Xe decodes every codec YouTube uses (AV1/VP9/H.264) in hardware, so the CPU stays free |
| History sync | Backend runs `yt-dlp --mark-watched` once a video has played for ~30 s | Videos you watch show up in your YouTube history and stop being recommended |
| Local storage | SQLite: profiles, resume positions, queue, cache | Resume positions and the queue are per profile |
| Autostart | systemd user services + KDE auto-login (Plasma Wayland) | Restarts on crash |

### What you accept with this approach
- **Google doesn't support it.** Accessing YouTube with your login through yt-dlp breaks YouTube's terms of service. YouTube *can* flag accounts that make many automated requests. CouchTube keeps requests low (caching, no bulk fetching), but the risk isn't zero. Test with **one account for a week** before adding everyone.
- **Read-only.** You can browse your feeds and add to history (by marking videos watched). You can't like, subscribe, or add to Watch Later from the TV. Do those on your phone (or in the YouTube TV fallback); they show up on the TV.
- **Login cookies are stored on the server's disk** (Firefox cookies aren't encrypted). Anyone who gets those files gets into the account. Keep the profile folders `chmod 700`, keep the backend local-only, and don't expose the server to the internet.

### Rejected alternatives
- **Chromium**: only supports uBlock Origin Lite, which is weak against YouTube ads; its cookies also need a keyring workaround.
- **YouTube Data API + Google sign-in**: no recommendations or watch history.
- **youtube.com/tv as the main UI**: not a custom interface. Kept as a **fallback tile** in Firefox with uBlock Origin, for when yt-dlp breaks or you need to like or subscribe.
- **iframe embed for playback**: captures keyboard focus, so Esc stops working; some videos won't play embedded; shows ads. *Revisited 2026-11-01:* mpv's direct streams started failing with HTTP 403, so playback now defaults to YouTube's embedded player inside the existing kiosk window (a page that keeps keyboard focus and mirrors mpv's keys and status), with mpv as the automatic fallback. See the README.

## Architecture

```
 Wireless keyboard
        │
        ▼
 ┌──────────────────────────┐  HTTP 127.0.0.1:7878  ┌─────────────────────────────┐
 │ Firefox --kiosk          │ ────────────────────▶ │ couchtube (Rust / Axum)     │
 │ React UI: profiles, home,│ ◀──────────────────── │  • profiles + sign-in       │
 │ search, channel, queue   │       JSON API        │  • feeds/search → yt-dlp    │
 └──────────────────────────┘                       │    (cookies of active prof.)│
                                                    │  • player → spawns mpv      │
 ┌──────────────────────────┐  spawn + IPC socket   │  • mark-watched after ~30 s │
 │ mpv --fullscreen, ≤1080p │ ◀──────────────────── │  • SQLite                   │
 │ VA-API, no ads           │ ────────────────────▶ │  • daily cookie refresh     │
 └──────────────────────────┘  position / end       └─────────────────────────────┘

 Fallback: Firefox (account profile + uBlock Origin) → youtube.com/tv
```

### Repo layout
The repo root is the Rust project.
```
Cargo.toml
src/
  main.rs         config, router, static files
  api.rs          HTTP handlers
  ytdlp.rs        running yt-dlp: search, channel, feeds, mark-watched
  profiles.rs     profiles, Firefox profile setup, sign-in, login check
  pairing.rs      phone pairing for sign-in: token, QR code, LAN page, xdotool typing
  pairing.html    phone page
  player.rs       the one player at a time: YouTube's embed page, legacy mpv + IPC, and Twitch live pages
  playback.rs     progress, watched sync, queue advancement
  feeds.rs        account feeds, persistent cache, stale-while-refresh and prefetch
  thumbs.rs       YouTube thumbnail cache in `<data>/thumbs/` (served by `/api/thumb/<id>`)
build.rs          embeds the git commit as `COUCHTUBE_COMMIT` (served by `/api/version`)
  discover.rs     For You discovery and ranking
  remote.rs       phone remote (port 7880, off until switched on) and remote.html
  cec.rs          HDMI-CEC TV power
  twitch.rs       Twitch mode routes: saved mode, Twitch profiles, browsing (see [`README-twitch.md`](README-twitch.md))
  gql.rs          Twitch's unofficial web API client (the default source, no registration)
  helix.rs        official Twitch Helix client (used once a Twitch application is registered)
  twitch_login.rs Twitch sign-in (Firefox folder, login cookie, check); a child module of profiles.rs
  vendor/         nacl.min.js (tweetnacl, inlined into the phone page)
  db.rs           SQLite profiles, progress, metadata, queue and per-profile cache
frontend/         Vite + React + TS
  src/App.tsx     screens, back stack, keyboard handling
  src/nav.ts      2D focus logic (unit-tested in nav.test.ts)
  src/api.ts      API client with an in-memory cache and stale-list refresh
  src/ModeGate.tsx     first screen: YouTube or Twitch
  src/ProfileGate.tsx  YouTube Who's watching, sign-in and phone pairing screens
  src/TwitchGate.tsx   Twitch Who's watching screen
  src/TwitchSignIn.tsx Twitch sign-in panel (QR code, phone code, Firefox)
  src/TwitchHome.tsx   Twitch Home, top live, categories, category and search screens
  src/format.ts, recent.ts  time formatting, recent searches
  src/styles.css  10-foot styling (1rem = 1/80 of screen width)
config/
  mpv/            legacy: mpv.conf, input.conf, scripts/sponsorblock.lua
  firefox/        policies.json (force-installs uBlock Origin + SponsorBlock), user.js
frontend/public/watch.html  embedded player page shown in the kiosk window
deploy/           install.sh, couchtube.service, couchtube-kiosk.{service,desktop}, couchtube-refresh{,.service,.timer}
packaging/        AppImage build
tests/            smoke tests (profiles, feeds, Twitch) with fake tools, SponsorBlock Lua test
```

### Crates
`axum` 0.8, `tokio` (full), `tower-http` (fs, trace), `serde`, `serde_json`, `anyhow`, `tracing`, `tracing-subscriber` (env-filter), `rusqlite` (bundled SQLite), `crypto_secretbox` (pinned, phone sign-in encryption), `sha2` (pinned, remote device tokens), `qrcode` (SVG QR codes).

### Frontend
React 19 + TypeScript + Vite 8, tested with vitest, linted with oxlint. No UI library: the app is one screen stack and a row/column focus model.

### API
The shared API table is in the [README](../README.md#api); each mode's endpoints are in its own README: [YouTube](README-youtube.md#api), [Twitch](README-twitch.md#api) and [Music](README-music.md#http-api).

### Data (SQLite, `~/.local/share/couchtube/couchtube.db`)
- `profiles(id, name, color, created_at)` and `settings(key, value)` store profiles and active selection
- `progress(profile_id, video_id, position, duration, watched, marked, completed, updated_at)` drives Continue watching and watched sync
- `queue(position, profile_id, video_id)`
- `videos(profile_id, video_id, json)` caches metadata per profile
- `feed_cache(profile_id, cache_key, json, expires_at)` stores feed/search/channel responses; expired rows are kept for `STALE_SECS` (7 days) so they can be shown while refreshing
- `<data>/thumbs/` (files, not SQLite) holds saved YouTube thumbnails, at most 2000, trimmed to the newest 1500
- `seeds` and `dismissed` back For You (see [`for-you.md`](for-you.md))
- `remote_devices` holds the phone remote's paired phones (token hashes only)
- `twitch_profiles` is a separate list from `profiles` (its own Guest); `settings` also keeps `mode` and `twitch_active_profile`

## Security rules

- Backend binds to **127.0.0.1 only** (it starts processes and holds account access). The one exception is the phone pairing port (7879), which answers only to the live sign-in token and the phone remote port (7880), which opens only while the remote is switched on and admits only phones paired with a device token (plain HTTP, trusted network only).
- The main server rejects requests whose `Host` isn't `127.0.0.1:<port>`/`localhost:<port>` (DNS rebinding), and port 7879 only listens during sign-in.
- Never put secrets in process arguments (readable via `ps`); pass them on stdin. Typing tools must fail closed if the target window isn't found.
- Validate video IDs (`^[A-Za-z0-9_-]{11}$`) and channel IDs (`^UC[A-Za-z0-9_-]{22}$`). Always build URLs yourself. Never use a shell. Put `--` before positional arguments.
- Profile folders `0700`. Never log cookies.
- Errors go to the UI as `{"error": "..."}` with short, user-readable messages; details stay in the server log.

## Keys

The keyboard map (UI and playback) is in the [YouTube README](README-youtube.md#keys). Legacy mpv bindings live in `config/mpv/input.conf`; the embedded player page (`watch.html`) accepts the same keys, plus `c` for subtitles.
