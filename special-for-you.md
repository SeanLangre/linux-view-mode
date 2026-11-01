# Special for you — our own recommendations

A new Home button opens a "For You" screen: a recommendation list built by CouchTube itself, not by YouTube's algorithm. The user seeds it with themes, videos and channels, and CouchTube discovers new videos from those seeds.

Status: implemented 2026-11-01 (build order below), not yet tried with real use. The algorithm is intentionally simple and will be tuned over time. `README.md` has the user-facing description; this file keeps the decisions.

## Concept

1. **Seeds**, stored per profile in SQLite (`seeds` table):
   - **Theme**: free-text topic, e.g. "retro computing", "sourdough".
   - **Video**: added by pasting a link, by a key on a card, or by phone remote.
   - **Channel**: added from a video's card.
   - **Exclusion**: typed as `not music` or `-music`; hides matching videos instead of seeding anything.
2. **Discovery**: a refresh builds candidates from the seeds:
   - Each theme becomes a yt-dlp search; pairs of themes are also searched together.
   - Each seed video becomes a search for its title.
   - Each seed channel contributes its latest uploads.
3. **Scoring** (first version, to be tuned):
   - The more seeds a candidate matches, the higher it scores.
   - Boosts for recency and length; half of the list is reserved for channels that are not seeds, and those need 100,000+ views.
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
- [x] 2. Discovery from themes and seed channels (`src/discover.rs`, `Feeds::foryou`, `GET /api/feed/foryou?page=N`). Sources: the 6 newest themes (12 results each) and 6 newest channels (8 each). Score: `1/(1+0.15·position)` per source listing, summed, plus a recency boost (revised 2026-11-01: +0.8 up to one year old, fading to +0.4 at two years and 0 from three) and a length boost (0 under 5 min, up to +0.6 at 40 min). Cached 10 min, cleared when seeds change. **S** on a video seeds its channel. Implemented 2026-11-01; not yet tried with real yt-dlp on screen. Video seeds are stored but unused until step 3.
- [x] 3. Video seeds become a search for the video's title (resolved once with yt-dlp, label updated to the title; the seed video itself is never suggested). **X** hides a video (`POST /api/foryou/dismiss`, `dismissed` table) and each dismissed video lowers its channel's others by 0.5, capped at 1.5. Cards show "Because: …" (the up to two sources that found it). Implemented 2026-11-01; not yet tried on screen.
- [x] 3b. Half of the list (30% until 2026-11-01) is reserved for the best videos from channels that are not channel seeds (`discover::mix`), spread evenly, so it is not only the seeded channels' own uploads. Implemented 2026-11-01.
- [x] 3c. Exclusion seeds: typing `not music` or `-music` adds an "exclude" seed; videos with that word or phrase (whole words, any case) in the title or channel name are dropped. Existing databases are migrated on start (the seeds table is rebuilt). Implemented 2026-11-01.
- [x] 3d. Blended searches: pairs among the 4 newest themes are searched together ("A B", 10 results; 3 pairs per refresh, rotating by the hour), labelled "A & B". A video that fits both themes then scores from several lists. Implemented 2026-11-01.
- [x] 3e. Videos from channels that are not channel seeds need 100,000+ views (unknown counts do not pass); seed channels are exempt (`MIN_NEW_CHANNEL_VIEWS`). Missing counts are looked up with yt-dlp, up to 8 per refresh, remembered a week (`Feeds::fill_view_counts`). Implemented 2026-11-01.
- [x] 3f. Exclusions also match plural forms, a few related words (`RELATED` in `src/discover.rs`: `kids` → children, cartoon, sing along…; `music` → song, lyrics, remix, phonk, lofi…) and, for single words of 4+ letters, the inside of a channel name (`kids` hides "kidstv123"). Videos missing a view count or date in one list take it from another list that has it. Implemented 2026-11-01.
- [x] 3g. Screen changes: a **History** button (CouchTube's own history, feed kind `local`), **W** marks a video watched or clears it (`POST /api/watched`, local only), watched/resume badges on every list, and the Search/Profiles footer hints are hidden on For You. Implemented 2026-11-01.
- [x] 4. Recorded in `plan.md` and `TODO.md`.
