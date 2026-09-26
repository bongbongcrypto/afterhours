# -*- coding: utf-8 -*-
"""What PARE's pSPY/USDG Morpho oracle on Robinhood Chain is configured to
accept, read from its own public getters (its source is verified on the
explorer as PareMorphoOracle): the oldest Chainlink print it will use, the
pool TWAP window of its PT leg and its sequencer-uptime feed. Then its live
price next to the Chainlink SPY feed. Read-only, stdlib; selectors from
keccak.py.
"""
import io
import sys
from datetime import datetime, timezone
from pathlib import Path

sys.stdout = io.TextIOWrapper(sys.stdout.buffer, encoding="utf-8", errors="replace")
sys.path.insert(0, str(Path(__file__).resolve().parent))
from jsonrpc import post  # noqa: E402  (backs off on HTTP 429)
from keccak import selector  # noqa: E402

RPC = "https://rpc.mainnet.chain.robinhood.com"
ORACLE = "0xc2414099151326C5d238B9F00609f1a12283B723"
SPY_FEED = "0x319724394D3A0e3669269846abE664Cd621f9f6A"


def call(to, data):
    out = post(RPC, {"jsonrpc": "2.0", "id": 1, "method": "eth_call",
                     "params": [{"to": to, "data": data}, "latest"]}, pause=0.3)
    return out.get("result"), out.get("error")


def word(sig):
    res, err = call(ORACLE, selector(sig))
    if err or not res or res == "0x":
        print("  %-16s (no answer: %s)" % (sig, (err or {}).get("message", "empty")[:50]))
        return None
    return int(res[2:66], 16)


print("PARE oracle %s" % ORACLE)
for sig in ("MIN_FEED_AGE()", "maxFeedAge()", "twapWindow()", "sequencerGrace()"):
    v = word(sig)
    if v is not None:
        print("  %-16s %d s (%.2f days)" % (sig, v, v / 86400))
for sig in ("feed()", "pool()", "sequencerFeed()"):
    v = word(sig)
    if v is not None:
        print("  %-16s 0x%040x%s" % (sig, v, "  (none)" if v == 0 else ""))
p = word("price()")

r, _ = call(SPY_FEED, selector("latestRoundData()"))
ans = int(r[2 + 64:2 + 128], 16)
at = int(r[2 + 192:2 + 256], 16)
now = datetime.now(timezone.utc)
print("\nChainlink SPY feed: %.2f, last print %s UTC (%.1f h ago)"
      % (ans / 1e8, datetime.fromtimestamp(at, timezone.utc).strftime("%a %H:%M"),
         (now.timestamp() - at) / 3600))
if p is not None:
    print("oracle price() raw = %d  (/1e24 = %.6f USDG per pSPY unit)" % (p, p / 1e24))
