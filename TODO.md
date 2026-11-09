# TODO

Checklist for the [roadmap](docs/roadmap.md). Remove items once they are verified on screen.

## Next: continue here (written 2026-11-09)

Everything below was built on 2026-11-08/09 and is **uncommitted**. Suggested commits: (1) Twitch step 4 (sign-in, Following), (2) caching and thumbnails, (3) build version, (4) the test cleanup and the Firefox-player rewrite of `feeds_smoke.py`, (5) docs.

**Caching and thumbnails: built, checked and documented (2026-11-09)**
- [ ] What changed: lists are fresh for 30 min (history 5 min, searches 6 h, channels 1 h, For You 30 min); an expired list is still shown at once for up to 7 days while one background request replaces it (`Feeds::refresh_later`), and the screen swaps in the new list about 6 s later (`x-cache: stale`, `refreshLater` in `frontend/src/api.ts`). YouTube thumbnails are saved in `<data>/thumbs/` (served by `/api/thumb/<id>`, only from `i.ytimg.com`, at most 2000 files, trimmed to the newest 1500) and every loaded list prefetches its pictures
- [ ] Decide: expired private feeds used to be purged on every write and are now kept up to 7 days (so they can be shown stale). `STALE_SECS` in `src/db.rs` sets it; lower it if that feels too long
- [ ] Not cached on purpose: Twitch pictures (live previews change constantly); the browser-side memory cache is still cleared on a profile switch (the backend copy makes that cheap now)
- [ ] On the TV: after the kiosk has been idle, Home opens instantly with the old lists and updates by itself; offline, saved lists and thumbnails still show; **U** forces a fresh list; check the size of `~/.local/share/couchtube/thumbs`

**Build version**
- [ ] Look at it on the TV: bottom-right of the service choice and both profile screens, `v0.1.0 · <commit>` (`-dirty` if tracked files had uncommitted changes, `unknown` if built outside a git checkout, e.g. from a tarball). Check that `deploy/install.sh` and the AppImage build (`packaging/build-appimage.sh`) show the commit. Bump `version` in `Cargo.toml` when you cut a release

**Twitch: what is left**
- [ ] Step 4 with a real login (the checklist is in the section below)
- [ ] Step 5 (built 2026-11-09, backend checked against the real Twitch, **not seen on screen**): search a channel → Enter opens its screen (live card first, then Past broadcasts; **T** switches to Top clips; **C** on any stream card opens its channel); a broadcast plays with arrows seeking ±10 s / ±60 s; press Esc, start it again: the card says Resume and it continues from there; a clip starts and Esc stops it. Check that the VOD starts at the saved time (`time=` in the embed) and that a VOD with ads/subscriber-only parts shows a clear failure. Still to build: the queue for VODs (needs its own Twitch queue table)
- [ ] Step 6: chat pane (`t`) and the phone remote's Twitch support
- [ ] Step 7 (only if uBlock Origin does not cope with Twitch's ads on the real TV): streamlink fallback

## Phase 7 — Polish (implemented 2026-10-25, not tried on the real TV)

- [ ] HDMI-CEC: needs `cec-client` or a `/dev/cec*` device; check **Turn TV off** appears and works
- [ ] Phone remote: pair a real phone, then pause, seek, volume, next, queue, pasted link; **Forget phones** locks it out

## For You (implemented 2026-11-01, see [`docs/for-you.md`](docs/for-you.md), not tried on screen)

- [ ] Home → For You: type a theme, check it appears as a seed and videos load (first load makes several yt-dlp calls)
- [ ] **S** on a video seeds its channel; **D** removes a focused seed; **U** refreshes
- [ ] Paste a video link: the seed's label turns into the video's title and similar videos appear
- [ ] **X** hides a video; "Because: …" shows on cards
- [ ] Exclusions: `not music` / `not kids` add "Not:" chips; check that matching videos (including a channel like "kidstv123") disappear after **U**
- [ ] Blended searches: with 2+ themes, "Because: A & B" appears on some cards
- [ ] 50/50 mix and the 2,000-view minimum: about every second card is from a channel that is not a seed, and niche themes' new channels now show up; seed-channel videos with few views still appear
- [ ] No Shorts and no videos of 3 minutes or less in For You; one channel does not fill the top rows; older seeds show up over the hours
- [ ] **W** marks a video watched (badge appears, it leaves For You) and clears it again; the **History** button opens CouchTube's own history
- [ ] Tune scoring (weights in `src/discover.rs`) after a week of real use

## Twitch mode, step 1 (implemented 2026-11-08, see [`docs/twitch-mode.md`](docs/twitch-mode.md); endpoints tried with curl, not seen on screen)

- [ ] First screen shows **YouTube** and **Twitch**; the last used one is highlighted and Enter picks it; ← → move
- [ ] YouTube: the profile screen and Home work as before; Esc on the profile list and Home → **Switch service** return to the choice
- [ ] Twitch: add a profile, pick it, switch profile, remove it with **D** (Guest can't be removed); YouTube profiles are not affected
- [ ] Restart the backend: the last mode and each service's active profile are remembered

## Twitch mode, step 2 (implemented 2026-11-08, see [`docs/twitch-mode.md`](docs/twitch-mode.md); checked against the real Twitch from a headless browser, not yet on the TV)

- [ ] On the TV: Top live streams, Categories (6 per row), a category's streams and Search load with real pictures; Load more works; `u` refreshes; Esc returns with the focus on the card you left
- [ ] Watch for the unofficial source breaking (it is not a supported API): if the screens show "could not reach Twitch", read the backend log; the client id repairs itself, but a schema or integrity-check change needs `src/gql.rs` updated

## Twitch mode, step 3: watching (implemented 2026-11-08, see [`docs/twitch-mode.md`](docs/twitch-mode.md); played in Firefox 159 against the real Twitch, not yet on the TV)

- [ ] Enter on a live stream: the Twitch player covers the kiosk window and starts by itself, with sound (the kiosk profile's autoplay settings apply; restart the kiosk once after updating so they do)
- [ ] **Ads with uBlock Origin:** watch a channel as a non-subscriber for a few minutes: are ads blocked, or does the player show a blank/black screen? This decides whether step 7 (streamlink) is needed
- [ ] Keys in the player: **Space** pause, **9**/**0** volume, **M** mute, **I** channel name, **Esc**/**Q** stop and return to the card you started from
- [ ] An offline channel (search for one) says "<name> is offline right now." and any key dismisses it
- [ ] Phone remote while watching: pause, volume and mute reach the Twitch page (seek, subtitles and next do nothing)
- [ ] Leaving Twitch (Esc to the profile list, **Switch service**) while a stream plays stops it
- [ ] Playback quality and CPU on the RN96 (Twitch picks the quality; check 1080p60 is smooth)

## Twitch mode, step 4: sign-in and Following (implemented 2026-11-08, see [`docs/twitch-mode.md`](docs/twitch-mode.md); tested against fakes only, **never with a real Twitch login**)

- [ ] Twitch → profile list → **Add profile** → name → the sign-in screen: Open Firefox sign-in opens Twitch's login page; after you log in the window closes by itself within a few seconds and the screen says "Signed in to Twitch as <you>". If the window never closes, the cookie is not called `auth-token` (check with `sqlite3` on a copy of `~/.local/share/couchtube/twitch/<id>/firefox/cookies.sqlite`)
- [ ] **Phone keyboard on Twitch:** scan the QR code, check the phone says "Twitch" and the codes match, press Yes. Step 1 types the username and presses **Tab**, step 2 types the password and presses Enter (Twitch's form has both fields on one page). If the first step submits or lands in the wrong field, change `Flow::submit_key` in `src/pairing.rs`
- [ ] Twitch asks for a verification code (two-step): step 3 on the phone types it
- [ ] **Following** (first item on the Twitch Home menu for a signed-in profile) lists the channels you follow that are live, most watched first. If it says Twitch did not accept the saved sign-in, the web API refused the cookie (or its integrity check); the fallback is the official device-code login
- [ ] **S** on a Twitch profile re-opens the sign-in; removing the profile deletes its folder; Guest never shows Following
- [ ] Optional: register a Twitch application (see "Registering later" in the doc) for a supported API; then check the official source against the real Helix API: top games, streams with `game_id`, search, Load more with cursors, token renewal

## Known issues

- [ ] Channel pages sometimes give no view count or upload date for a video (seen on a Safiya Nygaard upload); the card then shows none. For You fills it from other lists or looks it up; the channel screen does not.
- [ ] mpv's direct YouTube streams answered HTTP 403 on 2026-11-01; `ytdl-format` now tries HLS first. Check again if playback in mpv fails.
- [ ] mpv warnings (`client removed during hook handling`, `Packet corrupt`, `Invalid OBU length`, `Failed to parse temporal unit`) on AV1 1080p59.94 with VA-API. Seen twice (including a 40-minute video on 2026-10-25) and playback looked fine. Only switch `ytdl-format` away from AV1 (e.g. `[vcodec!^=av01]`) if playback visibly breaks; record the video ID if it recurs.
- [ ] 2026-10-25 batch, code and unit tests done, not tried with a real account:
  - Upload date on cards (`approximate_date`, `Video.timestamp`, `formatAge()`).
  - `U` refresh passes `refresh=true` and drops the frontend cache key. Still open: F5 in the kiosk doesn't refresh, and Recommended may legitimately return the same videos.
  - Load more: `?page=N` (24 per page, max 10) on `/api/channel/{id}` and `/api/feed/{kind}`; ↓ on the last row loads the next page.
