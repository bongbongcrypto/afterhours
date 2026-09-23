# -*- coding: utf-8 -*-
"""What it costs to move a stock/USDG Uniswap v3 pool, tick by tick.

Reads the pool's current price and in-range liquidity, walks the tick bitmap
and every initialized tick's liquidityNet within +-60% of the price, and
simulates a swap in each direction the way Uniswap v3 does (constant L
between initialized ticks, L changes at each crossing). Reports, for moves of
2%, 5% and 10% in the stock's price, the input the swap needs and the fees
it pays, and where in-range liquidity runs out (the price at which a swap
leaves every position: the move behind the "one second out of range" attack).

Floating point (double) is plenty for costs; no fixed-point exactness is
claimed here. Read-only, stdlib; selectors from keccak.py.

    python scripts/measure/pool_depth.py [--pool 0x...] [--stock 0x...]
"""
import argparse
import io
import json
import math
import sys
import time
import urllib.request
from pathlib import Path

sys.stdout = io.TextIOWrapper(sys.stdout.buffer, encoding="utf-8", errors="replace")
sys.path.insert(0, str(Path(__file__).resolve().parent))
from keccak import selector  # noqa: E402

RPC = "https://rpc.mainnet.chain.robinhood.com"
POOL = "0xaae0d815ee56e4092a5e5c2911e676fea50b2d6d"   # AAPL/USDG 0.05%
STOCK = "0xaF3D76f1834A1d425780943C99Ea8A608f8a93f9"  # AAPL
QUOTE_DEC, STOCK_DEC = 6, 18
SPAN = 0.60   # scan +-60% of the price for initialized ticks


def batch(calls):
    out = []
    for s in range(0, len(calls), 25):
        body = [{"jsonrpc": "2.0", "id": i, "method": "eth_call", "params": [{"to": t, "data": d}, "latest"]}
                for i, (t, d) in enumerate(calls[s:s + 25])]
        time.sleep(0.5)
        r = json.load(urllib.request.urlopen(urllib.request.Request(
            RPC, json.dumps(body).encode(), {"content-type": "application/json", "user-agent": "curl/8"}), timeout=60))
        by = {x["id"]: x for x in r}
        out += [by[i].get("result") for i in range(len(body))]
    return out


def words(h):
    h = h[2:]
    return [int(h[i:i + 64], 16) for i in range(0, len(h), 64)]


def signed(v, bits):
    return v - (1 << bits) if v >= 1 << (bits - 1) else v


def enc_int(v, bits=256):
    return format(v % (1 << 256), "064x")


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--pool", default=POOL)
    ap.add_argument("--stock", default=STOCK)
    ap.add_argument("--floor", type=float, default=2e17, help="the instance's minLiquidity")
    a = ap.parse_args()
    pool = a.pool

    s0, liq, spacing, fee, t0, mult = batch([
        (pool, selector("slot0()")), (pool, selector("liquidity()")), (pool, selector("tickSpacing()")),
        (pool, selector("fee()")), (pool, selector("token0()")), (a.stock, selector("uiMultiplier()"))])
    w = words(s0)
    sqrt_p = w[0] / 2 ** 96                      # sqrt(token1 raw per token0 raw)
    tick = signed(w[1], 256)
    L = words(liq)[0]
    spacing = signed(words(spacing)[0], 256)
    fee = words(fee)[0] / 1e6
    stock_is_token0 = ("0x" + t0[-40:]).lower() == a.stock.lower()
    m = words(mult)[0] / 1e18

    def usd_per_share(sp):
        p = sp * sp                                # token1 raw per token0 raw
        raw = p * 10 ** (STOCK_DEC - QUOTE_DEC) if stock_is_token0 else 10 ** (STOCK_DEC - QUOTE_DEC) / p
        return raw / m

    price0 = usd_per_share(sqrt_p)
    # initialized ticks within the span
    span_ticks = int(math.log(1 + SPAN) / math.log(1.0001)) + spacing
    lo_word = ((tick - span_ticks) // spacing) >> 8
    hi_word = ((tick + span_ticks) // spacing) >> 8
    word_ids = list(range(lo_word, hi_word + 1))
    bitmaps = batch([(pool, selector("tickBitmap(int16)") + enc_int(wd)) for wd in word_ids])
    ticks = []
    for wd, bm in zip(word_ids, bitmaps):
        b = int(bm, 16)
        for bit in range(256):
            if b >> bit & 1:
                ticks.append(((wd << 8) + bit) * spacing)
    nets = batch([(pool, selector("ticks(int24)") + enc_int(t)) for t in ticks])
    net = {t: signed(words(r)[1], 256) for t, r in zip(ticks, nets)}
    print("pool %s  fee %.2f%%  spacing %d  tick %d  in-range L %.3e  share multiplier %.8f"
          % (pool, fee * 100, spacing, tick, L, m))
    print("price now $%.4f per share; %d initialized ticks within +-%d%%\n" % (price0, len(ticks), SPAN * 100))

    def walk(up):
        """Move the tick up (up=True) or down; yield (sqrtP, amount0_in, amount1_in, L) at each crossing."""
        sp, l, a0, a1 = sqrt_p, L, 0.0, 0.0
        order = sorted((t for t in ticks if (t > tick if up else t <= tick)), reverse=not up)
        for t in order:
            nxt = 1.0001 ** (t / 2)
            if l > 0:
                if up:     # token1 in, sqrtP rises
                    a1 += l * (nxt - sp)
                else:      # token0 in, sqrtP falls
                    a0 += l * (1 / nxt - 1 / sp)
            sp = nxt
            yield sp, a0, a1, l
            l = l + net[t] if up else l - net[t]
            if l < 0:
                l = 0
        yield sp, a0, a1, l

    def cost(a0, a1):
        """USD value of what the swapper put in (token0/token1 raw amounts)."""
        if stock_is_token0:
            return a0 / 10 ** STOCK_DEC * price0 * m + a1 / 10 ** QUOTE_DEC
        return a0 / 10 ** QUOTE_DEC + a1 / 10 ** STOCK_DEC * price0 * m

    floor = a.floor
    sub = 600.0
    for label, up in (("stock price DOWN (sell the stock into the pool)", not stock_is_token0),
                      ("stock price UP (buy the stock with USDG)", stock_is_token0)):
        print(label)
        targets = [0.02, 0.05, 0.10]
        done, empty, thinnest = set(), None, None
        for sp, a0, a1, l in walk(up):
            move = usd_per_share(sp) / price0 - 1
            for tg in targets:
                if tg not in done and abs(move) >= tg:
                    usd = cost(a0, a1)
                    print("  %4.0f%%  input worth $%s, fee $%s" % (tg * 100, format(round(usd), ","), format(round(usd * fee), ",")))
                    done.add(tg)
            if l == 0 and empty is None:
                empty = (move, cost(a0, a1))
            # the thinnest region within 10% of the price: where a stay costs the least depth
            if abs(move) <= 0.10 and l > 0 and (thinnest is None or l < thinnest[0]):
                thinnest = (l, move, cost(a0, a1))
        for tg in targets:
            if tg not in done:
                print("  %4.0f%%  beyond the scanned ticks (+-%d%%)" % (tg * 100, SPAN * 100))
        if empty:
            print("  in-range liquidity runs out at %+.2f%%: input worth $%s, fee $%s"
                  % (empty[0] * 100, format(round(empty[1]), ","), format(round(empty[1] * fee), ",")))
        else:
            print("  in-range liquidity never runs out within +-%d%% (a wide or full-range position)" % (SPAN * 100))
        if thinnest:
            lt, mv, usd = thinnest
            # seconds s at liquidity lt that drag one 600 s sub-window's harmonic mean below the floor:
            # 600 / ((600 - s) / L + s / lt) < floor  <=>  s > (600/floor - 600/L) / (1/lt - 1/L)
            need = 600 / floor - sub / L
            gap = 1 / lt - 1 / L
            secs = need / gap if gap > 0 else float("inf")
            print("  thinnest liquidity within 10%%: %.3e at %+.2f%% (input worth $%s); a stay there must last %s"
                  " to drag one 10-minute window below the %.0e floor"
                  % (lt, mv * 100, format(round(usd), ","), ("%.0f s" % secs) if secs < 600 else "longer than the window", floor))
        print()


if __name__ == "__main__":
    main()
