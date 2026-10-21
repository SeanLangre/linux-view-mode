# TODO

Checklist for [`plan.md`](plan.md). Tick items as they're done; move finished sections to **Done** at the bottom.

## Now

- [ ] Test personal feeds, resume, queue and YouTube watched-history sync with a real account
- [ ] Phone keyboard: user said it "kind of worked"; find out what fell short (backend log line `phone typing failed`)
- [ ] Run `deploy/install.sh` on the dev machine or server and confirm the kiosk restarts after killing Firefox

## Sign-in follow-ups (second review, 2026-10-21)

- [ ] Encrypt the phone form against passive Wi-Fi sniffing: one-time key in the QR URL `#fragment`, inline crypto (e.g. tweetnacl; no WebCrypto on plain HTTP). Remaining main risk
- [ ] (Accepted, low) `type_and_enter`: the second focus check only guards Enter; if focus moves mid-typing (up to ~3 s for 256 characters) the text already went elsewhere

## Testing still to record

- [ ] Phase 3: on the dev machine (KDE Wayland), mpv opens above the Firefox kiosk and focus returns when it quits; add a KWin rule if not
- [ ] Phase 4: sign in two real accounts, check YouTube accepts both logins, and switch with `p`
- [ ] Phase 5: confirm Recommended/Subscriptions/History/Watch Later/Liked with a real account
- [ ] Phase 5: watch for at least 35 seconds and confirm the video appears in YouTube history
- [ ] Phase 5: stop and resume a video; check the same position and queue after a backend restart

## Phase 6 — Deploy and appliance mode

- [ ] KDE auto-login; no screen lock, blanking or sleep (manual Plasma settings, documented in the README)

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

## Phase 7 — Polish

- [ ] YouTube TV fallback tile (Firefox + uBlock Origin)
- [ ] Subtitle language per profile
- [ ] HDMI-CEC power control
- [ ] Phone remote (with authentication)

## Known issues

- [ ] Follow up on the reported mpv warnings: `client removed during hook handling`, `Packet corrupt`, `Invalid OBU length`, and `Failed to parse temporal unit`. The log confirms AV1 1080p59.94 playback with VA-API; whether playback visibly failed or the user quit at ~20 s is not confirmed. Record the video ID and reproduction steps if it recurs.

## Done

- **Phases 1–2** (`097bc9e`): backend skeleton, TV interface.
- **Phase 3 — Playback** (`9c21fe8`; user reported playback looked good): mpv fullscreen with VA-API and 1080p cap, IPC position/duration, `input.conf`/`mpv.conf`, SponsorBlock script, `/api/play`/`stop`/`status`, return to the same card.
- **Phase 4 — Profiles and sign-in** (`6d11404`; user reported the real flow worked, 2026-10-20): SQLite profiles, "Who's watching?", Guest, Firefox profile per account, login check, per-profile cookies for yt-dlp and mpv, `p` to switch.
- **Phase 5 — Account feeds**: Home rows (Continue watching, Recommended, Subscriptions), History/Watch Later/Liked, mark watched after ~30 s, resume, queue (`a`, `n`), per-profile cache and prefetch.
- **Phase 6 — Deploy** (not yet run on the server): `deploy/install.sh`, systemd user units (backend, kiosk, daily cookie refresh), Firefox `policies.json` via `install.sh`, "Exit to desktop" + `couchtube-kiosk.desktop` launcher.
- **Sign-in review 1 (2026-10-21)**:
  - Safety: typing fails closed unless the sign-in window is found and focused (no `--window`: Firefox ignores those synthetic events), separate email/password/code steps instead of the 4 s timer, text sent to `xdotool` on stdin, single-use steps, port 7879 open only during sign-in, `Host` check on the main API, cookie temp copy always removed.
  - Accessibility: URL shown as text under the QR code with better alt text, phone 2-step code field, real labels and `aria-describedby`, `lang="en"`, network-failure message.
  - README and plan updated to match.
- **Sign-in review 2 (2026-10-21)**: `signin.html` shows the QR code and URL; phone steps allow 3 tries; README limits order fixed.
- **Small fixes**: Home focus stays on the original video after the first search; `/api/search?limit=abc` returns JSON errors; placeholder Home searches replaced.
