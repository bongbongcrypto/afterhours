# -*- coding: utf-8 -*-
"""Was the "52h weekend silence" real, or does the feed update on weekends?
Walk the AAPL feed's last ~60 rounds and print the gap between each update, so
the actual weekend cadence is visible instead of assumed. Read-only, stdlib.

  latestRoundData()     0xfeaf968c
  getRoundData(uint80)  0x9a6fc8f5
"""
import io
import sys
from datetime import datetime, timezone
from pathlib import Path

sys.stdout = io.TextIOWrapper(sys.stdout.buffer, encoding="utf-8", errors="replace")
sys.path.insert(0, str(Path(__file__).resolve().parent))
from jsonrpc import post  # noqa: E402  (backs off on HTTP 429)
RPC = "https://rpc.mainnet.chain.robinhood.com"
FEED = "0x6B22A786bAa607d76728168703a39Ea9C99f2cD0"  # AAPL/USD
CALLS = 0


def call(data):
    global CALLS
    CALLS += 1
    return post(RPC, {"jsonrpc": "2.0", "id": 1, "method": "eth_call",
                      "params": [{"to": FEED, "data": data}, "latest"]}).get("result")


def dec(res):
    h = res[2:]
    w = [int(h[i:i + 64], 16) for i in range(0, 320, 64)]
    return w[0], w[1], w[3]  # roundId, answer, updatedAt


rid, _, _ = dec(call("0xfeaf968c"))
phase, seq = rid >> 64, rid & ((1 << 64) - 1)
print("latest round %d (phase %d, seq %d)" % (rid, phase, seq))

rows = []
for i in range(60):
    s = seq - i
    if s < 1:
        break
    res = call("0x9a6fc8f5" + format((phase << 64) + s, "064x"))
    if not res or len(res) < 322:
        continue
    _, ans, at = dec(res)
    if at:
        rows.append((at, ans / 1e8))

rows.sort()
print("\n  when (UTC)          price     gap since previous")
prev = None
big = []
for at, px in rows:
    dt = datetime.fromtimestamp(at, timezone.utc)
    gap = "" if prev is None else "%.1f h" % ((at - prev) / 3600)
    if prev and (at - prev) / 3600 > 12:
        big.append((datetime.fromtimestamp(prev, timezone.utc), dt, (at - prev) / 3600))
    print("  %s  $%7.2f   %s" % (dt.strftime("%a %m-%d %H:%M"), px, gap))
    prev = at

print("\ngaps over 12h (weekend/overnight closures):")
for a, b, h in big:
    print("  %s -> %s  = %.1f h silent" % (a.strftime("%a %m-%d %H:%M"),
                                           b.strftime("%a %m-%d %H:%M"), h))
print("\neth_calls: %d" % CALLS)
