# -*- coding: utf-8 -*-
"""How long did the NVDA/USD Chainlink feed on Robinhood Chain go silent over
the last weekend? Read-only: ~40 eth_calls, stdlib only, run from the PC.

Selectors computed with ethers, not recalled:
  latestRoundData()      0xfeaf968c
  getRoundData(uint80)   0x9a6fc8f5
"""
import io
import json
import sys
import urllib.request
from datetime import datetime, timezone

sys.stdout = io.TextIOWrapper(sys.stdout.buffer, encoding="utf-8", errors="replace")
RPC = "https://rpc.mainnet.chain.robinhood.com"
FEEDS = {
    "NVDA": "0x379EC4f7C378F34a1B47E4F3cbeBCbAC3E8E9F15",
    "SPY": "0x319724394D3A0e3669269846abE664Cd621f9f6A",
}
CALLS = 0


def call(to, data):
    global CALLS
    CALLS += 1
    body = json.dumps({"jsonrpc": "2.0", "id": 1, "method": "eth_call",
                       "params": [{"to": to, "data": data}, "latest"]}).encode()
    req = urllib.request.Request(RPC, body, {"content-type": "application/json",
                                             "user-agent": "curl/8"})
    out = json.load(urllib.request.urlopen(req, timeout=25))
    if "error" in out:
        return None
    return out["result"]


def decode(res):
    h = res[2:]
    w = [int(h[i:i + 64], 16) for i in range(0, 320, 64)]
    return {"roundId": w[0], "answer": w[1], "updatedAt": w[3]}


def latest(feed):
    return decode(call(feed, "0xfeaf968c"))


def round_at(feed, rid):
    res = call(feed, "0x9a6fc8f5" + format(rid, "064x"))
    if res is None or len(res) < 322:
        return None
    d = decode(res)
    return d if d["updatedAt"] else None


def ts(t):
    return datetime.fromtimestamp(t, timezone.utc).strftime("%a %m-%d %H:%M UTC")


def last_round_before(feed, hi, target):
    """Largest roundId in this phase whose updatedAt <= target."""
    phase = hi >> 64
    lo = (phase << 64) + 1
    while lo < hi:
        mid = (lo + hi + 1) // 2
        r = round_at(feed, mid)
        if r is None or r["updatedAt"] > target:
            hi = mid - 1
        else:
            lo = mid
    return lo


# Saturday 2026-09-12 12:00 UTC sits inside last weekend's closed window.
MID_WEEKEND = int(datetime(2026, 9, 12, 12, 0, tzinfo=timezone.utc).timestamp())

for name, feed in FEEDS.items():
    now = latest(feed)
    print("=== %s  latest: %s  price %.2f  (round %d of phase %d)"
          % (name, ts(now["updatedAt"]), now["answer"] / 1e8,
             now["roundId"] & ((1 << 64) - 1), now["roundId"] >> 64))
    before = last_round_before(feed, now["roundId"], MID_WEEKEND)
    a = round_at(feed, before)
    b = round_at(feed, before + 1)
    if not a or not b:
        print("   could not bracket the weekend (phase boundary?)")
        continue
    gap_h = (b["updatedAt"] - a["updatedAt"]) / 3600
    move = (b["answer"] - a["answer"]) / a["answer"] * 100
    print("   last print before the weekend : %s  %.2f" % (ts(a["updatedAt"]), a["answer"] / 1e8))
    print("   first print after             : %s  %.2f" % (ts(b["updatedAt"]), b["answer"] / 1e8))
    print("   silent for %.1f hours; price moved %+.2f%% across the gap" % (gap_h, move))
    print("   docs say reject if older than the 24h heartbeat -> unusable for %.1f of those hours"
          % max(0.0, gap_h - 24))
print("\neth_calls made: %d" % CALLS)
