# Reviews

Each round was an independent reviewer (a separate agent, given the repository
and, from round 4, a fixed scoring rubric, but no self-assessment) reading
the code, running the read-only measurement scripts and reporting defects.
Rounds 6 to 11 ran from an empty folder with a clean copy of the repository
as it will be published, the rubric and the form text; they saw the findings
below, not any score.
Round 12 was not an independent reviewer: it was a self-review of the
contract against Chainlink's documentation, and it corrected the unit model
that rounds 4 to 11 had built on (the rows it supersedes are listed in it and
left as they were written).
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

## Round 4 (2026-09-23): review against the judging criteria

| finding | resolution | commit |
|---|---|---|
| The docs called PARE the chain's single lending market; Morpho has 83 funded stock-collateral markets | census script `morpho_markets.py`; claim rewritten everywhere | bb82a42 |
| One second out of range refuses for 30 minutes, a cheap and repeatable way to stop liquidations | liquidity is the median of three 10-minute sub-windows; the cost of a refusal was re-measured in round 5 | 01d8366 |
| Manipulation-cost paragraph used reserves as a lower bound | replaced by a tick-by-tick depth map (`pool_depth.py`); the band caps the damage | 7f83d1a |
| Raw token units vs per-share feed (found while checking the census oracles): one token of raw balance is `uiMultiplier / 1e18` shares | `price()` multiplies by `uiMultiplier`; TWAP per share; NoData(5) when a multiplier change after the print moves the pool past the band | 01d8366 |
| USD feed vs USDG pool and loan token | documented (USDG/USD feed updates only on 0.5% moves) | bb82a42 |
| AAPL standby pools can never clear the floor | AAPL deploys with the primary only | bb82a42 |
| A zero-liquidity answer moved to the standby, contradicting the docs | an answering primary is always the venue | 01d8366 |
| Weekday gap and Labor Day citations stale | re-measured, sources dated | bb82a42 |
| Gas measured against test doubles only | stated; re-measure on mainnet | bb82a42 |
| e2e accepted any revert on the second `initialize`; revert checks ignored the reason | exact `AlreadyInitialized()` and `NoData(reason)` data | 01d8366 |
| Issuer-interface dependency and Stylus expiry undocumented | documented in DESIGN §5 and README | bb82a42 |
| The review count traced only to a progress log | this file | bb82a42 |
| Not deployed, no public repo, no live URL, no final video | carried to round 5 | |

## Round 5 (2026-09-23): second review against the judging criteria

| finding | resolution | commit |
|---|---|---|
| The whole-window average was paired with a per-sub-window depth check: a spike held inside one sub-window moved the price while the check let it through. Past about 8% the AAPL pool is nearly empty, so ten minutes there reached the band edge | the price is the median of the three sub-windows' mean ticks: a move has to show in two sub-windows, and at most one of them may be thin | 112179e |
| A weekday feed that is only quiet (13-21 h gaps) handed the price to the pool with a 10% band, although the feed would have printed on a 0.5% move | two bands: 1% while the last print is younger than the feed's 24 h heartbeat, 10% after (`heartbeat`, `quietBandBps`, InvalidConfig 15, `quietTier()`) | 112179e |
| The cheapest refusal was searched only within 10% of the price | `pool_depth.py` walks the pool's whole tick range and counts whole seconds: one second at +17.4% ($235k to get there), once every ten minutes, about $1.4k an hour in fees | e8d2730 |
| Found while re-measuring: liquidity clears the 2e17 floor only from -4.6% to +3.2% of today's price, so a genuine move past that refuses until LPs re-center | documented in README and DESIGN as a limit, with the floor as a per-instance trade-off | e8d2730 |
| "The one oracle with a closure policy" missed two census oracles with a four-day bound; "a five-day-old print" put PARE among the 83 markets | corrected in DESIGN, README, the live page and the video | e8d2730 |
| "Change one address" does not hold for Morpho, whose market oracle is fixed at creation | "open a market with AfterHours as its oracle" | e8d2730 |
| The live page read AAPL's standby pools, which the instance does not configure | the page reads exactly the instance's pools | 112179e |
| Stale comments and counts (observe points, reasons 1-4, the e2e size comment, test counts); video lines not yet true (a mainnet deployment, a Saturday capture, and a claim that no contract reads the pool, which the raw-pool adapters do) | refreshed; the video says what is true before deployment | e8d2730 |
| Not deployed, no public repo, no live URL, no final video | carried to round 6 | |

## Round 6 (2026-09-23): review from a clean copy of the repository as published

| finding | resolution | commit |
|---|---|---|
| Slide 6 showed an invented terminal capture under a mainnet-weekend heading | slide 6 shows only real reads: `probe.py` output after deployment, a dated capture of the live page before it; the placeholder and its numbers are gone | f6c31b5 |
| A token upgrade removing one of the three functions the oracle reads would freeze a Morpho market for good; the docs said a new instance fixes it | README and DESIGN state the permanent freeze and what still works (repay, debt-free collateral withdrawals, supply withdrawals, the feed's history); the three reads stay mandatory, with the reason | f6c31b5 |
| The 1% band's premise (the exchange traded within 0.5% of the print) does not hold after Friday's close; NVDA moved 1.12% across a measured weekend | Saturdays and Sundays (UTC) always get the wide band, a fixed property of the block timestamp with no calendar to maintain; the remaining delay (Friday evening, weekday holidays) is documented with the measured gaps | f6c31b5 |
| The answer's unit is USD in LIVE_FEED and USDG in ONCHAIN_TWAP, so a USDG depeg shows as a step at each change of session | documented | f6c31b5 |
| `feed_gap.py` crashed on the public RPC's rate limit | backs off like its siblings | f6c31b5 |
| The contract size, quoted to 0.1 KB, drifted with the lockfile | quoted rounded to the kilobyte | f6c31b5 |
| PROGRESS and `assets.json` disagreed with DEPLOYMENTS about AAPL's pools and floor | the manifest records a deployed instance's own inputs (`discover_assets.py`), and the live page reads them from it | f6c31b5 |
| ArbOS 61, Stylus 3, the 365-day expiry and the block time were listed as measured with no script | `scripts/measure/stylus_params.py` reads them from the chain's precompiles on mainnet and testnet | f6c31b5 |
| `Quote.band_bps` was written and never read | removed | f6c31b5 |
| `price()` reported "feed invalid" on an overflow while `state()` answered | its own reason, `NoData(6)`, raised by `price()` only | f6c31b5 |
| `decimals()` answered 0 before `initialize` | reverts `NotInitialized` | f6c31b5 |
| `getRoundData` ran the full evaluation for historical rounds and would stop with the token | history is read from the feed alone; the current round still mirrors `latestRoundData` | f6c31b5 |
| A per-share price could round to zero at absurd feed values | refused (NO_DATA 3); the property run asserts every answer is positive | f6c31b5 |
| "Verify it in five minutes" hid a 30-minute script | each row states its cost | f6c31b5 |
| The tick reference vectors differed by 1 ulp from their generator | the generator's 80-digit output, pasted verbatim | f6c31b5 |
| Found while re-checking the page: the public RPC rate-limits bursts (about 280 calls in two seconds) and its limit response carries a doubled CORS header, so the first load logged network errors and waited on retries | one RPC batch every 750 ms across the whole page, measured to stay under the limit | f6c31b5 |
| `initialize` can be front-run between deployment and configuration | kept: the deploy workflow fails on a reverted `initialize` and reads every field back, so a front-run costs a redeploy | |
| Not deployed, no public repo, no live URL, no final video | carried to round 7 | |

## Round 7 (2026-09-23): second review from a clean copy of the repository as published

| finding | resolution | commit |
|---|---|---|
| On a weekday the 1% band also judged corporate actions, so a 0.6% distribution became `NoData(5)`; the docs said a dividend stays inside the band | a corporate action is judged against the wide band whatever the print's age; a distribution is priced and held to the band that applies, only a split refuses | be241af |
| The 2e17 floor refused a real move past -4.5% / +3.2%, the moment a lending market most needs a price | AAPL's floor is 5e16: the pool clears it from -10.9% to +8.3%, about as far as the band; no reachable price refuses in one second any more | be241af |
| The quieter denial (ten seconds at +8.6%) was not documented | both refusal costs are in DESIGN for either floor | be241af |
| The observation-history guard lived only in the deploy workflow | `initialize` reads the primary's `slot0()` and refuses a cardinality below window + 1 (InvalidConfig 16) | be241af, ee82477 |
| Nothing called the oracle the way Morpho does | e2e deploys a Solidity consumer: `price()` and `latestRoundData()` through STATICCALL, and a refusal bubbling up unchanged | be241af |
| "One raw unit is `uiMultiplier / 1e18` shares" is off by 10^18 in five places | one sentence everywhere: a token of raw balance is `uiMultiplier / 1e18` shares | be241af |
| `weekend_swaps.py` gave up on the RPC's rate limit; `pool_depth.py` had no backoff | both back off for up to about five minutes; `pool_depth.py` reads everything at one block and prints it. The re-run reproduced 44,338 and 26,331 swaps, $5.14M and $4.17M | be241af |
| The "2% depth" in `assets.json` assumes in-range liquidity holds and is about twice the tick-walked cost | stated where the tiers are described | be241af |
| A clamped answer in the quiet tier is reported as fresh, hiding a large weekday move from a consumer's staleness check | kept, with the reason in DESIGN: the feeds print weekday moves within minutes in every session, and reporting the old time would let anyone make the oracle look stale by holding the pool 1% off for ten minutes | |
| Not deployed, no public repo, no live URL, no final video | carried to round 8 | |

## Round 8 (2026-09-23): third review from a clean copy of the repository as published

| finding | resolution | commit |
|---|---|---|
| The weekday 1% band rested on "the price just did not move 0.5%", which `feed_cadence.py`'s own prices contradict: AAPL was +0.78% and +0.53% across weekday gaps with no print in between | the narrow band applies only during the US regular session (Monday to Friday, 14:30-20:00 UTC); at every other hour the pool gets the wide band. `session_prints.py` measures why: inside the session the next print lands at most 0.69% (AAPL) and 0.56% (SPY) from the last, while outside it the next print often lands at the open (AAPL +0.84%, NVDA -1.01%) | ef3d3b1, db352cc |
| No sequencer-uptime check, listed without a reason | Chainlink lists none of its 58 feeds on Robinhood Chain as a sequencer-uptime feed (`feed_directory.py`); README and DESIGN say so | ef3d3b1, db352cc |
| The live page judged a corporate action on the band that applies, the contract on the wide band, and the self-check could not see the difference | the page uses the wide band too and refuses a per-share price that rounds to zero; its self-check runs four decision cases from the unit tests through the page's own rules (quiet session, after hours, a 3% distribution, a split) | ef3d3b1 |
| Every asset but AAPL in `assets.json` kept the primary-liquidity / 8 floor that round 7 rejected for AAPL | / 32, AAPL's ratio, for every asset; `discover_assets.py` writes it, and README says how to check an asset's range with `pool_depth.py` | ef3d3b1 |
| Holding a Morpho market in refusal costs about $1.5k an hour; the weekend total was missing | about $88k over a 60-hour weekend, next to the hourly figure | db352cc |
| The tick-math tests allowed 10 parts per billion, a million times looser than the module's stated bound, and README called the math exact | the tests assert the bound (1e-23 relative, one unit below tick 0) and AAPL's prices exactly; README states what the tests show | ef3d3b1 |
| The cost of a 1% move had drifted from the quoted $92k-100k | `pool_depth.py` re-run at block 70,432,466: $90k-102k, 10% at $243k up / $305k down; DESIGN notes that the figures move by a few percent between blocks | db352cc |
| `description()` and `quietTier()` answered before `initialize` | both revert `NotInitialized`; `config()` stays readable (its first field says whether initialize ran), `pools()` is empty, `version()` is a constant | ef3d3b1 |
| `MAX_TWAP_WINDOW` of one day could never pass: a uint16 cardinality holds at most 65,535 observations | 65,534, tested at the bound | ef3d3b1 |
| The submission text tied three silent windows to two swap figures | each figure names its window (submission text, outside this repository) | |
| Suspected: mainnet gas is higher than the figures measured on test doubles | README says the gas is measured on Solidity doubles and will be re-measured on mainnet | db352cc |
| Suspected: the deploy workflow, the one that must work on the day, had never run | its steps moved into `scripts/deploy.sh`, which e2e runs against a second instance on every push: a preflight that must refuse a primary one observation short, then preflight, deploy, initialize and a read-back that pass, and a read-back that must fail on a mismatched field | ab2fe8f |
| USD feed against a USDG pool; `initialize` can be front-run | kept, as documented | |
| Not deployed, no public repo, no live URL, no final video | carried to round 9 | |

## Round 9 (2026-09-23): fourth review from a clean copy of the repository as published

| finding | resolution | commit |
|---|---|---|
| The e2e asserted the narrow band only when CI happened to run inside the regular session, and the run a judge downloads had not | `result.txt` says which band ran; a scheduled run every weekday at 15:00 UTC asserts the narrow band on chain; the unit test covers every hour | 46330c3 |
| Nothing had read a real Uniswap v3 pool, the real feed or the real token; the contract's decoding of `observe` and `slot0` was tested only against doubles | a unit test serves the exact answers the real feed, the AAPL 0.05% pool and the stock and quote tokens gave at block 70,448,078 and checks the contract's answer against a separate Python port of the rules; `capture_reads.py` re-captures them at any block | 46330c3 |
| `price()` could refuse on an intermediate overflow where the result fits | the product is formed in 512 bits; tested at 1e50 per share | 46330c3 |
| README said `initialize` accepts the low-cardinality tier; it refuses it (InvalidConfig 16) | the tier says `initialize` refuses until someone raises the cardinality | 84b8921 |
| The floor margin (1.68e18, 34x) was stale and not pinned to a block | the quantity the contract compares, the median of three harmonic means, at block 70,448,078: 1.39e18, 28 times the floor | 84b8921 |
| The floor ratio was quoted as 34 rounded down to 32 | 33.6, rounded to 32 | 84b8921 |
| PARE's 5-day tolerance was listed as measured, but `pare_oracle.py` could not read it | the script reads PARE's own getters: `MIN_FEED_AGE` and `maxFeedAge` 5 days, `twapWindow` 1,800 s, no sequencer feed | 84b8921 |
| The narration said the feed goes silent for 52 hours every weekend | 52 to 57 hours | 84b8921 |
| A dividend after the last print moves pricing to the pool while the feed is fresh, and a thin pool then refuses | stated in README's "What it does not do", with the AAPL margin | 84b8921 |
| Suspected: the multiplier could change before `effectiveAt()` | the token's verified implementation returns a scheduled multiplier only once `block.timestamp >= effectiveAt()` and refuses a schedule in the past; cited in DESIGN | 84b8921 |
| Suspected: the AAPL pool's cardinality (1,801) leaves no margin | DESIGN says why exactly 1,801 suffices at one observation a second | 84b8921 |
| `initialize` can be front-run; other entries are described, not named | kept as documented | |
| Not deployed, no public repo, no live URL, no final video | carried to round 10 | |

## Round 10 (2026-09-23): fifth review from a clean copy of the repository as published

| finding | resolution | commit |
|---|---|---|
| The corporate-action guard covered one ordering only: a split scheduled but not yet in effect, with the feed already printing the post-split price, passed through, and `price()` valued it with the old multiplier (half the collateral) | while a split-sized change is pending, every read goes to the pool, which must put the print within the wide band, else `NoData(5)`; a fresh print the pool confirms passes through; dividend-sized pending changes do not read the pool | 9c2e4fe |
| The submission said 28 stocks could be deployed today, but 10 of them keep too little history and `initialize` refuses them | 28 have a feed and an observable pool, 18 pass `initialize` today, 14 meet the bar: in the submission text, the page, the progress notes and the narration | 9c2e4fe, 835a039 |
| The deploy preflight skipped the decimals, the multiplier and its schedule, all read by `initialize` | the preflight reads every value `initialize` reads | 9c2e4fe |
| The cheapest refusal was quoted as 7 seconds; 7.15 s of exposure takes eight whole seconds | `pool_depth.py` reports whole seconds; eight and five seconds, re-measured at block 70,474,692 | 9c2e4fe, 835a039 |
| DESIGN presented the scheduled narrow-band run as done; it had not run yet | it says the run is due; `result.txt` says whether a run asserted the narrow band | 835a039 |
| Nothing checks that the pool trades | stated in "What it does not do" | 835a039 |
| The upward push, the direction a borrower profits from, was not sized | DESIGN: up to about 8% more borrowing; inside the band it cannot leave bad debt at an LLTV of 90.9% or less | 835a039 |
| `abi/IAfterHours.sol` was never checked against the contract | CI compares it with `cargo stylus export-abi` (`scripts/abi_check.py`) | 9c2e4fe |
| `initializer` was recorded from `tx.origin` | `msg.sender` | 9c2e4fe |
| `NoData(1)` also covers a band overflow; `state()` zeroed the pool price on `NoData(5)`; the progress stamp was stale | the README row says so; the price is kept; restamped | 9c2e4fe, 835a039 |
| Suspected: a pool whose cardinality has just grown holds less history than its count | `initialize` calls `observe` at the four points, so such a pool is refused until the history is there; DESIGN says so | 835a039 |
| Other entries are described, not named or measured; no live market | kept unnamed; a live market needs the deployment | |
| Not deployed, no public repo, no live URL, no final video | carried to round 11 | |

## Round 11 (2026-09-23): sixth review from a clean copy of the repository as published

| finding | resolution | commit |
|---|---|---|
| The submission text said "measured gas" without saying it is measured against test doubles | it says so, and that mainnet will cost more (submission text, outside this repository) | |
| A storage comment still said `tx.origin`; a units comment still said "one raw token unit" | both corrected | aeb0e7b |
| The docs equated a refusal with a stale-feed market: a refusal blocks liquidation, while a market with no guard keeps liquidating at Friday's price | DESIGN and the submission use README's wording | aeb0e7b |
| `contract-client-gen` looked like a dead feature | kept: stylus-sdk's `#[public]` expansion gates code on it; Cargo.toml says so | aeb0e7b |
| Suspected: a future `effectiveAt()` with no scheduled multiplier would read as a pending split | cannot happen with the token's verified implementation: one call sets both, and `newUIMultiplier()` answers 1e18 when none is set; DESIGN cites it | aeb0e7b |
| `initialize` can be front-run; other entries are described, not named | kept, as documented | |
| Not deployed, no public repo, no live URL, no final video | deployed on Robinhood Chain mainnet on 2026-09-25 with its Morpho market (DEPLOYMENTS.md); the public repository, live page and final video follow | 5fb7341 |

## Round 12 (2026-09-27): unit model, a self-review against Chainlink's documentation

| finding | resolution | commit |
|---|---|---|
| The contract treated Chainlink's Robinhood feed as a price per share. Chainlink documents it as the token's price, "Token Price = Underlying Equity Market Price × Multiplier", the multiplier read from the token's `uiMultiplier()`, continuous through splits ([Robinhood tokenized equities](https://docs.chain.link/data-feeds/tokenized-equity-feeds/robinhood), "Total Return Value calculation"). So `price()` counted the multiplier twice in LIVE_FEED (0.057% high for AAPL today, ten times after a 10:1 split), and ONCHAIN_TWAP divided the pool's price of a raw token by it and banded that against a per-token print: after a 10:1 split before the last print, 9 to 10 times the collateral value. Found by a self-review against Chainlink's documentation, 2026-09-27 | every quantity is per token of raw balance: `price()` = answer × Morpho scale, the pool's price meets the band as it is, and the multiplier, its effective time and a pending multiplier are never read (in `initialize` or in any read). `NoData(5)` and `InvalidConfig(14)` are reserved codes, never raised. New tests: a 10:1 split (pause, multiplier 10, continuous print) and a reinvested dividend with every answer and `price()` unchanged or counted once; the multiplier is left unmocked so any read fails, and the call log is checked. The real-mainnet fixture replayed with the per-token rules (`capture_reads.py --replay`: answer 34,053,268,877, `price()` = answer × 1e16). The e2e split scenario replaces the `NoData(5)` ones, and the deploy read-back checks `price()` = answer × scale | 4c7680e |
| The same unit error in the tools: `probe.py`, `pool_depth.py` and the live page divided the pool's price by the multiplier, and `capture_reads.py`'s port multiplied `price()` by it | all per token; `probe.py` reads `state()` and `price()` at one block and flags a `price()` that is not the answer times the scale | 618b387 |
| The deployed instance `0x69190621e300cd2bc4cbb80777b517691ee80f65` keeps the per-share model for good (no owner, no upgrade); its Morpho market holds no supply | marked superseded in `deployments.json`; the page, the hourly status job and `mainnet_gas.py` skip it; README and DESIGN say not to integrate it. The fixed instance and a new market follow in a separate deploy step | d63ef9c |
| Superseded by the rows above, and kept as written: round 4, "Raw token units vs per-share feed" (`price()` × `uiMultiplier`, TWAP per share, `NoData(5)`); round 6, the three mandatory token reads and the per-share price that rounds to zero; round 7, a corporate action judged against the wide band and the one-sentence units wording; round 8, the page's corporate-action decisions; round 9, the 512-bit `price()` at 1e50 per share, the dividend-after-the-print limitation and the multiplier-schedule citation; round 10, the pending-split guard, the preflight's multiplier reads and the pool price kept on `NoData(5)`; round 11, a future `effectiveAt()` without a scheduled multiplier | the code they describe is gone; the one token read left is `oraclePaused()` | 4c7680e |
| Not deployed with the fixed code | carried: redeploy, new Morpho market, and the mainnet gas measured again (two fewer token reads per call) | |
