# TODO

Checklist for the [roadmap](docs/roadmap.md). Remove items once they are verified on screen.

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

## Known issues

- [ ] Channel pages sometimes give no view count or upload date for a video (seen on a Safiya Nygaard upload); the card then shows none. For You fills it from other lists or looks it up; the channel screen does not.
- [ ] mpv's direct YouTube streams answered HTTP 403 on 2026-11-01; `ytdl-format` now tries HLS first. Check again if playback in mpv fails.
- [ ] mpv warnings (`client removed during hook handling`, `Packet corrupt`, `Invalid OBU length`, `Failed to parse temporal unit`) on AV1 1080p59.94 with VA-API. Seen twice (including a 40-minute video on 2026-10-25) and playback looked fine. Only switch `ytdl-format` away from AV1 (e.g. `[vcodec!^=av01]`) if playback visibly breaks; record the video ID if it recurs.
- [ ] 2026-10-25 batch, code and unit tests done, not tried with a real account:
  - Upload date on cards (`approximate_date`, `Video.timestamp`, `formatAge()`).
  - `U` refresh passes `refresh=true` and drops the frontend cache key. Still open: F5 in the kiosk doesn't refresh, and Recommended may legitimately return the same videos.
  - Load more: `?page=N` (24 per page, max 10) on `/api/channel/{id}` and `/api/feed/{kind}`; ↓ on the last row loads the next page.
