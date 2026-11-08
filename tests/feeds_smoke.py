#!/usr/bin/env python3
"""Account feeds/cache, queue, resume and watched-sync integration test.

Videos play in the CouchTube window (Firefox), so there is no player process: the test plays the
part of the player page (`frontend/public/watch.html`) by sending the same reports it sends to
`POST /api/player/report` (position, pause state, "eof", "next"). yt-dlp is a fake; nothing
touches YouTube. Run `cargo build`, then `python3 tests/feeds_smoke.py`.
"""
import json
import os
from pathlib import Path
import socket
import sqlite3
import subprocess
import tempfile
import time
import urllib.error
import urllib.request

VIDEO = "aqz-KE-bpKQ"
OTHER = "rFZHOHl-L8A"


def main():
    repository = Path(__file__).resolve().parent.parent
    with tempfile.TemporaryDirectory(prefix="ct-feeds-") as temporary:
        root = Path(temporary)
        (root / "bin").mkdir()
        extractor = root / "bin/yt-dlp"
        # The fixture video is 10 minutes long: anything of 3 minutes or less is dropped as a Short.
        extractor.write_text('''#!/usr/bin/env python3
import json,os,pathlib,sys
root=pathlib.Path(os.environ['CT_TEST_ROOT']); args=sys.argv[1:]
with (root/'calls.jsonl').open('a') as log: log.write(json.dumps({'tool':'yt-dlp','args':args})+'\\n')
if (root/'fail-feed').exists() and '/feed/' in args[-1]: sys.exit(1)
print(json.dumps({'entries':[{'id':'aqz-KE-bpKQ','title':'Account fixture','duration':600,'channel':'Fixture'}]}))
''')
        extractor.chmod(0o700)
        with socket.socket() as probe:
            probe.bind(("127.0.0.1", 0))
            port = probe.getsockname()[1]
        environment = {**os.environ, "PATH": f"{root / 'bin'}:{os.environ['PATH']}", "CT_TEST_ROOT": str(root),
                       "RUST_LOG": "couchtube=debug", "COUCHTUBE_DATA_DIR": str(root / "data"), "XDG_RUNTIME_DIR": str(root), "COUCHTUBE_PORT": str(port)}
        client = urllib.request.build_opener(urllib.request.ProxyHandler({}))

        def request(path, body=None, method=None):
            req = urllib.request.Request(f"http://127.0.0.1:{port}/api/{path}",
                data=None if body is None else json.dumps(body).encode(), method=method, headers={"Content-Type": "application/json"})
            try:
                with client.open(req, timeout=5) as response:
                    return response.status, json.load(response)
            except urllib.error.HTTPError as error:
                return error.code, json.load(error)

        def wait_for(predicate, timeout=8):
            until = time.monotonic() + timeout
            while time.monotonic() < until:
                try:
                    result = predicate()
                    if result:
                        return result
                except (OSError, ValueError):
                    pass
                time.sleep(0.05)
            raise AssertionError("timed out waiting for feed/playback state")

        def raw(path):
            """A GET that also returns the response headers."""
            with client.open(f"http://127.0.0.1:{port}/api/{path}", timeout=5) as response:
                return dict(response.headers), json.load(response)

        def calls():
            log = root / "calls.jsonl"
            return [json.loads(line) for line in log.read_text().splitlines()] if log.exists() else []

        def status():
            return request("status")[1]

        def report(**fields):
            """What the player page sends: the same request, for the session that is playing."""
            session = status()["session"]
            assert session is not None, "nothing is playing"
            return request("player/report", {"session": session, **fields})

        def playing(video):
            current = status()
            return current if current["playing"] and current["id"] == video else None

        log_path = root / "backend.log"
        with log_path.open("w+") as log:
            backend = subprocess.Popen([str(repository / "target/debug/couchtube")], cwd=repository,
                                       env=environment, stdout=log, stderr=log)
            try:
                wait_for(lambda: request("health")[0] == 200)
                assert request("feed/recommended")[1] == []
                assert not calls(), "Guest unexpectedly requested an account feed"
                assert request("feed/not-a-feed")[0] == 400
                alice = request("profiles", {"name": "Alice"})[1]
                bob = request("profiles", {"name": "Bob"})[1]
                for profile in (alice, bob):
                    (root / "data/profiles" / profile["id"] / "firefox/cookies.sqlite").touch()
                request(f"profiles/{alice['id']}/select", {})

                # Each feed asks YouTube for the right address, once: the second request is cached.
                targets = {"recommended": "https://www.youtube.com/feed/recommended", "subscriptions": "https://www.youtube.com/feed/subscriptions",
                           "history": "https://www.youtube.com/feed/history", "watchlater": "https://www.youtube.com/playlist?list=WL",
                           "liked": "https://www.youtube.com/playlist?list=LL"}
                for kind, target in targets.items():
                    assert request(f"feed/{kind}")[1][0]["title"] == "Account fixture"
                    count = len([c for c in calls() if c["args"][-1] == target])
                    assert count >= 1, kind
                    request(f"feed/{kind}")
                    assert len([c for c in calls() if c["args"][-1] == target]) == count, f"{kind} was not cached"
                # An expired list is shown at once, marked stale, and refreshed once in the background.
                target = targets["recommended"]
                before = len([c for c in calls() if c["args"][-1] == target])
                with sqlite3.connect(root / "data/couchtube.db") as db:
                    db.execute("UPDATE feed_cache SET expires_at = unixepoch() - 100 WHERE cache_key = 'feed:recommended:0'")
                headers, body = raw("feed/recommended")
                assert body[0]["title"] == "Account fixture" and headers.get("x-cache") == "stale", headers
                raw("feed/recommended"); raw("feed/recommended")
                wait_for(lambda: len([c for c in calls() if c["args"][-1] == target]) == before + 1)
                time.sleep(0.5)
                assert len([c for c in calls() if c["args"][-1] == target]) == before + 1, "more than one background refresh"
                assert request("thumb/not-a-video-id") == (404, {"error": "no thumbnail for this video"})
                assert request("feed/recommended?refresh=true")[0] == 200
                check = request("check")[1]
                assert check["history"]["ok"] and check["videos"][0]["id"] == VIDEO and check["videos"][0]["video"] is True
                (root / "fail-feed").touch()
                assert request("check")[1]["history"]["ok"] is False
                assert request("feed/recommended?refresh=true")[0] == 502
                (root / "fail-feed").unlink()

                # The queue: validated, no duplicates, one per profile.
                assert request("queue", {"id": "bad"})[0] == 400
                request("queue", {"id": VIDEO})
                request("queue", {"id": VIDEO})
                request("queue", {"id": OTHER})
                assert len(request("queue")[1]) == 2
                request(f"profiles/{bob['id']}/select", {})
                assert request("queue")[1] == [] and request("feed/continue")[1] == []
                request(f"profiles/{alice['id']}/select", {})

                # Next plays the first queued video in the Firefox player page and takes it off the queue.
                assert request("next", {})[1]["playing"] is True
                current = wait_for(lambda: playing(VIDEO))
                assert current["page"].startswith(f"/watch.html?v={VIDEO}&s="), current
                assert [item["id"] for item in request("queue")[1]] == [OTHER]

                # The page reports a seek to 45 s and a pause; the backend shows it, and stop keeps the queue.
                report(position=45.0, duration=600.0, paused=True, title="Fixture video")
                current = wait_for(lambda: (lambda s: s if s["paused"] else None)(status()))
                assert (current["position"], current["duration"], current["title"]) == (45.0, 600.0, "Fixture video")
                request("stop", {})
                assert status()["playing"] is False
                assert len(request("queue")[1]) == 1, "stop consumed the next queued video"
                progress = request("feed/continue")[1]
                assert progress[0]["id"] == VIDEO and abs(progress[0]["resume_position"] - 45) < 1
                assert not any("--mark-watched" in c["args"] for c in calls()), "a seek counted as viewing time"

                # Playing it again resumes at 45 s.
                request("play", {"id": VIDEO})
                current = wait_for(lambda: playing(VIDEO))
                assert "&start=45" in current["page"], current["page"]

                # The video ends: it is marked finished and the next queued video starts by itself.
                report(position=599.0, duration=600.0, outcome="eof")
                wait_for(lambda: playing(OTHER))
                assert request("queue")[1] == []
                assert not any(item["id"] == VIDEO for item in request("feed/continue")[1]), "a finished video is still in Continue watching"

                # The page's "next" key (N) starts the next queued video too.
                request("queue", {"id": VIDEO})
                report(position=10.0, duration=600.0, outcome="next")
                wait_for(lambda: playing(VIDEO))
                assert request("queue")[1] == []

                # Watched sync: after about 30 s of real viewing, and only then, YouTube is told.
                # Seed 29 s of earlier viewing, then watch normally for a few seconds (the threshold stays the production 30 s).
                with sqlite3.connect(root / "data/couchtube.db") as db:
                    db.execute("UPDATE progress SET watched=29,marked=0 WHERE profile_id=? AND video_id=?", (alice["id"], VIDEO))
                position = 20.0
                until = time.monotonic() + 14
                while time.monotonic() < until and not any("--mark-watched" in c["args"] for c in calls()):
                    position += 0.5
                    report(position=position, duration=600.0, paused=False)
                    time.sleep(0.25)
                mark = [c for c in calls() if "--mark-watched" in c["args"]]
                assert mark, "no watched sync after enough viewing"
                assert "--simulate" in mark[-1]["args"] and "--no-flat-playlist" in mark[-1]["args"]
                assert str(root / "data/profiles" / alice["id"] / "firefox") in " ".join(mark[-1]["args"])

                # Another profile sees none of it, and playing stops when the profile changes.
                request(f"profiles/{bob['id']}/select", {})
                assert status()["playing"] is False
                assert request("feed/continue")[1] == [] and request("queue")[1] == []

                # Resume and queue state belong to the profile and survive a restart.
                request(f"profiles/{alice['id']}/select", {})
                request("queue", {"id": OTHER})
                backend.terminate()
                assert backend.wait(timeout=5) == 0
                backend = subprocess.Popen([str(repository / "target/debug/couchtube")], cwd=repository,
                                           env=environment, stdout=log, stderr=log)
                wait_for(lambda: request("health")[0] == 200)
                assert request("queue")[1][0]["id"] == OTHER
                request("queue", method="DELETE")
                assert request("queue")[1] == []
                print("Feeds smoke passed: sources/cache, isolation, resume, queue EOF/next/stop, watched sync, restart.")
            except BaseException:
                log.flush()
                print(log_path.read_text())
                print("Last status/queue:", request("status"), request("queue"))
                raise
            finally:
                if backend.poll() is None:
                    backend.terminate()
                    try:
                        backend.wait(timeout=5)
                    except subprocess.TimeoutExpired:
                        backend.kill()
                        backend.wait()


if __name__ == "__main__":
    main()
