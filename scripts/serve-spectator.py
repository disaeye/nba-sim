#!/usr/bin/env python3
"""Serve spectator files with transparent gzip and NDJSON support.

- game.json.gz / game.ticks.ndjson.gz are served with Content-Encoding: gzip
  when the client sends Accept-Encoding: gzip (browsers always do).
- When the client requests Accept-Encoding: br, zstd, gzip, deflate (any
  supported), the .ndjson.gz is served instead of the raw .ndjson when the
  plain file is absent or the compressed twin exists.
- Other files are served as-is.
"""
import os
import sys
from http.server import SimpleHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1] / "spectator"
PORT = int(os.environ.get("PORT", "4173"))


class Handler(SimpleHTTPRequestHandler):
    extensions_map = {**SimpleHTTPRequestHandler.extensions_map, ".json": "application/json"}

    def __init__(self, *args, **kwargs):
        super().__init__(*args, directory=str(ROOT), **kwargs)

    def send_head(self):
        path = self.path.split("?", 1)[0]
        # 1) game.json → game.json.gz (transparent gzip, as before)
        if path == "/game.json":
            compressed = ROOT / "game.json.gz"
            if compressed.is_file():
                self.path = "/game.json.gz"
                response = super().send_head()
                if response is not None:
                    self.send_header("Content-Encoding", "gzip")
                    self.send_header("Content-Type", "application/json; charset=utf-8")
                    self.send_header("Vary", "Accept-Encoding")
                return response
        # 2) .ndjson → serve the .gz twin when the client accepts gzip
        if path.endswith(".ndjson"):
            accept_enc = self.headers.get("Accept-Encoding", "")
            if "gzip" in accept_enc:
                compressed = ROOT / (path.lstrip("/") + ".gz")
                if compressed.is_file():
                    self.path = path + ".gz"
                    response = super().send_head()
                    if response is not None:
                        self.send_header("Content-Encoding", "gzip")
                        self.send_header("Content-Type", "application/x-ndjson; charset=utf-8")
                        self.send_header("Vary", "Accept-Encoding")
                    return response
        return super().send_head()

    def end_headers(self):
        if self.path.endswith(".gz"):
            self.send_header("Content-Encoding", "gzip")
            self.send_header("Vary", "Accept-Encoding")
        # Spectator data regenerates on every kernel change; a CDN edge
        # (EdgeOne/Cloudflare) caching an old game.json/ndjson would serve
        # a stale game until its TTL expires. Force revalidation: the
        # 304 path keeps it cheap, but a changed Last-Modified always
        # re-fetches.
        self.send_header("Cache-Control", "no-cache, must-revalidate")
        super().end_headers()

    def handle_error(self, request, client_address):
        # A client that disconnects mid-response (favicon 404, tab close,
        # refresh) raises BrokenPipe/ConnectionReset inside send_head —
        # without this override the whole server process dies. Swallow
        # those; anything else is re-raised so real faults stay visible.
        exc = sys.exc_info()[1]
        if isinstance(exc, (BrokenPipeError, ConnectionResetError)):
            return
        super().handle_error(request, client_address)


ThreadingHTTPServer(("0.0.0.0", PORT), Handler).serve_forever()
