#!/usr/bin/env python3
"""Profile integration test with fake Firefox/yt-dlp/mpv; never reads real cookies.
Run `cargo build`, then `python3 tests/profiles_smoke.py [--browser]`.
The optional keyboard UI check needs system Chromium and Node 22+.
"""
import json
import os
from pathlib import Path
import socket
import subprocess
import sys
import tempfile
import time
import urllib.error
import urllib.request


def main():
    root = Path(__file__).resolve().parent.parent
    with tempfile.TemporaryDirectory(prefix="ct-accounts-") as temporary:
        directory = Path(temporary)
        tools = directory / "bin"
        tools.mkdir()
        calls = directory / "calls.jsonl"
        fixture = '''#!/usr/bin/env python3
import json, os, pathlib, sys, time
root = pathlib.Path(os.environ['CT_TEST_ROOT'])
name = pathlib.Path(sys.argv[0]).name
args = sys.argv[1:]
with (root / 'calls.jsonl').open('a') as f:
    f.write(json.dumps({'tool': name, 'args': args, 'pid': os.getpid()}) + '\\n')
if name == 'firefox':
    folder = pathlib.Path(args[args.index('--profile') + 1])
    while not (root / ('close-' + folder.parent.name)).exists():
        time.sleep(0.05)
    (folder / 'cookies.sqlite').touch()
elif name == 'mpv':
    while True: time.sleep(1)
else:
    if args[-1] == ':ytsubs' and (root / 'fail-login').exists():
        print('SECRET_COOKIE_MUST_NOT_BE_LOGGED', file=sys.stderr)
        sys.exit(1)
    print(json.dumps({'channel': 'Fixture channel', 'entries': [
        {'id': 'aqz-KE-bpKQ', 'title': 'Fixture video', 'channel_id': 'UCSMOQeBJ2RAnuFungnQOxLg', 'duration': 30}]}))
'''
        for name in ("firefox", "yt-dlp", "mpv"):
            executable = tools / name
            executable.write_text(fixture)
            executable.chmod(0o700)
        with socket.socket() as probe:
            probe.bind(("127.0.0.1", 0))
            port = probe.getsockname()[1]
        environment = {**os.environ, "PATH": f"{tools}:{os.environ['PATH']}",
                       "CT_TEST_ROOT": str(directory), "COUCHTUBE_DATA_DIR": str(directory / "data"),
                       "XDG_RUNTIME_DIR": str(directory), "COUCHTUBE_PORT": str(port)}
        client = urllib.request.build_opener(urllib.request.ProxyHandler({}))

        def request(path, body=None, method=None):
            data = None if body is None else json.dumps(body).encode()
            req = urllib.request.Request(f"http://127.0.0.1:{port}/api/{path}", data=data,
                                         headers={"Content-Type": "application/json"}, method=method)
            try:
                with client.open(req, timeout=3) as response:
                    return response.status, json.load(response)
            except urllib.error.HTTPError as error:
                return error.code, json.load(error)

        def wait_for(predicate):
            deadline = time.monotonic() + 8
            while time.monotonic() < deadline:
                try:
                    result = predicate()
                    if result:
                        return result
                except (OSError, ValueError):
                    pass
                time.sleep(0.05)
            raise AssertionError("timed out waiting for account state")

        def logged(tool):
            return [entry for line in calls.read_text().splitlines()
                    if (entry := json.loads(line))["tool"] == tool] if calls.exists() else []

        def cookie_arg(profile):
            return f"firefox:{directory / 'data/profiles' / profile['id'] / 'firefox'}"

        log_path = directory / "backend.log"
        with log_path.open("w+") as log:
            backend = None
            try:
                backend = subprocess.Popen([str(root / "target/debug/couchtube")], cwd=root,
                                           env=environment, stdout=log, stderr=log)
                wait_for(lambda: request("health")[0] == 200)
                result = request("profiles")[1]
                assert result["active_id"] == "guest" and len(result["profiles"]) == 1
                for body in ({"name": " "}, {"name": "a\nb"}, {"name": "x" * 41},
                             {"name": "A", "color": "red"}, {}):
                    assert request("profiles", body)[0] == 400, body
                alice = request("profiles", {"name": " Alice ", "color": "#aabbcc"})[1]
                bob = request("profiles", {"name": "Bob"})[1]
                assert alice["name"] == "Alice" and alice["id"] != bob["id"]
                for profile in (alice, bob):
                    folder = directory / "data/profiles" / profile["id"] / "firefox"
                    assert folder.stat().st_mode & 0o777 == 0o700
                    assert (folder / "user.js").exists()
                assert request("profiles/guest", method="DELETE")[0] == 400
                assert request("profiles/guest/signin", {})[0] == 400
                assert request("profiles/missing/status")[0] == 404
                assert request("profiles/missing/select", {})[0] == 404
                assert request(f"profiles/{alice['id']}/select", {})[0] == 200
                assert request("search?q=no-cookies-yet")[0] == 200
                assert "--cookies-from-browser" not in logged("yt-dlp")[-1]["args"]
                (directory / "data/profiles" / alice["id"] / "firefox/cookies.sqlite").touch()
                assert request("search?q=fixture")[0] == 200
                args = logged("yt-dlp")[-1]["args"]
                assert "--ignore-config" in args and args[args.index("--cookies-from-browser") + 1] == cookie_arg(alice)
                assert request("channel/UCSMOQeBJ2RAnuFungnQOxLg")[0] == 200
                assert cookie_arg(alice) in logged("yt-dlp")[-1]["args"]
                assert request("play", {"id": "aqz-KE-bpKQ"})[0] == 200
                wait_for(lambda: logged("mpv"))
                args = logged("mpv")[-1]["args"]
                assert f"--ytdl-raw-options-append=cookies-from-browser={cookie_arg(alice)}" in args
                assert request(f"profiles/{alice['id']}/signin", {})[1]["state"] == "signing_in"
                assert request("status")[1]["playing"] is False
                wait_for(lambda: logged("firefox"))
                assert cookie_arg(alice).removeprefix("firefox:") in logged("firefox")[-1]["args"]
                for path, method in (("signin", "POST"), ("select", "POST"), ("", "DELETE")):
                    route = f"profiles/{alice['id']}" + (f"/{path}" if path else "")
                    assert request(route, {} if method == "POST" else None, method)[0] == 409
                (directory / f"close-{alice['id']}").touch()
                wait_for(lambda: request(f"profiles/{alice['id']}/status")[1]["state"] == "signed_in")
                assert logged("yt-dlp")[-1]["args"][-1] == ":ytsubs"
                (directory / "fail-login").touch()
                request(f"profiles/{alice['id']}/status?check=true")
                wait_for(lambda: request(f"profiles/{alice['id']}/status")[1]["state"] == "check_failed")
                (directory / "data/profiles" / bob["id"] / "firefox/cookies.sqlite").touch()
                assert request(f"profiles/{bob['id']}/select", {})[0] == 200
                assert request("search?q=fixture")[0] == 200
                assert cookie_arg(bob) in logged("yt-dlp")[-1]["args"]
                assert request("profiles/guest/select", {})[0] == 200
                request("search?q=guest")
                assert "--cookies-from-browser" not in logged("yt-dlp")[-1]["args"]
                request("play", {"id": "aqz-KE-bpKQ"})
                wait_for(lambda: len(logged("mpv")) == 2)
                assert not any("cookies-from-browser" in argument for argument in logged("mpv")[-1]["args"])
                request(f"profiles/{alice['id']}/select", {})
                assert request("status")[1]["playing"] is False
                assert request(f"profiles/{alice['id']}", method="DELETE")[0] == 200
                assert request("profiles")[1]["active_id"] == "guest"
                assert not (directory / "data/profiles" / alice["id"]).exists()
                assert (directory / "data/profiles" / bob["id"]).exists()
                (directory / "fail-login").unlink()
                if "--browser" in sys.argv:
                    subprocess.run(["node", str(root / "tests/profiles_browser.mjs"),
                                    f"http://127.0.0.1:{port}", str(directory)], check=True, timeout=60)
                # Persist profiles and active selection across backend restarts.
                request(f"profiles/{bob['id']}/select", {})
                backend.terminate()
                assert backend.wait(timeout=5) == 0
                backend = subprocess.Popen([str(root / "target/debug/couchtube")], cwd=root,
                                           env=environment, stdout=log, stderr=log)
                wait_for(lambda: request("health")[0] == 200)
                result = request("profiles")[1]
                assert result["active_id"] == bob["id"]
                assert any(profile["id"] == bob["id"] for profile in result["profiles"])
                log.flush()
                assert "SECRET_COOKIE_MUST_NOT_BE_LOGGED" not in log_path.read_text()
                print("Profiles smoke passed: validation, privacy, account isolation, sign-in/checks, Guest, removal, persistence.")
            except BaseException:
                log.flush()
                print(log_path.read_text())
                raise
            finally:
                if backend and backend.poll() is None:
                    backend.terminate()
                    try:
                        backend.wait(timeout=5)
                    except subprocess.TimeoutExpired:
                        backend.kill()
                        backend.wait()


if __name__ == "__main__":
    main()
