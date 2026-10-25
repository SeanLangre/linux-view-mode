# TODO

Checklist for [`plan.md`](plan.md). Tick items as they're done; move finished sections to **Done** at the bottom.

## Top priority

- [ ] YouTube history sync: first watch verified, rewatch left
  - Built: after ≥30 s of real watch time, `Playback::save` runs `yt-dlp --mark-watched --simulate` with the profile's cookies and sets `progress.marked=1`. Guest has no cookies, so it stays local.
  - Verified 2026-10-25: 68 s watched, `marked=1`, and `yt-dlp :ythis` (profile cookies) lists the video first.
  - To test: rewatch the same video ≥30 s. `Database::begin_viewing` should reset `watched`/`marked` and send it again.

- [ ] Single instance / closing the window shuts everything down (implemented 2026-10-25)
  - Built: backend `flock` on `$data_dir/couchtube.lock`; `AppRun` exits if `/api/health` answers, runs the kiosk Firefox with its own profile and kills the backend on exit (5 s `KILL` fallback); "Exit to desktop" closes the kiosk Firefox when not under systemd.
  - Checked (AppImage): after quitting, `pgrep -a 'couchtube|mpv|firefox|yt-dlp'` printed nothing.
  - To check: closing via the Firefox X, closing with a sign-in Firefox open, a second launch while one runs, "Exit to desktop". Then decide the README/`install.sh` wording for the systemd units (drop, or keep as optional appliance mode).

- [ ] Login page change (2026-10-25, untested on a real run): QR code only on the app's login page; **Yes, that's my phone** (or **Open Firefox sign-in**) opens Firefox straight on Google. `/signin` and `signin.html` are removed.
  - Possible issue: Google then asked for a 2FA code instead of "click to approve". Firefox command, `user.js`, profile and URL are unchanged; the only difference is the missing `Referer: http://127.0.0.1:…` and click path from the old local start page. We can't see why Google would use either, and a new profile with no history is likelier. If the tap prompt doesn't come back, restore a local start page that links to Google and compare.

## Sign-in follow-ups

- [ ] Run the rest of `encryption-plan.md`: the `tcpdump` check, the second-phone test (codes differ, **No**, new QR code, old link 404) and the confirmation prompt on the login page. (Encrypted sign-in itself is done; a real phone sign-in with confirmation worked 2026-10-21.)
- [ ] (Accepted, low) `type_and_enter`: the second focus check only guards Enter; if focus moves mid-typing (up to ~3 s for 256 characters) the text already went elsewhere

## Alternative login tests (`alternative_login.md`)

Result 2026-10-21: on the user's real account Google's password page offered no "Try another way". The account isn't to be changed, so passkey/prompt is only a bonus; the phone keyboard and `encryption-plan.md` stay the main path. The tests below need an account that has a passkey or phone prompt.

- [ ] Passkey QR sign-in in Firefox on the dev machine (record the Firefox version)
- [ ] Same, with Firefox started by the `couchtube` systemd user unit; if Bluetooth/D-Bus is missing, pass the D-Bus environment in `deploy/couchtube.service`
- [ ] Google prompt: email only; does Google skip the password, and do the cookies work afterwards?
- [ ] Daily cookie refresh still works for a passkey-signed-in profile
- [ ] Depending on results, reduce `encryption-plan.md`

## Phase 0 — Server checks (after the Arch install)

- [ ] Install packages; `vainfo` lists AV1, VP9, H.264
- [ ] mpv plays 1080p smoothly with hardware decoding
- [ ] mpv shows above the Firefox kiosk, focus returns afterwards
- [ ] yt-dlp with one signed-in Firefox profile returns `:ytrec`
- [x] `--mark-watched` shows up in YouTube history (2026-10-25, dev machine)
- [ ] Login still works after 2–3 days

## Phase 7 — Polish

- [ ] YouTube TV fallback tile (Firefox + uBlock Origin)
- [ ] Subtitle language per profile
- [ ] HDMI-CEC power control
- [ ] Phone remote (with authentication)

## Known issues

- [ ] mpv warnings (`client removed during hook handling`, `Packet corrupt`, `Invalid OBU length`, `Failed to parse temporal unit`) on AV1 1080p59.94 with VA-API. Seen twice (including a 40-minute video on 2026-10-25) and playback looked fine. Only switch `ytdl-format` away from AV1 (e.g. `[vcodec!^=av01]`) if playback visibly breaks; record the video ID if it recurs.
- [ ] 2026-10-25 batch, code and unit tests done, not tried with a real account:
  - Upload date on cards (`approximate_date`, `Video.timestamp`, `formatAge()`).
  - `U` refresh passes `refresh=true` and drops the frontend cache key. Still open: F5 in the kiosk doesn't refresh, and Recommended may legitimately return the same videos.
  - Load more: `?page=N` (24 per page, max 10) on `/api/channel/{id}` and `/api/feed/{kind}`; ↓ on the last row loads the next page.
