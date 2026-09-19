# AfterHours — design and evidence

Everything numeric in this document was measured on Robinhood Chain mainnet
with the read-only scripts in `scripts/measure/` (stdlib Python, no keys) or
read from verified contract source. Where something is assumed rather than
measured it says so.

## 1. The situation

Robinhood Chain (an Arbitrum chain, 0.1 s blocks, ~10M tx/day) trades ~190
tokenized US stocks against USDG around the clock. The price those tokens'
contracts rely on comes from Chainlink feeds that follow US market hours.

**The feed stops every weekend.** AAPL/USD, last 60 rounds (`feed_cadence.py`):

| period | feed silent | note |
|---|---|---|
| Fri 09-04 19:51 -> Tue 09-08 00:00 UTC | **76.2 h** | Labor Day weekend |
| Fri 09-11 19:51 -> Mon 09-14 00:00 UTC | **52.2 h** | ordinary weekend |
| weeknights | 13-17 h between prints, but prints exist at 02:24, 03:55, 08:05, 10:32 UTC | not closed — the price just did not move the 0.5% deviation threshold |

The feed's 24 h heartbeat is not honoured during the closure: the silence is
by design (`us_equities_24/5`). NVDA and SPY show the same pattern (51.9 h and
59.1 h, `feed_gap.py`).

**The token keeps trading while the feed sleeps.** Swap events of the three
AAPL/USDG Uniswap v3 pools inside each silent window (`weekend_swaps.py`):

| weekend | swaps | volume | fill vs the Monday opening print |
|---|---|---|---|
| Labor Day (76 h) | 44,338 | $5.14M | median 0.35%, p90 0.56%, max 1.67% |
| 09-11 -> 09-14 (52 h) | 26,331 | $4.17M | median 0.45%, p90 0.59%, max 2.26% |

One stock. The pool price moved during the weekend (six-hour averages ran from
+0.56% to -0.23% against the Monday open) and, on these quiet weekends,
tracked the next real print to within ~0.6%.

**The one live lending market accepts a five-day-old price.** PARE's
pSPY/USDG market on Morpho uses `PareMorphoOracle`
(`0xc2414099151326C5d238B9F00609f1a12283B723`, verified 2026-09-18). Its
verified source contains:

```solidity
/// @dev Shortest `maxFeedAge` that outlasts a Monday-holiday closure plus the feed's early stop.
uint256 public constant MIN_FEED_AGE = 5 days;
```

and refuses any deployment with a shorter tolerance. The stock leg is the
Chainlink feed, up to five days stale; the PT leg is a 30-minute Uniswap v3
`observe()` TWAP of the same chain's pool. Nobody prices the stock during the
closure; the pool TWAP is already a trusted component.

## 2. The problem

For 30% of every week (45% on holiday weeks) a contract that needs a stock
price on this chain has two options: use the last exchange print and hope, or
stop. Meanwhile the asset trades on-chain and its price moves.

- **Lending** cannot liquidate during the weekend even when the collateral is
  falling on-chain. Monday's gap lands all at once. The only defence is a low
  LLTV (62.5% on SPY; a single stock would need less), which makes the market
  capital-inefficient, and the tail risk sits with USDG suppliers.
- **Liquidators** are blind for two days.
- **Anything price-aware** (perps marks, stop/limit orders, rebalancing,
  automation) runs 24/5 on a 24/7 chain. Robinhood's own builder list asks for
  collateralized lending, price-aware contracts and perps.

The root cause is a single one: the only price source contracts trust is tied
to the exchange's hours, while the market for the token is not.

## 3. The product

AfterHours is a per-asset price contract with the same interface as the
Chainlink feed (`AggregatorV3Interface`) plus Morpho Blue's `IOracle`. A
lending market changes one address.

```
                     +-------------------------------+
   Chainlink feed -->|                               |--> latestRoundData()
   (24/5)            |   AfterHours (Stylus, Rust)   |--> price()        (Morpho)
   Uniswap v3 pool ->|   immutable, no owner         |--> state()        (session, reason,
   (24/7)            |                               |                    answer, twap, ...)
   Stock token ----->|  oraclePaused() flag          |
                     +-------------------------------+
```

Decision per read, in this order:

1. `oraclePaused()` on the stock token (the issuer's corporate-action flag,
   which Chainlink documents as advisory) -> **PAUSED**, reads revert. This
   wins over everything else: while a split or dividend is being processed
   neither the feed nor the pool price means what it says.
2. Feed round invalid (answer <= 0, `updatedAt` 0 or in the future) ->
   **NO_DATA(1)**, reads revert.
3. Feed age <= `liveMaxAge` -> **LIVE_FEED**: the feed's round, verbatim.
4. Feed age > `maxAnchorAge` -> **NO_DATA(4)**. No market closure lasts this
   long (5 days covers a holiday weekend); the feed has been deprecated or the
   stock is halted, and a band anchored to that print would only look fresh.
5. Otherwise the market is closed for this oracle's purposes. One
   `observe([twapWindow, 0])` call gives both legs:
   - it fails (not a v3 pool, no history) or returns the wrong shape ->
     **NO_DATA(3)**;
   - the harmonic-mean in-range liquidity over the window
     (`window * 2^128 / delta(secondsPerLiquidityCumulativeX128)`, Uniswap's
     own `OracleLibrary.consult`) is below `minLiquidity` -> **NO_DATA(2)**.
     A window average, not the spot value: liquidity added in the block
     before a read cannot make a pool that spent the window thin look deep.
     The flip side: one second of the window with no in-range liquidity
     (the price wandered outside every position, or a swap out and back)
     drags the mean to roughly the window length and the oracle refuses
     for the next 30 minutes. That is deliberate — a pool that just went
     empty is not a pool to price from — and it is why NO_DATA reads leave
     `state()` readable;
   - the mean tick converts to nothing usable -> **NO_DATA(3)**;
   - else **ONCHAIN_TWAP**: the time-weighted pool price, converted to the
     feed's decimals, then bounded to
     `[feedAnswer * (1 - band), feedAnswer * (1 + band)]`. If the TWAP sits
     outside, the edge is returned and `clamped = true`.

`latestRoundData()` in ONCHAIN_TWAP keeps the feed's round ids, reports the
last exchange print as `startedAt` and `block.timestamp` as `updatedAt`
(the answer is derived from trades happening now). Consequences for
consumers: staleness checks pass, `answeredInRound >= roundId` holds, but
`roundId` does not advance between closed-market reads, so it cannot be used
as a change detector. `getRoundData(id)` forwards historical rounds to the
feed and answers the feed's current round exactly like `latestRoundData()`;
the v2 getters (`latestAnswer/latestTimestamp/latestRound`) are provided for
older integrations. `state()` never reverts for market reasons, so dashboards
and keepers can see why the oracle refuses. In PAUSED and NO_DATA a Morpho
market built on AfterHours cannot borrow, withdraw collateral or liquidate
(those read the oracle); supplying and repaying keep working.

The configuration is written once by `initialize` and cannot be changed.
There is no owner, no pause switch, no upgrade path. `initialize` exercises
every read the oracle will ever make (token order, decimals, the pause flag,
`observe` for the configured window) so a wrong pool or a token without the
flag fails at deployment; the deploy workflow then reads the configuration
back and fails unless every field matches its inputs and the feed's
`description()` names the expected asset. Robinhood Chain mainnet
does not have the canonical StylusDeployer factory
(`0xcEcba2F1DC234f70Dd89F2041029807F8D03A990` has no code there, while it
exists on the testnet and Arbitrum One), so a Stylus constructor cannot be used
on mainnet; `initialize` is called by the same script that deploys and
activates the contract. A front-run `initialize` would only produce a
contract nobody integrates, and the deployer would redeploy.

### The band is a circuit breaker, not a price model

US exchanges halt single stocks that move more than 5-10% inside five minutes
(LULD bands) and index futures stop at 7/13/20%. AfterHours applies the same
idea to the closed session: while the exchange cannot print, the on-chain price
may carry the asset at most `maxDeviationBps` away from the last print. This
does two things at once:

- caps what any manipulation of the pool can achieve through this oracle
  (the payoff of moving the pool is bounded by the band);
- keeps a real overnight crash inside a range the lending market's LLTV was
  sized for, while still moving the price in the right direction so that
  liquidations become possible before Monday.

`clamped` tells consumers when the pool wanted to go further.

## 4. Parameters (AAPL deployment) and why

| parameter | value | reason |
|---|---|---|
| `liveMaxAge` | 21,600 s (6 h) | Regular-session prints arrive every few minutes; overnight (24/5 session) the feed is silent for up to 17 h because the price does not move 0.5%. Six hours means the pool takes over ~6 h after Friday's last print and during long overnight gaps, and the feed takes back over at the first print of a session. |
| `twapWindow` | 1,800 s (30 min) | Same window PARE trusts for its pool leg. With ~1 swap every 7 s on the weekend, 30 minutes averages ~250 fills. |
| `maxDeviationBps` | 1,000 (10%) | Single-stock LULD band for closed-session moves; AAPL's largest weekend gap in the measured windows was 0.25%. |
| `minLiquidity` | 2e17 | The 0.05% pool's 30-minute harmonic-mean liquidity measured 1.45e18 against a spot 1.61e18 (`pool_harmonic.py`, 2026-09-19), a 7.3x margin over the floor; refusing below roughly 1/8 of today's depth means a pool most LPs have left is not trusted. This is a sanity floor, not the manipulation defence (that is the band). |
| `maxAnchorAge` | 432,000 s (5 days) | The same bound PARE hard-codes as `MIN_FEED_AGE`: outlasts a Monday-holiday closure plus the feed's early Friday stop. Beyond it the feed is gone or the stock is halted. |

Manipulation cost, order of magnitude (`scripts/measure`, uniform-range model
which overstates depth away from the current tick; the pool's $355k USDG
reserve bounds it from below): pushing the current range 10% takes roughly
$0.4-1.5M of one-sided flow, which then has to be held for the whole 30-minute
window against ~$80k/hour of organic weekend flow, and unwound through the
same 0.05% fee and price impact. There is no external AAPL market to arbitrage
against on a Saturday, so organic flow is the only pressure; the hard cap on
what a manipulator can achieve through this oracle is the band, not the
liquidity floor.

## 5. What is measured, what is assumed

Measured: feed silence and cadence; weekend swap counts, volume and fill
quality on two weekends; the live lending oracle's 5-day tolerance and its use
of a pool TWAP; Stylus availability on mainnet and testnet (stylusVersion 3);
pool observation cardinality (1,500-1,801, so a 30-minute TWAP reads today);
the window harmonic-mean liquidity against spot (`pool_harmonic.py`); the
feed's `description()` (`Robinhood AAPL / USD`) and the stock's
`oraclePaused()`; the absence of the StylusDeployer on mainnet.

Assumed: that the pool keeps tracking fair value on a *news* weekend (both
measured weekends were quiet; the band exists precisely because this is not
guaranteed); that lending curators will adopt a 24/7 price at all (PARE's own
constant and comment say they dislike the 5-day rule, but no market has run on
AfterHours yet).

Not handled in v1: a Chainlink L2 sequencer-uptime check (PARE deploys with it
disabled on this chain too); multiple pools per asset; assets whose only pool
is against a token other than the loan token. Operational: like every Stylus
program, the contract needs re-activation after an ArbOS upgrade (anyone can
do it; reads revert until then).

## 6. Verification layers and cost per read

| layer | what it proves | where |
|---|---|---|
| 41 unit tests on a host that serves mocked calls exactly | decision logic, band, scaling, every refusal, exact numbers | `src/tests.rs`, `src/mockvm.rs` |
| tick-math reference vectors | 1.0001^tick and the price conversion against 60-digit decimal arithmetic | `src/tickmath.rs`, `scripts/measure/tick_vectors.py` |
| `cargo stylus check` against Robinhood testnet | the wasm compiles, fits and activates on Stylus v3 / ArbOS 61 | `.github/workflows/ci.yml` |
| end-to-end on a local Nitro node (ArbOS 61, Stylus 3, the same as Robinhood Chain) | the real wasm deployed, activated and initialised; ABI dispatch, storage, external calls, every session and revert asserted with exact values through `cast`; 39 assertions | `.github/workflows/e2e.yml`, `e2e/run.sh`, `e2e/src/Mocks.sol` |
| two independent adversarial reviews | 20 findings, all folded in (anchor-age cap, harmonic-mean liquidity, deploy verification, real feed description, ...) | PROGRESS.md |

Gas per read on the dev node (`cast estimate`, includes the 21k transaction
base): `latestRoundData()` 104,347 in LIVE_FEED, 125,440 in ONCHAIN_TWAP;
`price()` 106,441 / 127,532. Three external reads (pause flag, feed round,
pool observe) account for most of it; at Robinhood Chain's gas prices that is
a fraction of a cent, and a Morpho borrow or liquidation pays it once. Latency
is not a network property here: in LIVE_FEED the answer is the feed's own
round with no added delay; in ONCHAIN_TWAP the answer is by design a
30-minute average of the pool, so a genuine move shows up gradually over
that window rather than instantly, which is the manipulation trade-off the
band and window encode.

## 7. Why Stylus, why Robinhood Chain, why USDG

The TWAP and band arithmetic is fixed-point integer math on 256-bit values
with 512-bit intermediates; Rust with `alloy` primitives expresses it without
unchecked blocks or assembly, and the contract is unit-tested against
reference vectors computed independently in 60-digit decimal arithmetic. The
problem only exists where tokenized stocks trade around the clock with a
market-hours oracle, which today is Robinhood Chain, and the quote asset of
every stock pool there is USDG.
