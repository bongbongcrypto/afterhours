# -*- coding: utf-8 -*-
"""Read a deployed AfterHours next to its raw sources. Stdlib only, read-only.

    python scripts/probe.py --oracle 0x... [--rpc URL] [--watch 300]

Prints, side by side: the Chainlink feed's last print and age, the pool's
30-minute TWAP (computed here from observe()), and what AfterHours returns
(session, answer, band clamping). During a weekend the feed line goes stale
while the AfterHours line keeps moving; that is the demo.

Selectors computed with ethers, not recalled:
  latestRoundData() 0xfeaf968c   decimals() 0x313ce567   description() 0x7284e416
  observe(uint32[]) 0x883bdbfd
  price() 0xa035b1fe   state() 0xc19d93fb   config() 0x79502c55
"""
import argparse
import io
import json
import sys
import time
import urllib.request
from datetime import datetime, timezone

sys.stdout = io.TextIOWrapper(sys.stdout.buffer, encoding="utf-8", errors="replace")

SESSIONS = {0: "LIVE_FEED", 1: "ONCHAIN_TWAP", 2: "PAUSED", 3: "NO_DATA"}
REASONS = {0: "", 1: "feed invalid", 2: "pool too thin", 3: "twap unavailable",
           4: "last print older than maxAnchorAge"}

# Function selectors used below, all computed with ethers.id() from the
# Solidity signatures (AfterHours ones from the `cargo stylus export-abi` output).
SELECTORS = {
    "latestRoundData()": "0xfeaf968c",
    "decimals()": "0x313ce567",
    "description()": "0x7284e416",
    "observe(uint32[])": "0x883bdbfd",
    "price()": "0xa035b1fe",
    # AfterHours (ethers.id on the ABI exported by `cargo stylus export-abi`)
    "state()": "0xc19d93fb",
    "config()": "0x79502c55",
}


def rpc_call(rpc, to, data):
    body = json.dumps({"jsonrpc": "2.0", "id": 1, "method": "eth_call",
                       "params": [{"to": to, "data": data}, "latest"]}).encode()
    req = urllib.request.Request(rpc, body, {"content-type": "application/json",
                                             "user-agent": "curl/8"})
    out = json.load(urllib.request.urlopen(req, timeout=40))
    if "error" in out:
        return None, out["error"].get("message", "")
    return out["result"], None


def word(h, i):
    return int(h[2 + 64 * i: 2 + 64 * (i + 1)], 16)


def signed(n, bits=256):
    return n - (1 << bits) if n >= 1 << (bits - 1) else n


def ts(t):
    return datetime.fromtimestamp(t, timezone.utc).strftime("%a %m-%d %H:%M:%S UTC")


def twap_from_pool(rpc, pool, window, stock_is_token0, stock_dec, quote_dec, feed_dec):
    data = ("0x883bdbfd" + format(32, "064x") + format(2, "064x")
            + format(window, "064x") + format(0, "064x"))
    res, err = rpc_call(rpc, pool, data)
    if err or not res:
        return None, err
    # returns (int56[] tickCumulatives, uint160[] ...): offsets then arrays
    off0 = word(res, 0) // 32
    n = word(res, off0)
    cum = [signed(word(res, off0 + 1 + i)) for i in range(n)]
    delta = cum[1] - cum[0]
    mean = delta // window          # Python floors toward -inf already
    ratio = 1.0001 ** mean          # token1 per token0, raw
    if stock_is_token0:
        price = ratio * 10 ** (stock_dec + feed_dec) / 10 ** quote_dec
    else:
        price = 10 ** (stock_dec + feed_dec) / (ratio * 10 ** quote_dec)
    return int(price), None


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--oracle", required=True)
    ap.add_argument("--rpc", default="https://rpc.mainnet.chain.robinhood.com")
    ap.add_argument("--watch", type=int, default=0, help="repeat every N seconds")
    args = ap.parse_args()

    cfg, err = rpc_call(args.rpc, args.oracle, SELECTORS["config()"])
    if err:
        sys.exit("config() failed: %s" % err)
    initialized = bool(word(cfg, 0))
    feed = "0x" + cfg[2 + 64 * 2 + 24: 2 + 64 * 3]
    pool = "0x" + cfg[2 + 64 * 3 + 24: 2 + 64 * 4]
    stock_is_token0 = bool(word(cfg, 6))
    feed_dec, stock_dec, quote_dec = word(cfg, 7), word(cfg, 8), word(cfg, 9)
    live_max_age, twap_window, dev_bps = word(cfg, 10), word(cfg, 11), word(cfg, 12)
    min_liq, max_anchor = word(cfg, 13), word(cfg, 14)
    print("AfterHours %s  initialized=%s  feed=%s  pool=%s" % (args.oracle, initialized, feed, pool))
    print("  liveMaxAge=%ds twapWindow=%ds band=%.1f%% minLiquidity=%.3g maxAnchorAge=%ds  decimals feed/stock/quote=%d/%d/%d\n"
          % (live_max_age, twap_window, dev_bps / 100, min_liq, max_anchor, feed_dec, stock_dec, quote_dec))

    while True:
        now = int(time.time())
        r, err = rpc_call(args.rpc, feed, SELECTORS["latestRoundData()"])
        feed_answer = word(r, 1) / 10 ** feed_dec
        feed_at = word(r, 3)
        age_h = (now - feed_at) / 3600
        twap, terr = twap_from_pool(args.rpc, pool, twap_window, stock_is_token0,
                                    stock_dec, quote_dec, feed_dec)
        s, serr = rpc_call(args.rpc, args.oracle, SELECTORS["state()"])
        print("[%s]" % ts(now))
        print("  Chainlink : $%.4f  printed %s  (%.1f h ago)" % (feed_answer, ts(feed_at), age_h))
        if twap is not None:
            print("  pool TWAP : $%.4f  (%d s window, computed off-chain)" % (twap / 10 ** feed_dec, twap_window))
        else:
            print("  pool TWAP : unavailable (%s)" % terr)
        if serr or not s:
            print("  AfterHours: state() reverted: %s" % serr)
        else:
            session, reason = word(s, 0), word(s, 1)
            answer, fa, fu, tw, liq, clamped = (word(s, 2), word(s, 3), word(s, 4), word(s, 5),
                                                word(s, 6), bool(word(s, 7)))
            line = "  AfterHours: %s" % SESSIONS.get(session, session)
            if session in (0, 1):
                line += "  answer $%.4f" % (answer / 10 ** feed_dec)
            if session == 1:
                line += "  (twap $%.4f, %s, window liquidity %.3g)" % (
                    tw / 10 ** feed_dec, "CLAMPED to band" if clamped else "inside band", liq)
            if session == 3:
                line += "  (%s)" % REASONS.get(reason, reason)
            print(line)
            p, perr = rpc_call(args.rpc, args.oracle, SELECTORS["price()"])
            if perr:
                print("  Morpho price(): reverted (%s)" % perr[:60])
            else:
                scale = 36 + quote_dec - stock_dec - feed_dec
                print("  Morpho price(): %d  (= answer x 1e%d)" % (word(p, 0), scale))
        if not args.watch:
            break
        print()
        time.sleep(args.watch)


if __name__ == "__main__":
    main()
