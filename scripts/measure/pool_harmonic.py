# -*- coding: utf-8 -*-
"""The liquidity statistic AfterHours actually compares against its floor:
the harmonic-mean in-range liquidity over the TWAP window, from the pool's
own observe() (Uniswap OracleLibrary.consult), next to the spot liquidity().
Read-only, stdlib. Selectors computed with ethers on the server:
  observe(uint32[]) 0x883bdbfd   liquidity() 0x1a686502   oraclePaused() 0x7706ba52
  description() 0x7284e416
"""
import io
import json
import sys
import time
import urllib.request

sys.stdout = io.TextIOWrapper(sys.stdout.buffer, encoding="utf-8", errors="replace")
RPC = "https://rpc.mainnet.chain.robinhood.com"
POOL = "0xaae0d815ee56e4092a5e5c2911e676fea50b2d6d"   # AAPL/USDG 0.05%
STOCK = "0xaF3D76f1834A1d425780943C99Ea8A608f8a93f9"
FEED = "0x6B22A786bAa607d76728168703a39Ea9C99f2cD0"
WINDOWS = [1800, 600, 60]


def call(to, data):
    time.sleep(0.3)
    body = json.dumps({"jsonrpc": "2.0", "id": 1, "method": "eth_call",
                       "params": [{"to": to, "data": data}, "latest"]}).encode()
    req = urllib.request.Request(RPC, body, {"content-type": "application/json",
                                             "user-agent": "curl/8"})
    out = json.load(urllib.request.urlopen(req, timeout=40))
    if "error" in out:
        raise RuntimeError(out["error"])
    return out["result"]


def word(h, i):
    return int(h[2 + 64 * i: 2 + 64 * (i + 1)], 16)


spot = int(call(POOL, "0x1a686502"), 16)
print("spot liquidity()            = %.4e" % spot)
harmonic_1800 = 0
for w in WINDOWS:
    data = "0x883bdbfd" + format(32, "064x") + format(2, "064x") + format(w, "064x") + format(0, "064x")
    r = call(POOL, data)
    off1 = word(r, 1) // 32
    spl = [word(r, off1 + 1 + i) for i in range(word(r, off1))]
    delta = (spl[1] - spl[0]) % (1 << 160)
    harmonic = (w << 128) // delta if delta else 0
    if w == 1800:
        harmonic_1800 = harmonic
    print("harmonic over %5d s        = %.4e   (%.2fx spot)" % (w, harmonic, harmonic / spot if spot else 0))
r = call(FEED, "0x7284e416")
off = word(r, 0) // 32
ln = word(r, off)
print("feed description()          = %r" % bytes.fromhex(r[2 + 64 * (off + 1): 2 + 64 * (off + 1) + ln * 2]).decode())
print("stock oraclePaused()        = %s" % bool(int(call(STOCK, "0x7706ba52"), 16)))
FLOOR = float(sys.argv[1]) if len(sys.argv) > 1 else 5e16   # the AAPL instance's minLiquidity (DEPLOYMENTS.md)
print("floor %.0e vs harmonic(1800 s): %.1fx margin" % (FLOOR, harmonic_1800 / FLOOR))
