# -*- coding: utf-8 -*-
"""Does a quiet feed mean a quiet market? These feeds print when the price
moves 0.5% from the last print, so two consecutive prints are normally a
little over 0.5% apart; how far past that they get shows how far a move ran
before the feed printed it. Walk the last rounds of the AAPL, NVDA and SPY
feeds and split the gaps between prints by where they fall: inside one US
regular session as the contract reads it (Monday to Friday, 14:30-20:00 UTC,
open in both daylight-saving regimes), or across its edges and outside it.
Read-only, stdlib only, about 3 x (rounds + 1) eth_calls; run from the PC.

    python scripts/measure/session_prints.py [--rounds 150]

Selectors computed with keccak (scripts/measure/keccak.py checks itself):
  latestRoundData()     0xfeaf968c
  getRoundData(uint80)  0x9a6fc8f5
"""
import argparse
import io
import json
import sys
import time
import urllib.error
import urllib.request
from datetime import datetime, timezone

sys.stdout = io.TextIOWrapper(sys.stdout.buffer, encoding="utf-8", errors="replace")
RPC = "https://rpc.mainnet.chain.robinhood.com"
FEEDS = {
    "AAPL": "0x6B22A786bAa607d76728168703a39Ea9C99f2cD0",
    "NVDA": "0x379EC4f7C378F34a1B47E4F3cbeBCbAC3E8E9F15",
    "SPY": "0x319724394D3A0e3669269846abE664Cd621f9f6A",
}
NARROW = 0.01                  # the contract's quietBandBps (1%)
OPEN, CLOSE = 52_200, 72_000   # 14:30 and 20:00 UTC, seconds into the day (src/lib.rs)
CALLS = 0


def call(to, data):
    global CALLS
    CALLS += 1
    body = json.dumps({"jsonrpc": "2.0", "id": 1, "method": "eth_call",
                       "params": [{"to": to, "data": data}, "latest"]}).encode()
    req = urllib.request.Request(RPC, body, {"content-type": "application/json",
                                             "user-agent": "curl/8"})
    for attempt in range(10):
        try:
            time.sleep(0.25)
            out = json.load(urllib.request.urlopen(req, timeout=25))
            break
        except urllib.error.HTTPError as e:
            if e.code != 429 or attempt == 9:
                raise
            time.sleep(min(60, 2 ** (attempt + 1)))   # the public RPC rate-limits bursts
    return None if "error" in out else out["result"]


def decode(res):
    h = res[2:]
    w = [int(h[i:i + 64], 16) for i in range(0, 320, 64)]
    return w[0], w[1], w[3]  # roundId, answer, updatedAt


def in_session(t):
    return (t // 86400 + 3) % 7 < 5 and OPEN <= t % 86400 < CLOSE


def fmt(t):
    return datetime.fromtimestamp(t, timezone.utc).strftime("%a %m-%d %H:%M")


def move(g):
    return g[3] / g[2] - 1


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--rounds", type=int, default=150)
    args = ap.parse_args()
    for symbol, feed in FEEDS.items():
        rid, _, _ = decode(call(feed, "0xfeaf968c"))
        phase, seq = rid >> 64, rid & ((1 << 64) - 1)
        rows = []
        for i in range(args.rounds):
            if seq - i < 1:
                break
            res = call(feed, "0x9a6fc8f5" + format((phase << 64) + seq - i, "064x"))
            if res and len(res) >= 322:
                _, answer, at = decode(res)
                if at:
                    rows.append((at, answer))
        rows.sort()
        gaps = [(a, b, pa, pb) for (a, pa), (b, pb) in zip(rows, rows[1:])]
        inside = [g for g in gaps
                  if in_session(g[0]) and in_session(g[1]) and g[1] // 86400 == g[0] // 86400]
        outside = [g for g in gaps if g not in inside]
        print("%s: %d prints over %.0f days (%s -> %s UTC), %d of them inside the session window"
              % (symbol, len(rows), (rows[-1][0] - rows[0][0]) / 86400, fmt(rows[0][0]), fmt(rows[-1][0]),
                 sum(1 for r in rows if in_session(r[0]))))
        if inside:
            longest = max(inside, key=lambda g: g[1] - g[0])
            top = max(inside, key=lambda g: abs(move(g)))
            print("  inside one session window: %d gaps, the longest %.0f min (from %s); the largest move "
                  "between two prints %+.2f%% (%s -> %s); over 1%%: %d"
                  % (len(inside), (longest[1] - longest[0]) / 60, fmt(longest[0]), 100 * move(top),
                     fmt(top[0]), fmt(top[1])[-5:], sum(1 for g in inside if abs(move(g)) > NARROW)))
        print("  across the window's edges or outside it: %d gaps; over 1%%: %d; the largest moves between two prints:"
              % (len(outside), sum(1 for g in outside if abs(move(g)) > NARROW)))
        for g in sorted(outside, key=lambda g: -abs(move(g)))[:5]:
            print("    %+.2f%%  %s -> %s  (%.1f h)" % (100 * move(g), fmt(g[0]), fmt(g[1]), (g[1] - g[0]) / 3600))
    print("\neth_calls: %d" % CALLS)


if __name__ == "__main__":
    main()
