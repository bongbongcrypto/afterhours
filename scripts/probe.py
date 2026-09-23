# -*- coding: utf-8 -*-
"""Read a deployed AfterHours next to its raw sources. Stdlib only, read-only.

    python scripts/probe.py --oracle 0x... [--rpc URL] [--watch 300]

Prints, side by side: the Chainlink feed's last print and age, the primary
pool's price per share (the median of three 10-minute averages, computed here
from observe() and the token's uiMultiplier), the band that applies at the
print's age and the hour, and what AfterHours returns (session, answer, band
clamping). During a weekend the feed line goes stale while the AfterHours line
keeps moving; that is the demo.

Selectors computed with keccak (scripts/measure/keccak.py checks itself), not recalled:
  latestRoundData() 0xfeaf968c   decimals() 0x313ce567   description() 0x7284e416
  observe(uint32[]) 0x883bdbfd   uiMultiplier() 0xa60bf13d
  price() 0xa035b1fe   state() 0xc19d93fb   config() 0x79502c55   quietTier() 0x41fcfef0
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
           4: "last print older than maxAnchorAge", 5: "share multiplier changed since the last print",
           6: "answer too large for Morpho's scale (price() only)"}

# Function selectors used below, all computed with ethers.id() from the
# Solidity signatures (AfterHours ones from the `cargo stylus export-abi` output).
SELECTORS = {
    "latestRoundData()": "0xfeaf968c",
    "decimals()": "0x313ce567",
    "description()": "0x7284e416",
    "observe(uint32[])": "0x883bdbfd",
    "uiMultiplier()": "0xa60bf13d",
    "price()": "0xa035b1fe",
    # AfterHours (ethers.id on the ABI exported by `cargo stylus export-abi`)
    "state()": "0xc19d93fb",
    "config()": "0x79502c55",
    "quietTier()": "0x41fcfef0",
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


def points(window):
    """The contract's observe() points: three sub-windows, oldest first."""
    sub = window // 3
    return [window, 2 * sub, sub, 0]


def twap_from_pool(rpc, pool, window, stock_is_token0, stock_dec, quote_dec, feed_dec, multiplier):
    """(median sub-window price per share in feed decimals, median sub-window
    liquidity) or (None, error). Float arithmetic: a cross-check, not the
    contract's integer path."""
    pts = points(window)
    data = ("0x883bdbfd" + format(32, "064x") + format(len(pts), "064x")
            + "".join(format(s, "064x") for s in pts))
    res, err = rpc_call(rpc, pool, data)
    if err or not res:
        return None, err
    # returns (int56[] tickCumulatives, uint160[] secondsPerLiquidity): offsets then arrays
    off0, off1 = word(res, 0) // 32, word(res, 1) // 32
    n = word(res, off0)
    cum = [signed(word(res, off0 + 1 + i)) for i in range(n)]
    spl = [word(res, off1 + 1 + i) for i in range(n)]
    # each sub-window's mean tick (Python floors toward -inf, like the contract), then the median
    means = sorted((cum[k + 1] - cum[k]) // (pts[k] - pts[k + 1]) for k in range(3))
    ratio = 1.0001 ** means[1]            # token1 per token0, raw
    if stock_is_token0:
        raw = ratio * 10 ** (stock_dec + feed_dec) / 10 ** quote_dec
    else:
        raw = 10 ** (stock_dec + feed_dec) / (ratio * 10 ** quote_dec)
    liq = []
    for k in range(3):
        d = (spl[k + 1] - spl[k]) % (1 << 160)
        liq.append(((pts[k] - pts[k + 1]) << 128) // d if d else 0)
    return (int(raw * 10 ** 18 / multiplier), sorted(liq)[1]), None


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--oracle", required=True)
    ap.add_argument("--rpc", default="https://rpc.mainnet.chain.robinhood.com")
    ap.add_argument("--watch", type=int, default=0, help="repeat every N seconds")
    ap.add_argument("--json", action="store_true", help="one JSON line (for the status log), then exit")
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
    stock = "0x" + cfg[2 + 64 * 4 + 24: 2 + 64 * 5]
    qt, qerr = rpc_call(args.rpc, args.oracle, SELECTORS["quietTier()"])
    heartbeat, quiet_bps = (word(qt, 0), word(qt, 1)) if qt and not qerr else (None, None)
    print("AfterHours %s  initialized=%s  feed=%s  primary pool=%s" % (args.oracle, initialized, feed, pool))
    print("  liveMaxAge=%ds twapWindow=%ds band=%.1f%% (%s) minLiquidity=%.3g maxAnchorAge=%ds  decimals feed/stock/quote=%d/%d/%d\n"
          % (live_max_age, twap_window, dev_bps / 100,
             ("%.1f%% in the regular session while the print is under %ds old" % (quiet_bps / 100, heartbeat)) if heartbeat is not None else "no quiet tier",
             min_liq, max_anchor, feed_dec, stock_dec, quote_dec))

    while True:
        now = int(time.time())
        r, err = rpc_call(args.rpc, feed, SELECTORS["latestRoundData()"])
        feed_answer = word(r, 1) / 10 ** feed_dec
        feed_at = word(r, 3)
        age_h = (now - feed_at) / 3600
        m, merr = rpc_call(args.rpc, stock, SELECTORS["uiMultiplier()"])
        multiplier = word(m, 0) if m and not merr else 10 ** 18
        pooled, terr = twap_from_pool(args.rpc, pool, twap_window, stock_is_token0,
                                      stock_dec, quote_dec, feed_dec, multiplier)
        twap = pooled[0] if pooled else None
        s, serr = rpc_call(args.rpc, args.oracle, SELECTORS["state()"])
        print("[%s]" % ts(now))
        # Monday to Friday 14:30-20:00 UTC: the US regular session is open in both
        # daylight-saving regimes (src/lib.rs, in_regular_session)
        in_session = (now // 86400 + 3) % 7 < 5 and 52_200 <= now % 86400 < 72_000
        band = quiet_bps if heartbeat is not None and now - feed_at <= heartbeat and in_session else dev_bps
        print("  Chainlink : $%.4f  printed %s  (%.1f h ago; the band at this age and hour is %.1f%%)"
              % (feed_answer, ts(feed_at), age_h, band / 100))
        if twap is not None:
            print("  primary pool: $%.4f per share  (median of three %d s averages, median liquidity %.3g, share multiplier %.8f; computed off-chain)"
                  % (twap / 10 ** feed_dec, twap_window // 3, pooled[1], multiplier / 1e18))
        else:
            print("  primary pool: unavailable (%s)" % terr)
        if serr or not s:
            print("  AfterHours: state() reverted: %s" % serr)
        else:
            session, reason = word(s, 0), word(s, 1)
            answer, fa, fu, tw, liq, clamped = (word(s, 2), word(s, 3), word(s, 4), word(s, 5),
                                                word(s, 6), bool(word(s, 7)))
            used_pool = "0x" + s[2 + 64 * 8 + 24: 2 + 64 * 9]
            line = "  AfterHours: %s" % SESSIONS.get(session, session)
            if session in (0, 1):
                line += "  answer $%.4f" % (answer / 10 ** feed_dec)
            if session == 1:
                line += "  (twap $%.4f, %s, median liquidity %.3g, pool %s)" % (
                    tw / 10 ** feed_dec, "CLAMPED to band" if clamped else "inside band", liq, used_pool[:10])
            if session == 3:
                line += "  (%s)" % REASONS.get(reason, reason)
            print(line)
            p, perr = rpc_call(args.rpc, args.oracle, SELECTORS["price()"])
            if perr:
                print("  Morpho price(): reverted (%s)" % perr[:60])
            else:
                scale = 36 + quote_dec - stock_dec - feed_dec
                print("  Morpho price(): %d  (= answer x 1e%d x share multiplier %.8f)" % (word(p, 0), scale, multiplier / 1e18))
        if args.json:
            if serr or not s:
                rec = {"session": "read failed", "answer": "", "feed_answer": "%.4f" % feed_answer,
                       "feed_age_h": "%.1f" % age_h, "pool": "", "liquidity": ""}
            else:
                rec = {"session": SESSIONS.get(session, session)
                       + (" (%s)" % REASONS.get(reason, reason) if session == 3 else "")
                       + (" clamped" if session == 1 and clamped else ""),
                       "answer": ("%.4f" % (answer / 10 ** feed_dec)) if session in (0, 1) else "",
                       "feed_answer": "%.4f" % feed_answer, "feed_age_h": "%.1f" % age_h,
                       "pool": used_pool[:10] if session == 1 else "", "liquidity": ("%.3g" % liq) if liq else ""}
            print(json.dumps(rec))
            break
        if not args.watch:
            break
        print()
        time.sleep(args.watch)


if __name__ == "__main__":
    main()
