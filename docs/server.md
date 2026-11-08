# TV server: machines, setup and checks

## Machines

| | Dev machine | TV server |
|---|---|---|
| Role | Write, build, test | Runs CouchTube on the TV |
| Hardware | | **Acer Revo RN96** mini PC |
| OS | Arch Linux | **Windows right now** → to be replaced by Arch Linux (below) |
| Desktop | KDE Plasma (Wayland) | **KDE Plasma 6, Wayland** |
| Display | Monitor | **TV is the only screen** |
| GPU | NVIDIA RTX 5060 Ti | **Intel Iris Xe** (11th gen Tiger Lake, PCI `8086:9A49`): hardware decoding of **AV1**, VP9, HEVC, H.264 |
| Tools | Rust 1.99, Node 22, Firefox, mpv, yt-dlp, SQLite | To install (see Phase 0) |

Decided: server GPU is Intel Iris Xe, so AV1 is allowed. The code reaches the server via this git repo, pulled on the server.

## Workflow
- Develop and run everything locally on the dev machine. The full app works here too, just on the monitor instead of the TV.
- Push the code to a git repo (a private GitHub repo, for example). A USB stick also works.
- **On the server**, at the TV with the wireless keyboard: `git pull && ./deploy/install.sh`. The script builds the backend and frontend on the server, installs the files, the systemd user units and the Firefox policy, then restarts the services. No SSH needed.
- The server needs: `firefox mpv yt-dlp intel-media-driver libva-utils git` to run it, plus `rust nodejs npm` to build it. The first build on the RN96 takes a few minutes; later ones are quick.
- Later option: switch to an Arch `PKGBUILD` so updates become `makepkg -si` and pacman tracks the installed files.

## Server setup: Windows → Arch + KDE Plasma (on the RN96)
- Back up anything you want to keep from the Windows install. The disk will be wiped.
- Write the Arch ISO to a USB stick. In the RN96 firmware (BIOS), **turn off Secure Boot** and boot from USB.
- Run `archinstall`:
  - Profile: **Desktop → KDE Plasma**, greeter **SDDM**
  - Graphics driver: **Intel (open-source)**
  - Audio: **PipeWire**
  - Network: **NetworkManager**
  - Create a normal user (CouchTube runs as this user).
- After the first boot: Plasma 6 starts on Wayland by default. Set the TV's resolution and scaling in System Settings → Display, and check sound goes out over HDMI (System Settings → Sound).
- **Done when:** the RN96 boots into Plasma on the TV, with sound over HDMI.

## Phase 0: server checks
- Install: `sudo pacman -S --needed firefox mpv yt-dlp intel-media-driver libva-utils git rust nodejs npm`. Run `vainfo` and check it lists `VAProfileAV1Profile0`, `VAProfileVP9Profile0` and `VAProfileH264High` with `VAEntrypointVLD`. Confirm the session is Wayland (`echo $XDG_SESSION_TYPE`).
- *(legacy mpv check)* `mpv --fullscreen --hwdec=vaapi --ytdl-format='bv*[height<=1080]+ba/b[height<=1080]' https://www.youtube.com/watch?v=aqz-KE-bpKQ` plays smoothly (press `i` in mpv to confirm the codec and that hardware decoding is on).
- *(legacy mpv check)* With `firefox --kiosk` open, start mpv: does it show on top with focus, and does focus come back afterwards? Add a KWin rule if needed.
- Create a test profile: `firefox --new-instance --profile ~/ct-test https://accounts.google.com` (create the folder first), sign in **one** account, close it, then run `yt-dlp --cookies-from-browser firefox:$HOME/ct-test --flat-playlist -J --playlist-end 5 :ytrec`. Do you get your recommendations?
- `yt-dlp --cookies-from-browser firefox:$HOME/ct-test --mark-watched --simulate <url>`: does the video appear in your YouTube history?
- Run the `:ytrec` command again after 2–3 days, without opening Firefox, to see whether the login lasts.
- **Done when:** everything works, or each problem has a known fix.

## Deploy and appliance mode (Phase 6)
Written (`deploy/`, `POST /api/system/exit`, Home menu item); not yet run on the server. Auto-login and power settings stay manual and are in the [README](../README.md#running-as-a-tv-appliance).
- `install.sh` (run on the server): `cargo build --release`, `npm ci && npm run build`, install to `~/.local/share/couchtube/`, copy the systemd user units, `sudo` copy `policies.json` to `/etc/firefox/policies/`, `systemctl --user daemon-reload && systemctl --user restart couchtube couchtube-kiosk`.
- `couchtube.service` (backend, `Restart=on-failure`).
- `couchtube-kiosk.service`: `firefox --kiosk --new-instance --profile ~/.local/share/couchtube/kiosk-firefox http://127.0.0.1:7878`, `Restart=always`.
- `couchtube-refresh.timer`: daily re-check of each profile's sign-in (`deploy/couchtube-refresh`, through the API).
- KDE: auto-login into Plasma (Wayland); turn off screen lock, screen blanking and sleep.
- "Exit to desktop" item, and a KDE global shortcut to start the kiosk again.
- **Done when:** the TV boots into "Who's watching?", and killing Firefox brings it back.

## Risks to check first

mpv-specific risks (KWin stacking, mpv hardware decoding) moved to [`legacy-mpv.md`](legacy-mpv.md).

1. **Hardware decoding on the RN96**: `vainfo` must list AV1, VP9 and H.264; check that 1080p60 plays smoothly in Firefox with low CPU use. Keep the React UI light anyway (no heavy blur effects) so it stays smooth on integrated graphics.
2. **Logins expiring**: how long do the cookies keep working, and does the daily refresh fix it?
3. **Account flagging**: start with one account and few requests.
4. **yt-dlp breaking**: YouTube changes often break yt-dlp. Keep it updated, show a clear error in the UI, and keep the YouTube TV fallback ready.
5. **yt-dlp speed**: requests take about 2–5 s (a bit slower on the RN96). Show a loading state, use cached results first, prefetch the Home rows when a profile is picked.
