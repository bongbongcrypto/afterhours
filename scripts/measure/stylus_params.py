# -*- coding: utf-8 -*-
"""What Stylus looks like on Robinhood Chain: the ArbOS version, the Stylus
version and how long an activated program stays alive, read from the chain's
own precompiles on mainnet and testnet, plus the average block time.

  ArbSys  0x...64  arbOSVersion()   returns 55 + the ArbOS version
  ArbWasm 0x...71  stylusVersion()  the Stylus version programs activate under
                   expiryDays()     days before an activated program must be re-activated

Read-only, stdlib; selectors from keccak.py.

    python scripts/measure/stylus_params.py
"""
import io
import json
import sys
import time
import urllib.error
import urllib.request
from datetime import datetime, timezone
from pathlib import Path

sys.stdout = io.TextIOWrapper(sys.stdout.buffer, encoding="utf-8", errors="replace")
sys.path.insert(0, str(Path(__file__).resolve().parent))
from keccak import selector  # noqa: E402

CHAINS = {
    "Robinhood Chain mainnet (4663)": "https://rpc.mainnet.chain.robinhood.com",
    "Robinhood Chain testnet (46630)": "https://rpc.testnet.chain.robinhood.com",
}
ARBSYS = "0x0000000000000000000000000000000000000064"
ARBWASM = "0x0000000000000000000000000000000000000071"
# the canonical StylusDeployer factory; a Stylus constructor needs it on the chain
STYLUS_DEPLOYER = "0xcEcba2F1DC234f70Dd89F2041029807F8D03A990"
SPAN = 864_000   # blocks for the average block time (one day at 0.1 s)


def rpc(url, method, params):
    body = json.dumps({"jsonrpc": "2.0", "id": 1, "method": method, "params": params}).encode()
    req = urllib.request.Request(url, body, {"content-type": "application/json", "user-agent": "curl/8"})
    for attempt in range(6):
        try:
            time.sleep(0.25)
            out = json.load(urllib.request.urlopen(req, timeout=30))
            break
        except urllib.error.HTTPError as e:
            if e.code != 429 or attempt == 5:
                raise
            time.sleep(4 * (attempt + 1))   # public RPC rate limit: back off, never hammer
    if "error" in out:
        return None
    return out["result"]


def uint(url, to, sig):
    res = rpc(url, "eth_call", [{"to": to, "data": selector(sig)}, "latest"])
    return int(res, 16) if res and res != "0x" else None


def main():
    print("read %s UTC" % datetime.now(timezone.utc).strftime("%Y-%m-%d %H:%M"))
    for name, url in CHAINS.items():
        chain_id = int(rpc(url, "eth_chainId", []), 16)
        arbos = uint(url, ARBSYS, "arbOSVersion()")
        stylus = uint(url, ARBWASM, "stylusVersion()")
        expiry = uint(url, ARBWASM, "expiryDays()")
        code = rpc(url, "eth_getCode", [STYLUS_DEPLOYER, "latest"])
        print("%s  chainId %d" % (name, chain_id))
        print("  ArbOS %s (arbOSVersion() = %s)" % (arbos - 55 if arbos else "?", arbos))
        print("  stylusVersion() = %s   expiryDays() = %s" % (stylus, expiry))
        print("  StylusDeployer %s: %s" % (STYLUS_DEPLOYER, "has code" if code and code != "0x" else "no code"))
        # average block time over the last SPAN blocks (headers are kept for the whole chain)
        head = int(rpc(url, "eth_blockNumber", []), 16)
        t1 = int(rpc(url, "eth_getBlockByNumber", [hex(head), False])["timestamp"], 16)
        t0 = int(rpc(url, "eth_getBlockByNumber", [hex(head - SPAN), False])["timestamp"], 16)
        print("  block %d: %.3f s per block over the last %s blocks (%.1f days)"
              % (head, (t1 - t0) / SPAN, format(SPAN, ","), (t1 - t0) / 86400))


if __name__ == "__main__":
    main()
