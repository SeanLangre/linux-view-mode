# TODO

Checklist for [`plan.md`](plan.md). Tick items as they're done.

## Now

- [ ] Try the Phase 2 interface in `firefox --kiosk` on the dev machine (fonts, focus ring, smooth scrolling, scrollbar hidden)
- [ ] Commit Phase 2

## Phase 3 — Playback (implemented; desktop check pending)

- [x] `src/player.rs`: start mpv fullscreen (`--hwdec=vaapi`, 1080p cap, IPC socket), one player at a time, kill on stop
- [x] Read `time-pos` / `duration` over the mpv IPC socket
- [x] `config/mpv/input.conf`: Esc and `q` quit, arrow seeks, `i` overlay
- [x] `config/mpv/mpv.conf`: format cap, hwdec, OSD size readable from the couch
- [x] SponsorBlock mpv script
- [x] API: `POST /api/play {id, start?}`, `POST /api/stop`, `GET /api/status`
- [x] UI: Enter calls `/api/play`; show "Playing: <title>" while mpv runs; return to the same card afterwards
- [ ] Check on the dev machine (KDE Wayland): mpv opens above the Firefox kiosk and focus returns when it quits; add a KWin rule if not

## Server setup (whenever ready, on the RN96)

- [ ] Back up anything needed from Windows
- [ ] Install Arch with `archinstall`: KDE Plasma, SDDM, Intel driver, PipeWire, NetworkManager
- [ ] TV resolution/scaling and HDMI sound set in Plasma

## Phase 0 — Server checks (after the Arch install)

- [ ] Install packages; `vainfo` lists AV1, VP9, H.264
- [ ] mpv plays 1080p smoothly with hardware decoding
- [ ] mpv shows above the Firefox kiosk, focus returns afterwards
- [ ] yt-dlp with one signed-in Firefox profile returns `:ytrec`
- [ ] `--mark-watched` shows up in YouTube history
- [ ] Login still works after 2–3 days

## Phase 4 — Profiles and sign-in

- [ ] SQLite (`rusqlite`) + `db.rs`
- [ ] "Who's watching?" screen, add/remove profile, Guest
- [ ] Firefox profile per account, sign-in window, login check
- [ ] yt-dlp and mpv use the active profile's cookies
- [ ] `p` switches profile

## Phase 5 — Account feeds

- [ ] Home rows: Continue watching, Recommended, Subscriptions (replace the placeholder Music/Nature/Science rows)
- [ ] Screens: History, Watch Later, Liked
- [ ] Mark watched after ~30 s; resume positions; queue (`a`, `n`)
- [ ] Per-profile cache and prefetch

## Phase 6 — Deploy and appliance mode

- [ ] `deploy/install.sh`
- [ ] systemd user units: backend, kiosk, daily cookie refresh
- [ ] `config/firefox/policies.json` (uBlock Origin + SponsorBlock), `user.js`
- [ ] KDE auto-login; no screen lock, blanking or sleep
- [ ] "Exit to desktop" and a shortcut to restart the kiosk

## Phase 7 — Polish

- [ ] YouTube TV fallback tile (Firefox + uBlock Origin)
- [ ] Subtitle language per profile
- [ ] HDMI-CEC power control
- [ ] Phone remote (with authentication)

## Known issues / small fixes

- [ ] After the first search, Back to Home can land on the new "Recent searches" row instead of the video row you left
- [ ] `/api/search?limit=abc` returns a plain-text error instead of `{"error": ...}`
- [ ] Home loads three searches at startup (~1–2 s); fine for now, replaced by account feeds in Phase 5
