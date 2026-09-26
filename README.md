# AfterHours

**A 24/7 price for tokenized stocks on Robinhood Chain.**
Chainlink while the market is open; the on-chain pool, bounded, while it is closed.
Arbitrum Stylus (Rust). Chainlink `AggregatorV3Interface` (+ v2 getters) and Morpho Blue `IOracle` compatible.

## Start here

| | |
|---|---|
| Live page (your browser reads Robinhood Chain directly) | https://bongbongcrypto.github.io/afterhours/ |
| The AAPL instance on Robinhood Chain mainnet (4663) | [DEPLOYMENTS.md](DEPLOYMENTS.md): the current instance's address, deployment, activation, `initialize` and the Morpho AAPL/USDG market it prices. The first instance, `0x69190621e300cd2bc4cbb80777b517691ee80f65` (2026-09-25), treated the feed as a price per share and is superseded: do not integrate it (see [Units](#units)) |
| Answers through the 09-26 weekend closure, every 10 minutes | [`status/10min.md`](status/10min.md): LIVE_FEED while Friday's print was fresh, then ONCHAIN_TWAP from the pool. Recorded from the superseded first instance, so its ONCHAIN_TWAP answers are per share, 0.057% under the per-token price the current contract gives |
| Read it yourself (Python stdlib only) | `python scripts/probe.py --oracle <address>` prints the feed, the pool and what AfterHours answers, side by side, and checks that `price()` is the answer times Morpho's scale |
| Tests | 71 unit and property tests in CI; 89 on-chain assertions on a local ArbOS 61 node in e2e ([Actions](https://github.com/bongbongcrypto/afterhours/actions)) |

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
bounded to a band around the last exchange print. During the US regular
session the band is ±1% while that print is younger than the feed's 24-hour
heartbeat: the feed prints any 0.5% move then, so a quiet feed means a quiet
market. At every other hour it is ±10%, because overnight the feed can hold a
move until the next open. All numbers are measured; see [DESIGN.md](DESIGN.md).

## What it returns

| session | when | `latestRoundData().answer` | `price()` |
|---|---|---|---|
| `LIVE_FEED` (0) | feed no older than `liveMaxAge` | the feed's round, verbatim | answer × Morpho scale |
| `ONCHAIN_TWAP` (1) | otherwise, when the primary pool is deep enough (median of three 10-minute windows; a standby only while the primary cannot be observed) | the median of that pool's three 10-minute averages, clamped to ±`quietBandBps` of the last print during the US regular session (Monday to Friday, 14:30-20:00 UTC) while the print is younger than `heartbeat`, ±`maxDeviationBps` at every other hour and past the heartbeat | same |
| `PAUSED` (2) | issuer's `oraclePaused()` (corporate action) | reverts `IssuerPaused()` | reverts |
| `NO_DATA` (3) | 1 feed round invalid (or a print so large the band arithmetic overflows) / 2 pool too thin / 3 no TWAP / 4 last print older than `maxAnchorAge` (5 is reserved: only the superseded first instance raised it) | reverts `NoData(reason)` | reverts; also `NoData(6)` alone when the answer is too large for Morpho's scale (absurd prices only) |

### Units

Every answer is the price of one token of raw balance: 10^18 raw units, what
ERC-20 transfers, Uniswap pools and Morpho collateral count. That is the unit
Chainlink's Robinhood feeds price. [Chainlink's page for these
feeds](https://docs.chain.link/data-feeds/tokenized-equity-feeds/robinhood)
defines the reported price as "Token Price = Underlying Equity Market Price ×
Multiplier", with the multiplier read from the token's `uiMultiplier()`, so
the answer stays continuous through dividends and splits: in its 10:1 split
example the stock goes from $200 to $20, the multiplier from 1 to 10, and the
token price stays $200. The pool trades the same raw tokens, so its price is
compared with the print as it is, and `price()` is the answer times Morpho's
scale. AfterHours never reads the multiplier. During a large corporate action
the issuer sets `oraclePaused()`, AfterHours refuses, and Chainlink holds the
last good token price until the equity price and the new multiplier agree.

Correction: the first deployment (`0x69190621…0f65`, 2026-09-25) treated the
answer as a price per share. Its `price()` multiplies by `uiMultiplier` and its
pool price is divided by it: 0.057% off for AAPL today (multiplier 1.00057),
about ten times the collateral value after a 10:1 split. It is superseded; its
Morpho market holds no supply ([REVIEWS.md](REVIEWS.md), 2026-09-27).

`state()` returns `(session, reason, answer, feedAnswer, feedUpdatedAt, twap, liquidity, clamped, pool)`
and never reverts for market reasons. Solidity interface: [`abi/IAfterHours.sol`](abi/IAfterHours.sol).

## Live page

`web/index.html` shows AAPL's Chainlink feed, its primary pool's price (the
median of three 10-minute averages) and AfterHours' answer side by side, the feed's last seven days of
prints, and the same rules applied to every recommended stock. It is static:
every price, age and pool figure is read from Robinhood Chain by the
visitor's browser. The AfterHours column is the deployed instance's own
`state()`; next to it the page runs the contract's decision rules, ported
with the same integer math, on the feed and pool data of the same block and
says whether the two agree (on load it also checks its port against exact
prices and four band and 10:1-split decisions from the contract's
tests). The stocks table runs the same port on every recommended stock; of
those, AAPL has an instance so far.

```bash
python web/build_data.py
python -m http.server 8745 --directory web
```

Published at https://bongbongcrypto.github.io/afterhours/ by the manual `pages` workflow.

## Verify it in five minutes

| claim | how to check it |
|---|---|
| The feed goes silent for 52-57 hours every weekend | the live page's print tape, or `python scripts/measure/feed_cadence.py` (stdlib, about 60 reads, a minute) |
| AAPL trades $4-5M on-chain every weekend | `python scripts/measure/weekend_swaps.py` (slow: about 170 reads and tens of minutes, as the public RPC narrows each log query and rate-limits) |
| 83 funded Morpho markets lend against stock tokens, priced by stale-tolerant or raw-pool oracles | `python scripts/measure/morpho_markets.py` (every CreateMarket event, each market's supply and borrow, each oracle's feed; about 1,500 reads in batches, a few minutes) |
| 71 unit and property tests, 89 on-chain assertions on ArbOS 61 | the latest `ci` and `e2e` runs under Actions; the e2e artifact `result.txt` lists every assertion and the gas per read (on Solidity test doubles; the deployed instance's mainnet gas is in the next row) |
| Gas per read, `latestRoundData()` / `price()`: on the dev node against Solidity test doubles 107,540 / 109,632 in LIVE_FEED and 148,267 / 150,349 in ONCHAIN_TWAP; on mainnet, the superseded first instance in ONCHAIN_TWAP, 257,152 / 259,404 (2026-09-26 19:27 UTC, block 73,332,952), higher than on the dev node: the real pool's `observe()` searches its observation buffer, and the real feed and token are proxies. That instance also read the token's multiplier and its effective time on every call, two reads the current contract does not make, so the current instance is measured again once deployed. Arbitrum's gas includes an L1 data component for posting the calldata to the parent chain; it was 0 here, as the chain's L1 base fee estimate read 0 | `python scripts/measure/mainnet_gas.py`: `eth_estimateGas` from the zero address, the method `e2e/run.sh` uses through `cast estimate`, with the L1 part from Arbitrum's NodeInterface (`gasEstimateComponents`) and the cost in USD from Chainlink's ETH / USD feed (seconds). It measures the session the instance is in, so LIVE_FEED on mainnet is measured during a US trading session, while the feed is fresh |
| The contract decodes the real feed, pool and token: served the answers they gave at block 70,472,250, it prices AAPL exactly as a separate port of its rules does, and `price()` is that answer times 1e16 | `cargo test the_contract_reads_real_mainnet_answers` on `fixtures/aapl_mainnet.txt`; `python scripts/measure/capture_reads.py` re-captures them at the current block (13 requests, seconds), and `--replay <file>` re-derives the expected answer from a capture's bytes |
| Inside the regular session the feed prints each 0.5% move; outside it the next print often lands at the open, further away | `python scripts/measure/session_prints.py` (about 450 reads, ten minutes) |
| Chainlink lists no sequencer-uptime feed for the chain; the stock feeds have a 0.5% threshold and a 24-hour heartbeat | `python scripts/measure/feed_directory.py` (one request) |
| The price math matches independent 80-digit references: AAPL's prices to the integer, 1.0001^tick within 1e-23 of the reference (one unit below tick 0) | the live page's self-check line, and the vectors from `scripts/measure/tick_vectors.py`, pasted verbatim into `src/tickmath.rs` |
| It keeps answering while the market is closed | `status/10min.md`: the first (now superseded) instance read every 10 minutes from a server since 2026-09-25 22:59 UTC, in LIVE_FEED until Friday's last print was six hours old (01:50 UTC Saturday), then in ONCHAIN_TWAP from the pool (its answers there are per share, 0.057% under the per-token price); `python scripts/probe.py --oracle <address in DEPLOYMENTS.md>` reads the current instance now, and the live page on a weekend shows the Chainlink print hours old while AfterHours answers. `status/log.md` is the GitHub `status` job's sparser log (scheduled hourly, but GitHub starts it only every 2.4 to 6.3 hours) |
| Moving AAPL's pool 10% takes $234k up or $314k down; the pool clears the 5e16 depth floor from -10.9% to +8.3%; the cheapest refusal parks $235k at +12% for eight seconds, once every ten minutes (block 70,474,692) | `python scripts/measure/pool_depth.py ` (walks every initialized tick over the pool's whole range at one block; about 900 reads in batches, a minute) |
| Robinhood Chain runs ArbOS 61 with Stylus 3, programs expire after 365 days, mainnet has no StylusDeployer, blocks average 0.101 s | `python scripts/measure/stylus_params.py` (the chain's own precompiles, mainnet and testnet; seconds) |
| 28 stocks have a Chainlink feed and an observable pool; 18 of them pass `initialize` today, 14 meet the bar | `assets.json`, regenerated by `scripts/measure/discover_assets.py` (about an hour of reads) |

## What it does not do

- It does not predict Monday's open. During the closure it reports where the token trades on-chain; a gap caused by weekend news still lands on Monday, bounded by the band.
- During the US regular session it moves at most 1% from a print younger than 24 hours: the feed prints each 0.5% move then (`session_prints.py`). The contract reads the session from the block timestamp (Monday to Friday, 14:30-20:00 UTC, inside the session in both daylight-saving regimes) and keeps no holiday calendar, so on an exchange holiday that follows a trading day, and on a half day's afternoon, the 1% band applies until 20:00 UTC although the exchange is closed: a larger move then shows in full up to five and a half hours late.
- It does not price a move the pool's liquidity has not followed. LPs concentrate around the current price: AAPL's pool clears the 5e16 depth floor from -10.9% to +8.3% of today's price, about as far as the band reaches. A move past that, held for ten minutes, refuses (`NoData(2)`) until LPs re-center or the feed prints, because a pool nobody provides at that price is the pool that is cheap to push. The floor is set per instance: 2e17 would refuse past -4.5% / +3.2%, which also stops a price being held beyond that, but refuses the real moves this oracle exists to price.
- It does not follow liquidity. The primary pool is fixed at deployment; if liquidity leaves it for good, the instance refuses and a new one is deployed.
- It does not check that the pool trades. The depth floor measures liquidity providers, not flow: a deep pool nobody trades still answers with its last price. AAPL's pools cleared a swap every seven seconds over a weekend (`weekend_swaps.py`); check an asset's flow before deploying it.
- It relies on Chainlink's handling of corporate actions. The feed prices the token (the equity's price times the token's multiplier); while the issuer's `oraclePaused()` is set AfterHours refuses, and Chainlink documents that the feed holds the last good token price until the new multiplier and the equity price agree. If a feed ever printed the new equity price with the old multiplier, `LIVE_FEED` would pass it through, as any consumer of that feed would; in `ONCHAIN_TWAP` the band around that print would clamp the pool's price or refuse.
- It needs Chainlink's Robinhood tokenized-equity feed (`Robinhood <SYMBOL> / USD`), which prices the token. A plain per-share equity feed would undervalue the collateral by the multiplier; the deploy workflow checks the feed's `description()`. The quote token must be a plain ERC-20 (USDG): a pool against a rebasing or scaled token would need its own units.
- It treats USDG as one dollar. Chainlink's USDG/USD feed on this chain updates only on 0.5% moves, so reading it would correct only a larger depeg; that is a v2 option.
- It prices against one quote token per instance (USDG today).
- No L2 sequencer-uptime check: Chainlink lists none of its 58 feeds on Robinhood Chain as a sequencer-uptime feed (`scripts/measure/feed_directory.py`), so there is nothing to read. After a sequencer outage AfterHours answers at once, like every other oracle on this chain.
- It depends on the issuer's token interface. If an upgrade of the Robinhood token removed `oraclePaused()`, the one function AfterHours reads from it, every price read would revert for good (there is no admin by design). A Morpho market cannot change its oracle, so in a market priced by that instance borrowing and liquidation would stop permanently; borrowers could still repay and then withdraw their collateral, and lenders withdraw what is not lent out. The feed's history stays readable through `getRoundData`. A curator should size a market with that in mind; a new market on a new instance is the way forward.
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
scripts/measure/  the evidence: feed cadence and session prints, weekend swaps, pool depth, PARE's oracle
scripts/probe.py  read a deployed AfterHours next to the raw feed and pool (stdlib only)
scripts/deploy.sh the deployment steps, run by the deploy workflow and by e2e
scripts/abi_check.py  CI check that abi/IAfterHours.sol declares what the contract exports
web/              the live page (static HTML; reads the chain from the browser) and its data builder
.github/          ci (fmt, clippy, tests, cargo stylus check, ABI), e2e, manual deploy, scheduled status, pages, verify
status/           reads of the first (superseded) instance: 10min.md every 10 minutes from a server, log.md from the status job
```

## Build and test

Rust 1.91 (`rust-toolchain.toml`), `cargo-stylus` 0.10.9.

```bash
cargo test
cargo stylus check --endpoint https://rpc.testnet.chain.robinhood.com
cargo stylus export-abi
```

End-to-end (real wasm on a local Nitro dev node with Solidity doubles, 89
assertions, gas per read): `.github/workflows/e2e.yml` runs `e2e/run.sh`
against OffchainLabs' `nitro-devnode` upgraded to ArbOS 61 — the version
Robinhood Chain runs, and the one a 2-fragment program (about 38 KB) needs.

Deploy (deploys, activates, then runs the one-shot `initialize`):

```bash
cargo stylus deploy --endpoint <rpc> --private-key <key> --no-verify
cast send <address> "initialize(address,address[],address,uint64,uint32,uint64,uint128,uint64,uint64,uint64)" \
  <feed> "[<primary pool>]" <stock> 21600 1800 1000 50000000000000000 432000 86400 100
# liveMaxAge twapWindow maxDeviationBps minLiquidity maxAnchorAge heartbeat quietBandBps
# one to three pools of the stock against one quote token. The first is the primary and
# prices the asset; optional standbys are read only while the primary cannot be observed.
# initialize refuses a primary whose observation cardinality is below window + 1 (InvalidConfig 16).
```

The manual `deploy` workflow runs these steps with checks around them, all in
`scripts/deploy.sh`: a preflight of every read `initialize` will make before
any gas is spent, deploy and activate, `initialize`, and a read-back that
fails on any field that differs from the inputs, or when `price()` is not the
answer times Morpho's scale. The e2e run executes the same
script against a second instance on every push, including a preflight that
must refuse and a read-back that must fail.

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
expects: the answer, the price of one token of raw balance, times 1e16. On a
weekend it keeps answering, so liquidations and borrows work.
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
// session 0 LIVE_FEED, 1 ONCHAIN_TWAP, 2 PAUSED, 3 NO_DATA (reason 1-4)
// alert on: session 3 for more than a window; clamped == true; liquidity near the floor
```

`scripts/probe.py --oracle <address> --watch 300` prints the same next to the
raw feed and pool. A server records it every 10 minutes in `status/10min.md`; the
`status` workflow (scheduled hourly, but GitHub starts it only every 2.4 to 6.3
hours) appends it to `status/log.md`.

**Corporate actions** — while the issuer processes one, the token's
`oraclePaused()` flag makes AfterHours refuse (`IssuerPaused`). Otherwise
nothing is needed: the feed and the pool both price one token of raw balance,
and that price is continuous through dividends and splits (the multiplier goes
up by the factor the share price goes down by). A 10:1 split leaves every
answer and `price()` where they were; the unit tests and e2e assert it.

## Every stock, not just AAPL

`scripts/measure/discover_assets.py` asks the chain, for every token the
explorer lists as a Robinhood stock token (122 with 100+ holders), what
`initialize` and an attacker would check: that it is Robinhood's own (the
same token beacon as AAPL; 15 imitations are not), the pause flag, 18
decimals, a Chainlink feed named `Robinhood <SYMBOL> / USD`, and for each
USDG fee tier the liquidity, the observation cardinality, whether a
30-minute `observe` answers now and the USDG it takes to move the price 2%
(assuming the in-range liquidity holds across the move, which overstates it:
the tick-by-tick walk in `pool_depth.py` gives about half that for AAPL).
It writes the deploy inputs per asset to `assets.json`, with a floor of the
primary's liquidity divided by 32, about the ratio chosen for AAPL (1.68e18 / 5e16 = 33.6);
before deploying another asset, `pool_depth.py --pool <primary> --stock <token>
--floor <minLiquidity>` shows the range its pool clears. On 2026-09-23:

| tier | count | stocks |
|---|---|---|
| recommended: primary cardinality >= 1,801 and >= $50k of 2% depth | 14 | NVDA, SPCX, GOOGL, USO, AAPL, SPY, AMZN, MSFT, QQQ, CRCL, GME, SLV, TSLA, MU |
| the deploy workflow accepts them; 2% depth under $50k | 4 | META, PLTR, BABA, TSM |
| every other check passes, but the primary keeps fewer than 1,801 observations, so `initialize` refuses until someone raises it with `increaseObservationCardinalityNext(1801)` (anyone can, gas only) | 10 | SGOV, MSTR, DELL, AMD, INTC, USAR, ASML, SNDK, RKLB, IONQ |
| Robinhood's own, but no Chainlink feed or no USDG liquidity today | 79 | |

The same wasm serves all of them. A stock moves up a tier the day its feed
or pool appears, with no code change.

## Deployments

See [DEPLOYMENTS.md](DEPLOYMENTS.md).

## Status

Built for the Arbitrum Open House Singapore buildathon (September 2026).
Unaudited. MIT.
