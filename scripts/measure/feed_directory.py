# -*- coding: utf-8 -*-
"""What Chainlink's published feed directory for Robinhood Chain mainnet says:
how many feeds there are, whether one of them is an L2 sequencer-uptime feed
(on networks that have one it is listed with the price feeds), and the
heartbeat, deviation threshold and market hours of the feeds AfterHours and
its docs rely on. Read-only, stdlib, one HTTPS GET.

    python scripts/measure/feed_directory.py
"""
import io
import json
import sys
import urllib.request

sys.stdout = io.TextIOWrapper(sys.stdout.buffer, encoding="utf-8", errors="replace")
URL = "https://reference-data-directory.vercel.app/feeds-robinhood-mainnet.json"
SHOW = ("Robinhood AAPL / USD", "Robinhood NVDA / USD", "Robinhood SPY / USD")


def main():
    req = urllib.request.Request(URL, headers={"user-agent": "curl/8"})
    feeds = json.load(urllib.request.urlopen(req, timeout=30))
    print("%s\n%d feeds listed" % (URL, len(feeds)))
    uptime = [f for f in feeds
              if any(k in ("%s %s" % (f.get("name"), f.get("path"))).lower() for k in ("sequencer", "uptime"))]
    print("sequencer-uptime feeds: %s" % (", ".join(f["name"] for f in uptime) if uptime else "none"))
    hours = {}
    for f in feeds:
        h = (f.get("docs") or {}).get("marketHours")
        hours[h] = hours.get(h, 0) + 1
    print("market hours: %s" % ", ".join("%s %d" % (h, n) for h, n in sorted(hours.items(), key=lambda x: -x[1])))
    for f in feeds:
        name = f.get("name") or ""
        if name in SHOW or "USDG" in name:
            print("  %-22s %s  heartbeat %ss  threshold %s%%  market hours %s"
                  % (name, f.get("proxyAddress"), f.get("heartbeat"), f.get("threshold"),
                     (f.get("docs") or {}).get("marketHours")))


if __name__ == "__main__":
    main()
