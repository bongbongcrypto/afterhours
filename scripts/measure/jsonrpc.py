# -*- coding: utf-8 -*-
"""One JSON-RPC POST with the back-off the public Robinhood Chain RPC needs:
it answers a burst of reads with HTTP 429. Stdlib only.

    from jsonrpc import post
    out = post(RPC, {"jsonrpc": "2.0", "id": 1, "method": "eth_call", "params": [...]})
"""
import json
import time
import urllib.error
import urllib.request

HEADERS = {"content-type": "application/json", "user-agent": "curl/8"}


def post(url, payload, timeout=40, attempts=8, pause=0.25):
    """POST `payload` as JSON and return the decoded reply.

    On HTTP 429 wait the server's Retry-After seconds, or else 2, 4, 8 ... up to
    60 s, and try again; after `attempts` tries (about three minutes of waiting)
    the 429 is raised. Any other HTTP error is raised at once. `pause` spaces
    consecutive calls so a loop of reads does not trip the limit to begin with.
    """
    body = json.dumps(payload).encode()
    for attempt in range(attempts):
        time.sleep(pause)
        try:
            with urllib.request.urlopen(urllib.request.Request(url, body, HEADERS), timeout=timeout) as r:
                return json.load(r)
        except urllib.error.HTTPError as e:
            if e.code != 429 or attempt == attempts - 1:
                raise
            wait = e.headers.get("Retry-After") if e.headers else None
            time.sleep(float(wait) if wait and wait.isdigit() else min(60, 2 ** (attempt + 1)))
