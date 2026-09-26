# -*- coding: utf-8 -*-
"""Do people actually trade tokenized AAPL on Robinhood Chain while the stock
market (and its Chainlink feed) is closed, and what price do they get?

Walks the Uniswap v3 AAPL/USDG pools' Swap events across each weekend's
silent window (last Friday print -> first Monday print) and prices every
fill against those two true prints. Read-only, stdlib only, run from the PC.

Selectors/topics computed with ethers on the server, not recalled:
  latestRoundData()     0xfeaf968c      getRoundData(uint80)  0x9a6fc8f5
  getPool(addr,addr,u24) 0x1698ee82     token0()              0x0dfe1681
  Swap(address,address,int256,int256,uint160,uint128,int24)
    topic0 0xc42079f94a6350d7e6235f29174924f928cc2ac818eb64fed8004e115fbcca67
"""
import io
import json
import statistics
import sys
import time
import urllib.error
import urllib.request
from datetime import datetime, timezone

sys.stdout = io.TextIOWrapper(sys.stdout.buffer, encoding="utf-8", errors="replace")
RPC = "https://rpc.mainnet.chain.robinhood.com"
AAPL = "0xaF3D76f1834A1d425780943C99Ea8A608f8a93f9"
USDG = "0x5fc5360D0400a0Fd4f2af552ADD042D716F1d168"
FEED = "0x6B22A786bAa607d76728168703a39Ea9C99f2cD0"
FACTORY = "0x1f7d7550b1b028f7571e69a784071f0205fd2efa"
SWAP_TOPIC = "0xc42079f94a6350d7e6235f29174924f928cc2ac818eb64fed8004e115fbcca67"
FEES = [100, 500, 3000, 10000]
# Saturday noon inside each weekend we want to bracket.
WEEKENDS = [datetime(2026, 9, 5, 12, tzinfo=timezone.utc),
            datetime(2026, 9, 12, 12, tzinfo=timezone.utc)]
CALLS = 0


def rpc(method, params):
    global CALLS
    CALLS += 1
    body = json.dumps({"jsonrpc": "2.0", "id": 1, "method": method, "params": params}).encode()
    req = urllib.request.Request(RPC, body, {"content-type": "application/json",
                                             "user-agent": "curl/8"})
    for attempt in range(10):
        try:
            time.sleep(0.25)
            out = json.load(urllib.request.urlopen(req, timeout=60))
            break
        except urllib.error.HTTPError as e:
            if e.code != 429 or attempt == 9:
                raise
            # public RPC rate limit: back off (up to about five minutes in all), never hammer
            wait = e.headers.get("Retry-After") if e.headers else None
            time.sleep(float(wait) if wait and wait.isdigit() else min(60, 2 ** (attempt + 1)))
    if "error" in out:
        raise RuntimeError(out["error"])
    return out["result"]


def call(to, data):
    return rpc("eth_call", [{"to": to, "data": data}, "latest"])


def word(h, i):
    return int(h[2 + 64 * i: 2 + 64 * (i + 1)], 16)


def signed(n):
    return n - (1 << 256) if n >= 1 << 255 else n


def ts(t):
    return datetime.fromtimestamp(t, timezone.utc).strftime("%a %m-%d %H:%M")


# ---- feed: bracket each weekend's silence -------------------------------
def round_at(rid):
    res = call(FEED, "0x9a6fc8f5" + format(rid, "064x"))
    if not res or len(res) < 322:
        return None
    at = word(res, 3)
    return (word(res, 1) / 1e8, at) if at else None


def last_round_before(hi, target):
    phase = hi >> 64
    lo = (phase << 64) + 1
    while lo < hi:
        mid = (lo + hi + 1) // 2
        r = round_at(mid)
        if r is None or r[1] > target:
            hi = mid - 1
        else:
            lo = mid
    return lo


latest = call(FEED, "0xfeaf968c")
latest_rid = word(latest, 0)

# ---- blocks: timestamp -> block number ---------------------------------
def block_ts(n):
    b = rpc("eth_getBlockByNumber", [hex(n), False])
    return int(b["timestamp"], 16)


head = int(rpc("eth_blockNumber", []), 16)
head_ts = block_ts(head)
probe = head - 200000
probe_ts = block_ts(probe)
sec_per_block = (head_ts - probe_ts) / 200000.0
print("head block %d at %s UTC, ~%.2f s/block" % (head, ts(head_ts), sec_per_block))


def block_at(target):
    """First block whose timestamp >= target."""
    lo, hi = 0, head
    guess = head - int((head_ts - target) / sec_per_block)
    lo, hi = max(0, guess - 400000), min(head, guess + 400000)
    while lo < hi:
        mid = (lo + hi) // 2
        if block_ts(mid) < target:
            lo = mid + 1
        else:
            hi = mid
    return lo


# ---- pools --------------------------------------------------------------
def addr(a):
    return a[2:].lower().rjust(64, "0")


pools = []
for fee in FEES:
    res = call(FACTORY, "0x1698ee82" + addr(AAPL) + addr(USDG) + format(fee, "064x"))
    if res and int(res, 16):
        pool = "0x" + res[-40:]
        t0 = "0x" + call(pool, "0x0dfe1681")[-40:]
        pools.append((fee, pool, t0.lower() == USDG.lower()))
for fee, pool, usdg_is_0 in pools:
    print("pool %.2f%% %s  token0=%s" % (fee / 1e4, pool, "USDG" if usdg_is_0 else "AAPL"))


def get_logs(pool, a, b):
    # ~0.1 s blocks: a weekend is ~2M blocks. Ask for the whole range first and
    # only shrink when the node refuses, so the call count stays small.
    step = b - a + 1
    out = []
    lo = a
    while lo <= b:
        hi = min(b, lo + step - 1)
        try:
            out += rpc("eth_getLogs", [{"address": pool, "fromBlock": hex(lo), "toBlock": hex(hi),
                                        "topics": [SWAP_TOPIC]}])
            lo = hi + 1
        except RuntimeError as e:
            if step <= 5000:
                raise
            step //= 4
            print("   (getLogs range shrunk to %d blocks: %s)" % (step, str(e)[:70]))
    return out


# ---- per weekend ---------------------------------------------------------
for sat in WEEKENDS:
    before = last_round_before(latest_rid, int(sat.timestamp()))
    a = round_at(before)
    b = round_at(before + 1)
    if not a or not b:
        print("\ncould not bracket weekend of %s" % sat.date())
        continue
    fri_px, fri_at = a
    mon_px, mon_at = b
    print("\n=== weekend of %s: feed silent %s -> %s (%.1f h), $%.2f -> $%.2f (%+.2f%%)"
          % (sat.date(), ts(fri_at), ts(mon_at), (mon_at - fri_at) / 3600,
             fri_px, mon_px, (mon_px / fri_px - 1) * 100))
    blk_a, blk_b = block_at(fri_at), block_at(mon_at)
    print("   blocks %d .. %d (%d blocks)" % (blk_a, blk_b, blk_b - blk_a))
    fills = []
    for fee, pool, usdg_is_0 in pools:
        logs = get_logs(pool, blk_a, blk_b - 1)
        for lg in logs:
            d = lg["data"]
            a0, a1 = signed(word(d, 0)), signed(word(d, 1))
            usdg_raw, aapl_raw = (a0, a1) if usdg_is_0 else (a1, a0)
            if not usdg_raw or not aapl_raw:
                continue
            usdg = abs(usdg_raw) / 1e6
            aapl = abs(aapl_raw) / 1e18
            px = usdg / aapl
            side = "BUY " if aapl_raw < 0 else "SELL"   # pool sent AAPL -> taker bought
            fills.append((int(lg["blockNumber"], 16), fee, side, aapl, usdg, px))
    fills.sort()
    if not fills:
        print("   no swaps in any AAPL/USDG pool during the silence")
        continue
    # Block timestamps are interpolated between the two bracketing blocks
    # (0.1 s blocks -> minutes of error at most), so no per-block lookups.
    def when(n):
        return fri_at + (n - blk_a) * (mon_at - fri_at) / max(1, blk_b - blk_a)

    real = [f for f in fills if f[4] >= 1.0]          # ignore dust (< 1 USDG)
    buys = [f for f in real if f[2] == "BUY "]
    sells = [f for f in real if f[2] == "SELL"]
    print("   %d swaps (%d over 1 USDG): %.2f AAPL / %.0f USDG traded; buys %d / sells %d"
          % (len(fills), len(real), sum(f[3] for f in real), sum(f[4] for f in real),
             len(buys), len(sells)))
    # activity and pool price by 6-hour bucket
    print("   bucket (UTC start)   swaps   USDG     avg fill $   vs Fri    vs Mon")
    bucket = {}
    for f in real:
        k = int((when(f[0]) - fri_at) // 21600)
        bucket.setdefault(k, []).append(f)
    for k in sorted(bucket):
        fs = bucket[k]
        vol = sum(f[4] for f in fs)
        avg = vol / sum(f[3] for f in fs)
        print("   %s   %6d %9.0f   %8.2f   %+6.2f%%  %+6.2f%%"
              % (ts(fri_at + k * 21600), len(fs), vol, avg, (avg / fri_px - 1) * 100,
                 (avg / mon_px - 1) * 100))
    devs = [abs(f[5] / mon_px - 1) * 100 for f in real]
    worse = sum(1 for f in real if (f[2] == "BUY " and f[5] > mon_px) or
                (f[2] == "SELL" and f[5] < mon_px))
    big = sorted(real, key=lambda f: -f[4])[:8]
    print("   |fill - Monday open|: median %.2f%%, p90 %.2f%%, max %.2f%%; %d of %d takers did "
          "worse than waiting for the open"
          % (statistics.median(devs), sorted(devs)[int(len(devs) * 0.9)], max(devs), worse,
             len(real)))
    print("   largest fills:")
    for n, fee, side, aapl, usdg, px in big:
        print("     %s  %s %8.3f AAPL %9.0f USDG @ %.2f (%+.2f%% vs Mon)"
              % (ts(when(n)), side, aapl, usdg, px, (px / mon_px - 1) * 100))

print("\nrpc calls: %d" % CALLS)
