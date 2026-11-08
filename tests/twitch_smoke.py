#!/usr/bin/env python3
"""Twitch browsing integration test against a fake Helix server (no network, no real Twitch).

Run `cargo build`, then `python3 tests/twitch_smoke.py [--browser]`.
"""
import json
import os
from pathlib import Path
import socket
import subprocess
import sys
import tempfile
import threading
import time
import urllib.error
import urllib.parse
import urllib.request
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

CLIENT_ID = "testclientid0123456789"
CLIENT_SECRET = "testclientsecret0123456789"
ART = "https://static-cdn.jtvnw.net/ttv-boxart/509658-{width}x{height}.jpg"
THUMB = "https://static-cdn.jtvnw.net/previews-ttv/live_user_shroud-{width}x{height}.jpg"


class Fake(BaseHTTPRequestHandler):
    log = []  # (kind, detail)
    token = "tokenone"
    next_token = "tokentwo"
    expire_next_call = False
    fail_next_call = False

    def reply(self, status, body):
        data = json.dumps(body).encode()
        self.send_response(status)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(data)))
        self.end_headers()
        self.wfile.write(data)

    def do_POST(self):
        form = urllib.parse.parse_qs(self.rfile.read(int(self.headers.get("Content-Length", 0))).decode())
        ok = form.get("client_id") == [CLIENT_ID] and form.get("client_secret") == [CLIENT_SECRET] \
            and form.get("grant_type") == ["client_credentials"]
        Fake.log.append(("token", ok))
        if not ok:
            return self.reply(400, {"status": 400, "message": "invalid client"})
        issued, Fake.token = Fake.token, Fake.next_token
        self.reply(200, {"access_token": issued, "expires_in": 3600, "token_type": "bearer"})

    def do_GET(self):
        url = urllib.parse.urlparse(self.path)
        query = urllib.parse.parse_qs(url.query)
        Fake.log.append(("get", url.path, {k: v[0] for k, v in query.items()}, self.headers.get("Client-Id"), self.headers.get("Authorization")))
        if Fake.fail_next_call:
            Fake.fail_next_call = False
            return self.reply(500, {"message": "boom"})
        if Fake.expire_next_call:
            Fake.expire_next_call = False
            return self.reply(401, {"status": 401, "message": "Invalid OAuth token"})
        if url.path == "/helix/games/top":
            return self.reply(200, {"data": [{"id": "509658", "name": "Just Chatting", "box_art_url": ART},
                                               {"id": "32982", "name": "Grand Theft Auto V", "box_art_url": ART}],
                                    "pagination": {"cursor": "nextpage123"}})
        if url.path == "/helix/streams":
            return self.reply(200, {"data": [{"user_login": "shroud", "user_name": "shroud", "title": "Hello",
                                               "game_id": query.get("game_id", ["0"])[0], "game_name": "Game", "viewer_count": 4200,
                                               "started_at": "2026-11-08T10:00:00Z", "thumbnail_url": THUMB}], "pagination": {}})
        if url.path == "/helix/search/channels":
            return self.reply(200, {"data": [{"broadcaster_login": "lirik", "display_name": "LIRIK", "is_live": False,
                                               "title": query["query"][0], "game_name": "", "thumbnail_url": ""}], "pagination": {}})
        self.reply(404, {"message": "no"})

    def log_message(self, *args):
        pass


def free_port():
    with socket.socket() as probe:
        probe.bind(("127.0.0.1", 0))
        return probe.getsockname()[1]


def main():
    repository = Path(__file__).resolve().parent.parent
    fake_port = free_port()
    fake = ThreadingHTTPServer(("127.0.0.1", fake_port), Fake)
    threading.Thread(target=fake.serve_forever, daemon=True).start()
    client = urllib.request.build_opener(urllib.request.ProxyHandler({}))

    with tempfile.TemporaryDirectory(prefix="ct-twitch-") as temporary:
        root = Path(temporary)

        def start(env_extra):
            port = free_port()
            env = {**os.environ, "RUST_LOG": "couchtube=debug", "COUCHTUBE_DATA_DIR": str(root / f"data{port}"),
                   "XDG_RUNTIME_DIR": str(root), "COUCHTUBE_PORT": str(port), "COUCHTUBE_PAIR_PORT": str(free_port()),
                   "COUCHTUBE_REMOTE_PORT": str(free_port()), "NO_PROXY": "127.0.0.1", "no_proxy": "127.0.0.1", **env_extra}
            for key in ("COUCHTUBE_TWITCH_CLIENT_ID", "COUCHTUBE_TWITCH_CLIENT_SECRET"):
                if key not in env_extra:
                    env.pop(key, None)
            log = open(root / f"server{port}.log", "w")
            process = subprocess.Popen([str(repository / "target/debug/couchtube")], env=env, stdout=log, stderr=log, cwd=repository)
            return port, process, root / f"server{port}.log"

        def request(port, path):
            try:
                with client.open(f"http://127.0.0.1:{port}/api/{path}", timeout=10) as response:
                    return response.status, json.load(response)
            except urllib.error.HTTPError as error:
                return error.code, json.load(error)

        def wait_up(port):
            until = time.monotonic() + 10
            while time.monotonic() < until:
                try:
                    if request(port, "health")[0] == 200:
                        return
                except OSError:
                    time.sleep(0.05)
            raise AssertionError("backend did not start")

        # 1. Without credentials Twitch mode reports it instead of failing.
        port, process, _ = start({})
        try:
            wait_up(port)
            assert request(port, "twitch/status") == (200, {"configured": False})
            status, body = request(port, "twitch/categories")
            assert status == 503 and "not set up" in body["error"], (status, body)
        finally:
            process.terminate(); process.wait()

        # 2. With credentials, against the fake Helix.
        extra = {"COUCHTUBE_TWITCH_CLIENT_ID": CLIENT_ID, "COUCHTUBE_TWITCH_CLIENT_SECRET": CLIENT_SECRET,
                 "COUCHTUBE_TWITCH_API_URL": f"http://127.0.0.1:{fake_port}/helix",
                 "COUCHTUBE_TWITCH_TOKEN_URL": f"http://127.0.0.1:{fake_port}/oauth2/token"}
        port, process, logfile = start(extra)
        try:
            wait_up(port)
            assert request(port, "twitch/status") == (200, {"configured": True})

            status, body = request(port, "twitch/categories")
            assert status == 200, (status, body)
            assert [c["name"] for c in body["items"]] == ["Just Chatting", "Grand Theft Auto V"]
            assert body["items"][0]["box_art"].endswith("509658-285x380.jpg") and body["cursor"] == "nextpage123"
            gets = [e for e in Fake.log if e[0] == "get"]
            assert gets[-1][1] == "/helix/games/top" and gets[-1][2]["first"] == "24"
            assert gets[-1][3] == CLIENT_ID and gets[-1][4] == "Bearer tokenone", gets[-1]

            # Cached: the same request does not reach Twitch again; refresh=true does.
            before = len([e for e in Fake.log if e[0] == "get"])
            request(port, "twitch/categories")
            assert len([e for e in Fake.log if e[0] == "get"]) == before
            request(port, "twitch/categories?refresh=true")
            assert len([e for e in Fake.log if e[0] == "get"]) == before + 1
            # The token was fetched once and reused.
            assert [e for e in Fake.log if e[0] == "token"] == [("token", True)]

            status, body = request(port, "twitch/categories?cursor=nextpage123")
            assert status == 200 and Fake.log[-1][2]["after"] == "nextpage123"

            status, body = request(port, "twitch/streams?game_id=509658")
            assert status == 200 and body["items"][0]["login"] == "shroud" and body["items"][0]["viewers"] == 4200
            assert Fake.log[-1][2]["game_id"] == "509658"
            assert body["items"][0]["thumb"].endswith("live_user_shroud-440x248.jpg")
            assert request(port, "twitch/streams")[0] == 200 and "game_id" not in Fake.log[-1][2]

            status, body = request(port, "twitch/search?q=" + urllib.parse.quote("retro & games"))
            assert status == 200 and body["items"][0]["login"] == "lirik" and body["items"][0]["title"] == "retro & games"

            # Bad input never reaches Twitch.
            calls = len(Fake.log)
            for path in ("twitch/streams?game_id=12x", "twitch/streams?game_id=1;2", "twitch/categories?cursor=a%20b",
                         "twitch/search", "twitch/search?q=", "twitch/search?q=" + "x" * 101):
                status, body = request(port, path)
                assert status == 400, (path, status, body)
            assert len(Fake.log) == calls

            # An expired token is replaced and the request repeated.
            Fake.expire_next_call = True
            status, _ = request(port, "twitch/streams?game_id=777")
            assert status == 200 and Fake.log[-1][4] == "Bearer tokentwo", Fake.log[-1]

            # An upstream failure is a short, safe message.
            Fake.fail_next_call = True
            status, body = request(port, "twitch/streams?game_id=888")
            assert status == 502 and body == {"error": "could not reach Twitch, try again"}, (status, body)
            if "--browser" in sys.argv:
                # The keyboard UI against the same fake Twitch; needs Chromium and Node 22+.
                shots = Path(os.environ.get("TWITCH_SHOTS", root / "shots"))
                shots.mkdir(parents=True, exist_ok=True)
                subprocess.run(["node", str(repository / "tests/twitch_browser.mjs"), f"http://127.0.0.1:{port}", str(shots)], check=True, timeout=120)
        finally:
            process.terminate(); process.wait()
        log = logfile.read_text()
        assert CLIENT_SECRET not in log and "tokenone" not in log and "tokentwo" not in log, "secret or token in the log"
    fake.shutdown()
    print("twitch smoke test passed")


if __name__ == "__main__":
    main()
