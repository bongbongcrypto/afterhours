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

Decision per read:

1. `oraclePaused()` on the stock token (the issuer's corporate-action flag,
   which Chainlink documents as advisory) -> **PAUSED**, reads revert.
2. Feed round invalid (answer <= 0, `updatedAt` 0 or in the future) ->
   **NO_DATA(1)**, reads revert.
3. Feed age <= `liveMaxAge` -> **LIVE_FEED**: the feed's round, verbatim.
4. Otherwise the market is closed for this oracle's purposes:
   - in-range pool liquidity < `minLiquidity` -> **NO_DATA(2)**;
   - `observe([twapWindow, 0])` fails (pool without history) or the tick
     converts to nothing usable -> **NO_DATA(3)**;
   - else **ONCHAIN_TWAP**: the time-weighted pool price, converted to the
     feed's decimals, then bounded to
     `[feedAnswer * (1 - band), feedAnswer * (1 + band)]`. If the TWAP sits
     outside, the edge is returned and `clamped = true`.

`latestRoundData()` in ONCHAIN_TWAP keeps the feed's round ids, reports the
last exchange print as `startedAt` and `block.timestamp` as `updatedAt`
(the answer is derived from trades happening now). `state()` never reverts
for market reasons, so dashboards and keepers can see why the oracle refuses.

The configuration is written once by `initialize` and cannot be changed.
There is no owner, no pause switch, no upgrade path. Robinhood Chain mainnet
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
| `minLiquidity` | 2e17 | The 0.05% pool holds L = 1.6e18 today; refusing below 1/8 of that means a pool most LPs have left is not trusted. |

Manipulation cost, order of magnitude (`scripts/measure`, uniform-range model
which overstates depth away from the current tick; the pool's $355k USDG
reserve bounds it from below): pushing the current range 10% takes roughly
$0.4-1.5M of one-sided flow, which then has to be held for the whole 30-minute
window against ~$80k/hour of organic weekend volume and any arbitrageur, and
unwound through the same 0.05% fee and price impact. The maximum effect on the
oracle is the band.

## 5. What is measured, what is assumed

Measured: feed silence and cadence; weekend swap counts, volume and fill
quality on two weekends; the live lending oracle's 5-day tolerance and its use
of a pool TWAP; Stylus availability on mainnet and testnet (stylusVersion 3);
pool observation cardinality (1,500-1,801, so a 30-minute TWAP reads today);
the absence of the StylusDeployer on mainnet.

Assumed: that the pool keeps tracking fair value on a *news* weekend (both
measured weekends were quiet; the band exists precisely because this is not
guaranteed); that lending curators will adopt a 24/7 price at all (PARE's own
constant and comment say they dislike the 5-day rule, but no market has run on
AfterHours yet).

Not handled in v1: a Chainlink L2 sequencer-uptime check (PARE deploys with it
disabled on this chain too); multiple pools per asset; assets whose only pool
is against a token other than the loan token.

## 6. Why Stylus, why Robinhood Chain, why USDG

The TWAP and band arithmetic is fixed-point integer math on 256-bit values
with 512-bit intermediates; Rust with `alloy` primitives expresses it without
unchecked blocks or assembly, and the contract is unit-tested against
reference vectors computed independently in 60-digit decimal arithmetic. The
problem only exists where tokenized stocks trade around the clock with a
market-hours oracle, which today is Robinhood Chain, and the quote asset of
every stock pool there is USDG.
