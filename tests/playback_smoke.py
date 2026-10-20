#!/usr/bin/env python3
"""Exercise the playback API with real mpv, local audio, and no display/network.
Run `cargo build` first, then `python3 tests/playback_smoke.py` from the repo root.
"""
import json
import os
from pathlib import Path
import shutil
import signal
import socket
import subprocess
import tempfile
import time
import urllib.error
import urllib.request
import wave


def main():
    root = Path(__file__).resolve().parent.parent
    mpv = shutil.which("mpv")
    assert mpv, "mpv must be installed"
    with tempfile.TemporaryDirectory(prefix="ct-smoke-") as temporary:
        directory = Path(temporary)
        media = directory / "fixture.wav"
        with wave.open(str(media), "wb") as audio:
            audio.setparams((1, 2, 8000, 0, "NONE", "not compressed"))
            audio.writeframes(b"\0\0" * 8000 * 30)
        pids_file = directory / "pids"
        wrapper = directory / "mpv"
        wrapper.write_text(
            "#!/usr/bin/env python3\nimport os, sys\n"
            f"with open({str(pids_file)!r}, 'a') as f: f.write(str(os.getpid()) + '\\n')\n"
            "args = sys.argv[1:]\nargs = args[:args.index('--')]\n"
            f"os.execv({mpv!r}, [{mpv!r}, *args, '--vo=null', '--ao=null', "
            "'--hwdec=no', '--fullscreen=no', '--pause=yes', "
            f"'--force-media-title=Smoke fixture', '--', {str(media)!r}])\n"
        )
        wrapper.chmod(0o700)
        with socket.socket() as probe:
            probe.bind(("127.0.0.1", 0))
            port = probe.getsockname()[1]
        environment = {**os.environ, "PATH": f"{directory}:{os.environ['PATH']}",
                       "XDG_RUNTIME_DIR": str(directory), "COUCHTUBE_PORT": str(port)}
        log_path = directory / "backend.log"
        with log_path.open("w+") as log:
            backend = subprocess.Popen([str(root / "target/debug/couchtube")],
                                       cwd=root, env=environment, stdout=log, stderr=log)
            # Ignore any proxy configuration for these loopback requests.
            client = urllib.request.build_opener(urllib.request.ProxyHandler({}))

            def request(path, body=None):
                data = None if body is None else json.dumps(body).encode()
                req = urllib.request.Request(f"http://127.0.0.1:{port}/api/{path}", data=data,
                                             headers={"Content-Type": "application/json"})
                try:
                    with client.open(req, timeout=2) as response:
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
                raise AssertionError("timed out waiting for playback state")

            def ready():
                status = request("status")[1]
                return status if status.get("duration") == 30 else None

            def pids():
                return [int(line) for line in pids_file.read_text().splitlines()]

            try:
                wait_for(lambda: request("health")[0] == 200)
                assert request("status")[1]["playing"] is False
                for body in ({"id": "--bad"}, {"id": "aqz-KE-bpKQ", "start": -1}, {}):
                    code, error = request("play", body)
                    assert code == 400 and "error" in error, (code, error)
                assert request("play", {"id": "aqz-KE-bpKQ", "start": 2})[0] == 200
                status = wait_for(ready)
                assert status["playing"] and status["paused"] and status["title"] == "Smoke fixture"
                assert abs(status["position"] - 2) < 0.1, status
                first_pid = pids()[-1]
                assert request("play", {"id": "rFZHOHl-L8A"})[0] == 200
                wait_for(ready)
                assert not Path(f"/proc/{first_pid}").exists(), "old player was not reaped"
                # mpv's quit command is also what the Esc/q bindings invoke.
                ipc = next(directory.glob("couchtube-*/ipc"))
                with socket.socket(socket.AF_UNIX) as connection:
                    connection.connect(str(ipc))
                    connection.sendall(b'{"command":["quit"]}\n')
                wait_for(lambda: request("status")[1]["playing"] is False)
                assert request("status")[1]["error"] is None
                assert not list(directory.glob("couchtube-*")), "socket directory leaked"
                for _ in range(2):
                    assert request("stop", {})[1] == {"status": "stopped"}
                assert request("play", {"id": "aqz-KE-bpKQ"})[0] == 200
                wait_for(ready)
                assert request("stop", {})[0] == 200
                assert request("status")[1]["playing"] is False
                assert request("play", {"id": "aqz-KE-bpKQ"})[0] == 200
                wait_for(ready)
                backend.send_signal(signal.SIGTERM)
                assert backend.wait(timeout=5) == 0
                assert all(not Path(f"/proc/{pid}").exists() for pid in pids()), "player survived shutdown"
                assert not list(directory.glob("couchtube-*")), "socket directory leaked on shutdown"
                log.flush()
                contents = log_path.read_text()
                assert "Error parsing" not in contents and "Lua error" not in contents, contents
                print("Playback smoke passed: validation, real mpv IPC/title/position/pause, replacement, quit, stop, shutdown.")
            except BaseException:
                log.flush()
                print(log_path.read_text())
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
