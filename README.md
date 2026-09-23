# AfterHours

**A 24/7 price for tokenized stocks on Robinhood Chain.**
Chainlink while the market is open; the on-chain pool, bounded, while it is closed.
Arbitrum Stylus (Rust). Chainlink `AggregatorV3Interface` (+ v2 getters) and Morpho Blue `IOracle` compatible.

## Why

Robinhood Chain trades tokenized US stocks 24/7. Their Chainlink feeds follow US
market hours and go silent for **52-57 hours every weekend (76 on holiday weekends)**,
while the tokens keep trading on-chain: **$4-5M of AAPL alone changes hands per
weekend** in 26-44k swaps, at prices that move. Chainlink's own page for these
feeds says they may hold the last price while the market is closed, with no
heartbeat off-hours, and leaves the staleness bound to each integrator.

Lending against these tokens has started and stalled. On 2026-09-23 Morpho Blue on
this chain had **83 funded markets with a Robinhood stock token as collateral:
$0.88M supplied, $6.4k borrowed** (`scripts/measure/morpho_markets.py`). Their
oracles either keep Friday's print through the weekend (Morpho's standard
`ChainlinkOracleV2` has no staleness check, the largest markets' custom oracles
expose none, and the two that do allow four days) or read a raw Uniswap pool
price. Outside those markets, PARE's pSPY oracle hard-codes a five-day
tolerance. Each is a way of living with a price that stops for 30% of the week.

AfterHours answers during the closure with where the token actually trades: the
median of three 10-minute Uniswap v3 averages, refused when the pool is thin, and
bounded to a band around the last exchange print. On a weekday the band is ±1%
while that print is younger than the feed's 24-hour heartbeat (a feed that is
only quiet looks the same as one that has just closed); after that, and all
weekend, it is ±10%. All numbers are measured; see [DESIGN.md](DESIGN.md).

## What it returns

| session | when | `latestRoundData().answer` | `price()` |
|---|---|---|---|
| `LIVE_FEED` (0) | feed no older than `liveMaxAge`, and no share-multiplier change since its print | the feed's round, verbatim (per share) | answer × `uiMultiplier` × Morpho scale |
| `ONCHAIN_TWAP` (1) | otherwise, when the primary pool is deep enough (median of three 10-minute windows; a standby only while the primary cannot be observed) | the median of that pool's three 10-minute averages, per share, clamped to ±`quietBandBps` of the last print while it is younger than `heartbeat` on a weekday, ±`maxDeviationBps` after that and on Saturdays and Sundays (UTC) | same |
| `PAUSED` (2) | issuer's `oraclePaused()` (corporate action) | reverts `IssuerPaused()` | reverts |
| `NO_DATA` (3) | 1 feed round invalid / 2 pool too thin / 3 no TWAP / 4 last print older than `maxAnchorAge` / 5 a share-multiplier change after the last print moved the pool past the band (a split) | reverts `NoData(reason)` | reverts; also `NoData(6)` alone when the answer is too large for Morpho's scale (absurd prices only) |

Units: Chainlink prices one share; a Robinhood stock token is a scaled-UI token
whose raw unit is `uiMultiplier / 1e18` shares (1.00057 for AAPL, 1.0051 for SGOV
on 2026-09-23; dividends raise it, a split multiplies it). Every Chainlink-shaped
answer here is per share, like the feed. `price()` values raw collateral units, so
Morpho sees the right collateral value through dividends and splits.

`state()` returns `(session, reason, answer, feedAnswer, feedUpdatedAt, twap, liquidity, clamped, pool)`
and never reverts for market reasons. Solidity interface: [`abi/IAfterHours.sol`](abi/IAfterHours.sol).

## Live page

`web/index.html` shows AAPL's Chainlink feed, its primary pool's price (the
median of three 10-minute averages) and AfterHours' answer side by side, the feed's last seven days of
prints, and the same rules applied to every recommended stock. It is static:
every price, age and pool figure is read from Robinhood Chain by the
visitor's browser, and before an instance is deployed the AfterHours column
runs the contract's decision rules ported with the same integer math (the
page checks its port against the exact prices of the e2e test on load).
After deployment it reads the contract's `state()` and compares.

```bash
python web/build_data.py
python -m http.server 8745 --directory web
```

Published by the manual `pages` workflow once GitHub Pages is enabled.

## Verify it in five minutes

| claim | how to check it |
|---|---|
| The feed goes silent for 52-57 hours every weekend | the live page's print tape, or `python scripts/measure/feed_cadence.py` (stdlib, about 60 reads, a minute) |
| AAPL trades $4-5M on-chain every weekend | `python scripts/measure/weekend_swaps.py` (slow: about 250 reads and 30 minutes, as the public RPC narrows each log query) |
| 83 funded Morpho markets lend against stock tokens, priced by stale-tolerant or raw-pool oracles | `python scripts/measure/morpho_markets.py` (every CreateMarket event, each market's supply and borrow, each oracle's feed; about 1,500 reads in batches, a few minutes) |
| 67 unit and property tests, 73 on-chain assertions on ArbOS 61 | the latest `ci` and `e2e` runs under Actions; the e2e artifact `result.txt` lists every assertion and the gas per read |
| The price math is exact | the live page's self-check line, and the 80-digit reference vectors from `scripts/measure/tick_vectors.py`, pasted verbatim into `src/tickmath.rs` |
| It keeps answering while the market is closed | the live page on a weekend: the Chainlink print is hours old and AfterHours answers from the pool; after deployment, `python scripts/probe.py --oracle <address>` and the hourly `status/log.md` |
| Moving AAPL's pool 10% takes $234k up or $314k down; the pool clears the depth floor from -4.6% to +3.2%; the cheapest refusal parks $235k at +17% for a second, once every ten minutes | `python scripts/measure/pool_depth.py` (walks every initialized tick over the pool's whole range; about 900 reads in batches, a minute) |
| Robinhood Chain runs ArbOS 61 with Stylus 3, programs expire after 365 days, mainnet has no StylusDeployer, blocks average 0.101 s | `python scripts/measure/stylus_params.py` (the chain's own precompiles, mainnet and testnet; seconds) |
| 28 stocks can be deployed today, 14 meet the bar | `assets.json`, regenerated by `scripts/measure/discover_assets.py` (about an hour of reads) |

## What it does not do

- It does not predict Monday's open. During the closure it reports where the token trades on-chain; a gap caused by weekend news still lands on Monday, bounded by the band.
- On a weekday it moves at most 1% from a print younger than 24 hours. That is right while the feed is only quiet, but Friday evening after the close and a weekday exchange holiday look the same on-chain, so a move larger than 1% then shows in full only from Saturday 00:00 UTC or once the heartbeat has passed. Saturdays and Sundays always get the wide band: the weekend never moves, so no calendar has to be maintained.
- It does not price a large move the pool's liquidity has not followed. LPs concentrate around the current price: AAPL's pool clears the 2e17 depth floor from -4.6% to +3.2% of today's price. A move past that, held for ten minutes, refuses (`NoData(2)`) until LPs re-center or the feed prints, because a pool nobody provides at that price is the pool that is cheap to push. The floor is set per instance; a lower one answers further out and lets a manipulator hold the price in thinner water.
- It does not follow liquidity. The primary pool is fixed at deployment; if liquidity leaves it for good, the instance refuses and a new one is deployed.
- It treats USDG as one dollar. Chainlink's USDG/USD feed on this chain updates only on 0.5% moves, so reading it would correct only a larger depeg; that is a v2 option.
- It prices against one quote token per instance (USDG today).
- No L2 sequencer-uptime check.
- It depends on the issuer's token interface. If an upgrade of the Robinhood token removed `oraclePaused()`, `uiMultiplier()` or `effectiveAt()`, every price read would revert for good (there is no admin by design). A Morpho market cannot change its oracle, so in a market priced by that instance borrowing and liquidation would stop permanently; borrowers could still repay and then withdraw their collateral, and lenders withdraw what is not lent out. The feed's history stays readable through `getRoundData`. A curator should size a market with that in mind; a new market on a new instance is the way forward.
- Like every Stylus program it must be kept alive: activation lasts 365 days on this chain (`ArbWasm.expiryDays()`), after which anyone can re-activate it.
- Unaudited.

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
web/              the live page (static HTML; reads the chain from the browser) and its data builder
.github/          ci (fmt, clippy, tests, cargo stylus check, ABI), e2e, manual deploy, hourly status, pages
```

## Build and test

Rust 1.91 (`rust-toolchain.toml`), `cargo-stylus` 0.10.9.

```bash
cargo test
cargo stylus check --endpoint https://rpc.testnet.chain.robinhood.com
cargo stylus export-abi
```

End-to-end (real wasm on a local Nitro dev node with Solidity doubles, 73
assertions, gas per read): `.github/workflows/e2e.yml` runs `e2e/run.sh`
against OffchainLabs' `nitro-devnode` upgraded to ArbOS 61 — the version
Robinhood Chain runs, and the one a 2-fragment program (about 38 KB) needs.

Deploy (deploys, activates, then runs the one-shot `initialize`):

```bash
cargo stylus deploy --endpoint <rpc> --private-key <key> --no-verify
cast send <address> "initialize(address,address[],address,uint64,uint32,uint64,uint128,uint64,uint64,uint64)" \
  <feed> "[<primary pool>]" <stock> 21600 1800 1000 200000000000000000 432000 86400 100
# liveMaxAge twapWindow maxDeviationBps minLiquidity maxAnchorAge heartbeat quietBandBps
# one to three pools of the stock against one quote token. The first is the primary and
# prices the asset; optional standbys are read only while the primary cannot be observed.
# The deploy workflow refuses a primary whose observation cardinality is below window + 1.
```

## Integrating

**Morpho Blue market** — a market's oracle is fixed when it is created, so an
existing market cannot switch; a new one is opened with AfterHours as its `oracle`:

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
expects: the per-share answer times the token's `uiMultiplier`. On a weekend it
keeps answering, so liquidations and borrows work.
In PAUSED / NO_DATA it reverts, which freezes borrowing, liquidation and
withdrawing collateral against open debt (supply, repay and debt-free
withdrawals keep working) — the same behaviour a stale-feed guard would give,
but only when there is genuinely nothing to stand behind.

**Any Chainlink consumer** — swap the feed address:

```solidity
(, int256 answer,, uint256 updatedAt,) = AggregatorV3Interface(afterHoursAAPL).latestRoundData();
// answer > 0 in LIVE_FEED and ONCHAIN_TWAP. In ONCHAIN_TWAP updatedAt is the read
// time: AfterHours has already applied the staleness policy (maxAnchorAge, the
// band), so a consumer's own staleness check passes by design. Otherwise the
// call reverts (IssuerPaused / NoData) instead of returning a price it cannot defend.
```

**Keepers and dashboards** — `state()` never reverts for market reasons:

```solidity
(uint8 session, uint8 reason,,,, uint256 twap, uint128 liquidity, bool clamped, address pool) =
    IAfterHours(afterHoursAAPL).state();
// session 0 LIVE_FEED, 1 ONCHAIN_TWAP, 2 PAUSED, 3 NO_DATA (reason 1-5)
// alert on: session 3 for more than a window; clamped == true; liquidity near the floor
```

`scripts/probe.py --oracle <address> --watch 300` prints the same next to the
raw feed and pool; the hourly `status` workflow appends it to `status/log.md`.

**Corporate actions** — while the issuer processes one, the token's
`oraclePaused()` flag makes AfterHours refuse. When a new share multiplier
takes effect (`effectiveAt()`), the last print is in pre-action shares: AfterHours
stops passing it through and prices from the pool per share until the feed
prints again. A dividend moves the multiplier by a fraction of a percent and the
pool stays inside the band; a split moves it past the band and AfterHours
refuses (`NoData(5)`) instead of clamping to a wrong price.

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
