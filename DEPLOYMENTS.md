# Deployments

| network | chain id | asset | address | initialize params | tx |
|---|---|---|---|---|---|
| (none yet) | | | | | |

Inputs used for AAPL on Robinhood Chain mainnet (4663):

| input | value |
|---|---|
| feed (Chainlink AAPL/USD) | `0x6B22A786bAa607d76728168703a39Ea9C99f2cD0` |
| pool (Uniswap v3 AAPL/USDG 0.05%) | `0xaae0d815ee56e4092a5e5c2911e676fea50b2d6d`, the primary and only pool: observation cardinality 1,801 = window + 1, so it cannot be made unobservable. The 0.30% and 1% tiers are not configured: their liquidity (1.7e17 and 1.7e14 in `assets.json`, 2026-09-23) sits below the 2e17 floor, so as standbys they could only change a refusal's reason. |
| stock (AAPL) | `0xaF3D76f1834A1d425780943C99Ea8A608f8a93f9` |
| quote (read from the pool) | USDG `0x5fc5360D0400a0Fd4f2af552ADD042D716F1d168` |
| liveMaxAge | 21600 |
| twapWindow | 1800 |
| maxDeviationBps | 1000 (the band once the heartbeat has passed) |
| minLiquidity (median of three 10-minute harmonic means) | 200000000000000000 |
| maxAnchorAge | 432000 (5 days) |
| heartbeat | 86400 (the feed's 24 h heartbeat) |
| quietBandBps | 100 (the band while the last print is younger than the heartbeat) |
| expected feed description | `Robinhood AAPL / USD` (measured with `pool_harmonic.py`; the oracle's own `description()` returns `Robinhood AAPL / USD (AfterHours)`) |
