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
   - Boosts for recency, length and views per day multiply the relevance; one channel's further videos count less; half of the list is reserved for channels that are not seeds, and those need 2,000+ views. Shorts are never suggested.
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
- [x] 3h. Ranking revised (2026-11-01) after a real search showed niche themes' top results all under 100k views, so the old minimum emptied the new-channel half:
  - Boosts multiply relevance instead of adding to it: `relevance × (1 + 0.4·recency + 0.3·length + 0.3·popularity)`. Recency and length keep their curves (0–1); popularity is views per day since upload on a log scale (1 at 10,000/day, 0.5 at 100/day). Before, +1.4 of boosts outweighed a first place (1.0), so recent long videos beat better matches.
  - The minimum for non-seed channels drops from 100,000 to 2,000 views (spam guard only).
  - Each further video of one channel keeps 70% of the previous one's share (0.7, 0.49…), so one channel cannot fill the top.
  - Shorts are dropped everywhere, not only in For You (`ytdlp::parse_playlist`, so search, channels and feeds too): anything 3 minutes or shorter (search results link Shorts as normal watch URLs, so length is the only sign). Live streams have no length and stay. Since pages can now be shorter than 24, the frontend treats only an empty page (or one with nothing new) as the end of a list.
  - Seeds rotate: with more seeds of a kind than its maximum, the newest half of the slots go to the newest seeds and the rest rotate through the older ones by the hour, so no seed is ignored for good.
  - Exclusion related words narrowed: `music` no longer hides "mix", "audio" or "playlist"; `kids` no longer hides "cartoon".
- [x] 3i. First real use (2026-11-01) showed cheap horror "FULL MOVIE" uploads, hours old with ~10–50k views, all from the blend "interesting long documentary & spooky":
  - A blend's results must mention both themes in the title or channel name (`Source::needs`, `mentions_all`): one word of 3+ letters per theme, ignoring filler words like "interesting" and "long"; words of 5+ letters also match other endings ("documentaries", "spookiest"). On that list it dropped all 10 blend results.
  - Views per day are spread over at least 30 days (`POPULAR_MIN_DAYS`), so 15k views in five hours no longer counts as 15k a day.
- [x] 4. Recorded in `plan.md` and `TODO.md`.
