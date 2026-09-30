# -*- coding: utf-8 -*-
"""Screenshot the published live page for the video's tour, after its chain
reads have landed (headless Chrome over the DevTools protocol, stdlib only).

A plain `--screenshot` fires at the load event, before the page's RPC reads
return, so every figure would show "...". This loads the page once, waits,
then scrolls to each target and saves one PNG per target:

    python video/capture_page.py                     # video/out/page-1..3.png
    python video/capture_page.py --wait 30 --url https://bongbongcrypto.github.io/afterhours/

Targets: the top of the page, the element holding the band gauge, the
stocks table (CSS selectors below; a selector that matches nothing fails).
"""
import argparse
import base64
import json
import os
import socket
import struct
import subprocess
import sys
import tempfile
import time
import urllib.request
from pathlib import Path

HERE = Path(__file__).resolve().parent
CHROME = [r"C:\Program Files\Google\Chrome\Application\chrome.exe",
          r"C:\Program Files (x86)\Microsoft\Edge\Application\msedge.exe"]
PORT = 9341
TARGETS = [None, "#gauge", "#stocks-h"]


class Tab:
    def __init__(self, ws_url):
        host, path = ws_url[len("ws://"):].split("/", 1)
        name, port = host.split(":")
        self.s = socket.create_connection((name, int(port)))
        key = base64.b64encode(os.urandom(16)).decode()
        self.s.sendall(("GET /%s HTTP/1.1\r\nHost: %s\r\nUpgrade: websocket\r\nConnection: Upgrade\r\n"
                        "Sec-WebSocket-Key: %s\r\nSec-WebSocket-Version: 13\r\n\r\n" % (path, host, key)).encode())
        buf = b""
        while b"\r\n\r\n" not in buf:
            buf += self.s.recv(4096)
        self.buf = buf.split(b"\r\n\r\n", 1)[1]
        self.n = 0

    def _exact(self, n):
        while len(self.buf) < n:
            self.buf += self.s.recv(65536)
        out, self.buf = self.buf[:n], self.buf[n:]
        return out

    def _send(self, obj):
        data = json.dumps(obj).encode()
        mask = os.urandom(4)
        n = len(data)
        head = bytes([0x81]) + (bytes([0x80 | n]) if n < 126 else
                                bytes([0x80 | 126]) + struct.pack(">H", n) if n < 65536 else
                                bytes([0x80 | 127]) + struct.pack(">Q", n))
        self.s.sendall(head + mask + bytes(b ^ mask[i % 4] for i, b in enumerate(data)))

    def _recv(self):
        msg = b""
        while True:
            b0, b1 = self._exact(2)
            n = b1 & 0x7F
            if n == 126:
                n = struct.unpack(">H", self._exact(2))[0]
            elif n == 127:
                n = struct.unpack(">Q", self._exact(8))[0]
            msg += self._exact(n)
            if b0 & 0x80:
                return json.loads(msg)

    def call(self, method, **params):
        self.n += 1
        self._send({"id": self.n, "method": method, "params": params})
        while True:
            m = self._recv()
            if m.get("id") == self.n:
                if "error" in m:
                    raise RuntimeError("%s: %s" % (method, m["error"]))
                return m.get("result", {})


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--url", default="https://bongbongcrypto.github.io/afterhours/")
    ap.add_argument("--wait", type=float, default=25.0, help="seconds for the page's chain reads")
    ap.add_argument("--width", type=int, default=1600)
    ap.add_argument("--height", type=int, default=1000)
    a = ap.parse_args()
    exe = next((p for p in CHROME if os.path.exists(p)), None)
    if not exe:
        sys.exit("no Chrome or Edge found")
    out = HERE / "out"
    out.mkdir(exist_ok=True)
    profile = tempfile.mkdtemp(prefix="afterhours-cdp-")
    proc = subprocess.Popen([exe, "--headless=new", "--disable-gpu", "--no-first-run", "--no-default-browser-check",
                             "--hide-scrollbars", "--user-data-dir=" + profile, "--remote-debugging-port=%d" % PORT,
                             "--window-size=%d,%d" % (a.width, a.height), "about:blank"],
                            stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    try:
        page = None
        for _ in range(60):
            try:
                tabs = json.load(urllib.request.urlopen("http://127.0.0.1:%d/json" % PORT, timeout=2))
                page = next(t for t in tabs if t.get("type") == "page")
                break
            except Exception:  # noqa: BLE001
                time.sleep(0.3)
        if not page:
            sys.exit("the browser did not open a debugging port")
        tab = Tab(page["webSocketDebuggerUrl"])
        tab.call("Emulation.setDeviceMetricsOverride", width=a.width, height=a.height, deviceScaleFactor=1, mobile=False)
        tab.call("Page.enable")
        tab.call("Page.navigate", url=a.url)
        time.sleep(a.wait)
        for i, sel in enumerate(TARGETS, 1):
            js = ("window.scrollTo(0, 0); true" if sel is None else
                  "(() => { const e = document.querySelector(%s); if (!e) return false;"
                  " window.scrollTo(0, e.getBoundingClientRect().top + window.scrollY - 24); return true; })()"
                  % json.dumps(sel))
            ok = tab.call("Runtime.evaluate", expression=js, returnByValue=True)["result"].get("value")
            if not ok:
                sys.exit("nothing matches %s on %s" % (sel, a.url))
            time.sleep(1.0)
            shot = tab.call("Page.captureScreenshot", format="png")
            dest = out / ("page-%d.png" % i)
            dest.write_bytes(base64.b64decode(shot["data"]))
            print("wrote %s (%s)" % (dest, sel or "top"))
    finally:
        proc.kill()


if __name__ == "__main__":
    main()
