# Accounts and sign-in

How profiles, Google sign-in and feeds work. The user-facing steps are in the [README](../README.md#profiles-and-sign-in); the encryption design is in [`phone-signin-encryption.md`](phone-signin-encryption.md).

## Profiles
- Each profile = `{id, name, color}` plus a fixed **Guest** profile (signed out).
- Each has a Firefox profile folder at `~/.local/share/couchtube/profiles/<id>/firefox`, permissions 700, with a `user.js` (no first-run screens, no session restore, no telemetry).
- **uBlock Origin + SponsorBlock are built into the setup, not installed by hand**: `config/firefox/policies.json` is deployed to `/etc/firefox/policies/policies.json` and uses `ExtensionSettings` with `installation_mode: "force_installed"`. Every Firefox profile on the server (kiosk and accounts) gets them automatically, they update themselves, and they can't be removed. Writing our own blocker was rejected: YouTube changes its ads all the time and uBlock Origin's filters keep up with it; ours wouldn't.
- The kiosk interface uses **its own** separate Firefox profile (`~/.local/share/couchtube/kiosk-firefox`). Account logins never load in the interface.
- One profile is active at a time. Every yt-dlp and mpv call uses that profile's login.

## Signing in (once per account, from the TV)
1. "Add account" in the UI → backend opens a separate Firefox window (X11 mode, `MOZ_ENABLE_WAYLAND=0`):
   `firefox --new-instance --no-remote --profile <dir> https://accounts.google.com/ServiceLogin?continue=https://www.youtube.com`
   The login page in the app shows the phone-pairing QR code first. Firefox opens straight on Google (no local start page) when the user presses **Yes, that's my phone** after checking the code, or **Open Firefox sign-in** to use the TV keyboard. Open question: Google asked for a 2FA code instead of a tap after this change; the only difference is the missing local `Referer` and click path (see [`TODO.md`](../TODO.md)).
2. Sign in with the phone keyboard (below) or the TV keyboard. The cookies are the same either way and the window closes as in step 3.
3. The backend polls the profile's `cookies.sqlite` (a copy, since Firefox holds it open) every 2 s. Once YouTube's `LOGIN_INFO` cookie has been seen three times in a row, it closes Firefox so the cookies are flushed.
4. Backend checks the login works by fetching one item of `:ytsubs`. The profile shows a ✓ or "Sign in again".

## Phone pairing (`pairing.rs`)
- From when the profile is created (or sign-in opened) until the sign-in window closes, a random 128-bit token is live; the CouchTube login page shows `http://<LAN IP>:7879/p/<token>#<key>` as a QR code and as text. Before Firefox opens, typing is refused with "window isn't open yet" (HTTP 425), and an unused session expires after 15 minutes or when the user leaves the screen. Closing the window ends the token.
- Security notes: the key is in `Session` memory only and ends with the session; the URL containing it must never be logged (a comment at `info()` says so); the phone also keeps the key in its tab's `sessionStorage` so a reload works, and it is gone when the tab closes. The TV side is plaintext on purpose (localhost only; the on-screen code is only visible in the room).
- The phone page has three steps, each a separate request: email, password, and an optional 2-step code. Each types the text into the Firefox window and presses Enter. Each step can be sent up to 3 times per session (a failed attempt is given back); after that the phone page says to use the TV keyboard. There are no other typing routes.
- Typing fails closed: the window is found by process id, focused, and focus is checked before typing and before Enter. The text goes to `xdotool type --file -` on stdin, never as an argument. `--window` isn't used because it sends synthetic events Firefox ignores.
- Port 7879 listens on all interfaces **only while a sign-in window is open**, and is the only part of CouchTube reachable from the network during sign-in. Plain HTTP, but the phone's messages are sealed (XSalsa20-Poly1305) with a per-session key carried only in the QR URL's `#fragment`, with replay and step checks, and nothing is typed until the user confirms a code shown on both phone and TV. That stops passive sniffing, not an active attacker who rewrites the page: trusted network only; on shared Wi-Fi use the TV keyboard. Full analysis: [`phone-signin-encryption.md`](phone-signin-encryption.md).
- The main server rejects requests whose `Host` isn't `127.0.0.1:<port>` or `localhost:<port>`.

## Keeping the login alive
YouTube refreshes some login cookies over time, and a session that's never refreshed eventually logs out. A daily systemd timer (`deploy/couchtube-refresh`) calls `GET /api/profiles/<id>/status?check=true` for every profile, so yt-dlp reads the cookies and a lost login is noticed early. It does **not** start Firefox. Whether that is enough to keep logins alive, or a headless `firefox --headless --profile <dir> https://www.youtube.com` visit is needed, is still to be checked on the server (see [`server.md`](server.md)).

## Feeds (yt-dlp, with the active profile's login)
| Row / screen | Source |
|---|---|
| Home: Recommended | `https://www.youtube.com/feed/recommended` |
| Subscriptions | `https://www.youtube.com/feed/subscriptions` |
| History | `https://www.youtube.com/feed/history` |
| Watch Later | `https://www.youtube.com/playlist?list=WL` |
| Liked | `https://www.youtube.com/playlist?list=LL` |
| Search | `ytsearch24:<query>` (personalized when signed in) |
| Channel | `https://www.youtube.com/channel/<id>/videos` |

All with `--flat-playlist -J --playlist-end N`. Results are cached per profile: fresh for 30 min (history 5 min, search 6 h, channels 1 h). An expired list is still returned at once for up to 7 days (`STALE_SECS` in `src/db.rs`) while one background request (`Feeds::refresh_later`) replaces it; the response then carries `x-cache: stale`. At most one yt-dlp request runs at a time per profile.
