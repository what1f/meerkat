"""Serve design documents on loopback; optional production reference for pixel review."""
from http.server import SimpleHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path
from urllib.parse import unquote, urlsplit
import argparse

ROOT = Path(__file__).resolve().parents[3]
ALLOWED = (ROOT / 'docs/design', ROOT / 'apps/web/src/assets')


class DesignHandler(SimpleHTTPRequestHandler):
    def do_GET(self):
        requested = unquote(urlsplit(self.path).path)
        if requested.startswith(('/reference/', '/assets/')) and self.server.reference_dir:
            relative = requested.removeprefix('/reference/').lstrip('/')
            folder = ROOT / 'apps/web/dist' if relative.startswith('assets/') else self.server.reference_dir
            target = (folder / relative).resolve()
            allowed = target.is_relative_to(folder)
        else:
            target = (ROOT / requested.lstrip('/')).resolve()
            allowed = any(target.is_relative_to(folder) for folder in ALLOWED)
        if not allowed or not target.is_file():
            self.send_error(404)
            return
        payload = target.read_bytes()
        if self.server.still and target.suffix == '.html':
            # Pixel baselines compare the resting state, not a different pulse phase.
            payload = payload.replace(b'</head>', b'<style>*,*::before,*::after{animation:none!important;transition:none!important;caret-color:transparent!important}</style></head>')
        self.send_response(200)
        self.send_header('Content-Type', self.guess_type(str(target)))
        self.send_header('Content-Length', str(len(payload)))
        self.send_header('Cache-Control', 'no-store')
        self.end_headers()
        self.wfile.write(payload)


if __name__ == '__main__':
    parser = argparse.ArgumentParser()
    parser.add_argument('--port', type=int, default=8769)
    parser.add_argument('--reference-dir', type=Path)
    parser.add_argument('--still', action='store_true')
    args = parser.parse_args()
    server = ThreadingHTTPServer(('127.0.0.1', args.port), DesignHandler)
    server.reference_dir = args.reference_dir.resolve() if args.reference_dir else None
    server.still = args.still
    server.serve_forever()
