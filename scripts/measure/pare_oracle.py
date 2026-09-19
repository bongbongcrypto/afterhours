# -*- coding: utf-8 -*-
"""What does the one live lending market on Robinhood Chain (PARE pSPY/USDG on
Morpho, custom oracle) actually use for a price while the Chainlink SPY feed
is silent? Read-only, stdlib. Selectors computed with ethers on the server:
  price() 0xa035b1fe   latestRoundData() 0xfeaf968c   decimals() 0x313ce567
  description() 0x7284e416   BASE_FEED_1() 0xf50a4718   QUOTE_FEED_1() 0x56095e11
  SCALE_FACTOR() 0xce4b5bbe   aggregator() 0x245a7bfc   latestAnswer() 0x50d25bcd
"""
import io
import json
import sys
import time
import urllib.request
from datetime import datetime, timezone

sys.stdout = io.TextIOWrapper(sys.stdout.buffer, encoding="utf-8", errors="replace")
RPC = "https://rpc.mainnet.chain.robinhood.com"
ORACLE = "0xc2414099151326C5d238B9F00609f1a12283B723"
SPY_FEED = "0x319724394D3A0e3669269846abE664Cd621f9f6A"


def call(to, data):
    time.sleep(0.3)
    body = json.dumps({"jsonrpc": "2.0", "id": 1, "method": "eth_call",
                       "params": [{"to": to, "data": data}, "latest"]}).encode()
    req = urllib.request.Request(RPC, body, {"content-type": "application/json",
                                             "user-agent": "curl/8"})
    out = json.load(urllib.request.urlopen(req, timeout=40))
    return out.get("result"), out.get("error")


def show(label, to, sel):
    res, err = call(to, sel)
    if err or not res or res == "0x":
        print("  %-14s -> (no answer: %s)" % (label, (err or {}).get("message", "empty")[:50]))
        return None
    print("  %-14s -> %s" % (label, res if len(res) <= 66 else res[:66] + "..."))
    return res


print("PARE oracle %s" % ORACLE)
p = show("price()", ORACLE, "0xa035b1fe")
show("BASE_FEED_1()", ORACLE, "0xf50a4718")
show("QUOTE_FEED_1()", ORACLE, "0x56095e11")
show("SCALE_FACTOR()", ORACLE, "0xce4b5bbe")
show("aggregator()", ORACLE, "0x245a7bfc")
show("description()", ORACLE, "0x7284e416")
show("decimals()", ORACLE, "0x313ce567")
show("latestAnswer()", ORACLE, "0x50d25bcd")

r, _ = call(SPY_FEED, "0xfeaf968c")
ans = int(r[2 + 64:2 + 128], 16)
at = int(r[2 + 192:2 + 256], 16)
now = datetime.now(timezone.utc)
print("\nChainlink SPY feed: %.2f, last print %s UTC (%.1f h ago)"
      % (ans / 1e8, datetime.fromtimestamp(at, timezone.utc).strftime("%a %H:%M"),
         (now.timestamp() - at) / 3600))
if p:
    v = int(p, 16)
    print("oracle price() raw = %d" % v)
    for scale in (36, 24, 18, 8):
        print("  / 1e%d = %.6f" % (scale, v / 10 ** scale))
    print("  feed / 1e8 = %.6f" % (ans / 1e8))
