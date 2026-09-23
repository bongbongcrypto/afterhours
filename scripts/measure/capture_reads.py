# -*- coding: utf-8 -*-
"""Capture, at one pinned block, every answer the AAPL instance's reads get
from Robinhood Chain mainnet: the Chainlink feed, the primary pool (observe at
the four sub-window points, slot0, its tokens) and the stock and quote tokens.
The unit test `the_contract_reads_real_mainnet_answers` serves these exact
bytes to the contract, so its decoding of the real feed, pool and token is
tested without a deployment. The answer the contract must give is computed
here by a separate port of its rules (the live page's integer math, in
Python), from the same bytes. Read-only, stdlib, 16 requests.

    python scripts/measure/capture_reads.py > fixtures/aapl_mainnet.txt
"""
import io
import json
import sys
import time
import urllib.error
import urllib.request
from datetime import datetime, timezone
from pathlib import Path

sys.stdout = io.TextIOWrapper(sys.stdout.buffer, encoding="utf-8", errors="replace", newline="\n")
sys.path.insert(0, str(Path(__file__).resolve().parent))
from keccak import selector  # noqa: E402

RPC = "https://rpc.mainnet.chain.robinhood.com"
FEED = "0x6B22A786bAa607d76728168703a39Ea9C99f2cD0"    # Robinhood AAPL / USD
POOL = "0xaae0d815ee56e4092a5e5c2911e676fea50b2d6d"    # AAPL/USDG 0.05%, the instance's primary
STOCK = "0xaF3D76f1834A1d425780943C99Ea8A608f8a93f9"   # AAPL
QUOTE = "0x5fc5360D0400a0Fd4f2af552ADD042D716F1d168"   # USDG
# the AAPL instance's inputs (assets.json defaults and its deploy block)
LIVE_MAX_AGE, WINDOW, MAX_DEV, MIN_LIQ, MAX_ANCHOR, HEARTBEAT, QUIET = (
    21600, 1800, 1000, 50_000_000_000_000_000, 432000, 86400, 100)
POINTS = [WINDOW, 2 * (WINDOW // 3), WINDOW // 3, 0]


def post(method, params):
    body = json.dumps({"jsonrpc": "2.0", "id": 1, "method": method, "params": params}).encode()
    req = urllib.request.Request(RPC, body, {"content-type": "application/json", "user-agent": "curl/8"})
    for attempt in range(10):
        try:
            time.sleep(0.25)
            out = json.load(urllib.request.urlopen(req, timeout=30))
            break
        except urllib.error.HTTPError as e:
            if e.code != 429 or attempt == 9:
                raise
            time.sleep(min(60, 2 ** (attempt + 1)))   # the public RPC rate-limits bursts
    if "error" in out:
        raise SystemExit("%s failed: %s" % (method, out["error"]))
    return out["result"]


def words(h):
    h = h[2:]
    return [int(h[i:i + 64], 16) for i in range(0, len(h), 64)]


def signed(v, bits):
    return v - (1 << bits) if v >= 1 << (bits - 1) else v


# ---- the rules, ported from src/tickmath.rs and src/lib.rs (as web/index.html does) ----
Q96 = 1 << 96


def ratio_q96(tick):
    result, base, e = Q96, Q96 + Q96 // 10_000, abs(tick)
    while e:
        if e & 1:
            result = (result * base) >> 96
        e >>= 1
        if e:
            base = (base * base) >> 96
    return (Q96 << 96) // result if tick < 0 else result


def stock_price(ratio, stock_is_token0, sd, qd, fd):
    num, qp = 10 ** (sd + fd), 10 ** qd
    return (ratio * num) // (qp << 96) if stock_is_token0 else (num << 96) // (ratio * qp)


def in_session(t):
    return (t // 86400 + 3) % 7 < 5 and 52_200 <= t % 86400 < 72_000


def mean_tick(then, now, span):
    d = now - then
    m = d // span   # Python floors toward negative infinity, as Uniswap's OracleLibrary rounds
    return m


def main():
    head = int(post("eth_blockNumber", []), 16) - 5
    block = hex(head)
    ts = int(post("eth_getBlockByNumber", [block, False])["timestamp"], 16)

    obs_data = selector("observe(uint32[])") + format(32, "064x") + format(4, "064x") + "".join(
        format(p, "064x") for p in POINTS)
    reads = [
        ("feed", FEED, "decimals", selector("decimals()")),
        ("feed", FEED, "description", selector("description()")),
        ("feed", FEED, "latestRoundData", selector("latestRoundData()")),
        ("pool", POOL, "token0", selector("token0()")),
        ("pool", POOL, "token1", selector("token1()")),
        ("pool", POOL, "observe", obs_data),
        ("pool", POOL, "slot0", selector("slot0()")),
        ("stock", STOCK, "decimals", selector("decimals()")),
        ("stock", STOCK, "oraclePaused", selector("oraclePaused()")),
        ("stock", STOCK, "uiMultiplier", selector("uiMultiplier()")),
        ("stock", STOCK, "effectiveAt", selector("effectiveAt()")),
        ("quote", QUOTE, "decimals", selector("decimals()")),
    ]
    ret = {}
    for who, to, name, data in reads:
        ret[(who, name)] = post("eth_call", [{"to": to, "data": data}, block])

    # the rules on those bytes
    fd, sd, qd = (words(ret[(w, "decimals")])[0] for w in ("feed", "stock", "quote"))
    token0 = "0x" + ret[("pool", "token0")][-40:]
    stock_is_token0 = token0.lower() == STOCK.lower()
    rid, answer, _, updated, _ = words(ret[("feed", "latestRoundData")])[:5]
    answer = signed(answer, 256)
    paused = words(ret[("stock", "oraclePaused")])[0] != 0
    mult = words(ret[("stock", "uiMultiplier")])[0]
    eff = words(ret[("stock", "effectiveAt")])[0]
    w = words(ret[("pool", "observe")])
    a, b = w[0] // 32, w[1] // 32
    ticks = [signed(v, 256) for v in w[a + 1:a + 5]]
    spl = w[b + 1:b + 5]
    scale = 10 ** (36 + qd - sd - fd)

    want = {"session": 3, "reason": 0, "answer": 0, "twap": 0, "liquidity": 0, "clamped": 0}
    age = ts - updated
    rebased = updated < eff <= ts
    if paused:
        want["session"] = 2
    elif not (answer > 0 and 0 < updated <= ts):
        want["reason"] = 1
    elif age <= LIVE_MAX_AGE and not rebased:
        want.update(session=0, answer=answer)
    elif age > MAX_ANCHOR:
        want["reason"] = 4
    else:
        band = QUIET if age <= HEARTBEAT and in_session(ts) else MAX_DEV
        lo, hi = answer * (10_000 - band) // 10_000, answer * (10_000 + band) // 10_000
        wlo, whi = answer * (10_000 - MAX_DEV) // 10_000, answer * (10_000 + MAX_DEV) // 10_000
        spans = [POINTS[k] - POINTS[k + 1] for k in range(3)]
        deltas = [(spl[k + 1] - spl[k]) % (1 << 160) for k in range(3)]   # the accumulator wraps
        # no liquidity-seconds recorded reads as an empty sub-window; the mean saturates at uint128
        liqs = sorted(min((s << 128) // d, (1 << 128) - 1) if d else 0 for s, d in zip(spans, deltas))
        tick = sorted(mean_tick(ticks[k], ticks[k + 1], spans[k]) for k in range(3))[1]
        want["liquidity"] = liqs[1]
        if liqs[1] < MIN_LIQ:
            want["reason"] = 2
        else:
            twap = stock_price(ratio_q96(tick), stock_is_token0, sd, qd, fd) * 10 ** 18 // mult
            if twap == 0:
                want["reason"] = 3
            elif rebased and not wlo <= twap <= whi:
                want["reason"] = 5
            else:
                clamped = twap < lo or twap > hi
                want.update(session=1, twap=twap, answer=min(max(twap, lo), hi), clamped=int(clamped))
    price = want["answer"] * scale * mult // 10 ** 18 if want["session"] in (0, 1) else "refuse"

    when = datetime.fromtimestamp(ts, timezone.utc).strftime("%Y-%m-%d %H:%M:%S")
    print("# Robinhood Chain mainnet, block %d (%s UTC): every read the AAPL instance makes," % (head, when))
    print("# captured by scripts/measure/capture_reads.py. Expected values come from that script's own port")
    print("# of the rules. Feed print %s UTC, %.1f h before the block." % (
        datetime.fromtimestamp(updated, timezone.utc).strftime("%Y-%m-%d %H:%M"), age / 3600))
    print("timestamp %d" % ts)
    for who, addr in (("feed", FEED), ("pool", POOL), ("stock", STOCK), ("quote", QUOTE)):
        print("%s %s" % (who, addr))
    for who, _, name, _ in reads:
        print("ret %s %s %s" % (who, name, ret[(who, name)]))
    for k in ("session", "reason", "answer", "twap", "liquidity", "clamped"):
        print("want %s %s" % (k, want[k]))
    print("want price %s" % price)
    print("want round %d" % rid)


if __name__ == "__main__":
    main()
