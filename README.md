# CouchTube

A fullscreen, keyboard-only YouTube app for a TV. It runs on an Arch Linux box connected to the TV over HDMI and is used from the couch with a wireless keyboard: browse with the arrow keys, press Enter, and the video plays fullscreen with no ads.

- **Backend:** Rust + Axum on `127.0.0.1:7878`. Runs `yt-dlp` for search and channel pages, tracks playback, and plays videos in the Firefox window (YouTube's embedded player), with `mpv` as the fallback.
- **Frontend:** React + TypeScript + Vite, shown in Firefox kiosk mode.

Design notes live in [`docs/`](docs/): [`design.md`](docs/design.md) (decisions, architecture, security rules), [`accounts.md`](docs/accounts.md) (profiles, sign-in, feeds), [`phone-signin-encryption.md`](docs/phone-signin-encryption.md), [`for-you.md`](docs/for-you.md), [`server.md`](docs/server.md) (TV server setup and checks) and [`roadmap.md`](docs/roadmap.md), [`legacy-mpv.md`](docs/legacy-mpv.md) (mpv, no longer the main player). In progress: [`twitch-mode.md`](docs/twitch-mode.md) (Twitch mode). The working checklist is [`TODO.md`](TODO.md).

## Playback in the CouchTube window

By default a video plays inside the Firefox kiosk window that already shows CouchTube, so there is no second window to focus. The backend reports the player page (`frontend/public/watch.html`) in `/api/status` (`page`); the interface then covers the screen with it. The page shows YouTube's embedded player and reports position, duration and pause state to `POST /api/player/report`; the answer carries the commands from the phone remote. It keeps mpv's keys (**Space**, arrows, **9**/**0**, **M**, **C**, **I**, **N**, **Esc**/**Q**), skips paid sponsor segments, and honours the profile's subtitle language. A thin red line along the bottom edge shows progress; after a seek, a pause or **I** the full timeline with times appears. **C** (or the phone remote's subtitles button) switches subtitles on and off, using the profile's language when the video has it. Resume, progress, watched marks, the queue and "Continue watching" work as before. The backend writes `kiosk-firefox/user.js` at start so that Firefox may autoplay.

mpv is the legacy fallback: it plays the video instead, from the same position, when YouTube refuses to embed it or the page has not started within 20 seconds. Details, limits and `COUCHTUBE_PLAYER=mpv` are in [`docs/legacy-mpv.md`](docs/legacy-mpv.md). The kiosk profile is not signed in to YouTube (account logins stay out of the interface), so Premium, age-restricted videos and your account's ad settings do not apply; YouTube chooses the quality; ad skip buttons cannot be clicked, so install uBlock Origin through the Firefox policy (`config/firefox/policies.json`).

## Status

| Works now | Coming next |
|---|---|
| Search, channel pages, keyboard navigation, back stack, recent searches, Firefox playback (mpv legacy fallback), profiles/sign-in (with phone keyboard), account feeds, resume, queue, watched-history sync, and the deploy scripts (`deploy/`, written but not yet run on the server) | Real-account, real-TV and server checks (see `TODO.md`) |

Press Enter on a video to play it fullscreen (in the CouchTube window, or in mpv as described above). Esc or `q` stops playback; the interface keeps the same screen and selected card. Cards show a green "✓ Watched" badge for finished videos and "Resume 12:34" with a progress bar for started ones, on every list. The user has tested playback and profile sign-in successfully on the dev machine. Explicit Firefox kiosk stacking/focus-return and two-account switching checks remain on the checklist.

## Requirements

Arch Linux:

```bash
sudo pacman -S --needed rust nodejs npm yt-dlp firefox curl xdotool
sudo pacman -S --needed mpv   # optional: legacy fallback player
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

CouchTube opens on **What are we watching?**: choose **YouTube** or **Twitch** (the one used last is highlighted). Each service has its own **Who’s watching?** screen and its own profiles. On the YouTube one, choose Guest to browse signed out, or Add profile to connect an account. Esc on a profile screen, or **Switch service** in the YouTube Home menu, returns to the choice. Twitch mode is being built (see [`docs/twitch-mode.md`](docs/twitch-mode.md)): so far you can browse top live streams, categories and channel search, with no Twitch account or registration needed (it uses Twitch's unofficial web API); watching streams is the next step.

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
| `COUCHTUBE_TWITCH_CLIENT_ID`, `COUCHTUBE_TWITCH_CLIENT_SECRET` | unset | Optional. Twitch mode works without them, using Twitch's unofficial web API. Set both (from a free application at `dev.twitch.tv/console`) to switch to the official API; see [`docs/twitch-mode.md`](docs/twitch-mode.md). Never commit the secret |
| `COUCHTUBE_CEC_DEVICE` | `/dev/cec0` | HDMI-CEC device used by `cec-ctl` for **Turn TV off** |
| `COUCHTUBE_FRONTEND` | `frontend/dist` | Folder with the built frontend |
| `COUCHTUBE_PLAYER` | `browser` | Legacy: `mpv` plays every video in mpv; otherwise videos play in the CouchTube window |
| `COUCHTUBE_MPV_CONFIG` | `config/mpv` | Legacy: mpv config directory, including scripts (relative to the backend working directory) |
| `COUCHTUBE_DATA_DIR` | `$XDG_DATA_HOME/couchtube` or `~/.local/share/couchtube` | Private SQLite database and account Firefox profiles |
| `RUST_LOG` | `couchtube=info` | Log level, e.g. `couchtube=debug,tower_http=debug` |

## Profiles and sign-in

1. On **Who’s watching?**, choose **Add profile**, enter a name and colour, then **Create and sign in**. The next page shows a QR code for your phone.
2. Sign in with the **phone keyboard** or the TV keyboard. Scan the QR code and press **Yes, that's my phone** (see below), and a separate Firefox window opens straight on Google's sign-in page. To use the TV keyboard instead, choose **Open Firefox sign-in**. Complete any verification Google asks for. Google chooses the method (a 2-step code or a phone prompt) and CouchTube can't influence it. The window closes by itself a few seconds after YouTube's login cookie appears; to give up, close it with **Alt+F4**.
3. Wait for **YouTube sign-in works**, then choose **Continue to CouchTube**. A failed check can mean a connection or yt-dlp problem; use **Check sign-in** or **Open sign-in again** to retry.
4. Press **P** while browsing to switch profiles. On the picker, press **S** on a named profile to open sign-in (Firefox opens straight away) or **D** to remove it (with confirmation).

Each account has its own Firefox folder; Guest never loads account cookies, and a named profile with no cookie database browses signed out. Search results, feeds, recent searches, resume positions and queues are separated by profile. Removing a profile deletes its local sign-in files; deleting the active profile selects Guest.

### Signing in from your phone

The QR code opens a page on your phone that works as a keyboard for the Firefox sign-in window. It does not log in for you and does not use Bluetooth.

**How it works:** while the login page is open, the backend listens on port 7879 with a random 128-bit token and a random 256-bit key, shown as `http://<server LAN address>:7879/p/<token>#<key>` (QR code and text). The phone page reads the key after the `#` (browsers never send it), removes it from the address bar, and seals everything it sends. The backend decrypts and checks each message (replayed or wrong-step messages are rejected) and types it into the Firefox window with `xdotool`. Closing the sign-in window ends the session and closes the port.

**Using it:** scan the QR code and keep the phone page open. The phone shows a 6-digit code and the TV asks "Does it show this code?". Press **Yes, that's my phone** if they match, or **No** if not (the QR code is replaced and the old link stops working). Nothing can be typed until you confirm, and a second phone can't join afterwards. Firefox opens on Google when you press **Yes**. On the phone, go in order: **1. Email**, then once the TV shows the password page **2. Password**, then **3. 2-step code** if Google asks. Use the TV keyboard for anything else.

**Safety:**
- **Encrypted only against passive listening.** The page is plain HTTP, so someone who can tamper with the network (ARP spoofing, a rogue access point, a hacked router) can rewrite the page, read the key and see everything, with matching codes. They would have to act during the few minutes a sign-in is open. **On shared Wi-Fi, use the TV keyboard instead.** Only HTTPS with a certificate the phone trusts, or a native app, would fix it; neither is planned. See [`docs/phone-signin-encryption.md`](docs/phone-signin-encryption.md).
- **Port 7879 is the only part of CouchTube reachable from the network, and only while a sign-in is in progress.** An unused link expires after 15 minutes or when you leave the screen. The main API stays on `127.0.0.1` and rejects other `Host` headers. Don't forward the port on your router, and don't show the TV screen to people you don't trust, since it shows the key.
- Typed text is never logged. It goes to `xdotool` on stdin (not in `ps`), without a shell, limited to 256 characters on one line. If the window can't be found or focused, nothing is typed.
- Email, password and code can each be sent up to 3 times per sign-in. After that the phone page says to use the TV keyboard.

**Limits:**
- Needs `xdotool`. Firefox is started with `MOZ_ENABLE_WAYLAND=0` so `xdotool` can reach it under XWayland (no equivalent on KDE's Wayland). Under systemd, set `DISPLAY` for the service.
- The phone and server must be on the same network; guest networks and client isolation block it. With a VPN or several network cards the QR address may be wrong. Phones may ask for "Local network" permission; allow it. With a firewall, allow the port for your home network only, e.g. `sudo ufw allow from 192.168.50.0/24 to any port 7879 proto tcp`.
- Some phone password managers refuse to fill on `http://` pages; copy the password and paste it.
- The phone can't see the TV or click a field. The email field must already be focused. If you send the password while the email page is still showing, it is typed (and visible) in the email field, so wait for the password page.
- If the key is missing from the link (retyped without `#...`), the phone page says to scan again.

Weaknesses and planned fixes are tracked in [`TODO.md`](TODO.md).

The sign-in check requests one item from the account’s subscriptions. Home shows **Continue watching**, **Recommended** and **Subscriptions**; its menu opens **History**, **Watch later**, **Liked** and **Queue**. Guest and profiles without cookies have local watch history and can search, queue and resume; account feeds stay empty until sign-in. Daily cookie refresh is planned for deployment.

### Watching, resume and queue

- Enter plays a video from its saved position; **R** restarts the selected video from the beginning. Resume begins after at least five seconds of progress; completed videos and videos with less than five seconds remaining start fresh.
- **A** queues the selected video. **N** plays the next queued video (also inside mpv). On the Queue screen, Enter plays a selected item and removes it from the queue, **D** removes it, and the top controls play next or clear the queue.
- The next queued item starts automatically at the end of a video. Esc/q stops playback and keeps the remaining queue.
- The backend observes mpv over IPC and saves progress about every five seconds and when playback stops. This works without the browser polling.
- After roughly 30–35 seconds of viewing, a signed-in profile runs `yt-dlp --simulate --mark-watched` once for that video. Paused time and the distance skipped by seeking do not count as viewing. Failure is logged and can retry during a later viewing. Guest never sends watched-history updates.
- **U** refreshes the current Home/account feed or queue. Recommended, Subscriptions, Watch Later and Liked are cached per profile for 10 minutes; history for 2 minutes; searches for 30 minutes and channels for 10 minutes. Selecting an account prefetches Recommended and Subscriptions. The browser also briefly caches search/channel results.

Automated checks use simulated accounts and watched-history requests. Checked with a real account on 2026-10-25: after a 68 s viewing the video was first in the profile's YouTube history (read back with `yt-dlp :ythis`). Rewatching and the personal feeds still need a real-account check.

`config/firefox/user.js` is copied into account profiles automatically. The supplied extension policy installs uBlock Origin and SponsorBlock when deployed to Firefox’s system policy directory. To enable it on this dev machine, first review the file and any existing `/etc/firefox/policies/policies.json`, then merge or install it there and restart Firefox. CouchTube does not change system Firefox policies automatically. See [Mozilla’s ExtensionSettings reference](https://firefox-admin-docs.mozilla.org/reference/policies/extensionsettings/).

## For You

**For You** (Home menu) is CouchTube's own recommendation list, built from seeds you give it instead of YouTube's algorithm. The design notes are in [`docs/for-you.md`](docs/for-you.md).

- **Seeds** (per profile): type a **theme** ("retro computing") or paste a **video** link in the box at the top; **S** on any video adds its **channel**; **D** on a seed chip removes it. Typing `not music` or `-music` adds an **exclusion**.
- **Discovery** runs as Guest, so it never touches the account's history. Each refresh searches 6 themes (the 3 newest plus 3 older ones rotating by the hour), 3 blended pairs among the 4 newest themes (rotating by the hour, labelled "A & B"), 4 video seeds (by title), and reads 6 channels' latest uploads (video seeds and channels rotate the same way when there are more). Results are cached for 10 minutes; **U** refreshes.
- **Ranking:** a video scores higher the higher it sits in each list and the more lists find it, multiplied by bonuses for being recent (up to a year old), long (up to 40 minutes) and watched a lot per day since upload. Each further video from the same channel counts less. Shorts (3 minutes or less) are hidden in every list, not only here. Half of the list is reserved for channels that are not channel seeds, alternating with seed-channel videos. Videos from non-seed channels need 2,000+ views (missing counts are looked up with yt-dlp, up to 8 per refresh, remembered a week); seed channels are exempt.
- **Hidden:** videos you started or finished, videos you pressed **X** on (which also lowers other videos from that channel), and anything matching an exclusion. An exclusion matches whole words in the title or channel name, singular or plural, and a single word of 4+ letters also matches inside a channel name (`kids` hides "kidstv123"). A few topics also hide related words (`kids`: children, cartoon, sing along…; `music`: song, lyrics, remix, phonk, lofi…, see `RELATED` in `src/discover.rs`).
- **Cards** say why they were suggested ("Because: …"). **W** marks the focused video watched (or clears the mark); it stays local and is not sent to YouTube. The **History** button on the screen opens CouchTube's own history for the profile (not YouTube's). The Search and Profiles hints are hidden here; `/` and `p` still work.

## Subtitles, YouTube TV, TV power and the phone remote

- **Subtitles**: **Subtitles** in the Home menu toggles the profile's subtitles between Off and English. In the CouchTube window the player starts with that language's captions; in mpv, yt-dlp fetches it (manual subtitles first, else automatic captions) and **C** cycles the tracks. In the CouchTube window **C** switches them on and off. Off by default.
- **YouTube TV**: **YouTube TV** in the Home menu opens youtube.com/tv in a fullscreen Firefox as the active profile (so it is signed in, with uBlock Origin from the policy). Firefox gets a TV user agent for this window only. Close it with **Alt+F4** to come back. Use it to like or subscribe, or when yt-dlp breaks.
- **Turn TV off**: shown in the Home menu only when `cec-client` (libcec) or `cec-ctl` (v4l-utils) and a `/dev/cec*` device exist. Most PC graphics cards have no CEC; a Pulse-Eight USB adapter does.
- **Phone remote**: **Phone remote** in the Home menu → **Turn on**, then **Pair a phone**. Scan the QR code, check the phone shows the same code as the TV, and choose **Yes**. The phone then controls pause, seek, volume, mute, subtitles, next and stop, shows the queue, and can play or queue a pasted YouTube link. The port (`7880`) is closed until you turn the remote on. Each phone keeps a random token; the server stores only its hash, and **Forget phones** revokes all of them. It is plain HTTP, so anyone on your Wi-Fi who sniffs the traffic can copy a phone's token. Use it only on a network you trust.

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
| `d` | Remove the selected video on the Queue screen, or the focused seed on For You |
| `s` | Add the selected video's channel to For You |
| `x` | On For You, hide the selected video and see less like it |
| `w` | Mark the selected video watched, or clear the mark |
| `u` | Refresh Home, an account feed, or the queue |

In the search box, ↓ moves to the results and Esc goes back.

### During playback

These keys work in the CouchTube window and in mpv. Space pauses; ←/→ seek 10 seconds; ↑/↓ seek 60 seconds; `9`/`0` adjust volume; `m` mutes; `i` shows title and progress; `n` plays the next queued video; Esc/`q` quit. If Firefox retains focus, Esc stops playback through the API.

mpv settings (1080p cap, VA-API, SponsorBlock script, `ytdl-format`) are in [`docs/legacy-mpv.md`](docs/legacy-mpv.md).

## API

| Method | Path | Returns |
|---|---|---|
| GET | `/api/profiles` | `{profiles: [{id, name, color}], active_id}` |
| POST | `/api/profiles` | Body `{name, color?}`; creates a local profile |
| DELETE | `/api/profiles/<id>` | Removes the profile and its local sign-in files |
| POST | `/api/profiles/<id>/select` | Selects a profile and stops playback |
| POST | `/api/profiles/<id>/signin` | Opens that profile’s Firefox sign-in window |
| GET/POST/DELETE | `/api/pairing` | Phone URL and QR code (SVG) while a sign-in is in progress, otherwise 404; POST starts it before Firefox opens (the login page does this when it opens), DELETE cancels it if no window is open |
| PUT | `/api/profiles/<id>/subtitles` | Body `{language}` (`en`, `pt-BR`, …; `null` = off); saved per profile and used for every video |
| GET | `/api/profiles/<id>/status` | `{state, message}`; `?check=true` starts a login check |
| POST | `/api/system/exit` | Stops the kiosk service (only when it runs as the systemd unit) |
| POST | `/api/system/youtube-tv` | Opens youtube.com/tv in a Firefox window as the active profile (409 while a Firefox window for it is open) |
| GET/POST | `/api/system/tv` | GET `{available}` (a CEC tool and device exist); POST `{on}` switches the TV on or to standby over HDMI-CEC |
| GET/PUT | `/api/remote` | Phone remote state `{enabled, devices, url, svg, code, confirmed}`; PUT `{enabled}` opens or closes its LAN port |
| POST | `/api/remote/pair`, `/confirm`, `/reject` | Start a pairing link (10 minutes, one use), accept the phone's code `{code}`, or refuse it |
| DELETE | `/api/remote/devices` | Forget every paired phone |
| GET/PUT | `/api/mode` | GET `{mode}` (`youtube`, `twitch` or `null` before the first choice); PUT `{mode}` saves the choice and stops playback |
| GET/POST | `/api/twitch/profiles` | Twitch profiles `{profiles: [{id, name, color}], active_id}` (a list separate from `/api/profiles`, with its own Guest); POST `{name, color?}` creates one |
| DELETE | `/api/twitch/profiles/<id>` | Removes a Twitch profile (Guest can't be removed); the active one falls back to Guest |
| POST | `/api/twitch/profiles/<id>/select` | Selects the active Twitch profile and stops playback |
| GET | `/api/twitch/status` | `{source}`: `unofficial` (default) or `official` (both Twitch credentials set) |
| GET | `/api/twitch/categories`, `/streams`, `/search` | Twitch pages `{items, cursor}`: top categories; live streams (`?game_id=` for one category); channel search (`?q=`). All take `cursor` and `refresh=true`; with the unofficial source a cursor is the next page size (`n48`) |
| GET | `/api/health` | `{"status":"ok"}` |
| GET | `/api/check` | Diagnostics for the active profile: whether history loads and whether a sample of its videos and thumbnails are available |
| GET | `/api/feed/<kind>` | Video list: `foryou`, `continue`, `local` (CouchTube's own history), `recommended`, `subscriptions`, `history`, `watchlater`, `liked`; `?refresh=true` bypasses the feed cache; every list carries the profile's `completed` / `resume_position` |
| GET/POST | `/api/seeds` | For You seeds `[{id, kind, value, label}]` (`theme`, `video`, `channel`, `exclude`); POST `{kind, value, label?}` adds one and returns the list |
| DELETE | `/api/seeds/<id>` | Removes a seed |
| POST | `/api/foryou/dismiss` | Body `{id}`; hides a video from For You and lowers its channel |
| POST | `/api/watched` | Body `{id, watched}`; marks a video watched or clears the mark for the active profile (local only) |
| GET | `/api/queue` | Active profile’s queue in playback order |
| POST | `/api/queue` | Body `{id}`; appends once and returns the queue |
| DELETE | `/api/queue` or `/api/queue/<id>` | Clears the queue or removes one video |
| POST | `/api/next` | Stops playback and plays the next queued item; `{playing}` |
| GET | `/api/search?q=<text>&limit=<1-50>` | List of videos |
| GET | `/api/channel/<UC… id>` | `{id, name, videos}` |
| POST | `/api/play` | Body `{id, start?}`; starts/replaces playback (saved resume position unless `start` is supplied), returns `{"status":"starting"}` |
| POST | `/api/stop` | Stops playback, returns `{"status":"stopped"}` (safe when idle) |
| GET | `/api/status` | `{playing, page, id, session, title, position, duration, paused, error}`; `page` is the player page the CouchTube window must show (browser playback) |
| POST | `/api/player/report` | Used by the player page: `{session, position?, duration?, paused?, title?, outcome?, code?}`; answers `{commands}` (mpv-style, from the phone remote) |

A video is `{id, title, channel, channel_id, duration, view_count, thumb, timestamp?, resume_position?, completed?, reason?}`; `duration` is `null` for live streams. Errors are `{"error": "..."}`.

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
python3 tests/twitch_smoke.py       # Twitch browsing (unofficial and official sources) against fake Twitch servers; add --browser for the keyboard UI (needs Chromium and Node 22+)
python3 tests/profiles_smoke.py --browser  # keyboard UI check; needs Chromium and Node 22+
lua tests/sponsorblock_test.lua     # needs Lua; tests SponsorBlock without network access
```

## Project layout

```
src/            Rust backend (main.rs, api.rs, ytdlp.rs, player.rs, playback.rs, profiles.rs, pairing.rs, remote.rs, cec.rs, feeds.rs, db.rs, discover.rs for For You, twitch.rs, gql.rs and helix.rs for Twitch mode)
frontend/       React app (App.tsx, ModeGate.tsx, ProfileGate.tsx, TwitchGate.tsx, TwitchHome.tsx, nav.ts, api.ts, styles.css); public/watch.html is the player page
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
