# TODO

Checklist for [`plan.md`](plan.md). Tick items as they're done.

## Now

- [x] Phase 2 committed (`097bc9e`)
- [x] Phase 3 committed (`9c21fe8`); user reported playback looked good
- [x] Phase 4 committed (`6d11404`); user reported the profile/sign-in flow worked well
- [x] Implement Phase 5 account feeds, progress/history and queue
- [ ] Test personal feeds, resume, queue and YouTube watched-history sync with a real account

## Phase 3 — Playback (implemented; user test positive, focus follow-up pending)

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

## Phase 4 — Profiles and sign-in (implemented; user test positive)

- [x] SQLite (`rusqlite`) + `db.rs`
- [x] "Who's watching?" screen, add/remove profile, Guest
- [x] Firefox profile per account, sign-in window, login check
- [x] yt-dlp and mpv use the active profile's cookies
- [x] `p` switches profile

- [x] Try the profile/sign-in flow with a real account (user report, 2026-10-20)
- [ ] Explicitly verify two real accounts: sign in two real accounts, check YouTube accepts both logins, and switch with `p`

## Phase 5 — Account feeds (implemented; real-account check pending)

- [x] Home rows: Continue watching, Recommended, Subscriptions (replace the placeholder Music/Nature/Science rows)
- [x] Screens: History, Watch Later, Liked
- [x] Mark watched after ~30 s; resume positions; queue (`a`, `n`)
- [x] Per-profile cache and prefetch

- [ ] Confirm recommendations/subscriptions/history/Watch Later/Liked with a real account
- [ ] Watch for at least 35 seconds and confirm the video appears in YouTube history
- [ ] Stop and resume a video; check the same position and queue after a backend restart

## Phase 6 — Deploy and appliance mode

- [ ] `deploy/install.sh`
- [ ] systemd user units: backend, kiosk, daily cookie refresh
- [ ] Deploy the supplied `config/firefox/policies.json` (uBlock Origin + SponsorBlock) and `user.js`
- [ ] KDE auto-login; no screen lock, blanking or sleep
- [ ] "Exit to desktop" and a shortcut to restart the kiosk

## Phase 7 — Polish

- [ ] YouTube TV fallback tile (Firefox + uBlock Origin)
- [ ] Subtitle language per profile
- [ ] HDMI-CEC power control
- [ ] Phone remote (with authentication)

## Known issues / small fixes

- [x] Keep Home focus on the original video after the first search (recent searches now follow the stable video rows; browser regression test passes)
- [ ] `/api/search?limit=abc` returns a plain-text error instead of `{"error": ...}`
- [x] Replace the three placeholder Home searches with Continue watching, Recommended and Subscriptions
- [ ] Follow up on the reported mpv warnings: `client removed during hook handling`, `Packet corrupt`, `Invalid OBU length`, and `Failed to parse temporal unit`. The log confirms AV1 1080p59.94 playback with VA-API; whether playback visibly failed or the user quit at ~20 s is not confirmed. Record the video ID and reproduction steps if it recurs.
