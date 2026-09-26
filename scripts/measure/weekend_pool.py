# -*- coding: utf-8 -*-
"""Saturday snapshot: the on-chain AAPL/USDG pool on Robinhood Chain against
the Chainlink AAPL feed's last (Friday) print. Read-only, stdlib only.

Selectors computed with ethers on the server, not recalled:
  getPool(address,address,uint24)  0x1698ee82
  slot0()                          0x3850c7bd
  liquidity()                      0x1a686502
  balanceOf(address)               0x70a08231
  latestRoundData()                0xfeaf968c
"""
import io
import sys
from datetime import datetime, timezone
from pathlib import Path

sys.stdout = io.TextIOWrapper(sys.stdout.buffer, encoding="utf-8", errors="replace")
sys.path.insert(0, str(Path(__file__).resolve().parent))
from jsonrpc import post  # noqa: E402  (backs off on HTTP 429)
RPC = "https://rpc.mainnet.chain.robinhood.com"
AAPL = "0xaF3D76f1834A1d425780943C99Ea8A608f8a93f9"
USDG = "0x5fc5360D0400a0Fd4f2af552ADD042D716F1d168"
AAPL_FEED = "0x6B22A786bAa607d76728168703a39Ea9C99f2cD0"
FACTORIES = ["0x1f7d7550b1b028f7571e69a784071f0205fd2efa",
             "0x8876789976decbfcbbbe364623c63652db8c0904"]
FEES = [100, 500, 3000, 10000]
CALLS = 0


def call(to, data):
    global CALLS
    CALLS += 1
    return post(RPC, {"jsonrpc": "2.0", "id": 1, "method": "eth_call",
                      "params": [{"to": to, "data": data}, "latest"]}).get("result")


def addr(a):
    return a[2:].lower().rjust(64, "0")


def u(n):
    return format(n, "064x")


# feed
r = call(AAPL_FEED, "0xfeaf968c")[2:]
feed_price = int(r[64:128], 16) / 1e8
feed_at = int(r[192:256], 16)
now = datetime.now(timezone.utc)
age_h = (now.timestamp() - feed_at) / 3600
print("AAPL feed: $%.2f, printed %s, now %s -> %.1f h old"
      % (feed_price, datetime.fromtimestamp(feed_at, timezone.utc).strftime("%a %H:%M UTC"),
         now.strftime("%a %H:%M UTC"), age_h))

found = []
for f in FACTORIES:
    for fee in FEES:
        res = call(f, "0x1698ee82" + addr(AAPL) + addr(USDG) + u(fee))
        if res and int(res, 16) != 0:
            found.append((f, fee, "0x" + res[-40:]))
if not found:
    print("no AAPL/USDG v3 pool on either factory")
    sys.exit(0)

for f, fee, pool in found:
    s0 = call(pool, "0x3850c7bd")[2:]
    sqrt_p = int(s0[:64], 16)
    tick = int(s0[64:128], 16)
    tick = tick - (1 << 256) if tick >= 1 << 255 else tick
    liq = int(call(pool, "0x1a686502"), 16)
    bal_aapl = int(call(AAPL, "0x70a08231" + addr(pool)), 16) / 1e18
    bal_usdg = int(call(USDG, "0x70a08231" + addr(pool)), 16) / 1e6
    # token0 is the lower address; AAPL (0xaF..) > USDG (0x5f..) so token0 = USDG
    # price of token1 in token0 = (sqrtP/2^96)^2 ; token0 USDG 6 dec, token1 AAPL 18 dec
    ratio = (sqrt_p / 2 ** 96) ** 2           # token1 raw per token0 raw = AAPL-wei per USDG-unit
    aapl_in_usdg = 1e12 / ratio               # USDG per AAPL token (1e18 wei / 1e6 units)
    dev = (aapl_in_usdg / feed_price - 1) * 100
    print("\npool fee %.2f%% (%s) factory %s" % (fee / 1e4, pool, f[:10]))
    print("  reserves: %.4f AAPL + %.2f USDG   (everything anyone could ever pull out)"
          % (bal_aapl, bal_usdg))
    print("  pool mid price: $%.2f  vs feed $%.2f  -> %+.2f%%" % (aapl_in_usdg, feed_price, dev))
    print("  in-range liquidity L = %d" % liq)
print("\neth_calls made: %d" % CALLS)
