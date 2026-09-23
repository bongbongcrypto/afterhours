# -*- coding: utf-8 -*-
"""Every Morpho Blue market on Robinhood Chain mainnet, and how the ones
lending against tokenized stocks are priced.

For each CreateMarket event: loan and collateral token, oracle, LLTV, and the
market's current supply and borrow. Collateral counts as a Robinhood stock
token when it is on the same token beacon as AAPL (the genuineness test of
discover_assets.py). For stock-collateral markets with money in them the
oracle is probed: its feeds (Morpho's standard ChainlinkOracleV2 getters),
any staleness bound it exposes, and whether price() answers right now while
the underlying feed is silent.

Read-only, stdlib, JSON-RPC batches. Selectors come from keccak.py, which
checks itself against selectors observed on-chain.

    python scripts/measure/morpho_markets.py [--json out.json]
"""
import argparse
import io
import json
import sys
import time
import urllib.error
import urllib.request
from datetime import datetime, timezone
from pathlib import Path

sys.stdout = io.TextIOWrapper(sys.stdout.buffer, encoding="utf-8", errors="replace", line_buffering=True)
sys.path.insert(0, str(Path(__file__).resolve().parent))
from keccak import selector, topic  # noqa: E402

RPC = "https://rpc.mainnet.chain.robinhood.com"
MORPHO = "0x9D53d5E3bd5E8d4Cbfa6DB1ca238AEA02E651010"
AAPL = "0xaF3D76f1834A1d425780943C99Ea8A608f8a93f9"
BEACON_SLOT = "0xa3f0ad74e5423aebfd80d3ef4346578335a9a72aeaee59ff6cb3582b35133d50"
CALLS = 0


def post(body):
    global CALLS
    req = urllib.request.Request(RPC, json.dumps(body).encode(), {"content-type": "application/json", "user-agent": "curl/8"})
    for attempt in range(8):
        try:
            time.sleep(0.4)
            CALLS += len(body) if isinstance(body, list) else 1
            return json.load(urllib.request.urlopen(req, timeout=120))
        except urllib.error.HTTPError as e:
            if e.code != 429 or attempt == 7:
                raise
            time.sleep(5 * (attempt + 1))


def batch(calls, size=25):
    """calls: list of (method, params). Returns results in order (None on error)."""
    out = []
    for s in range(0, len(calls), size):
        chunk = calls[s:s + size]
        res = post([{"jsonrpc": "2.0", "id": i, "method": m, "params": p} for i, (m, p) in enumerate(chunk)])
        by = {r["id"]: r for r in res}
        out += [by.get(i, {}).get("result") for i in range(len(chunk))]
    return out


def eth_call(to, data):
    return ("eth_call", [{"to": to, "data": data}, "latest"])


def words(h):
    h = (h or "0x")[2:]
    return [int(h[i:i + 64], 16) for i in range(0, len(h) - 63, 64)]


def addr(w):
    return "0x%040x" % w


def text(res):
    try:
        b = bytes.fromhex(res[2:])
        if len(b) == 32:
            return b.rstrip(b"\0").decode()
        n = int.from_bytes(b[32:64], "big")
        return b[64:64 + n].decode()
    except Exception:  # noqa: BLE001
        return None


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--json")
    a = ap.parse_args()

    logs = post({"jsonrpc": "2.0", "id": 1, "method": "eth_getLogs", "params": [{
        "address": MORPHO, "topics": [topic("CreateMarket(bytes32,(address,address,address,address,uint256))")],
        "fromBlock": "0x0", "toBlock": "latest"}]})["result"]
    markets = []
    for lg in logs:
        w = words(lg["data"])
        markets.append({"id": lg["topics"][1], "block": int(lg["blockNumber"], 16), "loan": addr(w[0]),
                        "collateral": addr(w[1]), "oracle": addr(w[2]), "irm": addr(w[3]), "lltv": w[4] / 1e18})
    print("markets created on Morpho Blue %s: %d" % (MORPHO, len(markets)))

    tokens = sorted({m["loan"] for m in markets} | {m["collateral"] for m in markets})
    sym = batch([eth_call(t, selector("symbol()")) for t in tokens])
    dec = batch([eth_call(t, selector("decimals()")) for t in tokens])
    beacons = batch([("eth_getStorageAt", [t, BEACON_SLOT, "latest"]) for t in tokens + [AAPL]])
    genuine = beacons[-1]
    info = {}
    for t, s, d, b in zip(tokens, sym, dec, beacons[:-1]):
        info[t] = {"symbol": text(s) if s else None, "decimals": words(d)[0] if d and d != "0x" else None,
                   "stock": bool(b and genuine and int(b, 16) and b == genuine)}

    state = batch([eth_call(MORPHO, selector("market(bytes32)") + m["id"][2:]) for m in markets])
    for m, s in zip(markets, state):
        w = words(s)
        m["supply"], m["borrow"], m["lastUpdate"] = (w[0], w[2], w[4]) if len(w) >= 6 else (0, 0, 0)
        m["loanSymbol"] = info[m["loan"]]["symbol"]
        m["collSymbol"] = info[m["collateral"]]["symbol"]
        m["loanDecimals"] = info[m["loan"]]["decimals"] or 18
        m["stockCollateral"] = info[m["collateral"]]["stock"]

    live_stock = [m for m in markets if m["stockCollateral"] and m["supply"] > 0]
    print("markets with a Robinhood stock token as collateral: %d (%d with money supplied)"
          % (sum(1 for m in markets if m["stockCollateral"]), len(live_stock)))

    # probe the oracles of the stock markets that hold money
    probes = ["price()", "BASE_FEED_1()", "BASE_FEED_2()", "QUOTE_FEED_1()", "MIN_FEED_AGE()", "maxFeedAge()",
              "MAX_FEED_AGE()", "MAX_STALENESS()", "stalenessThreshold()"]
    oracles = sorted({m["oracle"] for m in live_stock})
    res = batch([eth_call(o, selector(p)) for o in oracles for p in probes])
    oinfo = {}
    for i, o in enumerate(oracles):
        r = dict(zip(probes, res[i * len(probes):(i + 1) * len(probes)]))
        oinfo[o] = {"price_answers_now": bool(r["price()"] and r["price()"] != "0x"),
                    "base_feed": addr(words(r["BASE_FEED_1()"])[0]) if r["BASE_FEED_1()"] and r["BASE_FEED_1()"] != "0x" else None,
                    "staleness_bound_s": next((words(r[p])[0] for p in probes[4:] if r[p] and r[p] != "0x"), None)}
    feeds = sorted({v["base_feed"] for v in oinfo.values() if v["base_feed"] and int(v["base_feed"], 16)})
    fres = batch([eth_call(f, selector("latestRoundData()")) for f in feeds] + [eth_call(f, selector("description()")) for f in feeds])
    now = time.time()
    finfo = {}
    for f, r, d in zip(feeds, fres[:len(feeds)], fres[len(feeds):]):
        w = words(r)
        finfo[f] = {"description": text(d) if d else None, "age_h": (now - w[3]) / 3600 if len(w) >= 5 else None}

    tot_s = tot_b = 0.0
    print("\n%-6s %-10s %-8s %6s %14s %14s  %-44s %s" % ("block", "collateral", "loan", "lltv", "supplied", "borrowed", "oracle / feed", "price() now | bound"))
    for m in sorted(live_stock, key=lambda m: -m["supply"]):
        k = 10 ** m["loanDecimals"]
        s, b = m["supply"] / k, m["borrow"] / k
        tot_s += s
        tot_b += b
        o = oinfo[m["oracle"]]
        feed = finfo.get(o["base_feed"] or "", {})
        m.update({"oracle_info": o, "feed_info": feed})
        print("%-6s %-10s %-8s %6.3f %14s %14s  %-44s %s | %s" % (
            str(m["block"])[-6:], (m["collSymbol"] or "?")[:10], (m["loanSymbol"] or "?")[:8], m["lltv"],
            format(round(s), ","), format(round(b), ","),
            "%s %s %s" % (m["oracle"][:10], (feed.get("description") or "")[:22],
                          ("%.1fh old" % feed["age_h"]) if feed.get("age_h") is not None else ""),
            "answers" if o["price_answers_now"] else "reverts",
            ("%.1f d" % (o["staleness_bound_s"] / 86400)) if o["staleness_bound_s"] else "none exposed"))
    print("\nstock-collateral markets with money: %d; supplied %s, borrowed %s (loan-token units, mostly USDG)"
          % (len(live_stock), format(round(tot_s), ","), format(round(tot_b), ",")))
    print("rpc calls %d; %s UTC" % (CALLS, datetime.now(timezone.utc).strftime("%Y-%m-%d %H:%M")))
    if a.json:
        io.open(a.json, "w", encoding="utf-8").write(json.dumps({"markets": markets, "oracles": oinfo, "feeds": finfo}, indent=1))


if __name__ == "__main__":
    main()
