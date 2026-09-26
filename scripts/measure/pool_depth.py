# -*- coding: utf-8 -*-
"""What it costs to move a stock/USDG Uniswap v3 pool, and to make AfterHours refuse.

Reads the pool's price and in-range liquidity, walks the tick bitmap and
every initialized tick's liquidityNet over the pool's whole tick range, and
simulates a swap in each direction the way Uniswap v3 does (constant L
between initialized ticks, L changes at each crossing). Reports, per
direction:

* the input a swap needs, and the fee it pays, to move the stock's price
  1% to 50%;
* where in-range liquidity runs out, if it does;
* the cheapest refusal. AfterHours refuses (NO_DATA 2) when two of its three
  10-minute sub-windows have a harmonic-mean liquidity below the instance's
  floor. Uniswap accumulates whole seconds of block time at the liquidity in
  range, so one second is the shortest stay that counts. The script finds the
  nearest price at which a stay of one second (and of up to 10 s and 60 s)
  drags a sub-window below the floor, and what reaching it takes. A refusal
  lasts while two sub-windows hold such a stay: one excursion every ten
  minutes keeps it going.

Floating point (double) is plenty for costs; no fixed-point exactness is
claimed here. Read-only, stdlib; selectors from keccak.py.

    python scripts/measure/pool_depth.py [--pool 0x...] [--stock 0x...] [--floor 5e16]
"""
import argparse
import io
import json
import math
import sys
import time
import urllib.error
import urllib.request
from datetime import datetime, timezone
from pathlib import Path

sys.stdout = io.TextIOWrapper(sys.stdout.buffer, encoding="utf-8", errors="replace")
sys.path.insert(0, str(Path(__file__).resolve().parent))
from keccak import selector  # noqa: E402

RPC = "https://rpc.mainnet.chain.robinhood.com"
POOL = "0xaae0d815ee56e4092a5e5c2911e676fea50b2d6d"   # AAPL/USDG 0.05%
STOCK = "0xaF3D76f1834A1d425780943C99Ea8A608f8a93f9"  # AAPL
QUOTE_DEC, STOCK_DEC = 6, 18
MAX_TICK = 887272
SUB = 600.0   # AfterHours' sub-window with the 30-minute window


def post(body):
    req = urllib.request.Request(RPC, json.dumps(body).encode(), {"content-type": "application/json", "user-agent": "curl/8"})
    for attempt in range(10):
        try:
            time.sleep(0.5)
            return json.load(urllib.request.urlopen(req, timeout=60))
        except urllib.error.HTTPError as e:
            if e.code != 429 or attempt == 9:
                raise
            # public RPC rate limit: back off (up to about five minutes in all), never hammer
            wait = e.headers.get("Retry-After") if e.headers else None
            time.sleep(float(wait) if wait and wait.isdigit() else min(60, 2 ** (attempt + 1)))


BLOCK = None   # every read is pinned to this block, so the walk sees one consistent pool


def batch(calls):
    out = []
    for s in range(0, len(calls), 25):
        body = [{"jsonrpc": "2.0", "id": i, "method": "eth_call", "params": [{"to": t, "data": d}, BLOCK]}
                for i, (t, d) in enumerate(calls[s:s + 25])]
        r = post(body)
        by = {x["id"]: x for x in r}
        out += [by[i].get("result") for i in range(len(body))]
    return out


def words(h):
    h = h[2:]
    return [int(h[i:i + 64], 16) for i in range(0, len(h), 64)]


def signed(v, bits):
    return v - (1 << bits) if v >= 1 << (bits - 1) else v


def enc_int(v):
    return format(v % (1 << 256), "064x")


def whole_seconds(x):
    """A stay counts in whole seconds; a stretch that never drags a sub-window below the floor stays infinite."""
    return math.ceil(x) if math.isfinite(x) else x


def usd(v):
    return "$" + format(round(v), ",")


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--pool", default=POOL)
    ap.add_argument("--stock", default=STOCK)
    ap.add_argument("--floor", type=float, default=5e16, help="the instance's minLiquidity (AAPL: 5e16)")
    ap.add_argument("--profile", type=float, default=0, help="also list every stretch's liquidity within this move, e.g. 0.3")
    a = ap.parse_args()
    pool = a.pool
    global BLOCK
    head = post({"jsonrpc": "2.0", "id": 1, "method": "eth_getBlockByNumber", "params": ["latest", False]})["result"]
    BLOCK = head["number"]
    print("block %d (%s UTC)" % (int(BLOCK, 16), datetime.fromtimestamp(int(head["timestamp"], 16), timezone.utc).strftime("%Y-%m-%d %H:%M")))

    s0, liq, spacing, fee, t0 = batch([
        (pool, selector("slot0()")), (pool, selector("liquidity()")), (pool, selector("tickSpacing()")),
        (pool, selector("fee()")), (pool, selector("token0()"))])
    w = words(s0)
    sqrt_p = w[0] / 2 ** 96                      # sqrt(token1 raw per token0 raw)
    tick = signed(w[1], 256)
    L = words(liq)[0]
    spacing = signed(words(spacing)[0], 256)
    fee = words(fee)[0] / 1e6
    stock_is_token0 = ("0x" + t0[-40:]).lower() == a.stock.lower()

    def usd_per_token(sp):
        """USDG per token of raw balance (1e18 raw units), the unit the feed prices."""
        p = sp * sp                                # token1 raw per token0 raw
        return p * 10 ** (STOCK_DEC - QUOTE_DEC) if stock_is_token0 else 10 ** (STOCK_DEC - QUOTE_DEC) / p

    price0 = usd_per_token(sqrt_p)
    # every initialized tick in the pool's range
    word_ids = list(range((-MAX_TICK // spacing) >> 8, (MAX_TICK // spacing >> 8) + 1))
    bitmaps = batch([(pool, selector("tickBitmap(int16)") + enc_int(wd)) for wd in word_ids])
    ticks = []
    for wd, bm in zip(word_ids, bitmaps):
        b = int(bm, 16)
        for bit in range(256):
            if b >> bit & 1:
                ticks.append(((wd << 8) + bit) * spacing)
    nets = batch([(pool, selector("ticks(int24)") + enc_int(t)) for t in ticks])
    net = {t: signed(words(r)[1], 256) for t, r in zip(ticks, nets)}
    print("pool %s  fee %.2f%%  spacing %d  tick %d  in-range L %.3e"
          % (pool, fee * 100, spacing, tick, L))
    span = sorted((usd_per_token(1.0001 ** (t / 2)) / price0 - 1) * 100 for t in (min(ticks), max(ticks)))
    print("price now $%.4f per token; %d initialized ticks, from %+.4g%% to %+.4g%% of the price; floor %.0e\n"
          % (price0, len(ticks), span[0], span[1], a.floor))

    def swap_in(up, l, sp_from, sp_to):
        """Raw (token0, token1) a swap puts in to move sqrtP from sp_from to sp_to at liquidity l."""
        if up:         # token1 in, sqrtP rises
            return 0.0, l * (sp_to - sp_from)
        return l * (1 / sp_to - 1 / sp_from), 0.0   # token0 in, sqrtP falls

    def segments(up):
        """Constant-liquidity stretches from the current price outward, in walk
        order: (move at the stretch's start, liquidity in it, the swap input's
        token0/token1 raw amounts that reach its start, sqrtP at its start)."""
        sp, l, a0, a1 = sqrt_p, L, 0.0, 0.0
        order = sorted((t for t in ticks if (t > tick if up else t <= tick)), reverse=not up)
        for t in order:
            yield usd_per_token(sp) / price0 - 1, l, a0, a1, sp
            nxt = 1.0001 ** (t / 2)
            d0, d1 = swap_in(up, l, sp, nxt)
            a0, a1 = a0 + d0, a1 + d1
            sp = nxt
            l = max(l + net[t] if up else l - net[t], 0)
        yield usd_per_token(sp) / price0 - 1, l, a0, a1, sp

    def sqrt_at(move):
        """sqrtP at which the stock's price is price0 * (1 + move)."""
        r = math.sqrt(1 + move)
        return sqrt_p * r if stock_is_token0 else sqrt_p / r

    def cost(a0, a1):
        """USD value, at today's price, of what the swapper put in (token0/token1 raw amounts)."""
        if stock_is_token0:
            return a0 / 10 ** STOCK_DEC * price0 + a1 / 10 ** QUOTE_DEC
        return a0 / 10 ** QUOTE_DEC + a1 / 10 ** STOCK_DEC * price0

    def stay_needed(l):
        """Seconds at liquidity l (Uniswap counts max(l, 1)) that pull one 600 s
        sub-window's harmonic mean below the floor, the rest of it at L:
        600 / ((600 - s) / L + s / l) < floor."""
        gap = 1 / max(l, 1) - 1 / L
        return (SUB / a.floor - SUB / L) / gap if gap > 0 else float("inf")

    for label, up in (("stock price DOWN (sell the stock into the pool)", not stock_is_token0),
                      ("stock price UP (buy the stock with USDG)", stock_is_token0)):
        print(label)
        segs = list(segments(up))
        if a.profile:
            for mv, l, a0, a1, _ in segs:
                if abs(mv) > a.profile:
                    break
                print("    from %+7.2f%%  liquidity %.2e  (input to get here %s)" % (mv * 100, l, usd(cost(a0, a1))))
        targets = [0.01, 0.02, 0.05, 0.10, 0.25, 0.50]
        sign = 1 if (up == stock_is_token0) else -1   # direction of the stock's price
        for tg in targets:
            # the stretch in which the price reaches the target, then the exact input within it
            nxt = next((k for k, s in enumerate(segs) if abs(s[0]) >= tg), None)
            if nxt is None or nxt == 0:
                print("  %4.0f%%  beyond the last initialized tick" % (tg * 100))
                continue
            mv, l, a0, a1, sp = segs[nxt - 1]
            d0, d1 = swap_in(up, l, sp, sqrt_at(sign * tg))
            usd_in = cost(a0 + d0, a1 + d1)
            print("  %4.0f%%  input worth %s, fee %s" % (tg * 100, usd(usd_in), usd(usd_in * fee)))
        # A price held for a whole sub-window sees only the liquidity there: beyond the first
        # stretch below the floor, holding the price refuses instead of pricing.
        wall = next((s for s in segs if s[1] < a.floor), None)
        if wall:
            print("  liquidity stays at or above the floor out to %+.2f%% (input worth %s); a price held beyond it "
                  "for a sub-window refuses instead of pricing" % (wall[0] * 100, usd(cost(wall[2], wall[3]))))
        empty = next((s for s in segs[1:] if s[1] == 0), None)
        if empty:
            print("  in-range liquidity runs out at %+.2f%%: input worth %s" % (empty[0] * 100, usd(cost(empty[2], empty[3]))))
        else:
            print("  in-range liquidity never runs out (a full-range position underlies the pool)")
        # cost grows along the walk, so the first stretch that works is the cheapest one;
        # the stretch past the last initialized tick sits at the end of the price range and is out of reach
        # the accumulator advances in whole seconds of block time, so a stay counts
        # only in whole seconds: 7.15 s of exposure needs an 8-second stay
        for limit in (1, 10, 60):
            found = next((s for s in segs[:-1] if whole_seconds(stay_needed(s[1])) <= limit), None)
            if not found:
                print("  no reachable price at which a stay of %d s drags a sub-window below the floor" % limit)
                continue
            mv, l, a0, a1, _ = found
            usd_in = cost(a0, a1)
            print("  cheapest refusal with a stay of <= %2d s: %+.2f%% (liquidity %.2e), a stay of %d whole seconds "
                  "(%.2f s computed); input worth %s, round-trip fee %s, %s an hour to keep it refusing"
                  % (limit, mv * 100, l, whole_seconds(stay_needed(l)), stay_needed(l), usd(usd_in),
                     usd(2 * usd_in * fee), usd(6 * 2 * usd_in * fee)))
        print()


if __name__ == "__main__":
    main()
