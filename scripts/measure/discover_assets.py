# -*- coding: utf-8 -*-
"""Build the multi-asset deployment manifest (`assets.json`).

Input: `robinhood_tokens.json` (every ERC-20 on the explorer whose name ends
in "• Robinhood Token", imitations included). For each candidate this script
asks the chain the same questions `initialize` will: does the token expose
`oraclePaused()`, does it have 18 decimals, is there a Chainlink feed named
"Robinhood <SYMBOL>-USD", and which USDG pools (fee tiers 0.01/0.05/0.3/1%)
exist with what in-range liquidity. Anything that fails a question is listed
as not deployable, with the reason. Read-only, stdlib, ~6 RPC calls per token.

    python scripts/measure/discover_assets.py            # writes assets.json
    python scripts/measure/discover_assets.py --top 20

Selectors computed with ethers on the server, not recalled:
  getPool(address,address,uint24) 0x1698ee82   liquidity() 0x1a686502
  decimals() 0x313ce567   oraclePaused() 0x7706ba52   symbol() 0x95d89b41
"""
import argparse
import io
import json
import re
import sys
import time
import urllib.error
import urllib.request
from pathlib import Path

sys.stdout = io.TextIOWrapper(sys.stdout.buffer, encoding="utf-8", errors="replace", line_buffering=True)
RPC = "https://rpc.mainnet.chain.robinhood.com"
FACTORY = "0x1f7d7550b1b028f7571e69a784071f0205fd2efa"
USDG = "0x5fc5360d0400a0fd4f2af552add042d716f1d168"
FEEDS_URL = "https://reference-data-directory.vercel.app/feeds-robinhood-mainnet.json"
FEES = [100, 500, 3000, 10000]
HERE = Path(__file__).resolve().parent
CALLS = 0


def rpc(method, params):
    global CALLS
    CALLS += 1
    body = json.dumps({"jsonrpc": "2.0", "id": 1, "method": method, "params": params}).encode()
    req = urllib.request.Request(RPC, body, {"content-type": "application/json",
                                             "user-agent": "curl/8"})
    for attempt in range(6):
        try:
            time.sleep(0.2)
            out = json.load(urllib.request.urlopen(req, timeout=40))
            break
        except urllib.error.HTTPError as e:
            if e.code != 429 or attempt == 5:
                raise
            time.sleep(4 * (attempt + 1))
    if "error" in out:
        return None
    return out["result"]


def call(to, data):
    return rpc("eth_call", [{"to": to, "data": data}, "latest"])


def word(h, i):
    return int(h[2 + 64 * i: 2 + 64 * (i + 1)], 16)


def addr_word(a):
    return a[2:].lower().rjust(64, "0")


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--top", type=int, default=0)
    ap.add_argument("--out", default=str(HERE.parents[1] / "assets.json"))
    args = ap.parse_args()

    feeds = json.load(urllib.request.urlopen(urllib.request.Request(
        FEEDS_URL, headers={"user-agent": "Mozilla/5.0"}), timeout=60))
    feed_by_symbol = {}
    for f in feeds:
        # "Robinhood AAPL / USD" for most, "Robinhood SGOV-USD" for a few
        m = re.match(r"^Robinhood (\S+)\s*[-/]\s*USD$", f.get("name", ""))
        if m:
            feed_by_symbol[m.group(1).upper()] = f["proxyAddress"]
    print("feeds: %d" % len(feed_by_symbol))

    candidates = json.load(io.open(HERE / "robinhood_tokens.json", encoding="utf-8"))["candidates"]
    assets, rejected = [], []
    seen_symbols = set()
    for symbol, token, holders in candidates:
        token_l = token.lower()
        why = []
        paused = call(token, "0x7706ba52")
        if paused in (None, "0x"):
            why.append("no oraclePaused()")
        dec_raw = call(token, "0x313ce567")
        decimals = word(dec_raw, 0) if dec_raw and dec_raw != "0x" else None
        if decimals != 18:
            why.append("decimals %s" % decimals)
        feed = feed_by_symbol.get(symbol.upper())
        if not feed:
            why.append("no feed")
        pools = []
        for fee in FEES:
            res = call(FACTORY, "0x1698ee82" + addr_word(token) + addr_word(USDG) + format(fee, "064x"))
            if res and int(res, 16):
                pool = "0x" + res[-40:]
                liq_raw = call(pool, "0x1a686502")
                pools.append({"address": pool, "fee": fee,
                              "liquidity": int(liq_raw, 16) if liq_raw and liq_raw != "0x" else 0})
        pools.sort(key=lambda p: -p["liquidity"])
        if not pools:
            why.append("no USDG pool")
        elif pools[0]["liquidity"] == 0:
            why.append("pools empty")
        if symbol.upper() in seen_symbols and not why:
            why.append("duplicate symbol (imitation?)")
        rec = {"symbol": symbol, "stock": token, "holders": holders, "decimals": decimals,
               "feed": feed, "pools": pools, "deployable": not why, "why_not": why}
        if why:
            rejected.append(rec)
        else:
            seen_symbols.add(symbol.upper())
            assets.append(rec)
        print("  %-8s %-44s %s" % (symbol, token, "ok  L=%.3g pools=%d" % (pools[0]["liquidity"], len(pools))
                                                     if not why else "no: " + "; ".join(why)))

    assets.sort(key=lambda a: -a["pools"][0]["liquidity"])
    if args.top:
        assets = assets[:args.top]
    manifest = {
        "chainId": 4663, "quote": USDG, "factory": FACTORY, "generated": time.strftime("%Y-%m-%d %H:%M UTC", time.gmtime()),
        "defaults": {"liveMaxAge": 21600, "twapWindow": 1800, "maxDeviationBps": 1000,
                     "maxAnchorAge": 432000, "minLiquidity": "1/8 of the deepest pool's liquidity at deployment"},
        "assets": assets,
        "not_deployable": [{"symbol": r["symbol"], "stock": r["stock"], "why": r["why_not"]} for r in rejected],
    }
    io.open(args.out, "w", encoding="utf-8", newline="\n").write(json.dumps(manifest, indent=2))
    print("\ndeployable: %d, not: %d; wrote %s; rpc calls %d" % (len(assets), len(rejected), args.out, CALLS))
    print("\ntop 15 by liquidity:")
    for a in assets[:15]:
        print("  %-6s L=%.3g  pools %s" % (a["symbol"], a["pools"][0]["liquidity"],
                                          " ".join("%.2f%%" % (p["fee"] / 1e4) for p in a["pools"])))


if __name__ == "__main__":
    main()
