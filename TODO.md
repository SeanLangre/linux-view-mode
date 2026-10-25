# TODO

Checklist for [`plan.md`](plan.md). Tick items as they're done; move finished sections to **Done** at the bottom.

## Phase 7 — Polish (implemented 2026-10-25, not tried on the real TV)

- [ ] Subtitles per profile: Home → Subtitles (Off/English), play a video that has captions; check the language shows and that **C** cycles
- [ ] YouTube TV tile: Home → YouTube TV opens signed in; the TV user agent works; Alt+F4 returns; the profile still signs in afterwards (`user.js` restored)
- [ ] HDMI-CEC: needs `cec-client` or a `/dev/cec*` device; check **Turn TV off** appears and works
- [ ] Phone remote: pair a real phone, then pause, seek, volume, next, queue, pasted link; **Forget phones** locks it out

## Known issues

- [ ] mpv warnings (`client removed during hook handling`, `Packet corrupt`, `Invalid OBU length`, `Failed to parse temporal unit`) on AV1 1080p59.94 with VA-API. Seen twice (including a 40-minute video on 2026-10-25) and playback looked fine. Only switch `ytdl-format` away from AV1 (e.g. `[vcodec!^=av01]`) if playback visibly breaks; record the video ID if it recurs.
- [ ] 2026-10-25 batch, code and unit tests done, not tried with a real account:
  - Upload date on cards (`approximate_date`, `Video.timestamp`, `formatAge()`).
  - `U` refresh passes `refresh=true` and drops the frontend cache key. Still open: F5 in the kiosk doesn't refresh, and Recommended may legitimately return the same videos.
  - Load more: `?page=N` (24 per page, max 10) on `/api/channel/{id}` and `/api/feed/{kind}`; ↓ on the last row loads the next page.
