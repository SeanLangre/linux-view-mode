# Twitch mode

Status: **in progress** (written 2026-11-08). Build steps 1-4 are implemented: mode choice, Twitch profiles, browsing top streams, categories and search on Twitch's unofficial web API, watching a live channel, and the Twitch sign-in with the **Following** list. **Step 4 has not been tried with a real Twitch login**; see its entry in the build order for what to check. VODs, chat and the phone remote's Twitch screens are still planned. A second mode of CouchTube for Twitch, used the same way as the YouTube side: keyboard only, fullscreen, from the couch.

**Decisions (2026-11-08):** (1) ads are handled best-effort with uBlock Origin; (2) the **first screen** is a choice between **YouTube** and **Twitch**; (3) Twitch has its **own, separate profiles**, not shared with the YouTube ones. (4) **Data source (decided after step 2): use Twitch's unofficial web API for now, register a Twitch application later.** Nothing needs registering to browse; see [Data source](#data-source-unofficial-now-official-later) for what that costs and how to switch. (5) **Following works like YouTube's sign-in**: a Firefox profile per Twitch profile and the login cookie, not a registered application.

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
| Google sign-in in a Firefox window, phone keyboard | **The same**: a Twitch sign-in in the Twitch profile's own Firefox folder, with the phone keyboard if wanted; the backend then reads Twitch's `auth-token` login cookie from the profile, like it reads YouTube's cookies. No token to register or store (step 4; **verify** that the web API accepts it, see Accounts) |
| `watch.html` embeds YouTube's player and reports to `/api/player/report` | New `twitch.html` embeds `player.twitch.tv` and reports through the same endpoint |
| mpv fallback (now [legacy](legacy-mpv.md)) | `streamlink` into mpv as an optional fallback (see Playback) |
| Cards, rows, load-more, `nav.ts` focus model | Reused unchanged; a "card" is a live stream, category or VOD |
| Resume positions and watched marks | VODs only, in the existing `progress` table (live streams have no position) |
| Queue | Not used on the Twitch side (decided 2026-11-09: no queue for VODs either) |
| For You seeds | **Left out.** "Following" plus categories replace it |
| YouTube TV tile | **Left out** (no Twitch equivalent is needed) |
| Subtitles per profile | **Left out** (no captions in Twitch mode) |

## Start screen and modes

CouchTube opens on a new **mode choice**: two big buttons, **YouTube** and **Twitch** (arrow keys, Enter). The choice leads to that service's own "Who's watching?" screen, and from there to its Home.
- The last used mode is highlighted, so the usual case is one Enter press. (Could later become an auto-start setting, see Open questions.)
- **Switch service** is an item in each Home menu; Esc on a "Who's watching?" screen goes back to the mode choice. Switching stops anything playing.
- The backend remembers the active profile **per mode**, so each service returns to its own last profile. The player page is chosen by the backend: `/api/status` carries `page` and an `id` of `twitch:<login>` for a Twitch stream, and the Twitch screens only react to ids with that prefix.
- Nothing in the YouTube mode changes: its profiles, feeds, For You seeds, history and queue are untouched by Twitch mode.

## Accounts and sign-in

- **No Twitch registration is needed for browsing** (the default, unofficial source). Registering a Twitch application is only needed for the **official** API: then set `COUCHTUBE_TWITCH_CLIENT_ID` and `COUCHTUBE_TWITCH_CLIENT_SECRET` (the secret is for the app token used when browsing as a guest; the device-code login in step 4 needs only the client id). The secret is never logged, never put in process arguments (it goes to `curl` on stdin) and must not be committed; with a systemd unit, put it in an `EnvironmentFile` with mode `0600`. See [Data source](#data-source-unofficial-now-official-later).
- **Following needs a login, and it works like the YouTube side (step 4, decided 2026-11-08).** "Add profile" opens a Firefox window on `twitch.tv/login` in that Twitch profile's own folder (`0700`, no extensions needed), reusing the sign-in machinery (the phone keyboard and the `xdotool` typing, the cookie watcher that closes the window). Twitch keeps the login in an `auth-token` cookie on `.twitch.tv`; the backend reads it from a copy of `cookies.sqlite` and sends it as `Authorization: OAuth <token>` with the website's client id to the web API (`src/gql.rs`), the way twitch.tv's own pages do. No application registration, no secret, no OAuth flow. **Not yet verified with a real login** (the code is written to make each of these a one-line change): the cookie name and header, that a followed-streams query works with it, and whether Twitch's integrity check blocks it (first pages worked without a login on 2026-11-08). The token is read per request and never logged or copied into the database; it ends with the sign-in files. The official device-code login (a user token for Helix) stays the fallback if the cookie route is blocked.
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
| Following | Yes, with the profile's Twitch login cookie (step 4; **not yet tried with a real login**) | Yes, with the device-code login (not built) |
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
4. Step 4 (the Following row) does **not** need this registration: it uses the profile's Twitch login cookie on the unofficial source. Registering only adds a supported API and the device-code login as a fallback.
5. What to check on the real API when switching (the Helix code has only been tested against a fake server): top games, streams with `game_id`, channel search, "Load more" with cursors, an expired token being renewed. These are in `TODO.md`.
6. Once registered and verified, decide whether to keep the unofficial source as a fallback (it is automatic when the credentials are missing) or remove `src/gql.rs`.

## Left out on purpose

These YouTube-mode features do not exist in Twitch mode (decided 2026-11-08), which also keeps the Twitch Home menu short:
- **For You** (seeds, discovery, exclusions, dismiss, `s`/`x` keys): Following and categories cover discovery.
- **YouTube TV** fallback tile.
- **Subtitles** (no Subtitles menu item, no per-profile language, no `c` key in the Twitch player).

Twitch Home menu: Following, Categories, Search, Phone remote, Switch service, Exit to desktop. The phone remote and exit stay because they are shared with the YouTube side.

## Screens

| Screen | Source |
|---|---|
| Following (live) | A followed-streams query with the profile's login cookie (unofficial; step 4) / Helix *Get Followed Streams* (official, needs a user token) |
| Categories | `games` (unofficial) / Helix *Get Top Games* |
| Category | `game(id){streams}` (unofficial) / Helix *Get Streams* with `game_id` |
| Search | `searchFor` (unofficial) / Helix *Search Channels* |
| Channel | Channel info plus live stream (if any), VODs from *Get Videos*, clips from *Get Clips* |
| VOD / clip | Played in the Twitch player page, resumable for VODs |

Results are cached per profile for a short time (live data about 1 minute, categories about 10 minutes), and `u` refreshes, as in the YouTube mode. Helix has rate limits; one request at a time per profile and the cache keep it far below them.

## Playback

**Default: Twitch's embedded player** in a new page `frontend/public/twitch.html`, shown by the kiosk window exactly like `watch.html`:
- `https://player.twitch.tv/js/embed/v1.js` creates the player with `parent: [location.hostname]`, i.e. `127.0.0.1` (the page is served from the backend). **Verified 2026-11-08:** Twitch answers `Content-Security-Policy: frame-ancestors http://127.0.0.1:* https://127.0.0.1:*` for `parent=127.0.0.1` (and the same for `localhost`), and in Firefox 159 with the kiosk profile's autoplay settings the embedded player started on its own, unmuted, and its position advanced in real time. VODs will use `video=<id>` in step 5.
- The page `frontend/public/twitch.html` reports `{session, position, paused, title}` to `POST /api/player/report` every 500 ms (the first `position` means "playing") and receives mpv-style commands from the phone remote, like `watch.html`. Keys: **Space** pause, **9**/**0** volume, **M** mute, **T** chat, **I** channel name, **Esc**/**Q** stop. Seeking, subtitles and next do nothing for a live stream, so arrows, **C** and **N** are ignored. Starting is `POST /api/twitch/play {login}`; the session has no progress, no watched sync and no queue, and **there is no mpv fallback**.
- **Failures:** the page reports an error when the channel goes offline (`-4`, message "<channel> is offline right now.") or when Twitch's script cannot load (`-1`), and the backend adds its own 20-second start timeout (`-3`, "Twitch did not start playing…"). The message stays on screen until a key is pressed. Autoplay refused by the browser shows "Press Space to play".
- **Ads (decided: best effort with uBlock Origin):** the embed plays Twitch ads unless the viewer is subscribed or has Turbo. The kiosk Firefox already force-installs uBlock Origin, which blocks many but not all Twitch ads and sometimes shows a blank screen instead (**not yet checked**: the Firefox test above used a fresh profile without the extension policy; check on the real TV). We accept that some ads may get through. The streamlink fallback stays optional, only for the case where the embed turns out unusable.
- **Fallback (optional, later):** `streamlink twitch.tv/<login> best` into mpv, with streamlink's ad-handling options. It depends on a third-party tool and Twitch's changing ad system, so it is a fallback after the embed, not a requirement. It would reuse the legacy mpv player code.

**Chat:** a toggleable read-only pane using Twitch's chat embed (`https://www.twitch.tv/embed/<login>/chat?parent=<host>`) or a plain text view; `t` shows or hides it. Off by default. Writing messages is out of scope (the TV has no good way to type, and sending needs more scopes).

## Backend

- `src/helix.rs`: the **official** read-only Helix client: app-token fetch and renewal on 401, a one-request-at-a-time gate, answer parsers and validators, plus the shared `Cache` and `curl` runner. Used only when both credentials are set.
- `src/gql.rs`: the **unofficial** client for Twitch's web API (GraphQL), the default. Same output types as Helix, so the routes and the frontend do not care which one runs.
- `src/twitch.rs`: the routes and `Source`, the switch between the two clients (official if both `COUCHTUBE_TWITCH_CLIENT_ID` and `_CLIENT_SECRET` are valid, otherwise unofficial). Done: `mode`, `twitch/profiles…`, `twitch/status` (`{source}`), `twitch/categories`, `twitch/streams?game_id=`, `twitch/search?q=` (all take `cursor` and `refresh`; 502 "could not reach Twitch" on upstream errors, 400 on bad ids/cursors/queries without calling Twitch). Planned: `login` (start and poll), `following`, `channel/<login>`, plus `/api/play` accepting a Twitch target.
- Validation like the YouTube IDs: logins `^[A-Za-z0-9_]{1,25}$`, VOD and category ids digits only, always build URLs ourselves, no shell. A Twitch target in the database is namespaced (`twitch:<login>` / `twitch:v<id>`) so it cannot collide with a YouTube id in `progress` or `queue`.
- `/api/twitch/play {login}` starts a live channel on `/twitch.html?c=<login>&s=<session>`; `/api/status` then reports `page` and `id: twitch:<login>`. The player lives in `src/player.rs` (`Player::play_twitch`, a session kind that reuses the page-report protocol and never starts mpv) and `src/playback.rs` (`Playback::play_twitch`, which keeps no progress session).
- **No new crate:** HTTP goes through the `curl` binary (already required for SponsorBlock), with its config on stdin so tokens and secrets never show up in `ps`. Values written into that config are checked first (credentials 10-64 letters and digits, URLs on a short allow-list) or escaped (GraphQL variables; a test round-trips awkward text through the escaping). Images are passed to the page only if they come from `static-cdn.jtvnw.net`.
- Test-only overrides: `COUCHTUBE_TWITCH_API_URL`, `COUCHTUBE_TWITCH_TOKEN_URL` (official), `COUCHTUBE_TWITCH_GQL_URL`, `COUCHTUBE_TWITCH_SITE_URL`, `COUCHTUBE_TWITCH_WEB_CLIENT_ID` (unofficial). URLs must be https or `http://127.0.0.1:`. They let `tests/twitch_smoke.py` run against fake Twitch servers.

## Frontend

- New **mode choice** screen first (see above), then the Twitch profile picker, then the Twitch home (Following, Top categories). `p` switches Twitch profile as it does on YouTube. One app with a mode flag, not two apps.
- Cards show channel name, title, category, viewer count and a red **LIVE** badge; VOD cards reuse the Resume badge.
- New keys only where needed: `t` chat on and off while watching. Everything else reuses the keys in the [YouTube README](README-youtube.md#keys).

## Security and limits

- The Twitch login cookie stays on this machine in the profile's Firefox folder (`0700`) and is sent only to Twitch. Port 7879 (phone typing) is open only during a sign-in, as on the YouTube side, with the same encryption and confirmation code.
- CouchTube never stores a Twitch password. Like the YouTube cookies, the login cookie gives access to the account: keep the profile folders private and the backend local-only, and read the account-flagging note in [`design.md`](design.md) (the same low request volume applies).
- Twitch's terms allow embedding the official player; the streamlink fallback is a grey area, which is another reason to keep it optional.
- Browsing uses an unofficial API (see [Data source](#data-source-unofficial-now-official-later)); it can break without notice, and the official source is the planned fix. Registering your own Twitch application takes a few minutes, once.

## Build order

1. [x] Mode choice screen (YouTube / Twitch), per-mode active profile, Twitch profiles and Guest. Implemented 2026-11-08: `GET/PUT /api/mode`, `GET/POST /api/twitch/profiles`, `DELETE /api/twitch/profiles/<id>`, `POST /api/twitch/profiles/<id>/select` in `src/twitch.rs`; `ModeGate.tsx` and `TwitchGate.tsx` in the frontend; a **Switch service** item in the YouTube Home menu. The Twitch side stops at a placeholder page after choosing a profile. Unit-tested and the endpoints were tried with curl; not yet seen on screen (see `TODO.md`). The kiosk's start URL did not need to change: `/` now simply opens the mode choice.
2. [x] Browsing as a guest. Implemented 2026-11-08: both sources (`src/gql.rs` unofficial, the default; `src/helix.rs` official), the `twitch/status|categories|streams|search` routes, and `TwitchHome.tsx` (Home with Top live streams, Categories, Search; a category's streams; Load more; `u` refresh, `/` search, Esc back with the focus restored). Enter on a stream says watching arrives in the next step. Tested with fake Twitch servers (`tests/twitch_smoke.py`) **and checked by hand against the real Twitch**: real categories, top streams, a category's streams and search all returned data, and the screens rendered with real thumbnails and box art.
3. [x] The player page for live streams. Implemented 2026-11-08: `frontend/public/twitch.html`, `POST /api/twitch/play`, `Player::play_twitch`, and the player frame, Esc-to-stop, focus return and error messages in `TwitchHome.tsx`. Checked: a unit test; `tests/twitch_smoke.py` (play, reports, offline, quit, stop); and **by hand against the real Twitch in Firefox 159**: the embed accepted `parent=127.0.0.1` and played. Headless Chromium cannot play it (no `PLAYING` event), so use Firefox for playback checks. **Not yet checked: ads with uBlock Origin, and the real TV.**
4. [x] Twitch sign-in and Following. Implemented 2026-11-08, **not tried with a real login**:
   - **Sign-in** (`src/twitch_login.rs`, a child of `profiles.rs`): each Twitch profile gets a Firefox folder at `<data>/twitch/<id>/firefox` (`0700`, apart from YouTube's). `POST /api/twitch/profiles/<id>/signin` opens Firefox on `https://www.twitch.tv/login` (X11 mode, for the phone typing); the backend polls a copy of `cookies.sqlite` every 2 s and, once Twitch's `auth-token` cookie has been there three times in a row, closes the window and asks Twitch who it is (`currentUser`). `GET …/status?check=true` re-checks. States are YouTube's (`signing_in`, `checking`, `signed_in`, `sign_in_again`, `check_failed`).
   - **Phone keyboard:** the same QR code, confirmation code and encryption as for Google, started with `POST /api/pairing?service=twitch`. The sealed `hello` reply tells the page the site is Twitch, so it words its steps for Twitch. Twitch's login form has the username and the password on one page, so the phone's first step presses **Tab** instead of Enter (`Flow` in `src/pairing.rs`); the password step submits.
   - **Following:** `GET /api/twitch/following` reads the token fresh from the profile's cookies (never stored or logged), and sends `currentUser { followedLiveUsers(first: 100) { … stream { … } } }` to the web API with `Authorization: OAuth <token>`; live channels only, most watched first, cached 60 s per profile. A rejected token (HTTP 401, or `currentUser` null) is a `409` that says to sign in again. The Twitch Home menu shows **Following** first for a signed-in (non-Guest) profile; the profile list has **S** to sign in and the sign-in screen is `TwitchSignIn.tsx`.
   - **Checked without a login (2026-11-08):** the query shapes validate against Twitch's schema (`currentUser { login displayName }` and `followedLiveUsers(first)`); a bogus token gets `401 The "Authorization" token is invalid`, so the `OAuth` header is understood. Tested against fakes in `tests/twitch_smoke.py` (sign-in with a fake Firefox that writes a cookie database, Following, a rejected token, removal, and no token in the log).
   - **To check with a real login, and where to change it if it fails:** (1) the cookie is `auth-token` on `.twitch.tv` (`COOKIE_NAME`, `COOKIE_HOST` in `src/twitch_login.rs`); (2) Twitch's login page takes Tab then Enter from the phone, and opens with the username field focused (`Flow::submit_key` in `src/pairing.rs`; `LOGIN_URL`); (3) the web API accepts the cookie's value as the OAuth token and does not fail its integrity check for it (`CURRENT_USER` and `FOLLOWING` in `src/gql.rs`; if it does, the fallback is the official device-code login); (4) Twitch's two-step verification page takes the phone's third step.
5. [~] Channel screen with VODs and clips (implemented 2026-11-09; backend checked against the real Twitch with curl, the pages **not yet seen on screen**). `GET /api/twitch/channel/<login>?kind=vods|clips` (`cursor`, `refresh`) returns `{login, name, live, items, cursor}`: the live stream if on, and past broadcasts (`type: ARCHIVE`, newest first) or the most watched clips; `404` for an unknown channel. Search results open this screen; **C** on any stream card opens its channel; **T** switches Broadcasts/Clips. `/api/twitch/play` also takes `{video, start?}` and `{clip}`. Broadcasts play in the embedded player (`video=v<id>`, `time=`) with seeking (arrows ±10 s/±60 s, remote seek), report position and duration, and **resume**: positions live in their own table `twitch_progress(profile_id → twitch_profiles, video_id, position, duration, completed)`, because `progress` belongs to the YouTube profiles; cards show Resume/Watched. Session ids are `twitch:vod:<id>` and `twitch:clip:<slug>` (not `twitch:v<id>`, which could be a channel called `v1`). A clip plays in `clips.twitch.tv/embed`, which has no scripting API: it starts and Esc stops it, with no position, pause or seek. **No queue for broadcasts, by decision.** **Still open:** the official (Helix) source for this screen, which always uses the web API like Following.
6. [~] Chat pane and phone remote (implemented 2026-11-09, **not seen on screen or on a phone**). **Chat:** **T** (or the phone's 💬 Chat button, the command `["cycle","chat"]`) shows Twitch's chat embed (`www.twitch.tv/embed/<login>/chat?parent=<host>&darkpopout`; Twitch's CSP allows `127.0.0.1`, checked with curl) in a 25 % pane on the right and narrows the video to 75 %; off by default, read-only, off by default. A **live stream** gets Twitch's chat embed (the frame is loaded the first time it is wanted). A **broadcast** has no embed, so its chat *replay* is drawn by the page itself: `GET /api/twitch/chat/<video>?t=<seconds>` returns `{items: [{at, name, color, text}], more}` from Twitch's web API (no sign-in; Twitch hands out the stored batch of about 60 lines that contains that moment, cached 10 min), and the page keeps the batches it has, fetches the next one when the video passes the end of what it holds or after a seek, and shows the last 40 lines that happened before the current time (emotes show as their names; colours are checked as `#rrggbb`). Clips have no chat. **Phone remote:** `/state` now carries `mode` and, in Twitch mode, the active Twitch profile and an empty queue; the phone page then shows Chat instead of CC, disables Next, hides the queue, and its box takes a channel name or twitch.tv link (`remote::twitch_login`) and plays it live; queue commands answer "Twitch has no queue". Pause, volume, mute and stop reach the Twitch page as before, and seek works for broadcasts. The Twitch Home menu got a **Phone remote** screen (turn on, pair, confirm, forget), the same as the YouTube one.
7. [ ] Optional: streamlink fallback, if the embed's ads turn out to be unbearable.

## Open questions

- **When to register a Twitch application:** no longer needed for Following (step 4 uses the login cookie). It stays useful for a supported API and as the fallback if Twitch blocks cookie use; see [Registering later](#registering-later-switching-to-the-official-api).
- **Ads in the embed:** the embed itself is verified; how well uBlock Origin handles Twitch's ads is only known on the real TV with a non-subscribed account. Step 7 exists only if it is not good enough.
- **`parent` on a bare IP:** answered 2026-11-08, Twitch accepts `127.0.0.1` and `localhost`; nothing to do.
- **Auto-start in the last mode:** the mode choice always shows first for now. A setting to skip it could come later.
- **Mode choice and the remembered mode:** the choice screen highlights the last used service but always shows. Esc on a YouTube or Twitch profile list returns to it.

## Using it today

Twitch mode is being built (see the sections above): so far you can browse top live streams, categories and channel search, with no Twitch account or registration needed (it uses Twitch's unofficial web API); you can sign a Twitch profile in (**S** on the profile list; Firefox on Twitch's login page, with the phone keyboard) so **Following** lists the channels you follow, and you can also **watch a live channel**: Enter on a stream opens Twitch's player in the CouchTube window (**Space** pause, **9**/**0** volume, **M** mute, **Esc** stop). You can also open a channel (search, or **C** on a stream card) to watch its past broadcasts (they resume where you stopped) and top clips (**T** switches), and press **T** while watching a live stream for a read-only chat pane. The phone remote works on the Twitch side too (pause, volume, mute, chat, stop, and a channel name or twitch.tv link to watch).

## API

| Method | Path | Returns |
|---|---|---|
| GET/POST | `/api/twitch/profiles` | Twitch profiles `{profiles: [{id, name, color}], active_id}` (a list separate from `/api/profiles`, with its own Guest); POST `{name, color?}` creates one |
| DELETE | `/api/twitch/profiles/<id>` | Removes a Twitch profile (Guest can't be removed); the active one falls back to Guest |
| POST | `/api/twitch/profiles/<id>/select` | Selects the active Twitch profile and stops playback |
| POST | `/api/twitch/play` | Body `{login}`, `{video, start?}` (a past broadcast, resumed from the active Twitch profile's saved position unless `start`) or `{clip}`; starts it in the CouchTube window (`/api/status` then shows `page: /twitch.html?…`, `id: twitch:<login>`); an offline channel or other failure appears as `error` there |
| POST | `/api/twitch/profiles/<id>/signin` | Opens Firefox on Twitch's login page in that profile's own folder (phone keyboard as for YouTube); closes itself once the login cookie appears |
| GET | `/api/twitch/profiles/<id>/status` | `{state, message}` like YouTube's; `?check=true` asks Twitch who the saved login belongs to |
| GET | `/api/twitch/channel/<login>` | `?kind=vods` (default) or `clips`, with `cursor`/`refresh`: `{login, name, live, items, cursor}`, the live stream (or `null`) plus past broadcasts or top clips; broadcasts carry the active profile's `resume_position`/`completed`; 404 for an unknown channel |
| GET | `/api/twitch/following` | Live channels the active Twitch profile follows `{items, cursor: null}`; 409 for Guest, an unsigned profile or a rejected login |
| GET | `/api/twitch/status` | `{source}`: `unofficial` (default) or `official` (both Twitch credentials set) |
| GET | `/api/twitch/categories`, `/streams`, `/search` | Twitch pages `{items, cursor}`: top categories; live streams (`?game_id=` for one category); channel search (`?q=`). All take `cursor` and `refresh=true`; with the unofficial source a cursor is the next page size (`n48`) |

The mode choice (`/api/mode`), the player status (`/api/status`) and the phone remote are shared with the other modes; see the main [README](../README.md#api).
