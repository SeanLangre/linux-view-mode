# CouchTube

A fullscreen, keyboard-only YouTube app for a TV. It runs on an Arch Linux box connected to the TV over HDMI and is used from the couch with a wireless keyboard: browse with the arrow keys, press Enter, and the video plays fullscreen with no ads.

- **Backend:** Rust + Axum on `127.0.0.1:7878`. Runs `yt-dlp` for search and channel pages, and starts `mpv` to play videos.
- **Frontend:** React + TypeScript + Vite, shown in Firefox kiosk mode.

The full design and roadmap are in [`plan.md`](plan.md); the working checklist is [`TODO.md`](TODO.md).

## Status

| Works now | Coming next |
|---|---|
| Search, channel pages, keyboard navigation, back stack, recent searches, mpv playback, profiles/sign-in (with phone keyboard), account feeds, resume, queue, watched-history sync, and the deploy scripts (`deploy/`, written but not yet run on the server) | Real-account and server checks, polish (Phase 7) |

Press Enter on a video to launch fullscreen mpv. Esc or `q` in mpv stops playback; the interface keeps the same screen and selected card. The user has tested playback and profile sign-in successfully on the dev machine. Explicit Firefox kiosk stacking/focus-return and two-account switching checks remain on the checklist.

## Requirements

Arch Linux:

```bash
sudo pacman -S --needed rust nodejs npm yt-dlp mpv firefox curl xdotool
```

`xdotool` is only needed for typing from a phone during sign-in (see below).

## Run it

```bash
# Build the frontend once (the backend serves frontend/dist)
cd frontend && npm install && npm run build && cd ..

# Start the backend
cargo run

# Open it like the TV will
firefox --kiosk http://127.0.0.1:7878
```

CouchTube opens on **Who’s watching?** Choose Guest to browse signed out, or Add profile to connect an account.

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
| `COUCHTUBE_FRONTEND` | `frontend/dist` | Folder with the built frontend |
| `COUCHTUBE_MPV_CONFIG` | `config/mpv` | mpv config directory, including scripts (relative to the backend working directory) |
| `COUCHTUBE_DATA_DIR` | `$XDG_DATA_HOME/couchtube` or `~/.local/share/couchtube` | Private SQLite database and account Firefox profiles |
| `RUST_LOG` | `couchtube=info` | Log level, e.g. `couchtube=debug,tower_http=debug` |

## Profiles and sign-in

1. On **Who’s watching?**, choose **Add profile**, enter a name and choose a colour, then press **Create and sign in**. A QR code for your phone appears first; Firefox opens only when you choose **Open Firefox sign-in**.
2. Scan the QR code with your phone (see below), then choose **Open Firefox sign-in**. A separate Firefox window opens on a start page; press **Enter** to continue to Google. Sign in with the TV keyboard or the phone and complete any verification. The window closes by itself a few seconds after YouTube's login cookie appears; to give up, close it with **Alt+F4**.
3. Wait for **YouTube sign-in works**, then choose **Continue to CouchTube**. A failed check can mean a connection or yt-dlp problem; use **Check sign-in** or **Open sign-in again** to retry.
4. Press **P** while browsing to switch profiles. On the picker, highlight a named profile and press **S** to open sign-in (this opens Firefox straight away) or **D** to remove it (with confirmation).

Each account has its own Firefox folder; Guest never loads account cookies. A named profile with no cookie database browses signed out. Profiles and the active selection persist in SQLite, while the picker still appears on startup. Switching profiles stops mpv and opens Home for the selected person. Search results, feeds, recent searches, resume positions and queues are separated by profile. Removing a profile deletes its local sign-in files; deleting the active profile selects Guest.

### Signing in from your phone

The QR code (shown in the CouchTube window) opens a page on your phone that works as a keyboard for the Firefox sign-in window. It does not log in for you and does not use Bluetooth.

How it works: from when you create the profile (or open sign-in) until the sign-in window closes, the backend opens port 7879, creates a random 128-bit token and shows `http://<server LAN address>:7879/p/<token>` as a QR code (and as text under it). The phone page sends text to the backend, which types it into the Firefox window with `xdotool`. Closing the sign-in window ends the session, closes the port, and the link stops working.

Using it: scan the QR code (or type the link) and keep the phone page open. Press Enter on the TV to continue to Google so the email field is showing. On the phone, do the steps in order: **1. Email** (typed, then Enter), then once the TV shows the password page, **2. Password** (typed, then Enter, so a password manager can fill it), then **3. 2-step code** if Google asks. Approve any 2-step prompt on your phone. Use the TV keyboard for anything else.

Safety:
- **The password crosses your Wi-Fi unencrypted.** The phone page uses plain HTTP, so someone else on the same network could read what you type. Use it only on a network you trust, never on shared or guest Wi-Fi. HTTPS would need a certificate the phone trusts, so it isn't used; an encrypted form is planned.
- **Port 7879 is the only part of CouchTube reachable from the network, and only while a sign-in is in progress.** If you never open Firefox, the link expires after 15 minutes or when you leave the screen. The main API stays on `127.0.0.1` and rejects requests with any other `Host`. The pairing port answers only to the live token, with 404 for anything else. Anyone who sees the QR code or link while the window is open can type into it, so don't show the TV screen to people you don't trust during sign-in.
- Typed text is never logged. It is sent to `xdotool` on stdin (not visible in `ps`) without a shell, and is limited to 256 characters on one line.
- Typing fails closed: if the sign-in window can't be found or doesn't take focus, nothing is typed.
- Each of email, password and code can be sent up to 3 times per sign-in (so a typo can be fixed). After that the phone page says to use the TV keyboard.
- Keep port 7879 closed to the internet. Do not forward it on your router.

Limits:
- Needs `xdotool`. If it is missing, the phone page shows "the TV could not type".
- Firefox is started with `MOZ_ENABLE_WAYLAND=0` so that `xdotool` can reach it under XWayland. There is no equivalent that works on KDE's Wayland.
- Some phone password managers refuse to fill on `http://` pages. If yours does, copy the password from it and paste it into the field.
- You choose when to send the password: if you tap it while the email page is still showing, it is typed into the email field and can appear on the TV screen. Wait for the password page.
- The phone cannot see the TV screen, and it cannot click a field for you. The email field must already be focused on the TV.
- If you run the backend as a systemd service, set `DISPLAY` for it, or `xdotool` cannot find the window.
- The phone and the server must be on the same network. Guest networks and routers with client isolation block it.
- If the server has a firewall, allow the port for your home network only, for example `sudo ufw allow from 192.168.50.0/24 to any port 7879 proto tcp`.
- The address in the QR code is the one the OS uses to reach the outside. With a VPN or several network cards it may be the wrong one.
- Phones may ask for "Local network" or "Nearby devices" permission in the browser. Allow it so the page can load.

These weaknesses and the planned fixes are tracked under **Sign-in review** in [`TODO.md`](TODO.md).

The sign-in check requests one item from the account’s subscriptions. Home shows **Continue watching**, **Recommended**, and **Subscriptions**. The Home menu opens **History**, **Watch later**, **Liked**, and **Queue**. Guest and profiles without cookies have local watch history and can search, queue and resume videos; account feeds stay empty until sign-in. Daily cookie refresh is planned for deployment.

### Watching, resume and queue

- Enter plays a video from its saved position; **R** restarts the selected video from the beginning. Resume begins after at least five seconds of progress; completed videos and videos with less than five seconds remaining start fresh.
- **A** queues the selected video. **N** plays the next queued video (also inside mpv). On the Queue screen, Enter plays a selected item and removes it from the queue, **D** removes it, and the top controls play next or clear the queue.
- The next queued item starts automatically at the end of a video. Esc/q stops playback and keeps the remaining queue.
- The backend observes mpv over IPC and saves progress about every five seconds and when playback stops. This works without the browser polling.
- After roughly 30–35 seconds of viewing, a signed-in profile runs `yt-dlp --simulate --mark-watched` once for that video. Paused time and the distance skipped by seeking do not count as viewing. Failure is logged and can retry during a later viewing. Guest never sends watched-history updates.
- **U** refreshes the current Home/account feed or queue. Recommended, Subscriptions, Watch Later and Liked are cached per profile for 10 minutes; history for 2 minutes; searches for 30 minutes and channels for 10 minutes. Selecting an account prefetches Recommended and Subscriptions. The browser also briefly caches search/channel results.

Automated checks use simulated accounts and watched-history requests. The new personal feeds and YouTube history update still need verification with a real account.

`config/firefox/user.js` is copied into account profiles automatically. The supplied extension policy installs uBlock Origin and SponsorBlock when deployed to Firefox’s system policy directory. To enable it on this dev machine, first review the file and any existing `/etc/firefox/policies/policies.json`, then merge or install it there and restart Firefox. CouchTube does not change system Firefox policies automatically. See [Mozilla’s ExtensionSettings reference](https://firefox-admin-docs.mozilla.org/reference/policies/extensionsettings/).

## Keys

| Key | Action |
|---|---|
| ← ↑ → ↓ | Move between cards |
| Enter | Play / select |
| `/` | Search |
| `c` | Open the selected video's channel |
| Esc / Backspace | Back |
| Home | Go to the home screen |
| `p` | Profile picker (while browsing) |
| `a` | Add the selected video to the queue |
| `n` | Play the next queued video |
| `r` | Restart the selected video from the beginning |
| `d` | Remove the selected video on the Queue screen |
| `u` | Refresh Home, an account feed, or the queue |

In the search box, ↓ moves to the results and Esc goes back.

### During playback

Space pauses; ←/→ seek 10 seconds; ↑/↓ seek 60 seconds; `9`/`0` adjust volume; `m` mutes; `i` shows title and progress; `n` plays the next queued video; Esc/`q` quit. If Firefox retains focus, Esc stops playback through the API.

`config/mpv/mpv.conf` caps video at 1080p and requests VA-API for the Intel TV server. The dev-machine test log confirms AV1 1080p59.94 using VA-API. Hardware-decoding support still needs checking separately on the TV server. The bundled SponsorBlock script uses `curl` to request paid sponsor segments and skips them automatically. A missing request or segment leaves playback running. Queue controls and watched-history sync are implemented; subtitles and appliance setup come later.

### Reported playback warnings

One dev-machine session logged `client removed during hook handling`, `Packet corrupt`, `Invalid OBU length` and `Failed to parse temporal unit`, while successfully starting AV1 playback with hardware decoding. The cause is unconfirmed; a local-file playback check did not reproduce the hook warning. See [mpv’s hook handling](https://github.com/mpv-player/mpv/blob/v0.41.0/player/command.c#L181-L196) and [FFmpeg’s AV1 parser](https://github.com/FFmpeg/FFmpeg/blob/master/libavcodec/cbs_av1.c#L720-L727) for the messages’ origins.

If it happens again, note the video ID, whether the picture freezes or breaks up, whether audio continues, and whether you pressed Esc/q or playback stopped itself. Save the surrounding terminal output. `Exiting... (Quit)` records a quit request; the supplied excerpt does not establish why it was issued.

## API

| Method | Path | Returns |
|---|---|---|
| GET | `/api/profiles` | `{profiles: [{id, name, color}], active_id}` |
| POST | `/api/profiles` | Body `{name, color?}`; creates a local profile |
| DELETE | `/api/profiles/<id>` | Removes the profile and its local sign-in files |
| POST | `/api/profiles/<id>/select` | Selects a profile and stops playback |
| POST | `/api/profiles/<id>/signin` | Opens that profile’s Firefox sign-in window |
| GET/POST/DELETE | `/api/pairing` | Phone URL and QR code (SVG) while a sign-in is in progress, otherwise 404; POST starts it before Firefox opens, DELETE cancels it if no window is open |
| GET | `/api/profiles/<id>/status` | `{state, message}`; `?check=true` starts a login check |
| POST | `/api/system/exit` | Stops the kiosk service (only when it runs as the systemd unit) |
| GET | `/api/health` | `{"status":"ok"}` |
| GET | `/api/feed/<kind>` | Video list: `continue`, `recommended`, `subscriptions`, `history`, `watchlater`, `liked`; `?refresh=true` bypasses the feed cache |
| GET | `/api/queue` | Active profile’s queue in playback order |
| POST | `/api/queue` | Body `{id}`; appends once and returns the queue |
| DELETE | `/api/queue` or `/api/queue/<id>` | Clears the queue or removes one video |
| POST | `/api/next` | Stops playback and plays the next queued item; `{playing}` |
| GET | `/api/search?q=<text>&limit=<1-50>` | List of videos |
| GET | `/api/channel/<UC… id>` | `{id, name, videos}` |
| POST | `/api/play` | Body `{id, start?}`; starts/replaces mpv (saved resume position unless `start` is supplied), returns `{"status":"starting"}` |
| POST | `/api/stop` | Stops mpv, returns `{"status":"stopped"}` (safe when idle) |
| GET | `/api/status` | `{playing, id, session, title, position, duration, paused, error}` |

A video is `{id, title, channel, channel_id, duration, view_count, thumb, resume_position?}`; `duration` is `null` for live streams. Errors are `{"error": "..."}`.

Playback positions are seconds; `start` must be finite and non-negative. Status fields can be `null` while loading or for live streams. `playing` means the mpv process is running, including while loading or paused. Starting is asynchronous; a later playback failure appears in status. mpv output goes to the backend log. The backend stops mpv on Ctrl+C or SIGTERM.

## Tests and checks

```bash
cargo test && cargo clippy          # backend, including profiles, player lifecycle and IPC
cd frontend && npx vitest run       # navigation, API caching, recent searches
npm run lint && npm run build       # lint + type-check
```

The optional playback smoke test uses real mpv with a generated local audio file, with screen and sound output disabled. It needs permission to bind loopback and Unix sockets:

```bash
cargo build && python3 tests/playback_smoke.py
python3 tests/feeds_smoke.py        # simulated feeds, queue, progress and watched sync
python3 tests/profiles_smoke.py     # simulated accounts; no real browser or Google sessions
python3 tests/profiles_smoke.py --browser  # keyboard UI check; needs Chromium and Node 22+
lua tests/sponsorblock_test.lua     # needs Lua; tests SponsorBlock without network access
```

## Project layout

```
src/            Rust backend (main.rs, api.rs, ytdlp.rs, player.rs, playback.rs, profiles.rs, feeds.rs, db.rs)
frontend/       React app (App.tsx, nav.ts, api.ts, styles.css)
config/mpv/     Playback settings, keys, SponsorBlock script
config/firefox/ Account preferences and extension policy
plan.md         Design and roadmap
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
