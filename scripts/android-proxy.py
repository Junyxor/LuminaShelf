"""Local CI CONNECT relay: forward TLS unchanged and limit shared downstream bytes."""
import asyncio
import json
import os
from pathlib import Path
import sys
import time
from urllib.parse import urlsplit

STATE = Path("smoke/network-proxy")
STATE.mkdir(parents=True, exist_ok=True)


def save(name, value):
    path = STATE / name
    temporary = path.with_name(path.name + "." + str(os.getpid()) + ".tmp")
    temporary.write_text(value, encoding="utf-8")
    temporary.replace(path)


class Relay:
    def __init__(self):
        self.lock = asyncio.Lock()
        self.next_byte = 0.0
        self.stats = {"forwardedBytes": 0, "limitedBytes": 0, "connections": 0, "errors": 0}

    def rate(self):
        return int((STATE / "rate").read_text(encoding="utf-8"))

    async def pace(self, count):
        async with self.lock:
            rate = self.rate()
            if rate == 0:
                self.next_byte = time.monotonic()
                return False
            self.next_byte = max(time.monotonic(), self.next_byte) + count / rate
            deadline = self.next_byte
        while deadline > time.monotonic() and self.rate() > 0:
            await asyncio.sleep(min(0.1, deadline - time.monotonic()))
        return self.rate() > 0

    async def copy(self, reader, writer, downstream):
        while data := await reader.read(8192):
            limited = await self.pace(len(data)) if downstream else False
            writer.write(data)
            await writer.drain()
            if downstream:
                self.stats["forwardedBytes"] += len(data)
                if limited:
                    self.stats["limitedBytes"] += len(data)
                save("stats.json", json.dumps(self.stats))
        if writer.can_write_eof():
            writer.write_eof()
            await writer.drain()

    async def connect(self, reader, writer):
        remote_writer = None
        tasks = []
        try:
            headers = await asyncio.wait_for(reader.readuntil(b"\r\n\r\n"), 15)
            first, remainder = headers.split(b"\r\n", 1)
            method, authority, version = first.decode("ascii").split()
            if method == "CONNECT":
                host, port = authority.rsplit(":", 1)
                request = None
                allowed = port == "443"
            else:
                target = urlsplit(authority)
                host, port = target.hostname, str(target.port or 80)
                path = target.path or "/"
                if target.query:
                    path += "?" + target.query
                request = f"{method} {path} {version}\r\n".encode("ascii") + remainder
                allowed = method in ["GET", "HEAD"] and target.scheme == "http" and port == "80"
            if not host or not allowed:
                writer.write(b"HTTP/1.1 405 Method Not Allowed\r\nContent-Length: 0\r\n\r\n")
                await writer.drain()
                return
            remote_reader, remote_writer = await asyncio.wait_for(asyncio.open_connection(host.strip("[]"), int(port)), 25)
            if request is None:
                writer.write(b"HTTP/1.1 200 Connection Established\r\n\r\n")
                await writer.drain()
            else:
                remote_writer.write(request)
                await remote_writer.drain()
            self.stats["connections"] += 1
            tasks = [asyncio.create_task(self.copy(reader, remote_writer, False)), asyncio.create_task(self.copy(remote_reader, writer, True))]
            await asyncio.gather(*tasks)
        except (OSError, ValueError, asyncio.IncompleteReadError, asyncio.LimitOverrunError, asyncio.TimeoutError) as error:
            self.stats["errors"] += 1
            self.stats["lastError"] = type(error).__name__
        finally:
            for task in tasks:
                if not task.done():
                    task.cancel()
            if tasks:
                await asyncio.gather(*tasks, return_exceptions=True)
            for stream in [writer, remote_writer]:
                if stream:
                    stream.close()
                    try:
                        await stream.wait_closed()
                    except OSError:
                        pass
            save("stats.json", json.dumps(self.stats))

    async def serve(self):
        save("rate", "0")
        save("stats.json", json.dumps(self.stats))
        server = await asyncio.start_server(self.connect, "127.0.0.1", 18888, limit=8192)
        save("ready", "18888")
        async with server:
            await server.serve_forever()


if __name__ == "__main__":
    command = sys.argv[1]
    if command == "serve":
        if os.environ.get("GITHUB_ACTIONS") != "true":
            raise SystemExit("This relay is for a disposable cloud CI runner only")
        asyncio.run(Relay().serve())
    elif command in ["full", "256"]:
        save("rate", "0" if command == "full" else "32000")
    elif command == "status":
        print((STATE / "stats.json").read_text(encoding="utf-8"))
    else:
        raise SystemExit(1)
