#!/usr/bin/env python3
"""Account feeds/cache, queue, resume and watched-sync integration test; all tools simulated."""
import json
import os
from pathlib import Path
import shutil
import socket
import sqlite3
import subprocess
import tempfile
import time
import urllib.error
import urllib.request


def main():
    repository = Path(__file__).resolve().parent.parent
    with tempfile.TemporaryDirectory(prefix="ct-feeds-") as temporary:
        root = Path(temporary)
        (root / "bin").mkdir()
        shutil.copy(repository / "tests/fake_mpv.py", root / "bin/mpv")
        (root / "bin/mpv").chmod(0o700)
        extractor = root / "bin/yt-dlp"
        extractor.write_text('''#!/usr/bin/env python3
import json,os,pathlib,sys
root=pathlib.Path(os.environ['CT_TEST_ROOT']); args=sys.argv[1:]
with (root/'calls.jsonl').open('a') as log: log.write(json.dumps({'tool':'yt-dlp','args':args})+'\\n')
if (root/'fail-feed').exists() and '/feed/' in args[-1]: sys.exit(1)
print(json.dumps({'entries':[{'id':'aqz-KE-bpKQ','title':'Account fixture','duration':120,'channel':'Fixture'}]}))
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

        def calls(tool):
            log = root / "calls.jsonl"
            return [entry for line in log.read_text().splitlines() if (entry := json.loads(line))["tool"] == tool] if log.exists() else []

        def ipc(command):
            path = next(root.glob("couchtube-*/ipc"))
            with socket.socket(socket.AF_UNIX) as connection:
                connection.connect(str(path))
                connection.sendall((json.dumps({"command": command, "request_id": 99}) + "\n").encode())
                with connection.makefile("r") as stream:
                    return json.loads(stream.readline())

        def ready():
            status = request("status")[1]
            return status if status.get("duration") == 120 else None

        log_path = root / "backend.log"
        with log_path.open("w+") as log:
            backend = subprocess.Popen([str(repository / "target/debug/couchtube")], cwd=repository,
                                       env=environment, stdout=log, stderr=log)
            try:
                wait_for(lambda: request("health")[0] == 200)
                assert request("feed/recommended")[1] == []
                assert not calls("yt-dlp"), "Guest unexpectedly requested an account feed"
                assert request("feed/not-a-feed")[0] == 400
                alice = request("profiles", {"name": "Alice"})[1]
                bob = request("profiles", {"name": "Bob"})[1]
                for profile in (alice, bob):
                    (root / "data/profiles" / profile["id"] / "firefox/cookies.sqlite").touch()
                request(f"profiles/{alice['id']}/select", {})
                targets = {"recommended": "https://www.youtube.com/feed/recommended", "subscriptions": "https://www.youtube.com/feed/subscriptions",
                           "history": "https://www.youtube.com/feed/history", "watchlater": "https://www.youtube.com/playlist?list=WL", "liked": "https://www.youtube.com/playlist?list=LL"}
                for kind, target in targets.items():
                    assert request(f"feed/{kind}")[1][0]["title"] == "Account fixture"
                    assert any(call["args"][-1] == target for call in calls("yt-dlp")), kind
                    count = len([call for call in calls("yt-dlp") if call["args"][-1] == target])
                    request(f"feed/{kind}")
                    assert len([call for call in calls("yt-dlp") if call["args"][-1] == target]) == count
                assert request("feed/recommended?refresh=true")[0] == 200
                check = request("check")[1]
                assert check["history"]["ok"] and check["videos"][0]["id"] == "aqz-KE-bpKQ" and check["videos"][0]["video"] is True
                (root / "fail-feed").touch()
                assert request("check")[1]["history"]["ok"] is False
                (root / "fail-feed").unlink()
                (root / "fail-feed").touch()
                assert request("feed/recommended?refresh=true")[0] == 502
                (root / "fail-feed").unlink()
                assert request("queue", {"id": "bad"})[0] == 400
                request("queue", {"id": "aqz-KE-bpKQ"})
                request("queue", {"id": "aqz-KE-bpKQ"})
                request("queue", {"id": "rFZHOHl-L8A"})
                assert len(request("queue")[1]) == 2
                request(f"profiles/{bob['id']}/select", {})
                assert request("queue")[1] == []
                assert request("feed/continue")[1] == []
                request(f"profiles/{alice['id']}/select", {})
                request("next", {})
                wait_for(ready)
                assert request("status")[1]["id"] == "aqz-KE-bpKQ"
                assert [item["id"] for item in request("queue")[1]] == ["rFZHOHl-L8A"]
                ipc(["seek", 45, "absolute"])
                ipc(["set_property", "pause", True])
                wait_for(lambda: request("status")[1]["paused"])
                request("stop", {})
                assert len(request("queue")[1]) == 1, "Esc/stop consumed the next queued video"
                progress = request("feed/continue")[1]
                assert progress[0]["id"] == "aqz-KE-bpKQ" and abs(progress[0]["resume_position"] - 45) < 1
                assert not any("--mark-watched" in call["args"] for call in calls("yt-dlp")), "seek incorrectly counted as viewing time"
                request("play", {"id": "aqz-KE-bpKQ"})
                wait_for(ready)
                assert any(arg.startswith("--start=45") for arg in calls("mpv")[-1]["args"])
                ipc(["test_eof"])
                wait_for(lambda: request("status")[1]["id"] == "rFZHOHl-L8A")
                wait_for(ready)
                assert request("queue")[1] == []
                assert not any(item["id"] == "aqz-KE-bpKQ" for item in request("feed/continue")[1])
                request("queue", {"id": "aqz-KE-bpKQ"})
                ipc(["quit", 5])
                wait_for(lambda: request("status")[1]["id"] == "aqz-KE-bpKQ")
                wait_for(ready)
                assert request("queue")[1] == []
                # Seed 29 seconds of previously viewed time, then watch normally.
                # The threshold is still the production 30 s; no account request is made for a seek.
                with sqlite3.connect(root / "data/couchtube.db") as db:
                    db.execute("UPDATE progress SET watched=29,marked=0 WHERE profile_id=? AND video_id=?", (alice["id"], "aqz-KE-bpKQ"))
                wait_for(lambda: any("--mark-watched" in call["args"] for call in calls("yt-dlp")), timeout=12)
                mark = [call for call in calls("yt-dlp") if "--mark-watched" in call["args"]][-1]
                assert "--simulate" in mark["args"] and "--no-flat-playlist" in mark["args"]
                assert str(root / "data/profiles" / alice["id"] / "firefox") in " ".join(mark["args"])
                request(f"profiles/{bob['id']}/select", {})
                assert request("status")[1]["playing"] is False
                assert request("feed/continue")[1] == []
                assert request("queue")[1] == []
                # Persistent resume/queue state and profile ownership survive a restart.
                request(f"profiles/{alice['id']}/select", {})
                request("queue", {"id": "rFZHOHl-L8A"})
                backend.terminate()
                assert backend.wait(timeout=5) == 0
                backend = subprocess.Popen([str(repository / "target/debug/couchtube")], cwd=repository,
                                           env=environment, stdout=log, stderr=log)
                wait_for(lambda: request("health")[0] == 200)
                assert request("queue")[1][0]["id"] == "rFZHOHl-L8A"
                request("queue", method="DELETE")
                assert request("queue")[1] == []
                print("Feeds smoke passed: sources/cache, isolation, resume, queue EOF/next/stop, watched sync, restart.")
            except BaseException:
                log.flush()
                print(log_path.read_text())
                print("Last status/queue:", request("status"), request("queue"))
                print("Recent tool calls:", calls("mpv")[-5:])
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
