# Legacy: mpv playback

CouchTube now plays videos in Firefox (YouTube's embedded player on `frontend/public/watch.html`, shown by the kiosk window). **mpv is legacy**: it is still in the code and still the automatic fallback, but new work targets the Firefox player. This page keeps what is specific to mpv. Related: [`design.md`](design.md), [`../README.md`](../README.md).

## When mpv still runs
mpv plays the video instead, from the same position, when YouTube refuses to embed it (many music videos, error 150/153) or the page has not started within 20 seconds (for example when no CouchTube window is open, or it is still on the profile screen). Set `COUCHTUBE_PLAYER=mpv` to always use mpv. Limits: the kiosk profile is not signed in to YouTube (account logins stay out of the interface), so Premium, age-restricted videos and your account's ad settings do not apply, and age-restricted videos fall back to mpv; YouTube chooses the quality; ad skip buttons cannot be clicked, so install uBlock Origin through the Firefox policy (`config/firefox/policies.json`).

`player.rs` runs one mpv at a time (`--fullscreen --hwdec=vaapi --ytdl-format=… --input-ipc-server=…`), reads `time-pos`/`duration` over one persistent IPC connection, saves progress about every five seconds, advances the queue at end of file and treats mpv's `n` as "next".

## Settings and keys
`config/mpv/mpv.conf` caps video at 1080p and requests VA-API for the Intel TV server. The dev-machine test log confirms AV1 1080p59.94 using VA-API. Hardware-decoding support still needs checking separately on the TV server. The bundled SponsorBlock script uses `curl` to request paid sponsor segments and skips them automatically. A missing request or segment leaves playback running. `ytdl-format` tries HLS streams first (YouTube's direct streams from the TV client began answering 403 on 2026-11-01), then the direct formats; AV1 is skipped unless nothing else exists.

mpv's bindings are in `config/mpv/input.conf` (Space pause, ←/→ 10 s, ↑/↓ 60 s, Esc/`q` stop, `n` next, `9`/`0` volume, `m` mute, `i` info). The Firefox player page mirrors them.

Environment: `COUCHTUBE_PLAYER=mpv` forces mpv for every video; `COUCHTUBE_MPV_CONFIG` sets the config directory.

## Server checks that only matter for mpv
Moved from [`server.md`](server.md): mpv appearing above the Firefox kiosk on KDE Wayland (KWin may keep Firefox in front; fix with a window rule: force fullscreen, keep above, accept focus), and `mpv --fullscreen --hwdec=vaapi …` playing 1080p60 smoothly with low CPU use. Hardware decoding by the browser's own player is a separate check.

## Reported playback warnings
One dev-machine session logged `client removed during hook handling`, `Packet corrupt`, `Invalid OBU length` and `Failed to parse temporal unit`, while successfully starting AV1 playback with hardware decoding. The cause is unconfirmed; a local-file playback check did not reproduce the hook warning. See [mpv’s hook handling](https://github.com/mpv-player/mpv/blob/v0.41.0/player/command.c#L181-L196) and [FFmpeg’s AV1 parser](https://github.com/FFmpeg/FFmpeg/blob/master/libavcodec/cbs_av1.c#L720-L727) for the messages’ origins.

If it happens again, note the video ID, whether the picture freezes or breaks up, whether audio continues, and whether you pressed Esc/q or playback stopped itself. Save the surrounding terminal output. `Exiting... (Quit)` records a quit request; the supplied excerpt does not establish why it was issued.

## Why mpv was the first player
mpv gives no ads, no embed restrictions and native keyboard controls. It stopped being the default on 2026-11-01 when its direct streams began failing with HTTP 403 (HLS is now tried first), and the embedded player inside the existing kiosk window avoids a second window that has to take focus.
