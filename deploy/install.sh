#!/bin/bash
# Builds CouchTube and installs it for the current user, with systemd user units and the Firefox
# extension policy. Run at the TV: git pull && ./deploy/install.sh
set -euo pipefail
cd "$(dirname "$0")/.."

for tool in cargo npm firefox mpv yt-dlp xdotool curl; do
    command -v "$tool" >/dev/null || { echo "install: '$tool' is required but not installed" >&2; exit 1; }
done

APP="$HOME/.local/share/couchtube/app"
UNITS="$HOME/.config/systemd/user"

(cd frontend && npm ci && npm run build)
cargo build --release

rm -rf "$APP.new"
mkdir -p "$APP.new/bin" "$APP.new/config"
install -m755 target/release/couchtube "$APP.new/bin/couchtube"
install -m755 deploy/couchtube-refresh "$APP.new/bin/couchtube-refresh"
cp -r frontend/dist "$APP.new/frontend"
cp -r config/mpv "$APP.new/config/mpv"
cp -r config/firefox "$APP.new/config/firefox"

# Stop the services while swapping files, so the running binary isn't replaced under them.
systemctl --user stop couchtube-kiosk couchtube 2>/dev/null || true
rm -rf "$APP.old"
[ -d "$APP" ] && mv "$APP" "$APP.old"
mv "$APP.new" "$APP"
rm -rf "$APP.old"

mkdir -p "$UNITS" "$HOME/.local/share/applications"
install -m644 deploy/couchtube.service deploy/couchtube-kiosk.service \
    deploy/couchtube-refresh.service deploy/couchtube-refresh.timer "$UNITS/"
install -m644 deploy/couchtube-kiosk.desktop "$HOME/.local/share/applications/"

# Firefox extension policy (uBlock Origin + SponsorBlock). Never overwrite one that differs.
POLICY=/etc/firefox/policies/policies.json
if [ -e "$POLICY" ] && ! cmp -s config/firefox/policies.json "$POLICY"; then
    echo "install: $POLICY already exists and differs; merge config/firefox/policies.json into it by hand" >&2
else
    sudo install -D -m644 config/firefox/policies.json "$POLICY"
fi

systemctl --user daemon-reload
systemctl --user enable couchtube.service couchtube-kiosk.service couchtube-refresh.timer
systemctl --user restart couchtube couchtube-kiosk
echo "CouchTube installed. For boot-to-TV set KDE auto-login and turn off screen lock, blanking and sleep (see README)."
