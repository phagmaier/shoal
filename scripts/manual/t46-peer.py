"""Disposable mismatched daemon for t46.sh; never connects outside its sandbox.

Core/version modes relay a real daemon's frames and change only hello's build
identity. Protocol mode refuses hello; stubborn mode also ignores SIGTERM to
exercise the five-second stop deadline. SIGUSR1 always cleans up the fixture.
"""

import asyncio
import contextlib
import json
import os
import pathlib
import signal
import struct
import sys


async def frame(reader):
    prefix = await reader.readexactly(4)
    length = struct.unpack("<I", prefix)[0]
    if not 1 <= length <= 64 * 1024 * 1024:
        raise ValueError("invalid frame length")
    return await reader.readexactly(length)


def encode(message):
    return b"\x01" + json.dumps(message).encode()


async def write(writer, payload):
    writer.write(struct.pack("<I", len(payload)) + payload)
    await writer.drain()


async def main(mode, socket, upstream=None):
    socket = pathlib.Path(socket)
    socket.parent.mkdir(mode=0o700, parents=True, exist_ok=True)
    stopped = asyncio.Event()
    loop = asyncio.get_running_loop()
    loop.add_signal_handler(signal.SIGUSR1, stopped.set)
    loop.add_signal_handler(signal.SIGTERM, lambda: None if mode == "stubborn" else stopped.set())
    tasks = set()

    async def peer(reader, writer):
        task = asyncio.current_task()
        tasks.add(task)
        remote = None
        pumps = []
        try:
            if mode in ("protocol", "stubborn"):
                hello = json.loads((await asyncio.wait_for(frame(reader), 5))[1:])
                await write(writer, encode({"jsonrpc": "2.0", "id": hello["id"], "error": {
                    "code": 1003, "message": "protocol mismatch", "data": {
                        "daemon_protocol": 2, "pid": os.getpid()}}}))
                return
            incoming, remote = await asyncio.open_unix_connection(upstream)
            shutting_down = False

            async def requests():
                nonlocal shutting_down
                while True:
                    payload = await frame(reader)
                    if payload[0] == 1:
                        message = json.loads(payload[1:])
                        if message.get("method") == "daemon.shutdown":
                            shutting_down = True
                    await write(remote, payload)

            async def responses():
                try:
                    while True:
                        payload = await frame(incoming)
                        if payload[0] == 1:
                            message = json.loads(payload[1:])
                            result = message.get("result", {})
                            if isinstance(result, dict) and "daemon_version" in result:
                                if mode == "core":
                                    result["core"] = "t46-old-core"
                                else:
                                    result["daemon_version"] = "t46-old-version"
                                payload = encode(message)
                        await write(writer, payload)
                finally:
                    if shutting_down:
                        stopped.set()

            pumps = [asyncio.create_task(requests()), asyncio.create_task(responses())]
            await asyncio.wait(pumps, return_when=asyncio.FIRST_COMPLETED)
        except (asyncio.IncompleteReadError, ConnectionError, TimeoutError):
            pass
        finally:
            for pump in pumps:
                pump.cancel()
            await asyncio.gather(*pumps, return_exceptions=True)
            writer.close()
            if remote:
                remote.close()
            tasks.discard(task)

    server = await asyncio.start_unix_server(peer, socket)
    try:
        async with server:
            await stopped.wait()
    finally:
        socket.unlink(missing_ok=True)
        for task in list(tasks):
            task.cancel()
        await asyncio.gather(*tasks, return_exceptions=True)


async def probe(mode, socket, pid=None):
    reader, writer = await asyncio.open_unix_connection(socket)
    try:
        await write(writer, encode({"jsonrpc": "2.0", "id": 1, "method": "hello", "params": {
            "protocol": 1, "client": "cli", "version": "0.1.0", "env": {}}}))
        while True:
            message = json.loads((await frame(reader))[1:])
            if message.get("id") != 1:
                continue
            result = message.get("result", {})
            if mode in ("protocol", "stubborn"):
                data = message.get("error", {}).get("data", {})
                return data.get("pid") == int(pid) and data.get("daemon_protocol") == 2
            if mode == "core":
                return result.get("core") == "t46-old-core"
            if mode == "version":
                return result.get("daemon_version") == "t46-old-version"
            return bool(result) and not any(str(result.get(key, "")).startswith("t46-") for key in ("core", "daemon_version"))
    finally:
        writer.close()
        await writer.wait_closed()


if __name__ == "__main__":
    with contextlib.suppress(KeyboardInterrupt):
        if sys.argv[1] == "probe":
            try:
                ok = asyncio.run(asyncio.wait_for(probe(*sys.argv[2:]), 1))
            except (OSError, TimeoutError, asyncio.IncompleteReadError):
                ok = False
            sys.exit(0 if ok else 1)
        asyncio.run(main(*sys.argv[1:]))
