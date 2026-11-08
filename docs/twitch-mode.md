# Twitch mode

Status: **planned** (written 2026-11-08), nothing implemented. A second mode of CouchTube for Twitch, used the same way as the YouTube side: keyboard only, fullscreen, from the couch. Many details of Twitch's APIs below come from memory of their public docs and are marked **verify** where a wrong guess would change the design.

**Decisions (2026-11-08):** (1) ads are handled best-effort with uBlock Origin; (2) the **first screen** is a choice between **YouTube** and **Twitch**; (3) Twitch has its **own, separate profiles**, not shared with the YouTube ones.

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
| Feeds from yt-dlp with the profile's Firefox cookies | Feeds from the **Twitch Helix API** with an OAuth token per profile |
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

- Twitch needs a **registered application** (client id, from the Twitch developer console). The user creates it once and puts the client id in a setting (`COUCHTUBE_TWITCH_CLIENT_ID`). Without it, Twitch mode shows a one-screen explanation instead of failing. A client **secret** is only needed for app tokens (search, categories, public streams) and must never be committed or logged; the device-code flow for a public client avoids needing it for the user token (**verify**; otherwise use the client-credentials flow for public data and keep the secret in a `0600` file).
- **Device code login**, per profile: TV shows `twitch.tv/activate` and a short code (also as a QR code for the phone); the backend polls until Twitch returns a token. Scope: `user:read:follows` (what the Following row needs).
- **Twitch profiles are separate from YouTube profiles.** Each Twitch profile is `{id, name, color}` plus its own token, in new tables (`twitch_profiles`, `twitch_tokens`) rather than a column on `profiles`, so the two lists never mix and removing one never touches the other. Someone can have a YouTube profile and a Twitch profile of the same name without any link between them. A Twitch profile needs no Firefox folder.
- Tokens are stored in the SQLite database (`0600`), refreshed when they expire. Treat them like the Firefox cookies: local disk only, never logged, deleted when the Twitch profile is removed.
- Each service has its own fixed **Guest** profile. A Twitch guest can use categories, search and watching, but not Following.
- Per-profile data (cache, resume positions, queue) is keyed by Twitch profile id, so it stays apart from YouTube data.
- This is read-only, like the YouTube side: no following, chatting or subscribing from the TV. (Chat is read-only too, see below.)

## Left out on purpose

These YouTube-mode features do not exist in Twitch mode (decided 2026-11-08), which also keeps the Twitch Home menu short:
- **For You** (seeds, discovery, exclusions, dismiss, `s`/`x` keys): Following and categories cover discovery.
- **YouTube TV** fallback tile.
- **Subtitles** (no Subtitles menu item, no per-profile language, no `c` key in the Twitch player).

Twitch Home menu: Following, Categories, Search, Queue (VODs only), Phone remote, Switch service, Exit to desktop. The phone remote and exit stay because they are shared with the YouTube side.

## Screens

| Screen | Source |
|---|---|
| Following (live) | Helix *Get Followed Streams* (needs the token) |
| Categories | Helix *Get Top Games* |
| Category | Helix *Get Streams* with `game_id`, paged by cursor |
| Search | Helix *Search Channels* / *Search Categories* |
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

- New `src/twitch.rs`: Helix client, token storage and refresh, device-code flow, caching. Added to `AppState` next to `feeds`.
- Routes under `/api/twitch/…` (all behind the existing localhost-only `Host` check): `login` (start and poll), `following`, `categories`, `category/<id>`, `search`, `channel/<login>`, plus `/api/play` accepting a Twitch target.
- Validation like the YouTube IDs: logins `^[A-Za-z0-9_]{1,25}$`, VOD and category ids digits only, always build URLs ourselves, no shell. A Twitch target in the database is namespaced (`twitch:<login>` / `twitch:v<id>`) so it cannot collide with a YouTube id in `progress` or `queue`.
- `/api/status` gets a `kind` (`youtube` or `twitch`) so the interface picks the right player page.
- No new crates are needed unless a plain HTTP client is: Helix is called with an HTTP client crate (for example `reqwest` with rustls; pinned, at least two weeks old, as for the other dependencies).

## Frontend

- New **mode choice** screen first (see above), then the Twitch profile picker, then the Twitch home (Following, Top categories). `p` switches Twitch profile as it does on YouTube. One app with a mode flag, not two apps.
- Cards show channel name, title, category, viewer count and a red **LIVE** badge; VOD cards reuse the Resume badge.
- New keys only where needed: `t` chat on and off while watching. Everything else reuses the keys in the [README](../README.md#keys).

## Security and limits

- Tokens never leave `127.0.0.1` except in requests to Twitch; the device-code page is on `twitch.tv`, so nothing about it is exposed on the LAN (unlike the Google phone pairing).
- No Twitch cookies and no Twitch password are ever typed or stored by CouchTube. Logging in with the device code gives only the scopes requested.
- Twitch's terms allow embedding the official player; the streamlink fallback is a grey area, which is another reason to keep it optional.
- The user must register their own Twitch application (a few minutes, once).

## Build order

1. [ ] Mode choice screen (YouTube / Twitch), per-mode active profile, Twitch profiles and Guest (`twitch_profiles` table, picker).
2. [ ] `twitch.rs` with an app token only: Top Games, Streams by category, Search; Twitch screens as a guest; client id setting and the "not set up" screen.
3. [ ] `twitch.html` player page for live streams, reporting through `/api/player/report`; check on the real TV that the embed plays with `parent=127.0.0.1` and what ads do with uBlock Origin.
4. [ ] Device-code login per Twitch profile, token storage and refresh, the **Following** row.
5. [ ] Channel screen with VODs and clips, VOD resume in `progress`, queue for VODs.
6. [ ] Chat pane (`t`), phone remote support for the Twitch player.
7. [ ] Optional: streamlink fallback, if the embed's ads turn out to be unbearable.

## Open questions

- **Ads in the embed:** decided to try uBlock Origin first; how well it works is only known on the real TV with a non-subscribed account. Step 7 exists only if it is not good enough.
- **`parent` on a bare IP:** if Twitch refuses `127.0.0.1`, options are a `localhost` host name (the backend already accepts `localhost:<port>`), or fetching the stream with streamlink and playing it in mpv only.
- **Auto-start in the last mode:** the mode choice always shows first for now. A setting to skip it could come later.
- **Existing profiles screen:** the YouTube "Who's watching?" screen is now reached through the mode choice, so its Esc/back behaviour and the kiosk's start URL need a small change (the appliance should still boot straight to the first screen).
