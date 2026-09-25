# Deployments

| network | chain id | asset | address | initialize params | tx |
|---|---|---|---|---|---|
| Robinhood Chain mainnet | 4663 | AAPL | `0x69190621e300cd2bc4cbb80777b517691ee80f65` | the inputs below | deploy `0x6ee581c608ab8da6b27671e291024effd7b165007cff27ce377deb7e685a379e`, activate `0x0da9d4ebe2e2268f1484508bb752e983e40d68843d58426384ae14e3d47db73f`, initialize `0xbbec5c96b9f9b4540fb58c8414bf9b893481c0bfa2217cc33d52ad413725b6f3` |

Deployed 2026-09-25 22:56 UTC by the manual `deploy` workflow from commit `38ec3f9`, built reproducibly in `offchainlabs/cargo-stylus-base:0.10.9` (wasm sha256 `66848ef80af973d90ce715568723021ada8e2647cf0bef2d460d95e498acac2b`). The workflow read every configured field back and found them equal to the inputs. Receipts: all four status 1; gas used 63,660 (deploy), 6,412,290 (activate), 489,663 (initialize), 197,601 (Morpho `createMarket`).

Morpho Blue market priced by this instance: loan USDG, collateral AAPL, LLTV 62.5%, market id `0x516829adb04c8351c09747755eed21f4514370e2df11a73c89054fef7de316bc`, created in tx `0xe7cc78e6bce947d8b1d95f5853d9ce0f11b30fdb2bc730fc69c2d085ebb65419`. Nothing of ours is supplied or borrowed in it.

First read after deployment (`scripts/probe.py`, 2026-09-25 22:57 UTC, a Friday after the US close): `LIVE_FEED`, answer $341.4532 = the feed's 19:49 UTC print; primary pool $341.0896 per share, median liquidity 8.56e17.

The first failed attempt sent nothing: the node refused the deploy transaction because cargo-stylus capped the fee at the base fee it had just read (35,854,000 < 36,026,000 wei). `scripts/deploy.sh` now caps at twice the current base fee.

Inputs used for AAPL on Robinhood Chain mainnet (4663):

| input | value |
|---|---|
| feed (Chainlink AAPL/USD) | `0x6B22A786bAa607d76728168703a39Ea9C99f2cD0` |
| pool (Uniswap v3 AAPL/USDG 0.05%) | `0xaae0d815ee56e4092a5e5c2911e676fea50b2d6d`, the primary and only pool: observation cardinality 1,801 = window + 1 when measured on 2026-09-23 and 3,000 in the deploy preflight on 2026-09-25, so it cannot be made unobservable. The 0.30% and 1% tiers are not configured: a standby is read only while the primary cannot be observed, and a primary keeping 1,801 observations cannot be made unobservable. |
| stock (AAPL) | `0xaF3D76f1834A1d425780943C99Ea8A608f8a93f9` |
| quote (read from the pool) | USDG `0x5fc5360D0400a0Fd4f2af552ADD042D716F1d168` |
| liveMaxAge | 21600 |
| twapWindow | 1800 |
| maxDeviationBps | 1000 (the band once the heartbeat has passed) |
| minLiquidity (median of three 10-minute harmonic means) | 50000000000000000 (5e16: the pool holds about 1.7e18 and clears 5e16 from -10.9% to +8.3% of the price, as far as the band reaches) |
| maxAnchorAge | 432000 (5 days) |
| heartbeat | 86400 (the feed's 24 h heartbeat) |
| quietBandBps | 100 (the band while the last print is younger than the heartbeat) |
| expected feed description | `Robinhood AAPL / USD` (measured with `pool_harmonic.py`; the oracle's own `description()` returns `Robinhood AAPL / USD (AfterHours)`) |
