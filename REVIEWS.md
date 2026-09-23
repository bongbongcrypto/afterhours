# Reviews

Each round was an independent reviewer (a separate agent, given the repository
and, from round 4, a fixed scoring rubric, but no self-assessment) reading
the code, running the read-only measurement scripts and reporting defects.
Every finding below was either fixed in the commit shown or is listed as
open with the reason.

## Round 1 (2026-09-19): contract logic

| finding | resolution | commit |
|---|---|---|
| A dead feed would be anchored forever, reported as fresh | `maxAnchorAge` cap, NO_DATA(4) | 13de478 |
| Spot pool liquidity can be faked in the block before a read | harmonic-mean liquidity over the window | 13de478 |
| A wrong pool fails only at the first weekend | `observe()` exercised at `initialize` | 13de478 |
| Chainlink consumers call `getRoundData` and the v2 getters | both implemented, current round mirrors `latestRoundData` | 13de478 |
| Pause flag must win over a stale or broken feed | PAUSED checked first | 13de478 |
| Decimals and window unbounded | bounds at `initialize` (InvalidConfig 7, 9) | 13de478 |
| The deploy script trusts its own inputs | receipt, config read-back and feed description verified | 13de478 |
| `minLiquidity` default not derived from the pool | 2e17 from the measured harmonic mean | 13de478 |
| Reference vectors not reproducible | `scripts/measure/tick_vectors.py` committed | 13de478 |
| Band arithmetic can overflow on absurd prints | fail closed before any pool read | f594de5 |

## Round 2 (2026-09-19): deployment and edges

| finding | resolution | commit |
|---|---|---|
| Feed description was assumed | real on-chain text `Robinhood AAPL / USD` checked | ccda240 |
| Deploy can spend gas before a failing read | preflight exercises every `initialize` read | ccda240 |
| Quote, decimals and initializer not verified after deploy | read back and compared | ccda240 |
| A feed answering `decimals()` but not rounds passes init | round read at `initialize` | ccda240 |
| Liquidity edges untested | empty second, uint160 wrap, u128 saturation tests | ccda240 |
| History while refusing | `getRoundData` serves history while PAUSED / NO_DATA | ccda240 |
| 30-minute refusal tail undocumented | documented (and removed in round 4) | ccda240 |
| Harmonic vs spot liquidity unmeasured | `scripts/measure/pool_harmonic.py` | ccda240 |

## Round 3 (2026-09-23): venue, observation history, evidence

| finding | resolution | commit |
|---|---|---|
| "Deepest pool wins" lets anyone who deepens a shallow tier choose the source | fixed primary; standbys only while it cannot be observed | 12f8919 |
| A pool with fewer than window + 1 observations can be forced to answer "OLD" | deploy preflight requires cardinality >= 1,801 | e19cd47 |
| Property test could pass without reaching every branch | counting runner asserts every session and reason | 12f8919 |
| Asset manifest trusted token names | beacon-verified discovery | e19cd47 |
| Foundry unpinned; pool list whitespace | pinned; normalized | e19cd47 |
| Band property did not pin the TWAP | exact TWAP and float cross-check | 12f8919 |
| e2e under-asserted the venue rule | venue sections; mock checks the request shape | 7c63c58 |
| Duplicate pools accepted | InvalidConfig 13 | 12f8919 |
| `state()` corner cases | zero pool/liquidity when no pool was read | 12f8919 |
| Stale numbers across docs; probe label | refreshed; label fixed | cb6ff8a |

## Round 4 (2026-09-23): blind review against the judging criteria

| finding | resolution | commit |
|---|---|---|
| The docs called PARE the chain's single lending market; Morpho has 83 funded stock-collateral markets | census script `morpho_markets.py`; claim rewritten everywhere | bb82a42 |
| One second out of range refuses for 30 minutes, a cheap and repeatable way to stop liquidations | liquidity is the median of three 10-minute sub-windows; the cost of a refusal was re-measured in round 5 | 01d8366 |
| Manipulation-cost paragraph used reserves as a lower bound | replaced by a tick-by-tick depth map (`pool_depth.py`); the band caps the damage | 7f83d1a |
| Raw token units vs per-share feed (found while checking the census oracles): one raw unit is `uiMultiplier` shares | `price()` multiplies by `uiMultiplier`; TWAP per share; NoData(5) when a multiplier change after the print moves the pool past the band | 01d8366 |
| USD feed vs USDG pool and loan token | documented (USDG/USD feed updates only on 0.5% moves) | bb82a42 |
| AAPL standby pools can never clear the floor | AAPL deploys with the primary only | bb82a42 |
| A zero-liquidity answer moved to the standby, contradicting the docs | an answering primary is always the venue | 01d8366 |
| Weekday gap and Labor Day citations stale | re-measured, sources dated | bb82a42 |
| Gas measured against test doubles only | stated; re-measure on mainnet | bb82a42 |
| e2e accepted any revert on the second `initialize`; revert checks ignored the reason | exact `AlreadyInitialized()` and `NoData(reason)` data | 01d8366 |
| Issuer-interface dependency and Stylus expiry undocumented | documented in DESIGN §5 and README | bb82a42 |
| The review count traced only to a progress log | this file | bb82a42 |
| Not deployed, no public repo, no live URL, no final video | carried to round 5 | |

## Round 5 (2026-09-23): second blind review against the judging criteria

| finding | resolution | commit |
|---|---|---|
| The whole-window average was paired with a per-sub-window depth check: a spike held inside one sub-window moved the price while the check let it through. Past about 8% the AAPL pool is nearly empty, so ten minutes there reached the band edge | the price is the median of the three sub-windows' mean ticks: a move has to show in two sub-windows, and at most one of them may be thin | 112179e |
| A weekday feed that is only quiet (13-21 h gaps) handed the price to the pool with a 10% band, although the feed would have printed on a 0.5% move | two bands: 1% while the last print is younger than the feed's 24 h heartbeat, 10% after (`heartbeat`, `quietBandBps`, InvalidConfig 15, `quietTier()`) | 112179e |
| The cheapest refusal was searched only within 10% of the price | `pool_depth.py` walks the pool's whole tick range and counts whole seconds: one second at +17.4% ($235k to get there), once every ten minutes, about $1.4k an hour in fees | this commit |
| Found while re-measuring: liquidity clears the 2e17 floor only from -4.6% to +3.2% of today's price, so a genuine move past that refuses until LPs re-center | documented in README and DESIGN as a limit, with the floor as a per-instance trade-off | this commit |
| "The one oracle with a closure policy" missed two census oracles with a four-day bound; "a five-day-old print" put PARE among the 83 markets | corrected in DESIGN, README, the live page and the video | this commit |
| "Change one address" does not hold for Morpho, whose market oracle is fixed at creation | "open a market with AfterHours as its oracle" | this commit |
| The live page read AAPL's standby pools, which the instance does not configure | the page reads exactly the instance's pools | 112179e |
| Stale comments and counts (observe points, reasons 1-4, 33 KB, test counts); video lines not yet true ("deployed on mainnet", "This is Saturday", "no contract can read that price") | refreshed; the video says what is true before deployment | this commit |
| Not deployed, no public repo, no live URL, no final video | open: need the owner's funding and approvals | |
