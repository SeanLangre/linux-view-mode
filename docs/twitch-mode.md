# Twitch mode

Status: **in progress** (written 2026-11-08). Build steps 1 and 2 are implemented (mode choice, Twitch profiles; browsing top streams, categories and search as a guest, on Twitch's unofficial web API); watching and everything after it is still planned. A second mode of CouchTube for Twitch, used the same way as the YouTube side: keyboard only, fullscreen, from the couch. Details of Twitch's APIs marked **verify** come from memory and have not been checked.

**Decisions (2026-11-08):** (1) ads are handled best-effort with uBlock Origin; (2) the **first screen** is a choice between **YouTube** and **Twitch**; (3) Twitch has its **own, separate profiles**, not shared with the YouTube ones. (4) **Data source (decided after step 2): use Twitch's unofficial web API for now, register a Twitch application later.** Nothing needs registering to browse; see [Data source](#data-source-unofficial-now-official-later) for what that costs and how to switch.

## Goal

Open CouchTube, choose **Twitch** on the first screen, pick a Twitch profile, and:
- see who you follow that is **live now**, with viewer counts and what they are playing;
- browse **categories** (games) and the live streams in each, and **search** channels;
- open a channel to watch it live, or play its **VODs** and clips;
- watch fullscreen in the CouchTube window, with an optional chat pane, no mouse and no second window.

Same appliance feel as the YouTube mode: arrow keys, Enter, Esc to go back, `p` for profiles.

## How it maps onto the YouTube mode

| YouTube mode | Twitch mode |
|---|---|
| Feeds from yt-dlp with the profile's Firefox cookies | Feeds from **Twitch's web API** (unofficial, no registration), later the official Helix API with an OAuth token per profile |
| Google sign-in in a Firefox window, phone keyboard | **Device-code login**: the TV shows a code, you enter it at `twitch.tv/activate` on your phone. No typing into Firefox, so no `xdotool` or phone-pairing page (**verify** that Twitch's device code flow works for a public client with `user:read:follows`) |
| `watch.html` embeds YouTube's player and reports to `/api/player/report` | New `twitch.html` embeds `player.twitch.tv` and reports through the same endpoint |
| mpv fallback (now [legacy](legacy-mpv.md)) | `streamlink` into mpv as an optional fallback (see Playback) |
| Cards, rows, load-more, `nav.ts` focus model | Reused unchanged; a "card" is a live stream, category or VOD |
| Resume positions and watched marks | VODs only, in the existing `progress` table (live streams have no position) |
| Queue | Not used for live streams; VODs can be queued |
| For You seeds | **Left out.** "Following" plus categories replace it |
| YouTube TV tile | **Left out** (no Twitch equivalent is needed) |
| Subtitles per profile | **Left out** (no captions in Twitch mode) |

## Start screen and modes

CouchTube opens on a new **mode choice**: two big buttons, **YouTube** and **Twitch** (arrow keys, Enter). The choice leads to that service's own "Who's watching?" screen, and from there to its Home.
- The last used mode is highlighted, so the usual case is one Enter press. (Could later become an auto-start setting, see Open questions.)
- **Switch service** is an item in each Home menu; Esc on a "Who's watching?" screen goes back to the mode choice. Switching stops anything playing.
- The mode is part of the frontend state and of `/api/status` (`kind`), so the right player page opens. The backend remembers the active profile **per mode**, so each service returns to its own last profile.
- Nothing in the YouTube mode changes: its profiles, feeds, For You seeds, history and queue are untouched by Twitch mode.

## Accounts and sign-in

- **No Twitch registration is needed for browsing** (the default, unofficial source). Registering a Twitch application is only needed for the **official** API: then set `COUCHTUBE_TWITCH_CLIENT_ID` and `COUCHTUBE_TWITCH_CLIENT_SECRET` (the secret is for the app token used when browsing as a guest; the device-code login in step 4 needs only the client id). The secret is never logged, never put in process arguments (it goes to `curl` on stdin) and must not be committed; with a systemd unit, put it in an `EnvironmentFile` with mode `0600`. See [Data source](#data-source-unofficial-now-official-later).
- **Device code login**, per profile: TV shows `twitch.tv/activate` and a short code (also as a QR code for the phone); the backend polls until Twitch returns a token. Scope: `user:read:follows` (what the Following row needs).
- **Twitch profiles are separate from YouTube profiles.** Each Twitch profile is `{id, name, color}` plus its own token, in new tables (`twitch_profiles`, `twitch_tokens`) rather than a column on `profiles`, so the two lists never mix and removing one never touches the other. Someone can have a YouTube profile and a Twitch profile of the same name without any link between them. A Twitch profile needs no Firefox folder.
- Tokens are stored in the SQLite database (`0600`), refreshed when they expire. Treat them like the Firefox cookies: local disk only, never logged, deleted when the Twitch profile is removed.
- Each service has its own fixed **Guest** profile. A Twitch guest can use categories, search and watching, but not Following.
- Per-profile data (cache, resume positions, queue) is keyed by Twitch profile id, so it stays apart from YouTube data.
- This is read-only, like the YouTube side: no following, chatting or subscribing from the TV. (Chat is read-only too, see below.)

## Data source: unofficial now, official later

Browsing uses one of two sources, chosen at start-up (`Source` in `src/twitch.rs`). Both return the same cards, so the screens do not change.

| | **Unofficial** (default, `src/gql.rs`) | **Official** (`src/helix.rs`) |
|---|---|---|
| What it is | The GraphQL API that twitch.tv itself calls, with the public client id the website ships | Twitch's documented Helix API |
| Registration | **None** | A free Twitch application; set `COUCHTUBE_TWITCH_CLIENT_ID` and `COUCHTUBE_TWITCH_CLIENT_SECRET` |
| Stability | Not supported by Twitch. It can change or be blocked without notice, like yt-dlp on the YouTube side | Documented and versioned |
| Terms | A grey area (automated use of the website's own API); read-only, public data, low request volume | Allowed |
| Paging | No cursors (see below); a bigger first page instead | Real cursors |
| Limits | Categories 100, top streams 30, a category's streams about 100, search about 15 | Much higher |
| Following | **Not available** (needs a logged-in user) | Yes, with the device-code login (step 4) |
| Needs a secret | No | Yes for guest browsing (app token); the user login needs only the client id |

### How the unofficial source works (verified 2026-11-08)
- `POST https://gql.twitch.tv/gql` with the header `Client-ID: <website client id>` and a JSON body `{query, variables}`. Search text and ids only ever travel in `variables`, never spliced into the query text.
- **Client id:** the website's own id, found as `clientId="…"` in the page source of `https://www.twitch.tv/` (currently `kimne78kx3ncx6brgo4mv6wki5h1ko`, compiled in). If Twitch answers `400 … "Client-ID" header is invalid`, the backend reads the current id from twitch.tv, logs a warning, and retries once. `COUCHTUBE_TWITCH_WEB_CLIENT_ID` overrides the compiled-in one.
- **No cursors.** Any `after:` argument fails Twitch's *integrity check* (`IntegrityCheckFailed`); the first page of any query works. So "Load more" asks for a **bigger first page** (`first: 24`, then 48, 72, up to Twitch's maximum) and the screen adds the cards it does not have yet. A cursor here is just the next size (`n48`). Twitch often returns a few fewer than asked (23 of 24), so a page that is at least 80% full counts as full.
- **Queries used:** `games(first)` for categories; `streams(first)` for all of Twitch (maximum 30); `game(id){streams(first)}` for one category (maximum about 100); `searchFor(userQuery, platform:"web", options:{targets:[{index:CHANNEL}]})` for channels (about 15, no paging, no `first`).
- If a query starts failing everywhere, check first whether twitch.tv changed its schema or the integrity check now also blocks first pages. The fix is either updating the queries in `src/gql.rs` or switching to the official source below.

### Registering later (switching to the official API)
1. Open the Twitch developer console (`dev.twitch.tv/console`), sign in, **Register Your Application**: any name, OAuth redirect URL `http://localhost`, category *Application Integration*, client type *Confidential* (for the secret). Note the **client id** and generate a **client secret**.
2. Start CouchTube with `COUCHTUBE_TWITCH_CLIENT_ID` and `COUCHTUBE_TWITCH_CLIENT_SECRET` set. `GET /api/twitch/status` then answers `{"source":"official"}`, and the screens use Helix with real cursors. If only one of the two is set, or a value is malformed, the backend logs that and stays on the unofficial source.
3. Keep the secret out of git and out of command lines: use a `0600` `EnvironmentFile` for the systemd unit.
4. **Step 4 (the Following row) needs this registration**, because a user token comes from the device-code flow with your application's client id. Until then the Following item stays out of the Twitch Home menu.
5. What to check on the real API when switching (the Helix code has only been tested against a fake server): top games, streams with `game_id`, channel search, "Load more" with cursors, an expired token being renewed. These are in `TODO.md`.
6. Once registered and verified, decide whether to keep the unofficial source as a fallback (it is automatic when the credentials are missing) or remove `src/gql.rs`.

## Left out on purpose

These YouTube-mode features do not exist in Twitch mode (decided 2026-11-08), which also keeps the Twitch Home menu short:
- **For You** (seeds, discovery, exclusions, dismiss, `s`/`x` keys): Following and categories cover discovery.
- **YouTube TV** fallback tile.
- **Subtitles** (no Subtitles menu item, no per-profile language, no `c` key in the Twitch player).

Twitch Home menu: Following, Categories, Search, Queue (VODs only), Phone remote, Switch service, Exit to desktop. The phone remote and exit stay because they are shared with the YouTube side.

## Screens

| Screen | Source |
|---|---|
| Following (live) | Helix *Get Followed Streams* (needs the token, so needs a registered application) |
| Categories | `games` (unofficial) / Helix *Get Top Games* |
| Category | `game(id){streams}` (unofficial) / Helix *Get Streams* with `game_id` |
| Search | `searchFor` (unofficial) / Helix *Search Channels* |
| Channel | Channel info plus live stream (if any), VODs from *Get Videos*, clips from *Get Clips* |
| VOD / clip | Played in the Twitch player page, resumable for VODs |

Results are cached per profile for a short time (live data about 1 minute, categories about 10 minutes), and `u` refreshes, as in the YouTube mode. Helix has rate limits; one request at a time per profile and the cache keep it far below them.

## Playback

**Default: Twitch's embedded player** in a new page `frontend/public/twitch.html`, shown by the kiosk window exactly like `watch.html`:
- `https://player.twitch.tv/?channel=<login>&parent=<host>` for live, `video=<id>` for VODs, `parent` being the host the page is served from (`127.0.0.1`). **Verify** that Twitch accepts `127.0.0.1` / `localhost` as a parent; if not, the page needs a different route (see Open questions).
- Reports `{session, position, duration, paused, ...}` to `POST /api/player/report` and receives commands from the phone remote, using the same protocol as `watch.html`. The keys are the same too (Space, arrows, `9`/`0`, `m`, `i`, Esc/`q`, but no `c` since there are no subtitles); for live streams seek keys do nothing.
- **Ads (decided: best effort with uBlock Origin):** the embed plays Twitch ads unless the viewer is subscribed or has Turbo. The kiosk Firefox already force-installs uBlock Origin, which blocks many but not all Twitch ads and sometimes shows a blank screen instead (**verify on the real TV**). We accept that some ads may get through. The streamlink fallback stays optional, only for the case where the embed turns out unusable.
- **Fallback (optional, later):** `streamlink twitch.tv/<login> best` into mpv, with streamlink's ad-handling options. It depends on a third-party tool and Twitch's changing ad system, so it is a fallback after the embed, not a requirement. It would reuse the legacy mpv player code.

**Chat:** a toggleable read-only pane using Twitch's chat embed (`https://www.twitch.tv/embed/<login>/chat?parent=<host>`) or a plain text view; `t` shows or hides it. Off by default. Writing messages is out of scope (the TV has no good way to type, and sending needs more scopes).

## Backend

- `src/helix.rs`: the **official** read-only Helix client: app-token fetch and renewal on 401, a one-request-at-a-time gate, answer parsers and validators, plus the shared `Cache` and `curl` runner. Used only when both credentials are set.
- `src/gql.rs`: the **unofficial** client for Twitch's web API (GraphQL), the default. Same output types as Helix, so the routes and the frontend do not care which one runs.
- `src/twitch.rs`: the routes and `Source`, the switch between the two clients (official if both `COUCHTUBE_TWITCH_CLIENT_ID` and `_CLIENT_SECRET` are valid, otherwise unofficial). Done: `mode`, `twitch/profiles…`, `twitch/status` (`{source}`), `twitch/categories`, `twitch/streams?game_id=`, `twitch/search?q=` (all take `cursor` and `refresh`; 502 "could not reach Twitch" on upstream errors, 400 on bad ids/cursors/queries without calling Twitch). Planned: `login` (start and poll), `following`, `channel/<login>`, plus `/api/play` accepting a Twitch target.
- Validation like the YouTube IDs: logins `^[A-Za-z0-9_]{1,25}$`, VOD and category ids digits only, always build URLs ourselves, no shell. A Twitch target in the database is namespaced (`twitch:<login>` / `twitch:v<id>`) so it cannot collide with a YouTube id in `progress` or `queue`.
- `/api/status` gets a `kind` (`youtube` or `twitch`) so the interface picks the right player page.
- **No new crate:** HTTP goes through the `curl` binary (already required for SponsorBlock), with its config on stdin so tokens and secrets never show up in `ps`. Values written into that config are checked first (credentials 10-64 letters and digits, URLs on a short allow-list) or escaped (GraphQL variables; a test round-trips awkward text through the escaping). Images are passed to the page only if they come from `static-cdn.jtvnw.net`.
- Test-only overrides: `COUCHTUBE_TWITCH_API_URL`, `COUCHTUBE_TWITCH_TOKEN_URL` (official), `COUCHTUBE_TWITCH_GQL_URL`, `COUCHTUBE_TWITCH_SITE_URL`, `COUCHTUBE_TWITCH_WEB_CLIENT_ID` (unofficial). URLs must be https or `http://127.0.0.1:`. They let `tests/twitch_smoke.py` run against fake Twitch servers.

## Frontend

- New **mode choice** screen first (see above), then the Twitch profile picker, then the Twitch home (Following, Top categories). `p` switches Twitch profile as it does on YouTube. One app with a mode flag, not two apps.
- Cards show channel name, title, category, viewer count and a red **LIVE** badge; VOD cards reuse the Resume badge.
- New keys only where needed: `t` chat on and off while watching. Everything else reuses the keys in the [README](../README.md#keys).

## Security and limits

- Tokens never leave `127.0.0.1` except in requests to Twitch; the device-code page is on `twitch.tv`, so nothing about it is exposed on the LAN (unlike the Google phone pairing).
- No Twitch cookies and no Twitch password are ever typed or stored by CouchTube. Logging in with the device code gives only the scopes requested.
- Twitch's terms allow embedding the official player; the streamlink fallback is a grey area, which is another reason to keep it optional.
- Browsing uses an unofficial API (see [Data source](#data-source-unofficial-now-official-later)); it can break without notice, and the official source is the planned fix. Registering your own Twitch application takes a few minutes, once.

## Build order

1. [x] Mode choice screen (YouTube / Twitch), per-mode active profile, Twitch profiles and Guest. Implemented 2026-11-08: `GET/PUT /api/mode`, `GET/POST /api/twitch/profiles`, `DELETE /api/twitch/profiles/<id>`, `POST /api/twitch/profiles/<id>/select` in `src/twitch.rs`; `ModeGate.tsx` and `TwitchGate.tsx` in the frontend; a **Switch service** item in the YouTube Home menu. The Twitch side stops at a placeholder page after choosing a profile. Unit-tested and the endpoints were tried with curl; not yet seen on screen (see `TODO.md`). The kiosk's start URL did not need to change: `/` now simply opens the mode choice.
2. [x] Browsing as a guest. Implemented 2026-11-08: both sources (`src/gql.rs` unofficial, the default; `src/helix.rs` official), the `twitch/status|categories|streams|search` routes, and `TwitchHome.tsx` (Home with Top live streams, Categories, Search; a category's streams; Load more; `u` refresh, `/` search, Esc back with the focus restored). Enter on a stream says watching arrives in the next step. Tested with fake Twitch servers (`tests/twitch_smoke.py`, `--browser` for the keyboard flow) **and checked by hand against the real Twitch**: real categories, top streams, a category's streams and search all returned data, and the screens rendered with real thumbnails and box art.
3. [ ] `twitch.html` player page for live streams, reporting through `/api/player/report`; check on the real TV that the embed plays with `parent=127.0.0.1` and what ads do with uBlock Origin.
4. [ ] Device-code login per Twitch profile, token storage and refresh, the **Following** row.
5. [ ] Channel screen with VODs and clips, VOD resume in `progress`, queue for VODs.
6. [ ] Chat pane (`t`), phone remote support for the Twitch player.
7. [ ] Optional: streamlink fallback, if the embed's ads turn out to be unbearable.

## Open questions

- **When to register a Twitch application:** needed for Following (step 4) and for a supported API. Until then the unofficial source carries browsing; see [Registering later](#registering-later-switching-to-the-official-api).
- **Ads in the embed:** decided to try uBlock Origin first; how well it works is only known on the real TV with a non-subscribed account. Step 7 exists only if it is not good enough.
- **`parent` on a bare IP:** if Twitch refuses `127.0.0.1`, options are a `localhost` host name (the backend already accepts `localhost:<port>`), or fetching the stream with streamlink and playing it in mpv only.
- **Auto-start in the last mode:** the mode choice always shows first for now. A setting to skip it could come later.
- **Mode choice and the remembered mode:** the choice screen highlights the last used service but always shows. Esc on a YouTube or Twitch profile list returns to it.
