"""Check shared rate and TCP half-close without opening any local service/socket."""
import asyncio
import importlib.util
import json
import os
from pathlib import Path
import sys
import time

sys.dont_write_bytecode = True
project = Path(__file__).resolve().parents[1]
output = project / "artifacts/testing/v08/relay-check"
output.mkdir(parents=True, exist_ok=True)
os.chdir(output)
spec = importlib.util.spec_from_file_location("network_relay", project / "scripts/android-proxy.py")
module = importlib.util.module_from_spec(spec)
spec.loader.exec_module(module)


class Sink:
    def __init__(self):
        self.data = bytearray()
        self.eof = False

    def write(self, data):
        self.data.extend(data)

    async def drain(self):
        await asyncio.sleep(0)

    def can_write_eof(self):
        return True

    def write_eof(self):
        self.eof = True

    def close(self):
        pass

    async def wait_closed(self):
        pass


async def main():
    relay = module.Relay()
    body = bytes(range(256)) * 128
    module.save("rate", "32000")
    readers = [asyncio.StreamReader(), asyncio.StreamReader()]
    sinks = [Sink(), Sink()]
    for reader in readers:
        reader.feed_data(body)
        reader.feed_eof()
    started = time.monotonic()
    await asyncio.gather(*(relay.copy(reader, sink, True) for reader, sink in zip(readers, sinks)))
    elapsed = time.monotonic() - started
    assert all(bytes(sink.data) == body for sink in sinks), "Relay changed opaque bytes"
    assert 1.9 <= elapsed < 8, "Shared rate budget was not enforced: " + str(elapsed)
    assert relay.stats["limitedBytes"] == 65536
    module.save("rate", "0")
    await relay.pace(8192)
    incoming = asyncio.StreamReader()
    incoming.feed_data(b"CONNECT 127.0.0.1:443 HTTP/1.1\r\n\r\nopaque request")
    incoming.feed_eof()
    response = asyncio.StreamReader()
    client = Sink()

    class Remote(Sink):
        def write_eof(self):
            super().write_eof()
            response.feed_data(body)
            response.feed_eof()

    remote = Remote()
    original_connect = module.asyncio.open_connection

    async def connect(*args, **kwargs):
        return response, remote

    module.asyncio.open_connection = connect
    try:
        await asyncio.wait_for(relay.connect(incoming, client), 3)
    finally:
        module.asyncio.open_connection = original_connect
    assert bytes(client.data).endswith(body), "Client half-close discarded the server response"
    assert remote.eof and client.eof, "TCP EOF was not forwarded"
    print(json.dumps({"opaqueBytesUnchanged": True, "sharedBytes": 65536, "elapsedSeconds": elapsed, "halfClosePreserved": True}))


asyncio.run(main())
