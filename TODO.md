# TODO

Checklist for [`plan.md`](plan.md). Tick items as they're done; move finished sections to **Done** at the bottom.

## Top priority

- [ ] Check whether watched videos are added to the profile's YouTube history
  - Already built: after ≥30 s of real watch time, `Playback::save` (`src/playback.rs`) runs `yt-dlp --mark-watched --simulate` with the profile's cookies (`ytdlp::mark_watched`). On success it sets `progress.marked=1`. Guest has no cookies, so its watches stay local only.
  - Test (same as the Phase 5 item below): on a signed-in profile, watch ≥35 s. Then check the backend log for `YouTube watched-history sync failed`, run `SELECT video_id, watched, marked FROM progress` in the profile DB, and check youtube.com/feed/history in that profile's Firefox.
  - Known gap: once `marked=1`, a rewatch is never sent again, so the video doesn't move back to the top of YouTube history. Possible fix: reset `marked` when a new viewing starts, or mark on every viewing ≥30 s.

- [ ] Only one instance at a time / closing the window shuts everything down — implemented 2026-10-25, still to test on a real run
  - Done: backend `flock` on `$data_dir/couchtube.lock` (checked: a second backend exits with a message). `AppRun` exits if `/api/health` already answers (starts `couchtube-kiosk` if that unit is active), starts Firefox with `--new-instance --no-remote --profile $DATA/kiosk-firefox`, and its trap kills the backend with a 5 s `KILL` fallback. "Exit to desktop" now closes the kiosk Firefox (`pkill -f kiosk-firefox`) when not under systemd.
  - Still to check: rebuild the AppImage, then `pgrep -a 'couchtube|mpv|firefox|yt-dlp'` after closing the window (including sign-in Firefox windows and mpv); README/`install.sh` wording for the systemd units (drop or keep as optional appliance mode).

## Sign-in follow-ups (second review, 2026-10-21)

- [x] (done 2026-10-21; a real phone sign-in with confirmation worked. Still to run from `encryption-plan.md`: the `tcpdump` check, the second-phone test (codes differ, **No**, new QR code, old link 404) and the confirmation prompt on `signin.html`) Encrypt the phone form against passive Wi-Fi sniffing: one-time key in the QR URL `#fragment`, inline crypto (e.g. tweetnacl; no WebCrypto on plain HTTP). Remaining main risk
- [ ] (Accepted, low) `type_and_enter`: the second focus check only guards Enter; if focus moves mid-typing (up to ~3 s for 256 characters) the text already went elsewhere

## Alternative login tests (`alternative_login.md`; wording is already in the UI and docs, results decide what stays)

- [x] Result 2026-10-21: on the user's real account, Google's password page offered no "Try another way". The account isn't to be changed, so passkey/prompt is only a bonus where available; the phone keyboard and `encryption-plan.md` stay the main path
✓? Record the Firefox version
- [ ] Same, with Firefox started by the `couchtube` systemd user unit; if Bluetooth/D-Bus is missing, pass the D-Bus environment in `deploy/couchtube.service`
- [ ] Google prompt: email only; does Google skip the password, and do the cookies work afterwards?
- [ ] Daily cookie refresh still works for a passkey-signed-in profile
- [ ] Depending on results, reduce `encryption-plan.md` (see "Effect on the encryption plan")

## Phase 0 — Server checks (after the Arch install)

- [ ] Install packages; `vainfo` lists AV1, VP9, H.264
- [ ] mpv plays 1080p smoothly with hardware decoding
- [ ] mpv shows above the Firefox kiosk, focus returns afterwards
- [ ] yt-dlp with one signed-in Firefox profile returns `:ytrec`
- [ ] `--mark-watched` shows up in YouTube history
- [ ] Login still works after 2–3 days

## Phase 7 — Polish

- [ ] YouTube TV fallback tile (Firefox + uBlock Origin)
- [ ] Subtitle language per profile
- [ ] HDMI-CEC power control
- [ ] Phone remote (with authentication)

## Known issues

- [ ] Follow up on the reported mpv warnings: `client removed during hook handling`, `Packet corrupt`, `Invalid OBU length`, and `Failed to parse temporal unit`. The log confirms AV1 1080p59.94 playback with VA-API; whether playback visibly failed or the user quit at ~20 s is not confirmed. Record the video ID and reproduction steps if it recurs.
- **Top priority batch (2026-10-25)**, code and unit tests done, not tried with a real account:
  - Upload date: `--extractor-args youtubetab:approximate_date` (verified live: channel and search entries get `timestamp`), `Video.timestamp`, `formatAge()` on the card meta line.
  - Refresh: `U` now passes `refresh=true` for search/channel/feed pages and drops the frontend cache key; "Refreshing…" toast. Still open from the old item: F5 in the kiosk doesn't refresh, and Recommended may legitimately return the same videos.
  - Load more: `?page=N` (24 per page, max 10 pages) on `/api/channel/{id}` and `/api/feed/{kind}`; cache keys `…:{page}`; `set_marked` uses `LIKE`; ↓ on the last row loads the next page (deduplicated, skeleton row, stops on a short or all-duplicate page). Home menu has Recommended and Subscriptions.
