# CouchTube — plan

A fullscreen, keyboard-only YouTube interface for the Arch Linux server connected to the TV over HDMI. It should feel like an appliance: it boots into a "Who's watching?" screen, you pick your Google account, browse your own recommendations and subscriptions with the arrow keys, and videos play fullscreen at up to 1080p with **no ads**. You never need a mouse, a terminal, or a browser tab.

The original chat transcript this plan came from is kept in `plan.chat.md`. How to run the code is in `README.md`; the working checklist is `TODO.md`.

### Status (2026-10-20)
| Phase | State |
|---|---|
| Server setup (Windows → Arch) | Not started |
| 0 — Server checks | Not started (needs the Arch install) |
| 1 — Backend skeleton | ✅ Done |
| 2 — TV interface | ✅ Done (still to try in `firefox --kiosk`) |
| 3 — Playback | Implemented; user test positive, explicit focus check pending |
| 4 — Profiles and sign-in | Implemented; user test positive, explicit two-account check pending |
| 5 — Account feeds | Implemented; real-account verification pending |
| 6–7 | Not started |

---

## 1. Machines

| | Dev machine (this one) | TV server |
|---|---|---|
| Role | Write, build, test | Runs CouchTube on the TV |
| Hardware | — | **Acer Revo RN96** mini PC |
| OS | Arch Linux | **Windows right now** → to be replaced by Arch Linux (see Server setup, Phase 0) |
| Desktop | KDE Plasma (Wayland) | **KDE Plasma 6, Wayland** |
| Display | Monitor | **TV is the only screen** |
| GPU | NVIDIA RTX 5060 Ti | **Intel Iris Xe** (11th gen Tiger Lake, PCI `8086:9A49`): hardware decoding of **AV1**, VP9, HEVC, H.264 |
| Tools | Rust 1.99, Node 22, Firefox, mpv, yt-dlp, SQLite | To install (see Phase 0) |

### Workflow
- Develop and run everything locally on the dev machine. The full app works here too, just on the monitor instead of the TV.
- Push the code to a git repo (a private GitHub repo, for example). A USB stick also works.
- **On the server**, at the TV with the wireless keyboard: `git pull && ./deploy/install.sh`. The script builds the backend and frontend on the server, installs the files, the systemd user units and the Firefox policy, then restarts the services. No SSH needed.
- The server needs: `firefox mpv yt-dlp intel-media-driver libva-utils git` to run it, plus `rust nodejs npm` to build it. The first build on the RN96 takes a few minutes; later ones are quick.
- Later option: switch to an Arch `PKGBUILD` so updates become `makepkg -si` and pacman tracks the installed files.

---

## 2. Key decisions

| Decision | Choice | Why |
|---|---|---|
| UI | React + TypeScript + Vite in **Firefox kiosk mode** (`firefox --kiosk`) | Polished 10-foot interface; Firefox also hosts the sign-in windows and the fallback tile |
| Browser | **Firefox** (not Chromium) | Runs the full **uBlock Origin**; Chromium no longer supports it. yt-dlp reads Firefox cookies directly, with no keyring setup |
| Backend | **Rust + Axum** on `127.0.0.1` | Handles accounts, yt-dlp, starting mpv, storage |
| Accounts | **Several Google accounts**, each signed in once in its **own Firefox profile**; yt-dlp reuses that login (`--cookies-from-browser firefox:<dir>`) | Gives real recommendations, subscriptions, history, Watch Later and Liked videos for each person |
| Playback | **mpv** (uses yt-dlp internally) with the active account's login | **No ads** (it gets the video streams directly), no embed restrictions, native keyboard controls |
| Ad blocking | Built in for playback (mpv); **uBlock Origin** + SponsorBlock in Firefox for the YouTube TV fallback; SponsorBlock script in mpv | No YouTube Premium, so every path is ad-free |
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
- **youtube.com/tv as the main UI**: not a custom interface. Kept as a **fallback tile** (Phase 7) in Firefox with uBlock Origin, for when yt-dlp breaks or you need to like or subscribe.
- **iframe embed for playback**: captures keyboard focus, so Esc stops working; some videos won't play embedded; shows ads.

---

## 3. Accounts in detail

### Profiles
- Each profile = `{id, name, color}` plus a fixed **Guest** profile (signed out).
- Each has a Firefox profile folder at `~/.local/share/couchtube/profiles/<id>/firefox`, permissions 700, with a `user.js` (no first-run screens, no session restore, no telemetry).
- **uBlock Origin + SponsorBlock are built into the setup, not installed by hand**: `config/firefox/policies.json` is deployed to `/etc/firefox/policies/policies.json` and uses `ExtensionSettings` with `installation_mode: "force_installed"`. Every Firefox profile on the server (kiosk and accounts) gets them automatically, they update themselves, and they can't be removed. Writing our own blocker was rejected: YouTube changes its ads all the time and uBlock Origin's filters keep up with it; ours wouldn't.
- The kiosk interface uses **its own** separate Firefox profile (`~/.local/share/couchtube/kiosk-firefox`). Account logins never load in the interface.
- One profile is active at a time. Every yt-dlp and mpv call uses that profile's login.

### Signing in (once per account, from the TV)
1. "Add account" in the UI → backend opens a separate Firefox window:
   `firefox --new-instance --profile <dir> https://accounts.google.com/ServiceLogin?continue=https://www.youtube.com`
2. Sign in with the keyboard (approve 2FA on your phone), then close the window (Ctrl+W).
3. Backend checks the login works by fetching one page of `:ytsubs`. The profile shows a ✓ or "Sign in again".

### Keeping the login alive
YouTube refreshes some login cookies over time, and a session that's never refreshed eventually logs out. Plan: once a day, a timer briefly runs `firefox --headless --profile <dir> https://www.youtube.com` (about 30 s) so the cookies get refreshed. *Phase 0 checks that this is needed and that it works.*

### Feeds (yt-dlp, with the active profile's login)
| Row / screen | Source |
|---|---|
| Home: Recommended | `https://www.youtube.com/feed/recommended` |
| Subscriptions | `https://www.youtube.com/feed/subscriptions` |
| History | `https://www.youtube.com/feed/history` |
| Watch Later | `https://www.youtube.com/playlist?list=WL` |
| Liked | `https://www.youtube.com/playlist?list=LL` |
| Search | `ytsearch24:<query>` (personalized when signed in) |
| Channel | `https://www.youtube.com/channel/<id>/videos` |

All with `--flat-playlist -J --playlist-end N`. Results are cached per profile (Home and Subscriptions about 10 min, history about 2 min, search about 30 min). At most one yt-dlp request runs at a time per profile.

---

## 4. Architecture

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
The repo root is the Rust project. ✅ = exists today.
```
Cargo.toml                 ✅
src/
  main.rs                  ✅ config, router, static files
  api.rs                   ✅ HTTP handlers
  ytdlp.rs                 ✅ running yt-dlp: search, channel (later: feeds, mark-watched)
  profiles.rs              ✅ profiles, Firefox profile setup, sign-in, login check (refresh timer in Phase 6)
  player.rs                ✅ mpv process + persistent IPC (one player at a time)
  playback.rs              ✅ progress, watched sync, queue advancement
  feeds.rs                 ✅ account feeds, persistent cache and prefetch
  db.rs                    ✅ SQLite profiles, progress, metadata, queue and per-profile cache
frontend/                  ✅ Vite + React + TS
  src/App.tsx              ✅ screens, back stack, keyboard handling
  src/nav.ts               ✅ 2D focus logic (unit-tested in nav.test.ts)
  src/api.ts               ✅ API client with a 10-minute cache
  src/styles.css           ✅ 10-foot styling (1rem = 1/80 of screen width)
config/
  mpv/                     ✅ mpv.conf, input.conf, scripts/sponsorblock.lua
  firefox/                 ✅ policies.json (force-installs uBlock Origin + SponsorBlock), user.js
deploy/
  install.sh                         # build + install on the server
  couchtube.service                  # backend
  couchtube-kiosk.service            # Firefox kiosk
  couchtube-refresh.{service,timer}  # daily cookie refresh
```

### Crates
In use: `axum` 0.8, `tokio` (full), `tower-http` (fs, trace), `serde`, `serde_json`, `anyhow`, `tracing`, `tracing-subscriber` (env-filter).
Profile storage uses `rusqlite` (bundled SQLite). Progress, queue and feed storage are implemented.

### Frontend
React 19 + TypeScript + Vite 8, tested with vitest, linted with oxlint. No UI library: the app is one screen stack and a row/column focus model.

### API (first version)
✅ = implemented.
| Method | Path | Purpose |
|---|---|---|
| GET | `/api/health` ✅ | `{"status":"ok"}` |
| GET/POST | `/api/profiles` ✅ | List / create profiles |
| DELETE | `/api/profiles/:id` ✅ | Remove a named profile and its local sign-in files |
| POST | `/api/profiles/:id/select` ✅ | Set the active profile |
| POST | `/api/profiles/:id/signin` ✅ | Open the sign-in window |
| GET | `/api/profiles/:id/status` ✅ | Login state/message; `?check=true` starts a check |
| GET | `/api/feed/:kind` ✅ | `continue`, `recommended`, `subscriptions`, `history`, `watchlater`, `liked` |
| GET | `/api/search?q=&limit=` ✅ | Search (default 24, max 50 results) |
| GET | `/api/channel/:id` ✅ | Channel name + latest 24 uploads |
| POST | `/api/play` `{id, start?}` / `/api/stop` ✅ | Start or stop mpv |
| GET | `/api/status` ✅ | Process running, video ID, title, position, duration, pause and error |
| GET/POST/DELETE | `/api/queue` ✅ | Queue for the active profile; DELETE `/api/queue/:id` removes one |
| POST | `/api/next` ✅ | Stop and play the next queued item |
| POST | `/api/fallback/youtube-tv` | Open youtube.com/tv in the active profile's Firefox |
| POST | `/api/system/exit` | Close kiosk and go back to the desktop |

### Data (SQLite, `~/.local/share/couchtube/couchtube.db`)
- `profiles(id, name, color, created_at)` and `settings(key, value)` store profiles and active selection
- `progress(profile_id, video_id, position, duration, watched, marked, completed, updated_at)` drives Continue watching and watched sync
- `queue(position, profile_id, video_id)`
- `videos(profile_id, video_id, json)` caches metadata per profile
- `feed_cache(profile_id, cache_key, json, expires_at)` stores feed/search/channel responses

### Security rules
- Backend binds to **127.0.0.1 only** (it starts processes and holds account access).
- Validate video IDs (`^[A-Za-z0-9_-]{11}$`) and channel IDs (`^UC[A-Za-z0-9_-]{22}$`). Always build URLs yourself. Never use a shell. Put `--` before positional arguments.
- Profile folders `0700`. Never log cookies.
- Errors go to the UI as `{"error": "..."}` with short, user-readable messages; details stay in the server log.

---

## 5. Keyboard map

**In the UI**
| Key | Action |
|---|---|
| ← ↑ → ↓ | Move between cards (rows and columns) |
| Enter | Play / open |
| Esc / Backspace | Back (on Home: open the profile picker, from Phase 4) |
| `/` | Jump to search |
| `c` | Open the selected video's channel |
| `a` | Add the selected video to the queue |
| `p` | Switch profile |
| Home | Go to the home screen |

**In mpv** (`config/mpv/input.conf`)
| Key | Action |
|---|---|
| Space / media Play | Pause / play |
| ← / → | Seek 10 s |
| ↑ / ↓ | Seek 60 s |
| Esc / `q` | Stop and go back to the UI (Esc is remapped from its default "exit fullscreen") |
| `n` | Next in queue |
| `9` / `0`, `m` | Volume, mute |
| `i` | Title and progress overlay |

---

## 6. Risks to check first

1. **mpv appearing above the Firefox kiosk on KDE Wayland**: KWin may keep Firefox in front. Fix: a KWin window rule for mpv (force fullscreen, keep above, accept focus).
2. **Hardware decoding on the RN96**: `vainfo` must list AV1, VP9 and H.264; mpv with `--hwdec=vaapi`; check that 1080p60 plays smoothly with low CPU use. Keep the React UI light anyway (no heavy blur effects) so it stays smooth on integrated graphics.
3. **Logins expiring**: how long do the cookies keep working, and does the daily refresh fix it?
4. **Account flagging**: start with one account and few requests.
5. **yt-dlp breaking**: YouTube changes often break yt-dlp. Keep it updated, show a clear error in the UI, and keep the YouTube TV fallback ready.
6. **yt-dlp speed**: requests take about 2–5 s (a bit slower on the RN96). Show a loading state, use cached results first, prefetch the Home rows when a profile is picked.

---

## 7. Phases

Each phase ends with something you can test.

Phases 1–5 run on the dev machine, so they don't have to wait for the server's Arch install. Server setup and Phase 0 can happen whenever you're ready.

### Server setup — Windows → Arch + KDE Plasma (on the RN96)
- Back up anything you want to keep from the Windows install. The disk will be wiped.
- Write the Arch ISO to a USB stick. In the RN96 firmware (BIOS), **turn off Secure Boot** and boot from USB.
- Run `archinstall`:
  - Profile: **Desktop → KDE Plasma**, greeter **SDDM**
  - Graphics driver: **Intel (open-source)**
  - Audio: **PipeWire**
  - Network: **NetworkManager**
  - Create a normal user (CouchTube runs as this user).
- After the first boot: Plasma 6 starts on Wayland by default. Set the TV's resolution and scaling in System Settings → Display, and check sound goes out over HDMI (System Settings → Sound).
- **Done when:** the RN96 boots into Plasma on the TV, with sound over HDMI.

### Phase 0 — Server checks (on the TV server)
- Install: `sudo pacman -S --needed firefox mpv yt-dlp intel-media-driver libva-utils git rust nodejs npm`. Run `vainfo` and check it lists `VAProfileAV1Profile0`, `VAProfileVP9Profile0` and `VAProfileH264High` with `VAEntrypointVLD`. Confirm the session is Wayland (`echo $XDG_SESSION_TYPE`).
- `mpv --fullscreen --hwdec=vaapi --ytdl-format='bv*[height<=1080]+ba/b[height<=1080]' https://www.youtube.com/watch?v=aqz-KE-bpKQ` plays smoothly (press `i` in mpv to confirm the codec and that hardware decoding is on).
- With `firefox --kiosk` open, start mpv: does it show on top with focus, and does focus come back afterwards? Add a KWin rule if needed.
- Create a test profile: `firefox --new-instance --profile ~/ct-test https://accounts.google.com` (create the folder first), sign in **one** account, close it, then run `yt-dlp --cookies-from-browser firefox:$HOME/ct-test --flat-playlist -J --playlist-end 5 :ytrec`. Do you get your recommendations?
- `yt-dlp --cookies-from-browser firefox:$HOME/ct-test --mark-watched --simulate <url>`: does the video appear in your YouTube history?
- Run the `:ytrec` command again after 2–3 days, without opening Firefox, to see whether the login lasts.
- **Done when:** everything works, or each problem has a known fix.

### Phase 1 — Backend skeleton (dev machine) ✅ done 2026-10-20
- Axum on `127.0.0.1:7878`, `ytdlp.rs` with search signed out, `/api/search`, serves `frontend/dist`.
- Measured: yt-dlp search takes about 1.2 s on the dev machine.
- **Done when:** `curl '127.0.0.1:7878/api/search?q=lofi'` returns clean JSON.

### Phase 2 — TV interface ✅ done 2026-10-20
Tested by driving the built app in headless Chromium at 1920×1080 with real key presses; still to try in `firefox --kiosk`. Added `/api/channel/:id` for the Channel screen.
- Vite + React + TS. Dev proxy `/api` → `:7878`.
- Screens: Home, Search, Results, Channel.
- 2D grid navigation: a visible focus ring, the focused card scrolled into view, and a remembered focus position per screen.
- 10-foot styling: dark theme, large text (≥ 24 px at 1080p), 16:9 thumbnails, nothing important near the screen edges, light on the RN96's GPU.
- Phase 2 initially used Music/Nature/Science placeholder rows. Phase 5 replaced them with account feeds. Enter launches mpv (added in Phase 3).
- The first-search Home focus quirk was fixed in Phase 5: recent searches follow the stable video rows, with a browser regression test.
- **Done when:** you can search and browse with the keyboard only, in `firefox --kiosk`.

### Phase 3 — Playback — implemented, user test positive
Backend tests cover process replacement, stop, failed exits and IPC replies; frontend tests cover playback requests and uncached status. A headless smoke test with real mpv and local audio passed (API validation, title/position/pause, replacement, quit, stop and backend shutdown). SponsorBlock has a Lua test for skipping and failure handling. The Firefox kiosk/KDE stacking and focus check remains manual.
- `player.rs`: mpv with `--fullscreen --hwdec=vaapi --ytdl-format=… --input-ipc-server=$XDG_RUNTIME_DIR/couchtube-<session>/ipc --config-dir=…`, one player at a time, reads `time-pos` and `duration` over IPC.
- SponsorBlock mpv script (paid sponsor segments; `curl`, asynchronous, leaves playback running on failure).
- The UI shows "Playing: <title>" while mpv runs.
- Enter calls `/api/play`; status is polled without caching, and the screen/focus stay in place during playback.
- Browser test: Firefox kiosk + mpv on top on KDE Wayland can be checked on the dev machine already (same desktop as the server).
- **Done when:** Enter plays, Esc returns to the same card.

### Phase 4 — Profiles and sign-in — implemented, user test positive
- SQLite stores profile names/colours and active selection; profile folders are private (`0700`) and the database is `0600`.
- The extension policy is supplied in `config/firefox/policies.json`; deployment to system Firefox policies remains manual until Phase 6.
- Search and channel requests use the selected profile’s cookies and run one at a time per profile. mpv receives the same cookie source. Guest ignores desktop yt-dlp config and never loads account cookies.
- The picker opens at startup; `p` opens it while browsing. Browser caches and recent searches are separated by profile.
- Sign-in status tracks Firefox opening/closing and a single-item `:ytsubs` check, with a manual retry.
- Verified with simulated account tools: isolation of cookies, Guest, login/check failures, deletion and restart persistence. Headless Chromium exercised Guest → add/sign-in → playback/return → remove → switch using keyboard events. Real mpv IPC confirmed cookie paths with spaces/commas are passed correctly. The user subsequently reported the real profile/sign-in flow worked well (2026-10-20). The exact two-account switching check remains to be confirmed.
- "Who's watching?" screen, add/remove profile, Firefox profile setup (`user.js`, uBlock Origin, SponsorBlock), sign-in window, login check, Guest profile.
- yt-dlp and mpv use the active profile's login.
- **Done when:** two accounts can be signed in and switched with `p`.

### Phase 5 — Account feeds — implemented, real-account verification pending
- SQLite adds per-profile video metadata, progress, queue and expiring response caches without replacing existing profiles.
- Guest has local history/resume/queue; account feeds require a cookie database. Feed sources use canonical YouTube URLs, including History, WL and LL.
- The backend observes mpv properties and end-file events over one persistent IPC connection. It saves progress every ~5 s and on stop, advances the queue at EOF, and treats mpv `n` (`quit 5`) as an explicit next request. Esc/q leaves the remaining queue intact.
- Watched sync runs once after ~30–35 s of accumulated actual viewing, using `--simulate --mark-watched --no-flat-playlist`. Pausing and seeking do not add skipped time; failures can retry in a later session. Guest never sends watched updates.
- Server caches retain responses for 10 min for account feeds/channels, 2 min for history and 30 min for search; selecting an account prefetches Recommended/Subscriptions. `u` refreshes the current feed.
- Automated checks cover cache/profile isolation, resume, stop versus EOF/next, watched-sync arguments, persistence, and keyboard feed/queue navigation. Personal feed contents and YouTube history sync still need a real-account test.
- Home rows: Continue watching (local), Recommended, Subscriptions. Separate screens for History, Watch Later and Liked.
- Mark videos watched after ~30 s. Resume positions and queue per profile.
- Per-profile cache; prefetch when a profile is picked.
- **Done when:** each person sees their own YouTube home, and what they watch appears in their YouTube history.

### Phase 6 — Deploy and appliance mode
- `install.sh` (run on the server): `cargo build --release`, `npm ci && npm run build`, install to `~/.local/share/couchtube/`, copy the systemd user units, `sudo` copy `policies.json` to `/etc/firefox/policies/`, `systemctl --user daemon-reload && systemctl --user restart couchtube couchtube-kiosk`.
- `couchtube.service` (backend, `Restart=on-failure`).
- `couchtube-kiosk.service`: `firefox --kiosk --new-instance --profile ~/.local/share/couchtube/kiosk-firefox http://127.0.0.1:7878`, `Restart=always`.
- `couchtube-refresh.timer`: daily cookie refresh.
- KDE: auto-login into Plasma (Wayland); turn off screen lock, screen blanking and sleep.
- "Exit to desktop" item, and a KDE global shortcut to start the kiosk again.
- **Done when:** the TV boots into "Who's watching?", and killing Firefox brings it back.

### Phase 7 — Polish (pick as needed)
- **YouTube TV fallback tile**: opens youtube.com/tv fullscreen in the active profile's Firefox (TV user agent, uBlock Origin + SponsorBlock). For liking/subscribing, or when yt-dlp is broken.
- Subtitles: language per profile.
- HDMI-CEC power control, if the hardware supports it.
- Phone remote over the LAN, only with authentication.

---

## 8. Open questions

- The user reported playback and sign-in worked well on the dev machine (2026-10-20). A supplied log confirms AV1 at 1920×1080, 59.94 fps with VA-API and PipeWire.
- That log also contains `client removed during hook handling`, `Packet corrupt`, `Invalid OBU length` and `Failed to parse temporal unit`, followed by `Exiting... (Quit)` at about 20 seconds. The warnings have no confirmed cause; whether playback failed visibly or the user quit is still unknown. Retain AV1 support while gathering a reproduction.
- Explicit Firefox kiosk stacking/focus-return and two-account switching checks remain to be recorded. Server-specific Phase 0 checks are separate.

### Decided
- Server GPU: **Intel Iris Xe** (Tiger Lake), so AV1 is allowed.
- Server OS: Windows today; it will be reinstalled as **Arch + KDE Plasma (Wayland)**.
- The code reaches the server via **this git repo**, pulled on the server.
