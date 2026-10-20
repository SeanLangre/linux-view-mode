# CouchTube — plan

A fullscreen, keyboard-only YouTube interface for the Arch Linux server connected to the TV over HDMI. It should feel like an appliance: it boots into a "Who's watching?" screen, you pick your Google account, browse your own recommendations and subscriptions with the arrow keys, and videos play fullscreen at up to 1080p with **no ads**. You never need a mouse, a terminal, or a browser tab.

The original chat transcript this plan came from is kept in `plan.chat.md`.

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
| Home: Recommended | `:ytrec` |
| Subscriptions | `:ytsubs` |
| History | `:ythistory` |
| Watch Later | `:ytwatchlater` |
| Liked | `:ytfav` |
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
```
couchtube/
  Cargo.toml
  src/
    main.rs        # config, router, static files
    api.rs         # HTTP handlers
    profiles.rs    # profiles, Firefox profile setup, sign-in, login check, refresh
    ytdlp.rs       # running yt-dlp: feeds, search, channel, mark-watched, cache
    player.rs      # mpv process + IPC (one player at a time)
    db.rs          # SQLite schema + queries
  frontend/        # Vite + React + TS
  config/
    mpv/           # mpv.conf, input.conf, scripts/sponsorblock
    firefox/       # policies.json (force-installs uBlock Origin + SponsorBlock), user.js
  deploy/
    install.sh                         # build + install on the server
    couchtube.service                  # backend
    couchtube-kiosk.service            # Firefox kiosk
    couchtube-refresh.{service,timer}  # daily cookie refresh
```

### Crates
`axum`, `tokio` (full), `tower-http` (fs), `serde`, `serde_json`, `rusqlite` (bundled), `anyhow`, `tracing`, `tracing-subscriber`.

### API (first version)
| Method | Path | Purpose |
|---|---|---|
| GET/POST/DELETE | `/api/profiles` | List / create / remove profiles |
| POST | `/api/profiles/:id/select` | Set the active profile |
| POST | `/api/profiles/:id/signin` | Open the sign-in window |
| GET | `/api/profiles/:id/status` | Is the login valid? |
| GET | `/api/feed/:kind` | `recommended`, `subscriptions`, `history`, `watchlater`, `liked` |
| GET | `/api/search?q=` | Search |
| GET | `/api/channel/:id` | Channel uploads |
| POST | `/api/play` `{id, start?}` / `/api/stop` | Start or stop mpv |
| GET | `/api/status` | Now playing: title and position |
| GET/POST/DELETE | `/api/queue` | Queue for the active profile |
| POST | `/api/fallback/youtube-tv` | Open youtube.com/tv in the active profile's Firefox |
| POST | `/api/system/exit` | Close kiosk and go back to the desktop |

### Data (SQLite, `~/.local/share/couchtube/couchtube.db`)
- `profiles(id, name, color, created_at)`
- `progress(profile_id, video_id, position, duration, updated_at)` drives "Continue watching"
- `queue(profile_id, pos, video_id)`
- `videos(id, title, channel_id, channel_name, duration, thumb)` caches metadata

### Security rules
- Backend binds to **127.0.0.1 only** (it starts processes and holds account access).
- Validate video IDs (`^[A-Za-z0-9_-]{11}$`) and channel IDs (`^UC[A-Za-z0-9_-]{22}$`). Always build URLs yourself. Never use a shell. Put `--` before positional arguments.
- Profile folders `0700`. Never log cookies.

---

## 5. Keyboard map

**In the UI**
| Key | Action |
|---|---|
| ← ↑ → ↓ | Move between cards (rows and columns) |
| Enter | Play / open |
| Esc / Backspace | Back (on Home: open the profile picker) |
| `/` | Jump to search |
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
- `cargo new couchtube`, Axum on `127.0.0.1:7878`, `ytdlp.rs` with search signed out, `/api/search`, serves `frontend/dist`.
- **Done when:** `curl '127.0.0.1:7878/api/search?q=lofi'` returns clean JSON.

### Phase 2 — TV interface
- Vite + React + TS. Dev proxy `/api` → `:7878`.
- Screens: Home, Search, Results, Channel.
- 2D grid navigation: a visible focus ring, the focused card scrolled into view, and a remembered focus position per screen.
- 10-foot styling: dark theme, large text (≥ 24 px at 1080p), 16:9 thumbnails, nothing important near the screen edges, light on the RN96's GPU.
- **Done when:** you can search and browse with the keyboard only, in `firefox --kiosk`.

### Phase 3 — Playback
- `player.rs`: mpv with `--fullscreen --hwdec=vaapi --ytdl-format=… --input-ipc-server=$XDG_RUNTIME_DIR/couchtube-mpv.sock --config-dir=…`, one player at a time, reads `time-pos` and `duration` over IPC.
- SponsorBlock mpv script.
- The UI shows "Playing: <title>" while mpv runs.
- **Done when:** Enter plays, Esc returns to the same card.

### Phase 4 — Profiles and sign-in
- "Who's watching?" screen, add/remove profile, Firefox profile setup (`user.js`, uBlock Origin, SponsorBlock), sign-in window, login check, Guest profile.
- yt-dlp and mpv use the active profile's login.
- **Done when:** two accounts can be signed in and switched with `p`.

### Phase 5 — Account feeds
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

None right now. Phase 0 results may raise new ones.

### Decided
- Server GPU: **Intel Iris Xe** (Tiger Lake), so AV1 is allowed.
- Server OS: Windows today; it will be reinstalled as **Arch + KDE Plasma (Wayland)**.
- The code reaches the server via **this git repo**, pulled on the server.
