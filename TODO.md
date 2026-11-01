# TODO

Checklist for [`plan.md`](plan.md). Tick items as they're done; move finished sections to **Done** at the bottom.

## Phase 7 — Polish (implemented 2026-10-25, not tried on the real TV)

- [ ] Subtitles per profile: Home → Subtitles (Off/English), play a video that has captions; check the language shows and that **C** cycles
- [ ] YouTube TV tile: Home → YouTube TV opens signed in; the TV user agent works; Alt+F4 returns; the profile still signs in afterwards (`user.js` restored)
- [ ] HDMI-CEC: needs `cec-client` or a `/dev/cec*` device; check **Turn TV off** appears and works
- [ ] Phone remote: pair a real phone, then pause, seek, volume, next, queue, pasted link; **Forget phones** locks it out

## For You (implemented 2026-11-01, see [`special-for-you.md`](special-for-you.md), not tried on screen)

- [ ] Home → For You: type a theme, check it appears as a seed and videos load (first load makes several yt-dlp calls)
- [ ] **S** on a video seeds its channel; **D** removes a focused seed; **U** refreshes
- [ ] Paste a video link: the seed's label turns into the video's title and similar videos appear
- [ ] **X** hides a video; "Because: …" shows on cards
- [ ] Exclusions: `not music` / `not kids` add "Not:" chips; check that matching videos (including a channel like "kidstv123") disappear after **U**
- [ ] Blended searches: with 2+ themes, "Because: A & B" appears on some cards
- [ ] 50/50 mix and the 100,000-view minimum: about every second card is from a channel that is not a seed; seed-channel videos with few views still appear
- [ ] **W** marks a video watched (badge appears, it leaves For You) and clears it again; the **History** button opens CouchTube's own history
- [ ] Tune scoring (weights in `src/discover.rs`) after a week of real use

## Playback in the CouchTube window (implemented 2026-11-01, headless-tested only)

- [ ] Enter on a video: the player covers the kiosk window with a picture, keys work, Esc returns to the same card
- [ ] Timeline: thin line always, full bar after a seek, pause or **I**
- [ ] Subtitles: start with the profile's language; **C** really hides the captions (only the player state was checked)
- [ ] Phone remote: pause, seek, volume, mute and subtitles reach the player page
- [ ] A music video (embedding refused) falls back to mpv at the same position; with no CouchTube window open, mpv takes over after 20 s
- [ ] Sponsor segments are skipped by the page
- [ ] Restart the kiosk once so the autoplay setting in `kiosk-firefox/user.js` applies

## Known issues

- [ ] Channel pages sometimes give no view count or upload date for a video (seen on a Safiya Nygaard upload); the card then shows none. For You fills it from other lists or looks it up; the channel screen does not.
- [ ] mpv's direct YouTube streams answered HTTP 403 on 2026-11-01; `ytdl-format` now tries HLS first. Check again if playback in mpv fails.
- [ ] mpv warnings (`client removed during hook handling`, `Packet corrupt`, `Invalid OBU length`, `Failed to parse temporal unit`) on AV1 1080p59.94 with VA-API. Seen twice (including a 40-minute video on 2026-10-25) and playback looked fine. Only switch `ytdl-format` away from AV1 (e.g. `[vcodec!^=av01]`) if playback visibly breaks; record the video ID if it recurs.
- [ ] 2026-10-25 batch, code and unit tests done, not tried with a real account:
  - Upload date on cards (`approximate_date`, `Video.timestamp`, `formatAge()`).
  - `U` refresh passes `refresh=true` and drops the frontend cache key. Still open: F5 in the kiosk doesn't refresh, and Recommended may legitimately return the same videos.
  - Load more: `?page=N` (24 per page, max 10) on `/api/channel/{id}` and `/api/feed/{kind}`; ↓ on the last row loads the next page.
