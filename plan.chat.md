i have a home server that is artch linux. i want to create an app that is a kind of view mode so i can easily watch youtube from the tv with a wireless keyboard.
That sounds like a great project for your Arch Linux home server. I'd build it as a TV-friendly YouTube interface — something between a media center and a simple kiosk browser, designed to be used from the couch with a wireless keyboard.

The key is to make it feel like an appliance rather than a desktop computer: launch it, see YouTube, search for videos, and watch them without dealing with windows, browser tabs, or tiny controls.

What I'd build
Top 20 films et séries polonais à voir en streaming en 2026

1. A TV-friendly home screen

Large video thumbnails, readable text, a dark theme, and sections for subscriptions, recommended videos, and watch history.

SAMSUNG 55" UHD 4K Smart tv UA55NU7100  Condition: 8/10 Delivery Add $50 $450, TV & Home Appliances, TV & Entertainment, TV on Carousell

2. Keyboard-first navigation

Use arrow keys to move around, Enter to select, Esc to go back, and / to jump to search. No mouse required.

YouTube na TV facilita acesso a melhores momentos de um vídeo - Canaltech

3. A proper viewing mode

Fullscreen playback, easy pause and seek controls, and a quick way to return to search or browse more videos.

The technology I'd recommend
For an Arch Linux server, I'd start with a small web app:

Frontend: React or plain HTML/CSS/JavaScript, designed for a 10-foot TV interface.

Backend: Node.js, if needed, for search integration and settings.

Playback: YouTube's embedded player or a normal browser playing YouTube directly.

Launch: A fullscreen Chromium kiosk if the server is connected to the TV, or a web page opened on the TV if it has a browser.

One important detail: YouTube search, account subscriptions, and playback have different integration requirements. We can keep the first version simple and add account integration later.

First, one question
How is your Arch Linux server connected to your TV?


The server is connected directly to the TV via HDMI.

The TV is separate; I'd access the app over the home network.

I have another device connected to the TV, such as a mini PC or Raspberry Pi.
What matters most to you?


A custom interface made specifically for TV and keyboard.

The simplest reliable way to watch YouTube.

A private, self-hosted setup with minimal tracking.
Once I know that, I can give you the actual architecture and start building the first working version.

My Arch Linux home server setup: Server connected directly to the TV via HDMI. My priority: custom TV interface. Recommend the architecture and help me implement it.
I'd build this as a dedicated YouTube TV shell for Arch Linux: a custom interface that starts automatically when the machine boots, fills the TV screen, and is navigable entirely with your wireless keyboard.

Since the server is connected directly to the TV via HDMI, you don't need to serve the interface to another device. The app can run locally, giving you direct control over the display, keyboard, and fullscreen playback.

1. Recommended architecture
Wireless keyboard

Arrows · Enter · Esc · Search · Media keys

Custom TV interface

React + TypeScript + CSS

Large thumbnails · Search · Browse · Queue

Chromium in kiosk mode

Fullscreen local web app

YouTube playback

Official embedded player or YouTube website

The stack
Component

Choice

Why

Operating system

Arch Linux

Your existing system

Display session

Existing desktop, or a lightweight Wayland session

Runs the app directly on the TV

App

React + TypeScript + Vite

Fast to develop and easy to customize

Browser

Chromium

Fullscreen kiosk and browser-based playback

Search

YouTube Data API or another permitted search integration

Find videos and display results

Autostart

systemd user service or desktop autostart

Launch into TV mode automatically

My recommendation: start with a local React app and Chromium, without a backend initially. We can add a small backend later if needed. This keeps the first version simple and avoids maintaining unnecessary services.

One playback consideration: use YouTube's supported player for the first version. Embedded playback can have restrictions, and some videos require watching on YouTube itself. We should design a graceful fallback for those cases.

The setup above is supported by Arch's Chromium packaging and systemd user services, and YouTube provides an official player API for playback control. 

ArchWiki
+2

2. Build the first working version
Let's first get the interface running before configuring boot automation. This first version will have a home screen, a search field, a grid of video results, and a player view. Search will use YouTube's public website, avoiding the need for an API key in the initial prototype.

Step A — Install the tools
In a terminal on the Arch machine:

sudo pacman -Syu
sudo pacman -S --needed nodejs npm chromium

Create the project:

mkdir -p ~/tv-youtube
cd ~/tv-youtube
npm create vite@latest . -- --template react-ts
npm install
npm run dev -- --host 127.0.0.1

Vite will print a local address, usually http://127.0.0.1:5173. Open that address in Chromium on the TV-connected machine to check that the development server works.

If you don't currently have a graphical desktop or Wayland session on the server, stop here before trying to launch Chromium: we'll need to set up the display session first.

Step B — Design the TV interface
Here's the first-version layout I'd aim for:

CouchTube
HOME · SEARCH · LIBRARY

What do you want to watch?
Search YouTube or pick up where you left off.

Search videos…

ENTER

Quick picks
4K Ultra HD Nature Reflection: Tranquil Mountain Lake at Sunset | 3840x2160 | Wallpaper Abyss

Nature & travel

Essential Tips for Producing Electronic Music Genres Like a Pro

Music

Die 7 Besten Node.js Backend Frameworks für Entwickler in 2025 - DEVjobs.de

Technology

Illustrative layout · Search and playback will be wired up in the app.

Step C — Replace src/App.tsx
This is a functional starter app with large selectable video cards, keyboard navigation, search, and an embedded player. The example cards are placeholders for known YouTube video IDs; the search button opens YouTube's search results directly for now.


import { useEffect, useState } from "react";
import "./App.css";

type Video = {
  id: string;
  title: string;
  channel: string;
};

const videos: Video[] = [
  {
    id: "jfKfPfyJRdk",
    title: "lofi hip hop radio",
    channel: "Lofi Girl",
  },
  {
    id: "M7lc1UVf-VE",
    title: "YouTube player demo",
    channel: "YouTube Developers",
  },
  {
    id: "aqz-KE-bpKQ",
    title: "Big Buck Bunny",
    channel: "Blender",
  },
];

export default function App() {
  const [query, setQuery] = useState("");
  const [selected, setSelected] = useState(0);
  const [playing, setPlaying] = useState<Video | null>(null);

  useEffect(() => {
    function onKeyDown(event: KeyboardEvent) {
      if (event.key === "Escape" && playing) {
        setPlaying(null);
        return;
      }

      if (playing) return;

      if (event.key === "ArrowRight") {
        setSelected((n) => Math.min(n + 1, videos.length - 1));
      }
      if (event.key === "ArrowLeft") {
        setSelected((n) => Math.max(n - 1, 0));
      }
      if (event.key === "Enter") {
        const el = document.activeElement;
        if (el?.tagName === "INPUT") return;
        setPlaying(videos[selected]);
      }
      if (event.key === "/") {
        event.preventDefault();
        document.querySelector<HTMLInputElement>("#search")?.focus();
      }
    }

    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  }, [playing, selected]);

  function search(event: React.FormEvent) {
    event.preventDefault();
    const q = query.trim();
    if (q) {
      window.location.href =
        "https://www.youtube.com/results?search_query=" +
        encodeURIComponent(q);
    }
  }

  return (
    <main className="app">
      <header>
        <div className="brand"><span>▶</span> CouchTube</div>
        <div className="hint">ARROWS TO BROWSE · ENTER TO PLAY</div>
      </header>

      {playing ? (
        <section className="player">
          <button className="back" onClick={() => setPlaying(null)}>
            ← Back <small>(Esc)</small>
          </button>
          <h2>{playing.title}</h2>
          <iframe
            key={playing.id}
            src={`https://www.youtube-nocookie.com/embed/${playing.id}?autoplay=1`}
            title={playing.title}
            allow="autoplay; encrypted-media; picture-in-picture; fullscreen"
            allowFullScreen
          />
          <p>{playing.channel}</p>
        </section>
      ) : (
        <>
          <section className="hero">
            <p className="eyebrow">YOUR LIVING ROOM, YOUR RULES</p>
            <h1>What do you want<br />to watch?</h1>
            <p className="muted">
              Find something good. Sit back. Enjoy.
            </p>
            <form onSubmit={search}>
              <input
                id="search"
                value={query}
                onChange={(e) => setQuery(e.target.value)}
                placeholder="Search YouTube…"
                aria-label="Search YouTube"
              />
              <button type="submit">Search ↗</button>
            </form>
          </section>

          <section>
            <h2>Quick picks</h2>
            <div className="videos">
              {videos.map((video, index) => (
                <button
                  className={`video ${selected === index ? "selected" : ""}`}
                  key={video.id}
                  onMouseEnter={() => setSelected(index)}
                  onFocus={() => setSelected(index)}
                  onClick={() => setPlaying(video)}
                >
                  <img
                    src={`https://i.ytimg.com/vi/${video.id}/hqdefault.jpg`}
                    alt=""
                  />
                  <span className="video-title">{video.title}</span>
                  <span className="channel">{video.channel}</span>
                </button>
              ))}
            </div>
          </section>

          <footer>
            <span>↑ ↓ ← → Navigate</span>
            <span>ENTER Play</span>
            <span>/ Search</span>
          </footer>
        </>
      )}
    </main>
  );
}

Step D — Replace src/App.css

:root {
  font-family: Inter, system-ui, sans-serif;
  color: #f7f7fb;
  background: #101116;
  font-synthesis: none;
  text-rendering: optimizeLegibility;
}

* { box-sizing: border-box; }

body {
  margin: 0;
  min-width: 720px;
  min-height: 100vh;
}

button, input { font: inherit; }

button { cursor: pointer; }

.app {
  max-width: 1600px;
  margin: 0 auto;
  padding: 32px 5vw;
}

header, footer {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 24px;
}

.brand {
  font-size: 27px;
  font-weight: 800;
  letter-spacing: -1px;
}

.brand span { color: #ff4545; }

.hint, footer {
  color: #999ba8;
  font-size: 13px;
  letter-spacing: 1px;
}

.hero { padding: 55px 0 35px; }

.eyebrow {
  color: #ff6b6b;
  font-size: 13px;
  font-weight: 700;
  letter-spacing: 2px;
}

h1 {
  margin: 16px 0;
  font-size: clamp(42px, 5vw, 72px);
  line-height: 1.04;
  letter-spacing: -3px;
}

h2 { font-size: 25px; }

.muted, .channel { color: #a3a5b1; }

form {
  display: flex;
  gap: 12px;
  margin-top: 28px;
  max-width: 760px;
}

input {
  flex: 1;
  min-width: 0;
  padding: 18px 20px;
  color: white;
  background: #24252d;
  border: 2px solid transparent;
  border-radius: 12px;
  font-size: 18px;
}

input:focus {
  outline: 3px solid #f7f7fb;
  outline-offset: 2px;
}

form button, .back {
  padding: 16px 24px;
  border: 0;
  border-radius: 12px;
  color: white;
  background: #e53945;
  font-weight: 700;
}

.videos {
  display: grid;
  grid-template-columns: repeat(3, minmax(0, 1fr));
  gap: 24px;
}

.video {
  min-width: 0;
  padding: 10px;
  text-align: left;
  color: inherit;
  background: #1a1b22;
  border: 3px solid transparent;
  border-radius: 15px;
}

.video.selected, .video:focus-visible {
  border-color: white;
  background: #292a34;
  outline: none;
  transform: scale(1.02);
}

.video img {
  display: block;
  width: 100%;
  aspect-ratio: 16 / 9;
  object-fit: cover;
  border-radius: 8px;
}

.video-title, .channel {
  display: block;
  margin: 12px 4px 5px;
  font-size: 17px;
  font-weight: 700;
}

.channel {
  font-size: 14px;
  font-weight: 400;
}

footer {
  justify-content: flex-start;
  flex-wrap: wrap;
  gap: 30px;
  padding: 36px 0 10px;
}

.player { padding-top: 32px; }

.back { margin-bottom: 10px; }
.back small { opacity: 0.8; }

.player iframe {
  display: block;
  width: 100%;
  aspect-ratio: 16 / 9;
  border: 0;
  border-radius: 12px;
  background: black;
}

@media (min-width: 1400px) {
  .app { padding-top: 42px; }
  .hero { padding-top: 65px; }
  .videos { gap: 30px; }
}

Run the app with:

npm run dev -- --host 127.0.0.1

Open http://127.0.0.1:5173 in Chromium. Select a card with the arrow keys and press Enter to play it. Press Esc to return.

3. Run it in fullscreen TV mode
Once the app works, build it for production:

cd ~/tv-youtube
npm run build
npm run preview -- --host 127.0.0.1

Open the preview URL shown by Vite, usually http://127.0.0.1:4173, in Chromium kiosk mode:

chromium \
  --user-data-dir="$HOME/.config/couchtube-browser" \
  --kiosk \
  --no-first-run \
  http://127.0.0.1:4173

This assumes you already have a graphical session running. Chromium's kiosk and app-style launch options are documented for Arch Linux. 

ArchWiki
+1

For now, keep the preview process running in the terminal. We'll configure a persistent service and automatic startup after we've confirmed playback and keyboard navigation work.

4. What we should improve next
The starter app establishes the visual style and playback flow, but it isn't yet the complete custom YouTube experience. I'd implement the next pieces in this order:

Real search inside the interface

Show actual search results as large cards instead of navigating away to YouTube. We can use the YouTube Data API with an API key, or evaluate a suitable alternative.

Complete keyboard navigation

Support Up/Down as well as Left/Right, visible focus, search shortcuts, a queue, and a reliable return-to-home action.

Subscriptions and watch history

Decide how to handle Google sign-in and personal recommendations. The official embedded player supports playback control through its IFrame API, but not every YouTube feature is available through an embed. 

Google for Developers
+1

Appliance-style startup

Start the interface when the graphical session launches, restore it if Chromium crashes, and make it easy to exit to the normal desktop.

One important question before we configure startup
Does your Arch Linux server already have a graphical desktop installed?

Which display setup are you using?


KDE Plasma

GNOME

Another desktop or window manager

No desktop — terminal only
Do you know your graphics hardware?


Intel integrated graphics

AMD graphics

NVIDIA graphics

Not sure
That determines the cleanest way to turn CouchTube into something you can switch on and use from the sofa without needing a mouse or a terminal.

should we make it in rust?
Yes — Rust could be a great choice for this project. I'd just use it for the parts where it provides a real benefit, rather than rewriting everything in Rust for the sake of it.

For your Arch Linux home server connected directly to your TV, I'd recommend Rust + a web-based frontend + Chromium for YouTube playback.

Three architectures worth considering
1. Rust backend + React frontend

My recommendation
Rust handles configuration, local storage, search integrations and app logic. React handles the TV interface, while Chromium handles playback. Easy to develop and customize.

2. Tauri + Rust + web frontend

A desktop application with a Rust core and embedded web UI. More application-like, with a native window and direct access to OS features. However, embedded WebView playback and compatibility need testing on Linux.

3. All Rust with a native UI

Something like Slint or Iced. Interesting if you want to learn Rust deeply, but integrating a full YouTube browsing and playback experience becomes significantly more complicated.

The stack I'd choose
Layer

Technology

Core application

Rust

HTTP server / API

Axum

Frontend

React + TypeScript

Styling

Custom CSS, designed for TV

Video playback

Chromium + YouTube IFrame Player API

Persistence

SQLite, via Rust

Deployment

systemd on Arch Linux

The important distinction is that Rust doesn't need to decode or stream YouTube video itself. Let YouTube and Chromium handle that. Rust gives us a robust core without having to reinvent browser functionality.

How the application would fit together
TV interface

React · keyboard navigation · thumbnails

Rust application

Axum · settings · history · search adapter

SQLite

Local settings

YouTube

Search and playback

Let's start with a Rust project
I'd begin with a small Rust HTTP server that serves the TV interface, then add the UI and playback features incrementally.

Install the tools on Arch:

sudo pacman -S --needed rustup nodejs npm chromium
rustup default stable

Create the project:

cargo new couchtube
cd couchtube

cargo add axum tokio --features tokio/full
cargo add tower-http --features fs
cargo add serde --features derive
cargo add serde_json

We can then add a React frontend in a frontend/ directory and have Rust serve the production build. During development, Vite can run separately and proxy API requests to Rust.

One design decision before we begin: do you want this to be a full desktop application, or a dedicated TV interface that launches in fullscreen and feels like a console?

My preference is the second: a fullscreen, keyboard-driven CouchTube experience, with Rust providing the core and Chromium providing reliable YouTube playback.

