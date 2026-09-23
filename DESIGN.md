# AfterHours — design and evidence

Everything numeric in this document was measured on Robinhood Chain mainnet
with the read-only scripts in `scripts/measure/` (stdlib Python, no keys) or
read from verified contract source. Where something is assumed rather than
measured it says so.

## 1. The situation

Robinhood Chain (an Arbitrum chain; 0.101 s average block time over the
864,000 blocks before 2026-09-23, `stylus_params.py`) trades tokenized US stocks against USDG
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
| weekdays | 13-21 h between prints (longest: Mon 09-21 16:43 -> Tue 13:30 UTC, 20.8 h), with prints at 02:24, 03:55, 08:05, 10:32 UTC on other nights | not closed; outside the regular session the next print often lands at the open, for AAPL up to 0.84% from the last (`session_prints.py`, see "Two bands") |

The feed's 24 h heartbeat is not honoured during the closure: the silence is
by design (`us_equities_24/5`). NVDA and SPY show the same pattern (51.9 h and
59.1 h, `feed_gap.py`). Chainlink's page for these feeds
(docs.chain.link, Data Feeds, Robinhood tokenized equities) says the same:
while the market is closed a feed may hold its last price, there is no
heartbeat off-hours, and integrators are to bound staleness themselves. The
same page defines the token's price as the equity's price times the token's
multiplier, which is what `price()` below returns per raw unit.

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
| unverified custom oracles from one deployer; their bytecode reads the Chainlink feed, the token's `uiMultiplier()` and `oraclePaused()` | 16 | $852k | Friday's print (14 expose no staleness bound; 2, with $13 supplied between them, allow four days) |
| Morpho's standard `ChainlinkOracleV2` layout on feeds named "RH… / USD" (the verified one has no staleness check) | 17 | $24.8k | Friday's print |
| the same layout on Chainlink's "Robinhood … / USD" feeds | 35 | $2.3k | Friday's print |
| adapters named "Uniswap V3 Pool Price" | 15 | $2.0k | the pool, without a band |

The two bounded ones (USO and AAPL at a 38.5% LLTV) expose `4.0 d`, which
outlasts an ordinary weekend: they answer Friday's print too. Outside these 83
markets, PARE's `PareMorphoOracle`
(`0xc2414099151326C5d238B9F00609f1a12283B723`, verified 2026-09-18) for its
pSPY/USDG market states its policy for the closure in its source:

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
contract that reads the feed swaps one address; a Morpho market, whose oracle
is fixed when it is created, is opened with AfterHours as its oracle.

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
   token (its verified `Stock` source) that keeps balances raw: one token of raw balance is `uiMultiplier / 1e18`
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
   when it is fresh: the read continues to the pool. The mirror case is a
   change scheduled but not yet in effect: until `effectiveAt()` the token
   keeps the old multiplier, while the feed may already print the post-action
   price. A pending change the size of a split (outside the wide band of the
   current multiplier, `newUIMultiplier()`) therefore also sends the read to
   the pool; a pending dividend-sized change does not, since it moves a raw
   unit's value by less than the band.
4. Feed age > `maxAnchorAge` -> **NO_DATA(4)**. No market closure lasts this
   long (5 days covers a holiday weekend); the feed has been deprecated or the
   stock is halted, and a band anchored to that print would only look fresh.
5. Otherwise the price comes from one fixed venue: the **primary** pool
   (`pools[0]`, the deepest fee tier when the instance is deployed). One
   `observe([1800, 1200, 600, 0])` call gives, for each of three 10-minute
   sub-windows, the mean tick and the harmonic-mean liquidity. Optional **standby** pools are read, in order, only while
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
   - the pool's price is the median of the three sub-windows' mean ticks. A
     move confined to one sub-window does not reach it, however far it goes:
     it has to show in two sub-windows' averages, and since at most one
     sub-window may be thin, at least one of them has to be spent at a price
     where the pool clears the floor. A whole-window mean did not have that
     property: past about 8% the AAPL pool is nearly empty (reaching +50%
     costs $1.2k more than reaching +10%), so a ten-minute excursion there
     moved the 30-minute mean to the band edge while the per-sub-window depth
     rule let it through (review round 5);
   - the median tick converts to nothing usable -> **NO_DATA(3)**;
   - the band: `quietBandBps` (1%) during the US regular session (Monday
     to Friday, 14:30-20:00 UTC) while the last print is at most
     `heartbeat` (24 h) old, `maxDeviationBps` (10%) at every other hour
     and past the heartbeat (see "Two bands" below);
   - the price per share (pool price of a raw unit divided by the multiplier)
     is bounded to `[feedAnswer * (1 - band), feedAnswer * (1 + band)]`. If a
     new multiplier took effect after the last print and the price per share
     is outside the wide band (`maxDeviationBps`, whatever the print's age),
     the print cannot anchor anything (a split or a merger it knows nothing
     about) -> **NO_DATA(5)**, with the pool's price per share kept in
     `state()`; the feed's next print ends it. The same holds while a
     split-sized change is pending: a print the pool, in the token's current
     units, puts beyond the wide band is one already in post-action shares.
     A print the pool confirms passes through as **LIVE_FEED** when it is
     fresh. A distribution of a few percent stays inside the wide band and is
     held to the band that applies, like any other move;
   - else **ONCHAIN_TWAP**: the price per share, or the band edge it crossed
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
split after the last print refusing rather than clamping), price and depth
judged per sub-window so one excursion can neither move the price nor switch
pricing off, a narrow band only while the exchange is open and the feed
prints every move, and a venue fixed at deployment so depth elsewhere cannot
redirect it.

What else addresses this weekend, as of 2026-09-23: the 83 Morpho markets
above; Chainlink's own documentation for these feeds, which leaves the
staleness bound to the integrator; and the other public entries of this
buildathon that take on the same gap. Those take three approaches: a haircut
on the last print that grows with time since the close, from an exchange
calendar kept in the contract; a view that reports blocking conditions such
as a stale feed; and off-chain monitoring of the gap between pool and
reference prices. Each decides how far to trust Friday's print. None of the
ones we could read prices the closed session from where the token trades,
and each can sit on top of this price rather than replace it.

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

### Two bands: a quiet feed means a quiet market only while the exchange is open

The feed prints when the price moves 0.5% from the last print, or on its
24-hour heartbeat (`feed_directory.py`). How far the next print lands from
the last one depends on the hour (`session_prints.py`: the last 150 rounds of
AAPL, NVDA and SPY on 2026-09-23, three to thirteen weeks of history):

| where the gap between two prints falls | gaps | largest move between the two prints |
|---|---|---|
| inside one regular session (Monday to Friday, 14:30-20:00 UTC) | 108 | AAPL 0.69%, SPY 0.56%; NVDA 1.43% once (below) |
| across the session's edges, overnight and at weekends | 330 | AAPL 0.84%, SPY 0.70%, NVDA 1.12% |

Inside the session the prints follow the price: each move is printed close
to its 0.5% trigger. The one exception, NVDA's +1.43% between 14:31 and 19:55
UTC on Friday 09-18, came while that print was younger than `liveMaxAge`, so
AfterHours was passing the feed through and no band applied. Outside the
session the next print often lands at the open: AAPL's +0.84% (Wednesday
21:23 to Thursday 13:33 UTC) and +0.78% (Monday 16:43 to Tuesday 13:30),
NVDA's -1.01% (Tuesday 09:06 to 13:32). Whether those moves happened
overnight or at the opening cross, a print from the night before says little
about where the market opens, and earnings and other news land after the
close, when a stock can move far more than 1% before the feed prints again.

So the band depends on the hour as well as the print's age. During the
regular session, while the print is younger than `heartbeat` (24 h), the pool
is held within `quietBandBps` (1%, twice the feed's threshold) of the print:
a feed that prints every 0.5% move leaves nothing larger for the pool to
price, and a pool more than 1% from a print that young is lagging or being
pushed. At every other hour, and once the heartbeat has passed, the pool is
the only current price and may move `maxDeviationBps` (10%) from the print,
as it may through the weekend.

The contract reads the session from the block timestamp: Monday to Friday,
14:30 to 20:00 UTC. The regular session runs 13:30-20:00 UTC in summer and
14:30-21:00 UTC in winter, so these hours are inside it in both regimes; the
first summer hour and the last winter hour get the wide band although the
exchange is open. What a clock cannot see are exchange holidays and half
days. On a holiday that follows a trading day (Good Friday, Juneteenth,
Thanksgiving and the like) and on a half day's afternoon, the narrow band
applies until 20:00 UTC although the exchange is closed, delaying a larger
move by up to five and a half hours. An immutable contract cannot keep a
holiday table current; the weekly clock never changes.

A clamped answer in the quiet tier is still reported as fresh
(`updatedAt` = now), on purpose. The narrow band applies only while the
exchange is open and the feed prints each 0.5% move; a pool that sits beyond
1% of a print that young is more likely dislocated or pushed than informed.
Reporting the old print's time instead would let anyone make the oracle look
stale to every consumer with a staleness bound, by holding the pool 1% off
for ten minutes, well inside the depth floor and far cheaper than a refusal.
`state()` shows `clamped` for a consumer that wants to act on it.

## 4. Parameters (AAPL deployment) and why

| parameter | value | reason |
|---|---|---|
| `liveMaxAge` | 21,600 s (6 h) | On weekdays the feed can go 13-21 h without a print, mostly overnight. Six hours means the pool takes over ~6 h after Friday's last print and during long weekday gaps, held to the 1% band during the regular session and the 10% band at other hours, and the feed takes back over at its next print. Over 09-16 -> 09-23 the feed was older than 6 h for 60% of the week, half of it on weekdays (the live page computes this from the feed's rounds). |
| `twapWindow` | 1,800 s (30 min) | Same window PARE trusts for its pool leg, judged in three 10-minute sub-windows. With ~1 swap every 7 s on the weekend, each sub-window averages ~85 fills. |
| `heartbeat` | 86,400 s (24 h) | The feed's own heartbeat (`feed_directory.py`). The longest weekday gap measured is 20.8 h; every weekend silence (52-76 h) passes it. Outside the regular session the quiet tier never applies. |
| `quietBandBps` | 100 (1%) | Twice the feed's 0.5% deviation threshold, and only during the regular session, where the next print landed at most 0.69% (AAPL) and 0.56% (SPY) from the last (`session_prints.py`). |
| `maxDeviationBps` | 1,000 (10%) | The band outside the regular session and once the heartbeat has passed: the single-stock LULD band for closed-session moves. Measured weekend gaps: AAPL 0.25%, SPY 0.68%, NVDA 1.12% (`feed_gap.py`). |
| `minLiquidity` | 5e16 | What the contract compares with the floor, the median of the 0.05% pool's three 10-minute harmonic means, moves with the pool: 1.39e18 and 1.68e18 in two captures forty minutes apart (blocks 70,448,078 and 70,472,250, `capture_reads.py`), 28 to 34 times the floor. Refusing below about 1/30 of that depth means a pool its LPs have all but left is not trusted. Liquidity clears it from -10.9% to +8.3% of today's price (`pool_depth.py`), about as far as the 10% band reaches, so a real move the LPs have not followed is priced up to the band. 2e17 (the value before review round 7) cleared only -4.5% / +3.2%: it also stopped a price being held beyond that, but refused the moves this oracle exists to price. |
| `maxAnchorAge` | 432,000 s (5 days) | The same bound PARE hard-codes as `MIN_FEED_AGE`: outlasts a Monday-holiday closure plus the feed's early Friday stop. Beyond it the feed is gone or the stock is halted. |

Keeping the primary observable. Uniswap v3 writes at most one observation
per second (per block timestamp; Robinhood Chain's ten blocks a second share
one), so a pool that stores fewer than `twapWindow + 1` observations can be
made to answer "OLD" for a 30-minute window by one small swap a second, and
the oracle would fall to a standby or refuse. `initialize` therefore reads
the primary's `slot0()` and refuses an observation cardinality below
`twapWindow + 1` (1,801) with InvalidConfig(16), and the deploy workflow checks
the same before spending gas; raising it is a permissionless, gas-only call
(`increaseObservationCardinalityNext`). The AAPL 0.05% pool holds 1,801
today, exactly the bound, which is enough: at one observation a second at
most, 1,801 observations always reach at least 1,800 seconds back. Right
after a pool's cardinality grows, the new slots fill one a second, so the
history can be shorter than the count for a while; `initialize` also calls
`observe` at the four points, so such a pool is refused until the history is
there, and it only lengthens after that. A standby may sit below the bound:
it only covers a primary that cannot answer.

What manipulation can buy, and what it costs. `pool_depth.py` walks the AAPL
0.05% pool over its whole tick range, every read at one block (block
70,474,692, 2026-09-23 11:23 UTC; 187 initialized ticks). Liquidity is
concentrated around the price: it clears the 5e16 floor from -10.9% to +8.3%
(the 2e17 floor only from -4.5% to +3.2%), and beyond that only a thin
full-range position remains. Moving the AAPL price 1% takes about $95k of
input, 10% about $234k of USDG up or $314k of AAPL down; the figures move by
several percent from block to block as liquidity shifts. The fee on that is
$47-165; the real cost is holding the move while
every trader who can reach another venue sells the premium back. What a move
buys through this oracle:

- nothing, if it stays inside one 10-minute sub-window, however far it goes
  (the price is the median sub-window);
- at most 1% during the regular session while the print is younger than
  the heartbeat, which includes exchange holidays that follow a trading day
  and the afternoons of half days (see "Two bands");
- otherwise, the level at which it can be held through a whole sub-window
  while the pool there clears the floor, with a second sub-window's average
  reaching it too. With the 5e16 floor that is about as far as the 10% band,
  which caps it in any case.

At a 62.5% LLTV a 10% push down could make positions above 56.25% LTV
liquidatable at Morpho's bonus. The band has to be sized to the market's
LLTV; a curator who wants no such window uses a tighter band or a lower LLTV.
The other direction is the one a borrower profits from: holding the pool up
through two sub-windows (twenty minutes; the pool clears the floor out to
+8.3%) lets them borrow up to that much more against the same collateral.
When the price returns the position is over the LLTV and liquidatable, but
the lender is not left with bad debt unless the push exceeds 1/LLTV - 1: an
upward push inside the 10% band cannot create bad debt in any market whose
LLTV is 90.9% or less.

Denial instead of manipulation. An attacker who cannot move the price can
still try to make the oracle refuse, which freezes borrowing, collateral
withdrawal and liquidation in a Morpho market until the feed prints again.
The pool's in-range liquidity never reaches zero (a full-range position
underlies it), so a refusal needs a stay in its thin stretches. Uniswap's
accumulator counts whole seconds of block time, so one second is the shortest
stay that counts, and with the 5e16 floor no reachable price refuses in one
second: the cheapest refusal is an eight-second stay at +12.3% (liquidity
6.1e14, $235k of USDG to get there) or five seconds at -26.6% ($330k of AAPL).
Stays count in whole seconds: the accumulator advances with block time, so
the 7.15 s the arithmetic asks for takes eight.
The median rule makes that necessary in two sub-windows, so a refusal takes
one such excursion every ten minutes for as long as it lasts. Each is a round
trip that pays about $235 in fees and leaves $235k parked 12% above the market
for eight seconds (eighty blocks), where anyone holding AAPL can sell into
it; the fees alone come to about $1.4k an hour, or about $84k to hold a
60-hour weekend closure, which is the figure to size a market against. With
the 2e17 floor one
second at +17.4% was enough, or ten seconds at +8.6%; before the median rule,
one second at zero liquidity per 30 minutes. A refusal never moves the price; it stops borrowing
and liquidation until the feed's next print, the behaviour a stale-feed guard
gives. A market with no guard keeps liquidating at Friday's price instead,
which is the price this oracle exists to replace.

## 5. What is measured, what is assumed

Measured: feed silence and cadence; weekend swap counts, volume and fill
quality on two weekends; every Morpho market on the chain and its oracle
(`morpho_markets.py`); PARE's 5-day feed tolerance, its 30-minute pool TWAP
and its empty sequencer feed, from its own getters (`pare_oracle.py`); the
share multiplier and its effective time on every deployable stock; the AAPL
pool's reserves and its liquidity over the whole tick range (`pool_depth.py`);
how far the stock feeds let a move run inside the regular session and
overnight (`session_prints.py`); Chainlink's feed list for the chain, with
each feed's heartbeat, threshold and market hours (`feed_directory.py`);
ArbOS 61, Stylus version 3 and program expiry (`ArbWasm.expiryDays()` = 365)
on mainnet and testnet, and the average block time (`stylus_params.py`);
pool observation cardinality per fee tier for every stock (`assets.json`;
AAPL: 1,801 on the 0.05% primary, 1,500 on the standbys);
the window harmonic-mean liquidity against spot (`pool_harmonic.py`); the
feed's `description()` (`Robinhood AAPL / USD`) and the stock's
`oraclePaused()`; the absence of the StylusDeployer on mainnet
(`stylus_params.py`).

Read from verified source: the stock token's multiplier schedule. The
token's implementation (`Stock`,
`0xb35490d6f9163DE4F80d88dc75c3516eb64C5aE2`, verified) returns a scheduled
multiplier from `uiMultiplier()` only once `block.timestamp >= effectiveAt()`,
and refuses to schedule one in the past, so the multiplier the oracle reads
never runs ahead of the time it compares with the last print. A schedule sets
`newUIMultiplier()` and `effectiveAt()` in the same call, and
`newUIMultiplier()` answers 1e18 when none is set, so a future `effectiveAt()`
always comes with the multiplier it schedules.

Assumed: that the pool keeps tracking fair value on a *news* weekend (both
measured weekends were quiet; the band exists precisely because this is not
guaranteed); that LPs re-center their ranges when the stock moves, because
the pool clears the 5e16 floor only from -10.9% to +8.3% of today's price
and a move past that, held for ten minutes, refuses until they do (no weekend
with a large move has been observed on this chain yet); that lending curators will adopt a 24/7 price at all (no market
has run on AfterHours yet; borrowing against stock tokens on this chain is
$6.4k today, so the demand is a bet, not a measurement).

Not handled in v1: a sequencer-uptime check. Chainlink's feed list for this
chain has 58 feeds and none of them is an L2 sequencer-uptime feed
(`feed_directory.py`, 2026-09-23), so there is nothing to read; PARE's
oracle here has no sequencer feed set either (`pare_oracle.py`). After a sequencer outage
AfterHours answers at once, like every oracle on this chain, and a market that
wants a grace period cannot get one from the chain today. Also not handled:
the USDG/USD rate: the feed is in USD, the pool
and Morpho's loan token in USDG, and USDG is treated as one dollar. Chainlink's
USDG/USD feed here (`0x61B7e5650328764B076A108EFF5fa7282a1B9aD2`) updates only
on 0.5% moves, so reading it would correct only a larger depeg, at the cost of
one more call. A depeg would also show as a step at every change of session,
because LIVE_FEED answers in USD and ONCHAIN_TWAP in USDG: with USDG at $0.98,
the answer would step by 2% on Friday evening and back on Monday; pools against a token other than the loan token
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
the guard is the second. The guard covers both orderings: a change that took
effect after the print, and a split-sized change still pending while the
feed may already print the post-action price.

Operational risks, both fail closed: every price read calls the issuer's
upgradeable token for `oraclePaused()`, `uiMultiplier()` and `effectiveAt()`;
if a token upgrade removed one of them, every price read would revert for
good, because there is no admin to repoint anything. A Morpho market cannot
change its oracle, so a market priced by that instance would stop borrowing
and liquidating permanently. Borrowers could still repay and then withdraw
their collateral (Morpho skips the oracle for a position without debt),
lenders could withdraw what is not lent out, and `getRoundData` keeps serving
the feed's history. The way forward would be a new instance and a new
market. All three reads stay mandatory on purpose: without the multiplier
`price()` cannot value a raw unit, and without `effectiveAt()` a split after
the last print would pass through at the pre-split price, valuing collateral
at twice what it is worth until the feed prints. A frozen market is the
lesser failure. And like every Stylus
program the contract must stay activated: activation lasts 365 days on this
chain and must be renewed after a Stylus version upgrade; anyone can pay to
re-activate it, and reads revert until someone does.

## 6. Verification layers and cost per read

| layer | what it proves | where |
|---|---|---|
| 76 unit and property tests on a host that serves mocked calls exactly | decision logic, both bands, scaling, the share multiplier, the fixed venue, the sub-window medians of price and depth, every refusal, exact numbers; property runs over random feed, pool and multiplier data reach every session and every refusal reason | `src/tests.rs`, `src/mockvm.rs` |
| real mainnet answers | the contract's reads of the real Chainlink feed, the real AAPL 0.05% pool (`observe` at the four points, `slot0`, its tokens) and the real stock and quote tokens, served byte for byte from one block, give the answer a separate Python port of the rules gives | `fixtures/aapl_mainnet.txt` (block 70,472,250), `scripts/measure/capture_reads.py`, `src/tests.rs` |
| tick-math reference vectors | 1.0001^tick against 80-digit decimal arithmetic within the module's bound (1e-23 relative, one unit below tick 0), and AAPL's prices exactly | `src/tickmath.rs`, `scripts/measure/tick_vectors.py` |
| `abi/IAfterHours.sol` against `cargo stylus export-abi` | the interface integrators are pointed at declares exactly the functions, return types and errors the contract exports | `scripts/abi_check.py`, `.github/workflows/ci.yml` |
| `cargo stylus check` against Robinhood testnet | the wasm compiles, fits and activates on Stylus v3 / ArbOS 61 | `.github/workflows/ci.yml` |
| end-to-end on a local Nitro node (ArbOS 61, Stylus 3, the same as Robinhood Chain) | the real wasm deployed, activated and initialised; ABI dispatch, storage, external calls, every session, the venue rule, sub-window dips, a spike inside one sub-window and a move held through two, both bands (the narrow one only on runs inside the regular session: a scheduled run every weekday at 15:00 UTC, the first due on 2026-09-23; `result.txt` says which ran), the multiplier and a split, and every revert's exact data asserted through `cast`, and a Solidity contract reading the oracle the way Morpho does, and the deploy workflow's own steps (`scripts/deploy.sh`) run against a second instance, with a preflight that must refuse and a read-back that must fail; 90 assertions | `.github/workflows/e2e.yml`, `e2e/run.sh`, `e2e/src/Mocks.sol` |
| ten independent review rounds, from round 4 against a fixed rubric; rounds 6 to 10 read a clean copy of the repository as it will be published, with no earlier scores | every finding and its fix, with the commit | `REVIEWS.md` |

Gas per read on the dev node (`cast estimate`, includes the 21k transaction
base; two pools configured, the primary answering): `latestRoundData()`
114,724 in LIVE_FEED, 155,673 in ONCHAIN_TWAP; `price()` 116,987 / 157,925.
These are against Solidity test doubles; the real feed, pool and beacon-proxy
token cost more per call and the figure will be re-measured on mainnet. Five
external reads (pause flag, multiplier, its effective time, feed round, pool
observe) account for most of it; at Robinhood Chain's gas prices that is
a fraction of a cent, and a Morpho borrow or liquidation pays it once. Latency
is not a network property here: in LIVE_FEED the answer is the feed's own
round with no added delay; in ONCHAIN_TWAP the answer is by design the
median of three 10-minute averages, so a genuine move starts to show after
ten minutes and shows in full after twenty; what lasts under ten minutes
never shows. That is the manipulation trade-off the window encodes.

## 7. Why Stylus, why Robinhood Chain, why USDG

What Stylus bought here is one body of Rust that is both the contract and
what the tests run. The decision code in `evaluate()` runs unchanged on a
host under `cargo test`: every CI run sends 3,000 random feed, pool and
multiplier cases through it and fails unless every session and every refusal
reason was reached, and the same code, compiled to wasm, then runs on a Nitro
node. The fixed-point math needs 512-bit intermediates (1.0001^tick by
square-and-multiply, the price conversion); `alloy`'s `U512` gives them
without assembly or unchecked blocks, checked against reference values
computed independently in 80-digit decimal arithmetic. There is no Solidity
twin to compare gas against, so no gas saving is claimed. The
problem only exists where tokenized stocks trade around the clock with a
market-hours oracle, which today is Robinhood Chain, and the quote asset of
every stock pool there is USDG.
