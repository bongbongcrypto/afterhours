# -*- coding: utf-8 -*-
"""Build the multi-asset deployment manifest (`assets.json`).

Input: `robinhood_tokens.json` (every ERC-20 on the explorer whose name ends
in "• Robinhood Token", imitations included). For each candidate this script
asks the chain what `initialize` and the price path will need, and what an
attacker would look for:

  genuine     the token is a BeaconProxy on the same beacon as AAPL (read from
              the EIP-1967 beacon slot; imitations have none)
  pause flag  `oraclePaused()` answers
  decimals    18
  feed        a Chainlink feed named "Robinhood <SYMBOL> / USD" (or "-USD"),
              with its on-chain `description()` copied for the deploy input
  pools       every USDG fee tier: in-range liquidity, observation
              cardinality, whether observe([1800, 0]) answers now, and the
              USDG it takes to move the price 2% inside the current range

"deployable" = initialize would pass today. "recommended" additionally needs
the primary (deepest) pool to hold cardinality >= window + 1 (so nobody can
make it unobservable by spamming writes) and >= $50k of 2% depth. Read-only,
stdlib, ~10 RPC calls per token.

    python scripts/measure/discover_assets.py            # writes assets.json

Selectors / slots computed with ethers on the server, not recalled:
  getPool(address,address,uint24) 0x1698ee82   liquidity() 0x1a686502
  decimals() 0x313ce567   oraclePaused() 0x7706ba52   description() 0x7284e416
  slot0() 0x3850c7bd   observe(uint32[]) 0x883bdbfd
  EIP-1967 beacon slot 0xa3f0ad74e5423aebfd80d3ef4346578335a9a72aeaee59ff6cb3582b35133d50
"""
import argparse
import io
import json
import math
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
AAPL = "0xaF3D76f1834A1d425780943C99Ea8A608f8a93f9"
BEACON_SLOT = "0xa3f0ad74e5423aebfd80d3ef4346578335a9a72aeaee59ff6cb3582b35133d50"
FEEDS_URL = "https://reference-data-directory.vercel.app/feeds-robinhood-mainnet.json"
FEES = [100, 500, 3000, 10000]
WINDOW = 1800
MIN_DEPTH_USD = 50_000
# Instances whose inputs were decided after discovery (DEPLOYMENTS.md): the
# manifest records what they deploy with, so it never disagrees with the docs.
INSTANCES = {
    "AAPL": {"pools": ["0xaae0d815ee56e4092a5e5c2911e676fea50b2d6d"], "min_liquidity": "50000000000000000",
             "why": "the AAPL instance: the 0.05% primary only (it keeps 1,801 observations, so no standby is ever read), floor 5e16"},
}
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


def beacon_of(token):
    raw = rpc("eth_getStorageAt", [token, BEACON_SLOT, "latest"])
    return ("0x" + raw[-40:]).lower() if raw and int(raw, 16) else None


def string_result(res):
    if not res or len(res) < 130:
        return None
    off = word(res, 0) // 32
    ln = word(res, off)
    return bytes.fromhex(res[2 + 64 * (off + 1): 2 + 64 * (off + 1) + ln * 2]).decode("utf-8", "replace")


def pool_facts(pool, stock):
    """liquidity, cardinality, observable now, USDG to move the price 2% in range."""
    liq_raw = call(pool, "0x1a686502")
    liquidity = int(liq_raw, 16) if liq_raw and liq_raw != "0x" else 0
    s0 = call(pool, "0x3850c7bd")
    sqrt_x96 = word(s0, 0) if s0 and len(s0) >= 2 + 64 * 5 else 0
    cardinality = word(s0, 3) if s0 and len(s0) >= 2 + 64 * 5 else 0
    obs = call(pool, "0x883bdbfd" + format(32, "064x") + format(2, "064x")
               + format(WINDOW, "064x") + format(0, "064x"))
    observable = bool(obs and obs != "0x")
    depth = 0.0
    if sqrt_x96 and liquidity:
        s = sqrt_x96 / 2 ** 96                      # sqrt(token1 raw / token0 raw)
        k = math.sqrt(1.02) - 1
        usdg_is_token0 = USDG < stock.lower()       # token0 is the lower address
        raw = liquidity * k / s if usdg_is_token0 else liquidity * k * s
        depth = raw / 1e6                           # USDG has 6 decimals
    return {"address": pool, "liquidity": liquidity, "cardinality": cardinality,
            "observable": observable, "depth_2pct_usd": round(depth)}


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--out", default=str(HERE.parents[1] / "assets.json"))
    args = ap.parse_args()

    feeds = json.load(urllib.request.urlopen(urllib.request.Request(
        FEEDS_URL, headers={"user-agent": "Mozilla/5.0"}), timeout=60))
    feed_by_symbol = {}
    for f in feeds:
        m = re.match(r"^Robinhood (\S+)\s*[-/]\s*USD$", f.get("name", ""))
        if m:
            feed_by_symbol[m.group(1).upper()] = f["proxyAddress"]
    genuine_beacon = beacon_of(AAPL)
    print("feeds: %d; genuine beacon (read from AAPL): %s" % (len(feed_by_symbol), genuine_beacon))
    assert genuine_beacon, "AAPL has no beacon: the reference itself is wrong"

    candidates = json.load(io.open(HERE / "robinhood_tokens.json", encoding="utf-8"))["candidates"]
    assets, rejected = [], []
    seen = set()
    for symbol, token, holders in candidates:
        why, notes = [], []
        if beacon_of(token) != genuine_beacon:
            why.append("not on the Robinhood token beacon")
        paused = call(token, "0x7706ba52")
        if paused in (None, "0x"):
            why.append("no oraclePaused()")
        dec_raw = call(token, "0x313ce567")
        decimals = word(dec_raw, 0) if dec_raw and dec_raw != "0x" else None
        if decimals != 18:
            why.append("decimals %s" % decimals)
        feed = feed_by_symbol.get(symbol.upper())
        description = string_result(call(feed, "0x7284e416")) if feed else None
        if not feed:
            why.append("no feed")
        pools = []
        for fee in FEES:
            res = call(FACTORY, "0x1698ee82" + addr_word(token) + addr_word(USDG) + format(fee, "064x"))
            if res and int(res, 16):
                facts = pool_facts("0x" + res[-40:], token)
                facts["fee"] = fee
                pools.append(facts)
        pools.sort(key=lambda p: -p["liquidity"])
        if not pools:
            why.append("no USDG pool")
        elif pools[0]["liquidity"] == 0:
            why.append("pools empty")
        elif not pools[0]["observable"]:
            why.append("deepest pool does not answer observe(1800) now")
        if symbol.upper() in seen and not why:
            why.append("duplicate symbol")
        rec = {"symbol": symbol, "stock": token, "holders": holders, "decimals": decimals,
               "feed": feed, "feed_description": description, "deployable": not why, "why_not": why}
        if not why:
            seen.add(symbol.upper())
            primary = pools[0]
            standbys = [p for p in pools[1:] if p["liquidity"] > 0][:2]
            if primary["cardinality"] < WINDOW + 1:
                notes.append("primary cardinality %d < %d: raise with increaseObservationCardinalityNext(%d) "
                             "before deploying (anyone can; gas only)" % (primary["cardinality"], WINDOW + 1, WINDOW + 1))
            if primary["depth_2pct_usd"] < MIN_DEPTH_USD:
                notes.append("primary 2%% depth $%d < $%d" % (primary["depth_2pct_usd"], MIN_DEPTH_USD))
            deploy = {
                "feed": feed, "expected_description": description,
                "pools": [primary["address"]] + [p["address"] for p in standbys],
                "stock": token, "min_liquidity": str(primary["liquidity"] // 8),
            }
            if symbol.upper() in INSTANCES:
                inst = INSTANCES[symbol.upper()]
                deploy.update({"pools": inst["pools"], "min_liquidity": inst["min_liquidity"], "instance": inst["why"]})
            rec.update({
                "pools": pools,
                "deploy": deploy,
                "recommended": not notes, "notes": notes,
            })
            assets.append(rec)
        else:
            rec["pools"] = pools
            rejected.append(rec)
        state = ("ok   2%%-depth $%-9s card %-5d %s" % (
            format(pools[0]["depth_2pct_usd"], ","), pools[0]["cardinality"],
            "recommended" if rec.get("recommended") else "; ".join(notes))
            if not why else "no: " + "; ".join(why))
        print("  %-8s %s" % (symbol, state))

    assets.sort(key=lambda a: -a["pools"][0]["depth_2pct_usd"])
    manifest = {
        "chainId": 4663, "quote": USDG, "factory": FACTORY,
        "generated": time.strftime("%Y-%m-%d %H:%M UTC", time.gmtime()),
        "genuine_beacon": genuine_beacon,
        "defaults": {"liveMaxAge": 21600, "twapWindow": WINDOW, "maxDeviationBps": 1000,
                     "maxAnchorAge": 432000, "heartbeat": 86400, "quietBandBps": 100,
                     "min_liquidity": "primary liquidity / 8 at discovery time",
                     "recommended": "primary cardinality >= %d and 2%% depth >= $%d" % (WINDOW + 1, MIN_DEPTH_USD)},
        "assets": assets,
        "not_deployable": [{"symbol": r["symbol"], "stock": r["stock"], "why": r["why_not"]} for r in rejected],
    }
    io.open(args.out, "w", encoding="utf-8", newline="\n").write(json.dumps(manifest, indent=2) + "\n")
    rec_n = sum(1 for a in assets if a["recommended"])
    print("\ndeployable %d (recommended %d), not deployable %d; wrote %s; rpc calls %d"
          % (len(assets), rec_n, len(rejected), args.out, CALLS))


if __name__ == "__main__":
    main()
