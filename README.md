# AfterHours

**A 24/7 price for tokenized stocks on Robinhood Chain.**
Chainlink while the market is open; the on-chain pool, bounded, while it is closed.
Arbitrum Stylus (Rust). Chainlink `AggregatorV3Interface` (+ v2 getters) and Morpho Blue `IOracle` compatible.

## Why

Robinhood Chain trades tokenized US stocks 24/7. Their Chainlink feeds follow US
market hours and go silent for **52 hours every weekend (76 on holiday weekends)**,
while the tokens keep trading on-chain: **$4-5M of AAPL alone changes hands per
weekend** in 26-44k swaps, at prices that move. The only live lending market on the
chain copes by hard-coding a **five-day** feed tolerance — so for 30% of every week
every price-dependent contract either trusts a dead price or stops.

AfterHours answers during the closure with where the token actually trades: a
30-minute Uniswap v3 TWAP, refused when the pool is thin, and bounded to a
circuit-breaker band around the last exchange print. All numbers are measured;
see [DESIGN.md](DESIGN.md).

## What it returns

| session | when | `latestRoundData().answer` | `price()` |
|---|---|---|---|
| `LIVE_FEED` (0) | feed no older than `liveMaxAge` | the feed's round, verbatim | feed × Morpho scale |
| `ONCHAIN_TWAP` (1) | feed older; the primary pool is deep enough over the window (a standby only while the primary cannot be observed) | that pool's TWAP clamped to ±`maxDeviationBps` of the last print | same |
| `PAUSED` (2) | issuer's `oraclePaused()` (corporate action) | reverts `IssuerPaused()` | reverts |
| `NO_DATA` (3) | feed round invalid / pool too thin over the window / no TWAP / last print older than `maxAnchorAge` | reverts `NoData(reason)` | reverts |

`state()` returns `(session, reason, answer, feedAnswer, feedUpdatedAt, twap, liquidity, clamped, pool)`
and never reverts for market reasons. Solidity interface: [`abi/IAfterHours.sol`](abi/IAfterHours.sol).

## Layout

```
src/lib.rs        the contract (Stylus SDK 0.10, no owner, no upgrade)
src/tickmath.rs   1.0001^tick in Q96 with 512-bit intermediates, reference-vector tested
src/tests.rs      unit tests; every external read is mocked with exact calldata
src/mockvm.rs     TestVM wrapper that serves the matched mock (stylus-test 0.10.9 serves the last registered)
e2e/              end-to-end on a local Nitro node: Solidity doubles + cast scenario, gas per read
abi/              Solidity interface for integrators
scripts/measure/  the evidence: feed cadence, weekend swaps, pool depth, PARE's oracle
scripts/probe.py  read a deployed AfterHours next to the raw feed and pool (stdlib only)
.github/          ci (fmt, clippy, tests, cargo stylus check, ABI) and manual deploy
```

## Build and test

Rust 1.91 (`rust-toolchain.toml`), `cargo-stylus` 0.10.9.

```bash
cargo test
cargo stylus check --endpoint https://rpc.testnet.chain.robinhood.com
cargo stylus export-abi
```

End-to-end (real wasm on a local Nitro dev node with Solidity doubles, 50
assertions, gas per read): `.github/workflows/e2e.yml` runs `e2e/run.sh`
against OffchainLabs' `nitro-devnode` upgraded to ArbOS 61 — the version
Robinhood Chain runs, and the one a 2-fragment (33 KB) program needs.

Deploy (deploys, activates, then runs the one-shot `initialize`):

```bash
cargo stylus deploy --endpoint <rpc> --private-key <key> --no-verify
cast send <address> "initialize(address,address[],address,uint64,uint32,uint64,uint128,uint64)" \
  <feed> "[<pool 0.05%>,<pool 0.30%>,<pool 1%>]" <stock> 21600 1800 1000 200000000000000000 432000
# liveMaxAge twapWindow maxDeviationBps minLiquidity(window harmonic mean) maxAnchorAge
# up to three pools of the stock against one quote token. The first is the primary and
# prices the asset; the others are standbys, read only while the primary cannot be observed.
# The deploy workflow refuses a primary whose observation cardinality is below window + 1.
```

## Integrating

**Morpho Blue market** — AfterHours is the market's `oracle`; nothing else changes:

```solidity
MarketParams({
    loanToken:       USDG,          // 0x5fc5360D0400a0Fd4f2af552ADD042D716F1d168
    collateralToken: AAPL,          // 0xaF3D76f1834A1d425780943C99Ea8A608f8a93f9
    oracle:          afterHoursAAPL,
    irm:             AdaptiveCurveIrm, // 0x2BD3d5965B26B51814AC95127B2b80dD6CcC0fa1 on Robinhood Chain
    lltv:            0.625e18
});
```

`price()` returns USDG per raw AAPL unit scaled by 1e36, exactly what Morpho
expects; on a weekend it keeps answering, so liquidations and borrows work.
In PAUSED / NO_DATA it reverts, which freezes borrow, withdrawCollateral and
liquidate (supply and repay keep working) — the same behaviour a stale-feed
guard would give, but only when there is genuinely nothing to stand behind.

**Any Chainlink consumer** — swap the feed address:

```solidity
(, int256 answer,, uint256 updatedAt,) = AggregatorV3Interface(afterHoursAAPL).latestRoundData();
// answer > 0 and updatedAt fresh in both LIVE_FEED and ONCHAIN_TWAP; the call
// reverts (IssuerPaused / NoData) instead of returning a price it cannot defend.
```

**Keepers and dashboards** — `state()` never reverts for market reasons:

```solidity
(uint8 session, uint8 reason,,,, uint256 twap, uint128 liquidity, bool clamped, address pool) =
    IAfterHours(afterHoursAAPL).state();
// session 0 LIVE_FEED, 1 ONCHAIN_TWAP, 2 PAUSED, 3 NO_DATA (reason 1-4)
// alert on: session 3 for more than a window; clamped == true; liquidity near the floor
```

`scripts/probe.py --oracle <address> --watch 300` prints the same next to the
raw feed and pool; the hourly `status` workflow appends it to `status/log.md`.

**Corporate actions** — the oracle prices raw token units, as the pool and the
feed do (Robinhood's stock tokens scale their UI balance with a multiplier),
so a split changes nothing here; while the issuer processes one, the token's
`oraclePaused()` flag makes AfterHours refuse until the feed is back.

## Every stock, not just AAPL

`scripts/measure/discover_assets.py` asks the chain, for every token the
explorer lists as a Robinhood stock token (122 with 100+ holders), what
`initialize` and an attacker would check: that it is Robinhood's own (the
same token beacon as AAPL; 15 imitations are not), the pause flag, 18
decimals, a Chainlink feed named `Robinhood <SYMBOL> / USD`, and for each
USDG fee tier the liquidity, the observation cardinality, whether a
30-minute `observe` answers now and the USDG it takes to move the price 2%.
It writes the deploy inputs per asset to `assets.json`. On 2026-09-23:

| tier | count | stocks |
|---|---|---|
| recommended: primary cardinality >= 1,801 and >= $50k of 2% depth | 14 | NVDA, SPCX, GOOGL, USO, AAPL, SPY, AMZN, MSFT, QQQ, CRCL, GME, SLV, TSLA, MU |
| the deploy workflow accepts them; 2% depth under $50k | 4 | META, PLTR, BABA, TSM |
| `initialize` accepts them; the primary's cardinality must be raised first (anyone can, gas only) | 10 | SGOV, MSTR, DELL, AMD, INTC, USAR, ASML, SNDK, RKLB, IONQ |
| Robinhood's own, but no Chainlink feed or no USDG liquidity today | 79 | |

The same wasm serves all of them. A stock moves up a tier the day its feed
or pool appears, with no code change.

## Deployments

See [DEPLOYMENTS.md](DEPLOYMENTS.md).

## Status

Built for the Arbitrum Open House Singapore buildathon (September 2026).
Unaudited. MIT.
