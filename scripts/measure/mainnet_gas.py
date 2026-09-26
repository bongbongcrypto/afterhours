# -*- coding: utf-8 -*-
"""Gas per read of the deployed AfterHours AAPL instance on Robinhood Chain
mainnet, measured the way e2e/run.sh measures it on the dev node (`cast
estimate`): eth_estimateGas for latestRoundData() and price(), sent from the
zero address, at the latest block. Read-only: estimates and calls, no
transaction, stdlib.

On an Arbitrum chain the gas of a transaction also buys the posting of its
calldata to the parent chain, so eth_estimateGas includes an L1 data part
priced from the chain's L1 base fee estimate. Arbitrum's NodeInterface
(gasEstimateComponents at 0x...c8, answered by the node itself) splits it
out; the rest is the 21,000 transaction base plus execution. The session the
instance is in comes from its state(), the cost in dollars from Chainlink's
ETH / USD feed on this chain (address from Chainlink's feed directory, see
feed_directory.py; the script checks its description()).

The public RPC keeps recent state only, so each session is measured while
the instance is in it: LIVE_FEED during a US trading session while the feed
is under six hours old, ONCHAIN_TWAP overnight and on weekends.

    python scripts/measure/mainnet_gas.py [--oracle 0x...] [--rpc URL]

Selectors from keccak.py: latestRoundData() 0xfeaf968c, price() 0xa035b1fe,
state() 0xc19d93fb, description() 0x7284e416,
gasEstimateComponents(address,bool,bytes) 0xc94e6eeb.
"""
import argparse
import io
import sys
from datetime import datetime, timezone
from pathlib import Path

sys.stdout = io.TextIOWrapper(sys.stdout.buffer, encoding="utf-8", errors="replace")
sys.path.insert(0, str(Path(__file__).resolve().parent))
from jsonrpc import post  # noqa: E402
from keccak import selector  # noqa: E402

RPC = "https://rpc.mainnet.chain.robinhood.com"
ORACLE = "0x69190621e300cd2bc4cbb80777b517691ee80f65"   # AfterHours AAPL, DEPLOYMENTS.md
NODE_INTERFACE = "0x00000000000000000000000000000000000000c8"
ETH_USD = "0x78F3556b67E17Df817D51Ef5a990cDaF09E8d3A9"  # Chainlink ETH / USD, 8 decimals
ZERO = "0x" + "00" * 20
SESSIONS = {0: "LIVE_FEED", 1: "ONCHAIN_TWAP", 2: "PAUSED", 3: "NO_DATA"}


def rpc(url, method, params):
    out = post(url, {"jsonrpc": "2.0", "id": 1, "method": method, "params": params})
    if "error" in out:
        raise SystemExit("%s failed: %s" % (method, out["error"]))
    return out["result"]


def words(h):
    return [int(h[2 + 64 * i: 66 + 64 * i], 16) for i in range((len(h) - 2) // 64)]


def text(h):
    return bytes.fromhex(h[2 + 128: 2 + 128 + 2 * words(h)[1]]).decode()


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--oracle", default=ORACLE)
    ap.add_argument("--rpc", default=RPC)
    a = ap.parse_args()

    head = rpc(a.rpc, "eth_getBlockByNumber", ["latest", False])
    block, ts = int(head["number"], 16), int(head["timestamp"], 16)
    at = hex(block)   # every read below at this one block

    def call(to, data):
        return rpc(a.rpc, "eth_call", [{"from": ZERO, "to": to, "data": data}, at])

    session = SESSIONS.get(words(call(a.oracle, selector("state()")))[0], "?")
    if text(call(ETH_USD, selector("description()"))) != "ETH / USD":
        raise SystemExit("%s is not the ETH / USD feed" % ETH_USD)
    eth_usd = words(call(ETH_USD, selector("latestRoundData()")))[1] / 1e8

    print("Robinhood Chain mainnet, block %d, %s UTC"
          % (block, datetime.fromtimestamp(ts, timezone.utc).strftime("%Y-%m-%d %H:%M")))
    print("AfterHours %s, session %s; ETH / USD %.2f (Chainlink)\n" % (a.oracle, session, eth_usd))
    print("  %-18s %9s %8s %10s %16s %8s" % ("read", "estimate", "L1 part", "base+exec", "cost at base fee", "USD"))
    for sig in ("latestRoundData()", "price()"):
        data = selector(sig)
        est = int(rpc(a.rpc, "eth_estimateGas", [{"from": ZERO, "to": a.oracle, "data": data}, at]), 16)
        # gasEstimateComponents(address to, bool contractCreation, bytes data)
        arg = (a.oracle[2:].lower().rjust(64, "0") + "0" * 64 + format(0x60, "064x")
               + format(len(data[2:]) // 2, "064x") + data[2:].ljust(64, "0"))
        total, l1, base_fee, l1_base_fee = words(call(NODE_INTERFACE, selector("gasEstimateComponents(address,bool,bytes)") + arg))
        eth = est * base_fee / 1e18
        print("  %-18s %9s %8s %10s %12.3g ETH %8.4f"
              % (sig, format(est, ","), format(l1, ","), format(total - l1, ","), eth, eth * eth_usd))
        if total != est:
            print("    (NodeInterface total %s differs from eth_estimateGas %s)" % (format(total, ","), format(est, ",")))
    print("\n  base fee %.4f gwei; the chain's L1 base fee estimate %d wei. The L1 part pays for posting the"
          "\n  calldata to the parent chain and is priced from that estimate; base+exec includes the 21,000"
          "\n  transaction base, which a transaction reading the oracle (a Morpho borrow or liquidation)"
          "\n  pays once for all its work." % (base_fee / 1e9, l1_base_fee))


if __name__ == "__main__":
    main()
