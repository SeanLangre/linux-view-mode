#!/usr/bin/env python3
"""Local mpv IPC fixture for feed/progress tests. No screen, sound, or network."""
import asyncio
import json
import os
from pathlib import Path
import sys
import time


async def main():
    args = sys.argv[1:]
    root = Path(os.environ["CT_TEST_ROOT"])
    with (root / "calls.jsonl").open("a") as log:
        log.write(json.dumps({"tool": "mpv", "args": args, "pid": os.getpid()}) + "\n")
    socket = next(arg.split("=", 1)[1] for arg in args if arg.startswith("--input-ipc-server="))
    start = next((float(arg.split("=", 1)[1]) for arg in args if arg.startswith("--start=")), 0.0)
    state = {"media-title": "IPC fixture video", "time-pos": start, "duration": 120.0, "pause": False}
    clients = {}
    finished = asyncio.Event()
    ending = {"reason": "quit", "code": 0}

    async def send(writer, event):
        try:
            writer.write((json.dumps(event) + "\n").encode())
            await writer.drain()
        except (OSError, ConnectionError):
            pass

    async def changed(name):
        for writer, names in list(clients.items()):
            if name in names:
                await send(writer, {"event": "property-change", "name": name, "data": state[name]})

    async def handle(reader, writer):
        clients[writer] = set()
        try:
            while raw := await reader.readline():
                message = json.loads(raw)
                command = message["command"]
                response = {"error": "success", "request_id": message.get("request_id", 0)}
                if command[0] == "observe_property":
                    clients[writer].add(command[2])
                    await send(writer, {"event": "property-change", "name": command[2], "data": state.get(command[2])})
                elif command[0] == "get_property":
                    response["data"] = state.get(command[1])
                elif command[0] == "seek":
                    state["time-pos"] = float(command[1])
                    await changed("time-pos")
                elif command[0] == "set_property":
                    state[command[1]] = command[2]
                    await changed(command[1])
                elif command[0] == "test_eof":
                    ending["reason"] = "eof"
                    state["time-pos"] = state["duration"]
                    await changed("time-pos")
                    finished.set()
                elif command[0] == "quit":
                    ending["code"] = int(command[1]) if len(command) > 1 else 0
                    finished.set()
                await send(writer, response)
        except (OSError, ConnectionError):
            pass
        finally:
            clients.pop(writer, None)
            writer.close()

    async def clock():
        previous = time.monotonic()
        while not finished.is_set():
            await asyncio.sleep(0.1)
            now = time.monotonic()
            if not state["pause"]:
                state["time-pos"] += now - previous
                await changed("time-pos")
                if state["time-pos"] >= state["duration"]:
                    ending["reason"] = "eof"
                    finished.set()
            previous = now

    server = await asyncio.start_unix_server(handle, path=socket)
    task = asyncio.create_task(clock())
    await finished.wait()
    for writer in list(clients):
        await send(writer, {"event": "end-file", "reason": ending["reason"]})
    await asyncio.sleep(0.05)
    task.cancel()
    server.close()
    for writer in list(clients):
        writer.close()
    await server.wait_closed()
    return ending["code"]


if __name__ == "__main__":
    sys.exit(asyncio.run(main()))
