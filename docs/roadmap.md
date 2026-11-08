# Roadmap

Each phase ends with something you can test. The open checklist is [`TODO.md`](../TODO.md); design is in [`design.md`](design.md).

## Status (2026-11-08)
| Phase | State |
|---|---|
| 1 — Backend skeleton | ✅ Done |
| 2 — TV interface | ✅ Done (still to try in `firefox --kiosk`) |
| 3 — Playback | Implemented; user test positive |
| 4 — Profiles and sign-in | Implemented with phone pairing; user test positive |
| 5 — Account feeds | Implemented; history sync verified for a first watch |
| For You (own recommendations) | Implemented (seeds, exclusions, blended searches, 50/50 new channels, 2k-view minimum, no Shorts, History button, watched mark), see [`for-you.md`](for-you.md); not tried on screen |
| Playback in the CouchTube window | Implemented 2026-11-01 (timeline, subtitles, remote commands, mpv fallback); user test positive |
| 6 — Deploy | Written (`deploy/`, exit to desktop), see [`server.md`](server.md) |
| 7 — Polish | Implemented 2026-10-25; real-hardware checks in `TODO.md` |
| Twitch mode | Planned (separate Twitch profiles, start screen chooses the service), design in [`twitch-mode.md`](twitch-mode.md) |

Phases 1–5 ran on the dev machine, so they didn't wait for the server's Arch install. Server setup and Phase 0 ([`server.md`](server.md)) can happen whenever you're ready.

## Phases 1–5 (details in the code and git history)
- **1 — Backend skeleton ✅ 2026-10-20.** Axum on `127.0.0.1:7878`, `ytdlp.rs`, `/api/search`, serves `frontend/dist`. yt-dlp search takes about 1.2 s here.
- **2 — TV interface ✅ 2026-10-20.** Vite + React + TS (dev proxy `/api` → `:7878`); Home, Search, Results, Channel; 2D keyboard navigation with a remembered focus per screen; 10-foot dark styling, light on the GPU. Tested in headless Chromium at 1080p; still to try in `firefox --kiosk`.
- **3 — Playback ✅, user test positive.** (mpv, now legacy: see [`legacy-mpv.md`](legacy-mpv.md); the default player is the Firefox one.) `player.rs` runs one mpv at a time (`--fullscreen --hwdec=vaapi --ytdl-format=… --input-ipc-server=…`) and reads `time-pos`/`duration` over IPC; SponsorBlock mpv script; Enter plays, Esc returns to the same card.
- **4 — Profiles and sign-in ✅, user test positive.** SQLite profiles (folders `0700`, database `0600`), Guest never loads cookies, per-profile caches and recent searches, "Who's watching?" screen, per-account Firefox profile (`user.js`, uBlock Origin, SponsorBlock), sign-in window that closes itself once the login cookie appears, login check via one `:ytsubs` item, and phone pairing with encryption and a confirmation code (2026-10-21; see [`phone-signin-encryption.md`](phone-signin-encryption.md)). yt-dlp and mpv use the active profile's login.
- **5 — Account feeds ✅, real-account verification partly done.** Per-profile metadata, progress, queue and expiring caches in SQLite. Home rows: Continue watching (local), Recommended, Subscriptions; separate History, Watch Later, Liked and Queue screens. The backend follows mpv over one persistent IPC connection: it saves progress every ~5 s and on stop, advances the queue at EOF, and treats mpv `n` as "next". Watched sync runs once after ~30–35 s of actual viewing (`--simulate --mark-watched --no-flat-playlist`; pausing and seeking don't count). Caches: 10 min for account feeds/channels, 2 min for history, 30 min for search; `u` refreshes. History sync verified with a real account on 2026-10-25 (first watch, read back with `yt-dlp :ythis`).

## Phase 6 — Deploy and appliance mode
Written; not yet run on the server. See [`server.md`](server.md).

## Phase 7 — Polish (implemented 2026-10-25; real-hardware checks in `TODO.md`)
- **YouTube TV fallback tile**: opens youtube.com/tv fullscreen in the active profile's Firefox (TV user agent, uBlock Origin + SponsorBlock). For liking/subscribing, or when yt-dlp is broken.
- Subtitles: language per profile.
- HDMI-CEC power control, if the hardware supports it.
- Phone remote over the LAN: off by default, QR pairing confirmed on the TV, per-phone bearer token (`src/remote.rs`).

## Open questions
- The user reported playback and sign-in worked well on the dev machine (2026-10-20). A supplied log confirms AV1 at 1920×1080, 59.94 fps with VA-API and PipeWire.
- That log also contains `client removed during hook handling`, `Packet corrupt`, `Invalid OBU length` and `Failed to parse temporal unit`, followed by `Exiting... (Quit)` at about 20 seconds. The warnings have no confirmed cause; whether playback failed visibly or the user quit is still unknown. Retain AV1 support while gathering a reproduction.
- Explicit Firefox kiosk stacking/focus-return and two-account switching checks remain to be recorded. Server-specific Phase 0 checks are separate.
