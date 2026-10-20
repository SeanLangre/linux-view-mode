# CouchTube

A fullscreen, keyboard-only YouTube app for a TV. It runs on an Arch Linux box connected to the TV over HDMI and is used from the couch with a wireless keyboard: browse with the arrow keys, press Enter, and the video plays fullscreen with no ads.

- **Backend:** Rust + Axum on `127.0.0.1:7878`. Runs `yt-dlp` for search and channel pages, and (from Phase 3) starts `mpv` to play videos.
- **Frontend:** React + TypeScript + Vite, shown in Firefox kiosk mode.

The full design and roadmap are in [`plan.md`](plan.md); the working checklist is [`TODO.md`](TODO.md).

## Status

| Works now | Coming next |
|---|---|
| Search, channel pages, keyboard navigation, back stack, recent searches | Playback in mpv (Phase 3), Google accounts (Phase 4), your own feeds (Phase 5), boot-to-TV setup (Phase 6) |

Pressing Enter on a video shows a "Playback arrives in Phase 3" message for now.

## Requirements

Arch Linux:

```bash
sudo pacman -S --needed rust nodejs npm yt-dlp mpv firefox
```

## Run it

```bash
# Build the frontend once (the backend serves frontend/dist)
cd frontend && npm install && npm run build && cd ..

# Start the backend
cargo run

# Open it like the TV will
firefox --kiosk http://127.0.0.1:7878
```

Firefox in kiosk mode has no window controls: close it with **Alt+F4**.

### Frontend development (live reload)

Keep `cargo run` running, then in another terminal:

```bash
cd frontend && npm run dev
```

and open `http://127.0.0.1:5173`. Vite forwards `/api` calls to the backend.

### Settings

| Environment variable | Default | Meaning |
|---|---|---|
| `COUCHTUBE_PORT` | `7878` | Port to listen on (always `127.0.0.1` only) |
| `COUCHTUBE_FRONTEND` | `frontend/dist` | Folder with the built frontend |
| `RUST_LOG` | `couchtube=info` | Log level, e.g. `couchtube=debug,tower_http=debug` |

## Keys

| Key | Action |
|---|---|
| ← ↑ → ↓ | Move between cards |
| Enter | Play / select |
| `/` | Search |
| `c` | Open the selected video's channel |
| Esc / Backspace | Back |
| Home | Go to the home screen |

In the search box, ↓ moves to the results and Esc goes back.

## API

| Method | Path | Returns |
|---|---|---|
| GET | `/api/health` | `{"status":"ok"}` |
| GET | `/api/search?q=<text>&limit=<1-50>` | List of videos |
| GET | `/api/channel/<UC… id>` | `{id, name, videos}` |

A video is `{id, title, channel, channel_id, duration, view_count, thumb}`; `duration` is `null` for live streams. Errors are `{"error": "..."}`.

## Tests and checks

```bash
cargo test && cargo clippy          # backend
cd frontend && npx vitest run       # navigation logic
npm run lint && npm run build       # lint + type-check
```

## Project layout

```
src/            Rust backend (main.rs, api.rs, ytdlp.rs)
frontend/       React app (App.tsx, nav.ts, api.ts, styles.css)
plan.md         Design and roadmap
TODO.md         Checklist
plan.chat.md    The original chat this project came from
```
