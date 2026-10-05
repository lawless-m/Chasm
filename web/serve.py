"""Serve web/ for the browser REPL with cross-origin isolation.

SharedArrayBuffer and Atomics.wait need the page to be cross-origin
isolated, which these two response headers provide. The repository's
`examples/` is also served, at `/examples/`, for the headless examples page
(`test/examples.html`).

Usage: python3 web/serve.py [PORT]    (default 8000)
"""

import functools
import http.server
import pathlib
import sys
import urllib.parse

ROOT = pathlib.Path(__file__).resolve().parent


class Handler(http.server.SimpleHTTPRequestHandler):
    extensions_map = {
        **http.server.SimpleHTTPRequestHandler.extensions_map,
        ".wasm": "application/wasm",
        ".js": "text/javascript",
        ".mjs": "text/javascript",
    }

    def translate_path(self, path):
        clean = urllib.parse.unquote(path.split("?", 1)[0].split("#", 1)[0])
        if clean.startswith("/examples/"):
            rest = clean[len("/examples/") :]
            if ".." in rest.split("/"):
                return str(ROOT / "no such file")
            return str(ROOT.parent / "examples" / rest)
        return super().translate_path(path)

    def end_headers(self):
        self.send_header("Cross-Origin-Opener-Policy", "same-origin")
        # The BRIDGE client script is cross-origin and sends no CORP header.
        coep = "credentialless" if self.path.endswith("bridge.html") else "require-corp"
        self.send_header("Cross-Origin-Embedder-Policy", coep)
        self.send_header("Cache-Control", "no-store")
        super().end_headers()


def main():
    port = int(sys.argv[1]) if len(sys.argv) > 1 else 8000
    handler = functools.partial(Handler, directory=str(ROOT))
    with http.server.ThreadingHTTPServer(("localhost", port), handler) as httpd:
        print(f"Whackford REPL at http://localhost:{port}/", flush=True)
        httpd.serve_forever()


if __name__ == "__main__":
    main()
