#!/usr/bin/env python3
"""Twitch browsing integration test against fake Twitch servers (no network, no real Twitch).

Covers both sources: the unofficial web API (the default, no registration) and the official Helix
API (when COUCHTUBE_TWITCH_CLIENT_ID and _SECRET are set).

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
OLD_WEB_ID = "oldwebclientid0123456789abc"
NEW_WEB_ID = "newwebclientid0123456789abc"
ART = "https://static-cdn.jtvnw.net/ttv-boxart/509658-{width}x{height}.jpg"
THUMB = "https://static-cdn.jtvnw.net/previews-ttv/live_user_shroud-{width}x{height}.jpg"


class Fake(BaseHTTPRequestHandler):
    log = []  # (kind, ...)
    # Official API state.
    token = "tokenone"
    next_token = "tokentwo"
    expire_next_call = False
    fail_next_call = False
    # Unofficial API state.
    web_id = NEW_WEB_ID
    total_categories = 60
    gql_fail = None  # "http" or "integrity" for the next call

    def reply(self, status, body, kind="application/json"):
        data = body.encode() if isinstance(body, str) else json.dumps(body).encode()
        self.send_response(status)
        self.send_header("Content-Type", kind)
        self.send_header("Content-Length", str(len(data)))
        self.end_headers()
        self.wfile.write(data)

    # ---- unofficial: POST /gql, GET / (the website) -----------------------------------------
    def gql(self, body):
        query, variables = body["query"], body.get("variables", {})
        client = self.headers.get("Client-ID")
        Fake.log.append(("gql", client, query, variables))
        if client != Fake.web_id:
            return self.reply(400, {"error": "Bad Request", "status": 400, "message": 'The "Client-ID" header is invalid.'})
        if Fake.gql_fail == "http":
            Fake.gql_fail = None
            return self.reply(500, {"message": "boom"})
        if Fake.gql_fail == "integrity":
            Fake.gql_fail = None
            return self.reply(200, {"errors": [{"message": "failed integrity check"}], "data": {"games": None}})
        first = variables.get("first", 0)
        if "games(" in query:
            count = min(first, Fake.total_categories)
            names = ["Just Chatting", "Grand Theft Auto V"] + [f"Game {i}" for i in range(2, 200)]
            edges = [{"cursor": "c", "node": {"id": str(1000 + i), "name": names[i], "boxArtURL": ART.replace("{width}", "285").replace("{height}", "380")}} for i in range(count)]
            return self.reply(200, {"data": {"games": {"edges": edges}}})
        if "searchFor" in query:
            return self.reply(200, {"data": {"searchFor": {"channels": {"edges": [
                {"item": {"login": "lirik", "displayName": "LIRIK", "stream": None, "profileImageURL": "",
                          "broadcastSettings": {"title": variables["q"], "game": None}}}, {"item": {}}]}}}})
        node = {"id": "1", "title": "Hello", "viewersCount": 4200, "createdAt": "2026-11-08T10:00:00Z",
                "previewImageURL": THUMB.replace("{width}", "440").replace("{height}", "248"),
                "broadcaster": {"login": "shroud", "displayName": "shroud"}, "game": {"id": variables.get("id", "32982"), "name": "Game"}}
        edges = [{"node": node}] * min(first, 3)
        if "game(id" in query:
            return self.reply(200, {"data": {"game": {"streams": {"edges": edges}}}})
        return self.reply(200, {"data": {"streams": {"edges": edges}}})

    # ---- official: POST /oauth2/token, GET /helix/... ---------------------------------------
    def do_POST(self):
        raw = self.rfile.read(int(self.headers.get("Content-Length", 0))).decode()
        if self.path == "/gql":
            return self.gql(json.loads(raw))
        form = urllib.parse.parse_qs(raw)
        ok = form.get("client_id") == [CLIENT_ID] and form.get("client_secret") == [CLIENT_SECRET] \
            and form.get("grant_type") == ["client_credentials"]
        Fake.log.append(("token", ok))
        if not ok:
            return self.reply(400, {"status": 400, "message": "invalid client"})
        issued, Fake.token = Fake.token, Fake.next_token
        self.reply(200, {"access_token": issued, "expires_in": 3600, "token_type": "bearer"})

    def do_GET(self):
        url = urllib.parse.urlparse(self.path)
        if url.path == "/":
            Fake.log.append(("site",))
            return self.reply(200, f'<html><script>clientId="{NEW_WEB_ID}"</script></html>', "text/html")
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


def gql_calls():
    return [e for e in Fake.log if e[0] == "gql"]


def main():
    repository = Path(__file__).resolve().parent.parent
    fake_port = free_port()
    fake = ThreadingHTTPServer(("127.0.0.1", fake_port), Fake)
    threading.Thread(target=fake.serve_forever, daemon=True).start()
    client = urllib.request.build_opener(urllib.request.ProxyHandler({}))
    base_url = f"http://127.0.0.1:{fake_port}"

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

        def post(port, path, body):
            req = urllib.request.Request(f"http://127.0.0.1:{port}/api/{path}", data=json.dumps(body).encode(),
                                         headers={"Content-Type": "application/json"}, method="POST")
            try:
                with client.open(req, timeout=10) as response:
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

        def bad_input(port):
            calls = len(Fake.log)
            for path in ("twitch/streams?game_id=12x", "twitch/streams?game_id=1;2", "twitch/categories?cursor=a%20b",
                         "twitch/search", "twitch/search?q=", "twitch/search?q=" + "x" * 101):
                status, body = request(port, path)
                assert status == 400, (path, status, body)
            assert len(Fake.log) == calls, "bad input must not reach Twitch"

        # 1. The default, unofficial source: no credentials, Twitch's web API.
        unofficial = {"COUCHTUBE_TWITCH_GQL_URL": f"{base_url}/gql", "COUCHTUBE_TWITCH_SITE_URL": f"{base_url}/",
                      "COUCHTUBE_TWITCH_WEB_CLIENT_ID": OLD_WEB_ID}
        port, process, logfile = start(unofficial)
        try:
            wait_up(port)
            assert request(port, "twitch/status") == (200, {"source": "unofficial"})

            # The website's client id changed: the backend reads the new one from the site and retries.
            Fake.total_categories = 60
            status, body = request(port, "twitch/categories")
            assert status == 200, (status, body)
            assert [e[0] for e in Fake.log].count("site") == 1
            assert [c[1] for c in gql_calls()] == [OLD_WEB_ID, NEW_WEB_ID], "old id rejected once, then the new one"
            assert len(body["items"]) == 24 and body["cursor"] == "n48"
            assert body["items"][0]["name"] == "Just Chatting" and body["items"][0]["box_art"].endswith("1000-285x380.jpg") is False
            assert body["items"][0]["box_art"].endswith("509658-285x380.jpg")
            assert gql_calls()[-1][3] == {"first": 24}

            # Cached; refresh bypasses it. Later requests reuse the new id without visiting the site again.
            before = len(gql_calls())
            request(port, "twitch/categories")
            assert len(gql_calls()) == before
            request(port, "twitch/categories?refresh=true")
            assert len(gql_calls()) == before + 1 and gql_calls()[-1][1] == NEW_WEB_ID
            assert [e[0] for e in Fake.log].count("site") == 1

            # "Load more" asks for a bigger first page (cursors would trip Twitch's integrity check).
            status, body = request(port, "twitch/categories?cursor=n48")
            assert status == 200 and len(body["items"]) == 48 and body["cursor"] == "n72" and gql_calls()[-1][3] == {"first": 48}
            status, body = request(port, "twitch/categories?cursor=n72")
            assert len(body["items"]) == 60 and body["cursor"] == "n96"
            status, body = request(port, "twitch/categories?cursor=n96")
            assert len(body["items"]) == 60 and body["cursor"] is None, "a short page is the last one"
            status, body = request(port, "twitch/categories?cursor=n9999")
            assert gql_calls()[-1][3] == {"first": 100}, "limited to what Twitch allows"

            # Streams: all of Twitch is capped at 30, a category goes through its id.
            status, body = request(port, "twitch/streams")
            assert status == 200 and body["items"][0]["login"] == "shroud" and body["items"][0]["viewers"] == 4200
            assert "game(id" not in gql_calls()[-1][2] and gql_calls()[-1][3] == {"first": 24}
            request(port, "twitch/streams?cursor=n48")
            assert gql_calls()[-1][3] == {"first": 30}
            status, body = request(port, "twitch/streams?game_id=509658")
            assert status == 200 and "game(id" in gql_calls()[-1][2] and gql_calls()[-1][3] == {"id": "509658", "first": 24}
            assert body["items"][0]["thumb"].endswith("live_user_shroud-440x248.jpg") and body["items"][0]["game_id"] == "509658"

            # Search text travels as a variable, so quotes and backslashes arrive untouched.
            text = 'retro "games" \\ x'
            status, body = request(port, "twitch/search?q=" + urllib.parse.quote(text))
            assert status == 200 and gql_calls()[-1][3] == {"q": text}, gql_calls()[-1]
            assert [c["login"] for c in body["items"]] == ["lirik"] and body["items"][0]["live"] is False and body["cursor"] is None
            assert body["items"][0]["title"] == text

            bad_input(port)

            # Watching: /twitch/play starts the player page; the page's reports drive the status.
            for bad in ({"login": ""}, {"login": "a b"}, {"login": "x/../y"}, {"login": "a" * 26}, {}, {"login": 5}):
                assert post(port, "twitch/play", bad)[0] == 400, bad
            assert request(port, "status")[1]["playing"] is False
            assert post(port, "twitch/play", {"login": "shroud"}) == (200, {"status": "starting"})
            status = request(port, "status")[1]
            session = status["session"]
            assert status["playing"] and status["id"] == "twitch:shroud" and status["page"] == f"/twitch.html?c=shroud&s={session}", status
            # A report for another session is ignored; the right one sets the position and pause state.
            assert post(port, "player/report", {"session": session + 1, "position": 9.0}) == (200, {"commands": []})
            assert request(port, "status")[1]["position"] is None
            assert post(port, "player/report", {"session": session, "position": 1.5, "paused": False, "title": "shroud"}) == (200, {"commands": []})
            status = request(port, "status")[1]
            assert (status["position"], status["paused"], status["title"]) == (1.5, False, "shroud")
            # The page says the channel is offline: no mpv, a clear message, the session is over.
            post(port, "player/report", {"session": session, "outcome": "error", "code": -4})
            status = request(port, "status")[1]
            assert status["playing"] is False and status["error"] == "shroud is offline right now.", status
            # Esc on the page is a normal stop and leaves no error behind.
            post(port, "twitch/play", {"login": "lirik"})
            session = request(port, "status")[1]["session"]
            post(port, "player/report", {"session": session, "outcome": "quit"})
            status = request(port, "status")[1]
            assert status["playing"] is False and status["error"] is None, status
            # /api/stop also ends a Twitch session, and a new play clears an old error.
            post(port, "twitch/play", {"login": "lirik"})
            assert request(port, "status")[1]["playing"] is True
            assert post(port, "stop", {})[0] == 200 and request(port, "status")[1]["playing"] is False

            # Failures are a short, safe message.
            for failure in ("http", "integrity"):
                Fake.gql_fail = failure
                status, body = request(port, f"twitch/categories?refresh=true&cursor=n{24 + 24 * (failure == 'http')}")
                assert status == 502 and body == {"error": "could not reach Twitch, try again"}, (failure, status, body)

        finally:
            process.terminate(); process.wait()

        if "--browser" in sys.argv:
            # The keyboard UI against the same fake Twitch, on a fresh backend (empty cache); needs Chromium and Node 22+.
            Fake.total_categories = 26
            port, process, _ = start(unofficial)
            try:
                wait_up(port)
                shots = Path(os.environ.get("TWITCH_SHOTS", root / "shots"))
                shots.mkdir(parents=True, exist_ok=True)
                subprocess.run(["node", str(repository / "tests/twitch_browser.mjs"), f"http://127.0.0.1:{port}", str(shots)], check=True, timeout=120)
            finally:
                process.terminate(); process.wait()

        # 2. The official source, once a Twitch application is registered: just set the two variables.
        official = {"COUCHTUBE_TWITCH_CLIENT_ID": CLIENT_ID, "COUCHTUBE_TWITCH_CLIENT_SECRET": CLIENT_SECRET,
                    "COUCHTUBE_TWITCH_API_URL": f"{base_url}/helix", "COUCHTUBE_TWITCH_TOKEN_URL": f"{base_url}/oauth2/token"}
        before_official = len(Fake.log)
        port, process, logfile = start(official)
        try:
            wait_up(port)
            assert request(port, "twitch/status") == (200, {"source": "official"})

            status, body = request(port, "twitch/categories")
            assert status == 200, (status, body)
            assert [c["name"] for c in body["items"]] == ["Just Chatting", "Grand Theft Auto V"]
            assert body["items"][0]["box_art"].endswith("509658-285x380.jpg") and body["cursor"] == "nextpage123"
            gets = [e for e in Fake.log if e[0] == "get"]
            assert gets[-1][1] == "/helix/games/top" and gets[-1][2]["first"] == "24"
            assert gets[-1][3] == CLIENT_ID and gets[-1][4] == "Bearer tokenone", gets[-1]
            assert not [e for e in Fake.log[before_official:] if e[0] == "gql"], "the official source does not use the web API"

            before = len([e for e in Fake.log if e[0] == "get"])
            request(port, "twitch/categories")
            assert len([e for e in Fake.log if e[0] == "get"]) == before
            request(port, "twitch/categories?refresh=true")
            assert len([e for e in Fake.log if e[0] == "get"]) == before + 1
            assert [e for e in Fake.log if e[0] == "token"] == [("token", True)]

            status, body = request(port, "twitch/categories?cursor=nextpage123")
            assert status == 200 and Fake.log[-1][2]["after"] == "nextpage123"
            status, body = request(port, "twitch/streams?game_id=509658")
            assert status == 200 and body["items"][0]["login"] == "shroud" and Fake.log[-1][2]["game_id"] == "509658"
            status, body = request(port, "twitch/search?q=" + urllib.parse.quote("retro & games"))
            assert status == 200 and body["items"][0]["title"] == "retro & games"

            bad_input(port)

            Fake.expire_next_call = True
            status, _ = request(port, "twitch/streams?game_id=777")
            assert status == 200 and Fake.log[-1][4] == "Bearer tokentwo", Fake.log[-1]
            Fake.fail_next_call = True
            status, body = request(port, "twitch/streams?game_id=888")
            assert status == 502 and body == {"error": "could not reach Twitch, try again"}, (status, body)
        finally:
            process.terminate(); process.wait()
        log = logfile.read_text()
        assert CLIENT_SECRET not in log and "tokenone" not in log and "tokentwo" not in log, "secret or token in the log"

        # 3. Only half of the pair set: fall back to the unofficial source and say so.
        port, process, logfile = start({"COUCHTUBE_TWITCH_CLIENT_ID": CLIENT_ID, **unofficial})
        try:
            wait_up(port)
            assert request(port, "twitch/status") == (200, {"source": "unofficial"})
        finally:
            process.terminate(); process.wait()
        assert "using the unofficial Twitch API" in logfile.read_text()
    fake.shutdown()
    print("twitch smoke test passed")


if __name__ == "__main__":
    main()
