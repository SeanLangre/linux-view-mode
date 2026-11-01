# Special for you — our own recommendations

A new Home button opens a "For You" screen: a recommendation list built by CouchTube itself, not by YouTube's algorithm. The user seeds it with themes, videos and channels, and CouchTube discovers new videos from those seeds.

Status: design agreed 2026-11-01. The algorithm is intentionally incomplete and will be tuned over time.

## Concept

1. **Seeds**, stored per profile in SQLite (`seeds` table):
   - **Theme**: free-text topic, e.g. "retro computing", "sourdough".
   - **Video**: added by pasting a link, by a key on a card, or by phone remote.
   - **Channel**: added from a video's card.
2. **Discovery**: a refresh builds candidates from the seeds:
   - Each theme becomes a yt-dlp search.
   - Each seed video contributes its related videos and its uploader.
   - Each seed channel contributes its latest uploads.
3. **Scoring** (first version, to be tuned):
   - The more seeds a candidate matches, the higher it scores.
   - Small boost for recency.
   - Anything already in the profile's history is dropped.
   - A dismissed video is a negative signal.
4. **UI**:
   - New Home button opens the For You screen, reusing the card grid and load-more paging.
   - A key on any card adds it to the seeds or dismisses it.
   - A small seed manager edits themes and shows why a video was suggested ("because: retro computing").
5. **Caching**: results are cached like the other feeds; `U` refreshes. Keep yt-dlp request volume low (account-flagging risk, see `plan.md`).

## Decision

Discovery runs as **Guest** (no account cookies), decided 2026-11-01, so searches never touch the account's history or flagging risk. The active profile is used only to leave out videos it has already started (local `progress` table). Guest caches are shared between profiles.

## Build order

- [x] 1. Home button, For You screen, `seeds` table, `GET/POST /api/seeds`, `DELETE /api/seeds/{id}`. Type a theme or paste a link to add a seed; **D** removes the focused seed. Implemented 2026-11-01; not yet tried on screen.
- [x] 2. Discovery from themes and seed channels (`src/discover.rs`, `Feeds::foryou`, `GET /api/feed/foryou?page=N`). Sources: the 6 newest themes (12 results each) and 6 newest channels (8 each). Score: `1/(1+0.15·position)` per source listing, summed, plus up to 0.5 for uploads under 30 days old. Cached 10 min, cleared when seeds change. **S** on a video seeds its channel. Implemented 2026-11-01; not yet tried with real yt-dlp on screen. Video seeds are stored but unused until step 3.
- [x] 3. Video seeds become a search for the video's title (resolved once with yt-dlp, label updated to the title; the seed video itself is never suggested). **X** hides a video (`POST /api/foryou/dismiss`, `dismissed` table) and each dismissed video lowers its channel's others by 0.5, capped at 1.5. Cards show "Because: …" (the up to two sources that found it). Implemented 2026-11-01; not yet tried on screen.
- [x] 4. Recorded in `plan.md` and `TODO.md`.
