# -*- coding: utf-8 -*-
"""Capture, at one pinned block, every answer an AAPL instance's reads get
from Robinhood Chain mainnet: the Chainlink feed, the primary pool (observe at
the four sub-window points, slot0, its tokens) and the stock and quote tokens.
The unit test `the_contract_reads_real_mainnet_answers` serves these exact
bytes to the contract, so its decoding of the real feed, pool and token is
tested without a deployment. The answer the contract must give is computed
here by a separate port of its rules (the live page's integer math, in
Python), from the same bytes. Read-only, stdlib, 13 requests: the block, the
ten reads the contract makes, and the token's uiMultiplier(), which the
contract does not read (the feed and the pool both price one token of raw
balance) and which is recorded as a comment only.

    python scripts/measure/capture_reads.py > fixtures/aapl_mainnet.txt

--replay FILE re-derives the expected answer from the bytes of an earlier
capture with the current rules, without a network request, and prints the
fixture again (reads the contract no longer makes are dropped):

    python scripts/measure/capture_reads.py --replay fixtures/aapl_mainnet.txt
"""
import argparse
import io
import json
import re
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
    """Price of one token of raw balance (10^sd raw units) in feed decimals."""
    num, qp = 10 ** (sd + fd), 10 ** qd
    return (ratio * num) // (qp << 96) if stock_is_token0 else (num << 96) // (ratio * qp)


def in_session(t):
    return (t // 86400 + 3) % 7 < 5 and 52_200 <= t % 86400 < 72_000


def mean_tick(then, now, span):
    d = now - then
    m = d // span   # Python floors toward negative infinity, as Uniswap's OracleLibrary rounds
    return m


def contract_reads():
    """Every read the contract makes: (who, name, calldata)."""
    obs_data = selector("observe(uint32[])") + format(32, "064x") + format(4, "064x") + "".join(
        format(p, "064x") for p in POINTS)
    return [
        ("feed", "decimals", selector("decimals()")),
        ("feed", "description", selector("description()")),
        ("feed", "latestRoundData", selector("latestRoundData()")),
        ("pool", "token0", selector("token0()")),
        ("pool", "token1", selector("token1()")),
        ("pool", "observe", obs_data),
        ("pool", "slot0", selector("slot0()")),
        ("stock", "decimals", selector("decimals()")),
        ("stock", "oraclePaused", selector("oraclePaused()")),
        ("quote", "decimals", selector("decimals()")),
    ]


def rules(ret, ts):
    """The contract's answer on these bytes: (want, price, round id, print time)."""
    fd, sd, qd = (words(ret[(w, "decimals")])[0] for w in ("feed", "stock", "quote"))
    token0 = "0x" + ret[("pool", "token0")][-40:]
    stock_is_token0 = token0.lower() == STOCK.lower()
    rid, answer, _, updated, _ = words(ret[("feed", "latestRoundData")])[:5]
    answer = signed(answer, 256)
    paused = words(ret[("stock", "oraclePaused")])[0] != 0
    w = words(ret[("pool", "observe")])
    a, b = w[0] // 32, w[1] // 32
    ticks = [signed(v, 256) for v in w[a + 1:a + 5]]
    spl = w[b + 1:b + 5]
    scale = 10 ** (36 + qd - sd - fd)

    want = {"session": 3, "reason": 0, "answer": 0, "twap": 0, "liquidity": 0, "clamped": 0}
    age = ts - updated
    if paused:
        want["session"] = 2
    elif not (answer > 0 and 0 < updated <= ts):
        want["reason"] = 1
    elif age <= LIVE_MAX_AGE:
        want.update(session=0, answer=answer)
    elif age > MAX_ANCHOR:
        want["reason"] = 4
    else:
        band = QUIET if age <= HEARTBEAT and in_session(ts) else MAX_DEV
        lo, hi = answer * (10_000 - band) // 10_000, answer * (10_000 + band) // 10_000
        spans = [POINTS[k] - POINTS[k + 1] for k in range(3)]
        deltas = [(spl[k + 1] - spl[k]) % (1 << 160) for k in range(3)]   # the accumulator wraps
        # no liquidity-seconds recorded reads as an empty sub-window; the mean saturates at uint128
        liqs = sorted(min((s << 128) // d, (1 << 128) - 1) if d else 0 for s, d in zip(spans, deltas))
        tick = sorted(mean_tick(ticks[k], ticks[k + 1], spans[k]) for k in range(3))[1]
        want["liquidity"] = liqs[1]
        if liqs[1] < MIN_LIQ:
            want["reason"] = 2
        else:
            # the pool's price of one token of raw balance: the unit the feed prints
            twap = stock_price(ratio_q96(tick), stock_is_token0, sd, qd, fd)
            if twap == 0:
                want["reason"] = 3
            else:
                clamped = twap < lo or twap > hi
                want.update(session=1, twap=twap, answer=min(max(twap, lo), hi), clamped=int(clamped))
    # Morpho: the answer times the scale, no share multiplier
    price = want["answer"] * scale if want["session"] in (0, 1) else "refuse"
    return want, price, rid, updated


def emit(block, ts, ret, mult, note):
    want, price, rid, updated = rules(ret, ts)
    when = datetime.fromtimestamp(ts, timezone.utc).strftime("%Y-%m-%d %H:%M:%S")
    print("# Robinhood Chain mainnet, block %d (%s UTC): every read an AAPL instance makes," % (block, when))
    print("# captured by scripts/measure/capture_reads.py. Expected values come from that script's own port")
    print("# of the rules. Feed print %s UTC, %.1f h before the block." % (
        datetime.fromtimestamp(updated, timezone.utc).strftime("%Y-%m-%d %H:%M"), (ts - updated) / 3600))
    if mult is not None:
        print("# The token's uiMultiplier() at this block: %d. AfterHours does not read it: the feed" % mult)
        print("# and the pool both price one token of raw balance, so the answer is not scaled by it.")
    if note:
        print("# " + note)
    print("timestamp %d" % ts)
    for who, addr in (("feed", FEED), ("pool", POOL), ("stock", STOCK), ("quote", QUOTE)):
        print("%s %s" % (who, addr))
    for who, name, _ in contract_reads():
        print("ret %s %s %s" % (who, name, ret[(who, name)]))
    for k in ("session", "reason", "answer", "twap", "liquidity", "clamped"):
        print("want %s %s" % (k, want[k]))
    print("want price %s" % price)
    print("want round %d" % rid)


def replay(path):
    """Block, timestamp, ret lines and recorded multiplier of an earlier capture."""
    block, ts, mult, ret = None, None, None, {}
    for line in io.open(path, encoding="utf-8"):
        m = re.match(r"# Robinhood Chain mainnet, block (\d+) ", line)
        if m:
            block = int(m.group(1))
        m = re.match(r"# The token's uiMultiplier\(\) at this block: (\d+)\.", line)
        if m:
            mult = int(m.group(1))
        f = line.split()
        if f and f[0] == "timestamp":
            ts = int(f[1])
        elif f and f[0] == "ret":
            ret[(f[1], f[2])] = f[3]
    if ("stock", "uiMultiplier") in ret:   # a capture made while the contract still read it
        mult = words(ret[("stock", "uiMultiplier")])[0]
    if block is None or ts is None:
        raise SystemExit("%s: no block or timestamp line" % path)
    return block, ts, ret, mult


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--replay", metavar="FILE", help="re-derive the answer from an earlier capture's bytes")
    args = ap.parse_args()
    if args.replay:
        block, ts, ret, mult = replay(args.replay)
        emit(block, ts, ret, mult, "Expected values re-derived from these bytes with --replay on %s UTC."
             % datetime.now(timezone.utc).strftime("%Y-%m-%d"))
        return

    head = int(post("eth_blockNumber", []), 16) - 5
    block = hex(head)
    ts = int(post("eth_getBlockByNumber", [block, False])["timestamp"], 16)
    to = {"feed": FEED, "pool": POOL, "stock": STOCK, "quote": QUOTE}
    ret = {}
    for who, name, data in contract_reads():
        ret[(who, name)] = post("eth_call", [{"to": to[who], "data": data}, block])
    mult = words(post("eth_call", [{"to": STOCK, "data": selector("uiMultiplier()")}, block]))[0]
    emit(head, ts, ret, mult, "")


if __name__ == "__main__":
    main()
