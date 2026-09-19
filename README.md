# AfterHours

**A 24/7 price for tokenized stocks on Robinhood Chain.**
Chainlink while the market is open; the on-chain pool, bounded, while it is closed.
Arbitrum Stylus (Rust). Chainlink `AggregatorV3Interface` + Morpho Blue `IOracle` compatible.

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
| `LIVE_FEED` (0) | feed younger than `liveMaxAge` | the feed's round, verbatim | feed × Morpho scale |
| `ONCHAIN_TWAP` (1) | feed older; pool deep enough | pool TWAP clamped to ±`maxDeviationBps` of the last print | same |
| `PAUSED` (2) | issuer's `oraclePaused()` (corporate action) | reverts `IssuerPaused()` | reverts |
| `NO_DATA` (3) | feed round invalid / pool too thin / no TWAP | reverts `NoData(reason)` | reverts |

`state()` returns `(session, reason, answer, feedAnswer, feedUpdatedAt, twap, liquidity, clamped)`
and never reverts for market reasons. Solidity interface: [`abi/IAfterHours.sol`](abi/IAfterHours.sol).

## Layout

```
src/lib.rs        the contract (Stylus SDK 0.10, no owner, no upgrade)
src/tickmath.rs   1.0001^tick in Q96 with 512-bit intermediates, reference-vector tested
src/tests.rs      TestVM unit tests; every external read is mocked with exact calldata
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

Deploy (deploys, activates, then runs the one-shot `initialize`):

```bash
cargo stylus deploy --endpoint <rpc> --private-key <key> --no-verify
cast send <address> "initialize(address,address,address,uint64,uint32,uint64,uint128)" \
  <feed> <pool> <stock> 21600 1800 1000 200000000000000000
```

## Deployments

See [DEPLOYMENTS.md](DEPLOYMENTS.md).

## Status

Built for the Arbitrum Open House Singapore buildathon (September 2026).
Unaudited. MIT.
