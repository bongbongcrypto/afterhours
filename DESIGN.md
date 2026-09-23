# AfterHours — design and evidence

Everything numeric in this document was measured on Robinhood Chain mainnet
with the read-only scripts in `scripts/measure/` (stdlib Python, no keys) or
read from verified contract source. Where something is assumed rather than
measured it says so.

## 1. The situation

Robinhood Chain (an Arbitrum chain; 0.101 s average block time over the
864,000 blocks before 2026-09-23) trades tokenized US stocks against USDG
around the clock. The explorer lists 122 tokens named "... • Robinhood
Token" with 100+ holders; 107 of them are Robinhood's own (the same token
beacon as AAPL) and 28 have both a Chainlink feed and an observable USDG pool
(`assets.json`, 2026-09-23). The price those tokens'
contracts rely on comes from Chainlink feeds that follow US market hours.

**The feed stops every weekend.** AAPL/USD rounds (`feed_cadence.py`, which reads the
last 60 rounds: the Labor Day row is from its 2026-09-19 run, the 09-18 row from
2026-09-23; `weekend_swaps.py` reproduces both weekend windows):

| period | feed silent | note |
|---|---|---|
| Fri 09-04 19:51 -> Tue 09-08 00:00 UTC | **76.2 h** | Labor Day weekend |
| Fri 09-11 19:51 -> Mon 09-14 00:00 UTC | **52.2 h** | ordinary weekend |
| Fri 09-18 15:11 -> Mon 09-21 00:00 UTC | **56.8 h** | ordinary weekend; Friday's last print came mid-session, the price then moved less than 0.5% before the close (re-measured 2026-09-23) |
| weekdays | 13-21 h between prints (longest: Mon 09-21 16:43 -> Tue 13:30 UTC, 20.8 h), with prints at 02:24, 03:55, 08:05, 10:32 UTC on other nights | not closed — the price just did not move the 0.5% deviation threshold |

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

**Lending against the tokens exists, and lives with the stale price.** Morpho
Blue on this chain (`0x9D53d5E3bd5E8d4Cbfa6DB1ca238AEA02E651010`) had 275
markets on 2026-09-23. 83 of them hold supply against a Robinhood stock token
as collateral: **$0.88M supplied, $6.4k borrowed**, a utilization under 1%
(`morpho_markets.py`, every CreateMarket event and each market's state). By
oracle:

| oracle behind the market | markets | supplied | on a weekend it answers |
|---|---|---|---|
| unverified custom oracles from one deployer; their bytecode reads the Chainlink feed, the token's `uiMultiplier()` and `oraclePaused()` | 16 | $852k | Friday's print (no staleness bound exposed) |
| Morpho's standard `ChainlinkOracleV2` layout on feeds named "RH… / USD" (the verified one has no staleness check) | 17 | $24.8k | Friday's print |
| the same layout on Chainlink's "Robinhood … / USD" feeds | 35 | $2.3k | Friday's print |
| adapters named "Uniswap V3 Pool Price" | 15 | $2.0k | the pool, without a band |

The one oracle that states a policy for the closure is PARE's
`PareMorphoOracle` (`0xc2414099151326C5d238B9F00609f1a12283B723`, verified
2026-09-18) for its pSPY/USDG market. Its source contains:

```solidity
/// @dev Shortest `maxFeedAge` that outlasts a Monday-holiday closure plus the feed's early stop.
uint256 public constant MIN_FEED_AGE = 5 days;
```

and refuses any deployment with a shorter tolerance. Its stock leg is the
Chainlink feed, up to five days stale; its PT leg is a 30-minute Uniswap v3
`observe()` TWAP of the same chain's pool, so the pool TWAP is already a
trusted component here. No market prices the stock during the closure with
both a live source and a bound.

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
   Uniswap v3 pools >|   immutable, no owner         |--> state()        (session, reason,
   (24/7, up to 3)   |                               |                    answer, twap, pool, ...)
   Stock token ----->|  oraclePaused, uiMultiplier   |
                     +-------------------------------+
```

Decision per read, in this order:

0. Units. Chainlink prices one share. A Robinhood stock token is a scaled-UI
   token (its verified `Stock` source): one raw unit is `uiMultiplier / 1e18`
   shares, the multiplier grows with every reinvested dividend (AAPL 1.00057,
   SPY 1.0017, SGOV 1.0051 on 2026-09-23) and a split multiplies it. Every
   Chainlink-shaped answer here is per share; `price()` multiplies by the
   token's current `uiMultiplier()` because Morpho counts raw collateral units.
   The pool also trades raw units, so its TWAP is divided by the multiplier
   before it meets the band.
1. `oraclePaused()` on the stock token (the issuer's corporate-action flag,
   which Chainlink documents as advisory) -> **PAUSED**, reads revert. This
   wins over everything else: while a split or dividend is being processed
   neither the feed nor the pool price means what it says.
2. Feed round invalid (answer <= 0, `updatedAt` 0 or in the future) ->
   **NO_DATA(1)**, reads revert.
3. Feed age <= `liveMaxAge` and no new share multiplier took effect since the
   print (`effectiveAt()` is not between the print and now) -> **LIVE_FEED**:
   the feed's round, verbatim. If a multiplier did take effect after the
   print, that print is in pre-action shares and is not passed through, even
   when it is fresh: the read continues to the pool.
4. Feed age > `maxAnchorAge` -> **NO_DATA(4)**. No market closure lasts this
   long (5 days covers a holiday weekend); the feed has been deprecated or the
   stock is halted, and a band anchored to that print would only look fresh.
5. Otherwise the price comes from one fixed venue: the **primary** pool
   (`pools[0]`, the deepest fee tier when the instance is deployed). One
   `observe([1800, 1200, 600, 0])` call gives the tick cumulatives for the
   30-minute TWAP and the liquidity cumulatives for three 10-minute
   sub-windows. Optional **standby** pools are read, in order, only while
   every pool before them cannot be observed at all (`observe` reverts, for
   example "OLD", or answers the wrong shape). A primary that answers is the
   venue, thin or not. Picking the deepest pool at read time would let anyone
   who deepens a shallow tier for one window choose the price source; with a
   fixed venue a manipulator has to move the pool the market trades in.
   - no configured pool can be observed -> **NO_DATA(3)**, `pool` = 0;
   - the median of the three sub-windows' harmonic-mean in-range liquidity
     (`span * 2^128 / delta(secondsPerLiquidityCumulativeX128)`, Uniswap's
     own `OracleLibrary.consult`, per sub-window) is below `minLiquidity` ->
     **NO_DATA(2)**. Window averages, not the spot value: liquidity added in
     the block before a read cannot make a pool that spent the window thin
     look deep. The median, not the whole-window mean: one second with no
     in-range liquidity (a swap that pushes the price out of every position
     and back) collapses one sub-window's mean to about 600, and two deep
     sub-windows still carry the read. To hold the oracle in refusal, the
     pool has to be emptied in two of every three 10-minute sub-windows;
   - the mean tick converts to nothing usable -> **NO_DATA(3)**;
   - the TWAP per share (pool price of a raw unit divided by the multiplier)
     is bounded to `[feedAnswer * (1 - band), feedAnswer * (1 + band)]`. If a
     new multiplier took effect after the last print and the TWAP per share
     is outside that band, the print cannot anchor anything (a split or a
     merger it knows nothing about) -> **NO_DATA(5)**; the feed's next print
     ends it. A dividend moves the multiplier by a fraction of a percent and
     stays inside the band;
   - else **ONCHAIN_TWAP**: the TWAP per share, or the band edge it crossed
     with `clamped = true`.

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
the share multiplier and its effective time, `observe` at the four points) so
a wrong pool or a token without those functions fails at deployment; the deploy workflow then reads the configuration
back and fails unless every field matches its inputs and the feed's
`description()` names the expected asset. Robinhood Chain mainnet
does not have the canonical StylusDeployer factory
(`0xcEcba2F1DC234f70Dd89F2041029807F8D03A990` has no code there, while it
exists on the testnet and Arbitrum One), so a Stylus constructor cannot be used
on mainnet; `initialize` is called by the same script that deploys and
activates the contract. A front-run `initialize` would only produce a
contract nobody integrates, and the deployer would redeploy.

### Prior art, and what is new here

Checking a reported price against a Uniswap TWAP is not new: Compound's
`UniswapAnchoredView` accepted a reporter's price only within bounds of a
Uniswap TWAP anchor. AfterHours inverts the roles for a market that closes:
the exchange feed is the anchor, and the on-chain TWAP is the price while the
anchor cannot print. The parts that are specific to this chain are the closed
session as an explicit state instead of a staleness error, the issuer's
corporate-action surface (the pause flag and the share multiplier, with a
split after the last print refusing rather than clamping), liquidity judged
per sub-window so one excursion cannot switch pricing off, and a venue fixed
at deployment so depth elsewhere cannot redirect it. The risk layers other
teams build for the same weekend (haircuts until the next open, stale-feed
flags) can sit on top of this price rather than replace it.

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
| `liveMaxAge` | 21,600 s (6 h) | On weekdays the feed can go 13-21 h without a print because the price does not move 0.5%. Six hours means the pool takes over ~6 h after Friday's last print and during long weekday gaps, and the feed takes back over at its next print. Over 09-16 -> 09-23 the feed was older than 6 h for 60% of the week, half of it on weekdays (the live page computes this from the feed's rounds). |
| `twapWindow` | 1,800 s (30 min) | Same window PARE trusts for its pool leg. With ~1 swap every 7 s on the weekend, 30 minutes averages ~250 fills. |
| `maxDeviationBps` | 1,000 (10%) | Single-stock LULD band for closed-session moves; AAPL's largest weekend gap in the measured windows was 0.25%. |
| `minLiquidity` | 2e17 | The 0.05% pool's 30-minute harmonic-mean liquidity measured 1.45e18 against a spot 1.61e18 (`pool_harmonic.py`, 2026-09-19), a 7.3x margin over the floor; refusing below roughly 1/8 of today's depth means a pool most LPs have left is not trusted. This is a sanity floor, not the manipulation defence (that is the band). |
| `maxAnchorAge` | 432,000 s (5 days) | The same bound PARE hard-codes as `MIN_FEED_AGE`: outlasts a Monday-holiday closure plus the feed's early Friday stop. Beyond it the feed is gone or the stock is halted. |

Keeping the primary observable. Uniswap v3 writes at most one observation
per second (per block timestamp; Robinhood Chain's ten blocks a second share
one), so a pool that stores fewer than `twapWindow + 1` observations can be
made to answer "OLD" for a 30-minute window by one small swap a second, and
the oracle would fall to a standby or refuse. The deploy workflow therefore
refuses a primary whose observation cardinality is below `twapWindow + 1`
(1,801); raising it is a permissionless, gas-only call
(`increaseObservationCardinalityNext`). The AAPL 0.05% pool holds 1,801
today. A standby may sit below the bound: it only covers a primary that
cannot answer.

What manipulation can buy, and what it costs. `pool_depth.py` walks the AAPL
0.05% pool tick by tick (2026-09-23; 184 initialized ticks within 60% of the
price). Moving the AAPL price 10% down takes about $317k of AAPL sold into the
pool, 10% up about $234k of USDG, 5% about $263k / $217k. The fee on that is
only $120-160; the real cost is holding the move for the whole 30-minute
window while every trader who can reach another venue sells the attacker's
premium back. The hard cap is the band: the oracle moves at most 10% from the
last print, and at a 62.5% LLTV that could make positions between 56.25% and
62.5% LTV liquidatable at Morpho's bonus. The band has to be sized to the
market's LLTV; a curator who wants no such window uses a tighter band or a
lower LLTV.

Denial instead of manipulation. An attacker who cannot move the price can
still try to make the oracle refuse, which freezes borrowing, collateral
withdrawal and liquidation in a Morpho market until the feed prints again. In
this pool in-range liquidity never reaches zero within 60% of the price (a
wide position underlies it), so "one second out of range" is not available.
The cheapest refusal `pool_depth.py` finds is to buy about $233k of AAPL,
pushing the price 9% up into a thin band (3.5e15), and to hold it there for 9
seconds, which drags one 10-minute window's harmonic mean under the 2e17
floor. The median rule makes that necessary in two of every three windows,
for as long as the refusal must last, each time with $233k exposed at a 9%
premium to anyone who sells into it. Before the rule one such stay per 30
minutes was enough. A refusal never moves the price; it delays liquidation to
the feed's next print, which is where a stale-feed market already is today.

## 5. What is measured, what is assumed

Measured: feed silence and cadence; weekend swap counts, volume and fill
quality on two weekends; every Morpho market on the chain and its oracle
(`morpho_markets.py`); PARE's 5-day tolerance and its use of a pool TWAP; the
share multiplier and its effective time on every deployable stock; the AAPL
pool's reserves; Stylus availability on mainnet and testnet (stylusVersion 3)
and program expiry (`ArbWasm.expiryDays()` = 365);
pool observation cardinality per fee tier for every stock (`assets.json`;
AAPL: 1,801 on the 0.05% primary, 1,500 on the standbys);
the window harmonic-mean liquidity against spot (`pool_harmonic.py`); the
feed's `description()` (`Robinhood AAPL / USD`) and the stock's
`oraclePaused()`; the absence of the StylusDeployer on mainnet.

Assumed: that the pool keeps tracking fair value on a *news* weekend (both
measured weekends were quiet; the band exists precisely because this is not
guaranteed); that lending curators will adopt a 24/7 price at all (no market
has run on AfterHours yet; borrowing against stock tokens on this chain is
$6.4k today, so the demand is a bet, not a measurement).

Not handled in v1: a Chainlink L2 sequencer-uptime check (PARE deploys with it
disabled on this chain too); the USDG/USD rate: the feed is in USD, the pool
and Morpho's loan token in USDG, and USDG is treated as one dollar. Chainlink's
USDG/USD feed here (`0x61B7e5650328764B076A108EFF5fa7282a1B9aD2`) updates only
on 0.5% moves, so reading it would correct only a larger depeg, at the cost of
one more call; pools against a token other than the loan token
(every pool of an asset must share one quote); combining several pools into
one price, or following liquidity to another fee tier. The primary is fixed
at deployment and a standby only covers a primary that cannot be observed;
if liquidity leaves the primary for good, the instance refuses (NO_DATA 2)
and a new instance is deployed with the new primary (one workflow run).
Following liquidity automatically is exactly the lever a manipulator would
pull.

The multiplier guard sees only the latest scheduled change: the token exposes
one `effectiveAt()`. If a change took effect after the last print and the
issuer scheduled another one before the feed printed again, the first change
would pass unseen until the next print. For a dividend that is a fraction of
a percent; for a split the issuer's `oraclePaused()` flag, which Robinhood
sets while it processes a corporate action, is the primary protection and
the guard is the second.

Operational risks, both fail closed: every read calls the issuer's upgradeable
token for `oraclePaused()`, `uiMultiplier()` and `effectiveAt()`; if a token
upgrade removed one of them, reads would revert until a new instance is
deployed, because there is no admin to repoint anything. And like every Stylus
program the contract must stay activated: activation lasts 365 days on this
chain and must be renewed after a Stylus version upgrade; anyone can pay to
re-activate it, and reads revert until someone does.

## 6. Verification layers and cost per read

| layer | what it proves | where |
|---|---|---|
| 60 unit and property tests on a host that serves mocked calls exactly | decision logic, band, scaling, the share multiplier, the fixed venue, the sub-window median, every refusal, exact numbers; property runs over random feed, pool and multiplier data reach every session and every refusal reason | `src/tests.rs`, `src/mockvm.rs` |
| tick-math reference vectors | 1.0001^tick and the price conversion against 60-digit decimal arithmetic | `src/tickmath.rs`, `scripts/measure/tick_vectors.py` |
| `cargo stylus check` against Robinhood testnet | the wasm compiles, fits and activates on Stylus v3 / ArbOS 61 | `.github/workflows/ci.yml` |
| end-to-end on a local Nitro node (ArbOS 61, Stylus 3, the same as Robinhood Chain) | the real wasm deployed, activated and initialised; ABI dispatch, storage, external calls, every session, the venue rule, sub-window dips, the multiplier and a split, and every revert's exact data asserted through `cast`; 62 assertions | `.github/workflows/e2e.yml`, `e2e/run.sh`, `e2e/src/Mocks.sol` |
| four independent review rounds | every finding and its fix, with the commit | `REVIEWS.md` |

Gas per read on the dev node (`cast estimate`, includes the 21k transaction
base; two pools configured, the primary answering): `latestRoundData()`
111,501 in LIVE_FEED, 155,908 in ONCHAIN_TWAP; `price()` 113,658 / 158,055.
These are against Solidity test doubles; the real feed, pool and beacon-proxy
token cost more per call and the figure will be re-measured on mainnet. Five
external reads (pause flag, multiplier, its effective time, feed round, pool
observe) account for most of it; at Robinhood Chain's gas prices that is
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
reference vectors computed independently in 60-digit decimal arithmetic.
There is no Solidity twin to compare gas against; Stylus was chosen for the
math and the tooling (property tests over the real decision code), not for a
measured gas saving. The
problem only exists where tokenized stocks trade around the clock with a
market-hours oracle, which today is Robinhood Chain, and the quote asset of
every stock pool there is USDG.
