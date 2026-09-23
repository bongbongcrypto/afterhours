# -*- coding: utf-8 -*-
"""Write web/data.json, the static half of the live page.

The page reads every price, age and pool figure from Robinhood Chain itself;
this file only tells it where to look: which feed, pools and token belong to
each stock, the oracle's parameters, and which instances are deployed.

Sources:
  assets.json       discovery manifest (scripts/measure/discover_assets.py)
  deployments.json  deployed AfterHours instances (written after a deploy)

    python web/build_data.py
"""
import io
import json
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
OUT = Path(__file__).resolve().parent / "data.json"

# The oracle's parameters are the manifest's defaults, which are the deploy
# workflow's defaults. Each stock's pools and floor come from its deploy block
# in assets.json, which carries the AAPL instance's own inputs (DEPLOYMENTS.md),
# so the page reads exactly the pools an instance reads.
PARAM_KEYS = ("liveMaxAge", "twapWindow", "maxDeviationBps", "maxAnchorAge", "heartbeat", "quietBandBps")
HERO = "AAPL"


def primary_cardinality(asset):
    primary = asset["deploy"]["pools"][0].lower()
    return next(p["cardinality"] for p in asset["pools"] if p["address"].lower() == primary)


def main():
    manifest = json.load(io.open(ROOT / "assets.json", encoding="utf-8"))
    deployed = {}
    for d in json.load(io.open(ROOT / "deployments.json", encoding="utf-8"))["deployments"]:
        if d.get("chainId", 4663) == 4663:
            deployed[d["asset"].upper()] = d["address"]

    quote = manifest["quote"].lower()
    assets = []
    for a in manifest["assets"]:
        if not a.get("recommended") and a["symbol"].upper() not in deployed:
            continue
        stock = a["stock"]
        dep = a["deploy"]
        assets.append({
            "symbol": a["symbol"],
            "stock": stock,
            "feed": dep["feed"],
            "description": dep["expected_description"],
            "pools": dep["pools"],
            "poolFees": [next(("%g%%" % (x["fee"] / 10000)) for x in a["pools"] if x["address"].lower() == addr.lower())
                         for addr in dep["pools"]],
            "stockIsToken0": stock.lower() < quote,
            "stockDecimals": a["decimals"],
            "minLiquidity": dep["min_liquidity"],
            "depth2pctUsd": a["pools"][0]["depth_2pct_usd"],
            "oracle": deployed.get(a["symbol"].upper()),
        })
    assets.sort(key=lambda x: (x["symbol"] != HERO, -x["depth2pctUsd"]))
    if not assets or assets[0]["symbol"] != HERO:
        sys.exit("the hero asset %s is not in the manifest" % HERO)

    data = {
        "source": "assets.json generated %s; deployments.json" % manifest["generated"],
        "chainId": 4663,
        "rpc": "https://rpc.mainnet.chain.robinhood.com",
        "explorer": "https://robinhoodchain.blockscout.com",
        "repo": "https://github.com/bongbongcrypto/afterhours-oracle",
        "quote": {"symbol": "USDG", "address": manifest["quote"], "decimals": 6},
        "params": {k: manifest["defaults"][k] for k in PARAM_KEYS},
        "hero": HERO,
        "counts": {
            "explorerTokens": len(manifest["assets"]) + len(manifest["not_deployable"]),
            "withFeedAndPool": len(manifest["assets"]),
            # initialize refuses a primary keeping fewer than twapWindow + 1 observations
            "passInitialize": sum(1 for a in manifest["assets"]
                                  if primary_cardinality(a) >= manifest["defaults"]["twapWindow"] + 1),
            "recommended": sum(1 for a in manifest["assets"] if a["recommended"]),
        },
        "assets": assets,
    }
    io.open(OUT, "w", encoding="utf-8", newline="\n").write(json.dumps(data, indent=1) + "\n")
    print("wrote %s: %d assets, %d deployed" % (OUT, len(assets), sum(1 for a in assets if a["oracle"])))


if __name__ == "__main__":
    main()
