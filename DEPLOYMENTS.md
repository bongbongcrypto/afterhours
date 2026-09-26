# Deployments

| network | chain id | asset | address | status | initialize params | tx |
|---|---|---|---|---|---|---|
| Robinhood Chain mainnet | 4663 | AAPL | `0x88b628472e595725178cc3e5e2ec70ada67f80f0` | current (per-token units) | the inputs below | deploy `0x6929dee2e3c399cd60d2f38d646f6634e810c0b07b585fff531737974695d572`, activate `0x63a37a1c65d090c9d0365c8174a6fe94b56636d7c23cf60b5617b3b710eaf3db`, initialize `0x1597e10caf4687593e54d0b9aacb4d5d41e4ce1f5daceb9fdca70052a953a83c` |
| Robinhood Chain mainnet | 4663 | AAPL | `0x69190621e300cd2bc4cbb80777b517691ee80f65` | superseded 2026-09-27, do not integrate ([below](#superseded--do-not-integrate)) | the inputs below | deploy `0x6ee581c608ab8da6b27671e291024effd7b165007cff27ce377deb7e685a379e`, activate `0x0da9d4ebe2e2268f1484508bb752e983e40d68843d58426384ae14e3d47db73f`, initialize `0xbbec5c96b9f9b4540fb58c8414bf9b893481c0bfa2217cc33d52ad413725b6f3` |

## Current: `0x88b628472e595725178cc3e5e2ec70ada67f80f0`

Deployed 2026-09-26 22:50 UTC (block 73,453,764) by the manual `deploy` workflow from commit `3388ded`: the merge of the per-token unit fix (pull request #1, merge `3acbbfc`) plus a rustfmt commit, the commit CI (73 tests) and e2e (90 assertions) passed on. Built reproducibly in `offchainlabs/cargo-stylus-base:0.10.9` (wasm sha256 `b44e30c8d4090187ac1203915963f8c2288dbfa3582ae9e84de82e203f1a0325`, 38,202 bytes, 2 fragments). The workflow read every configured field back and found them equal to the inputs, and found `price()` = answer × 10^16 at block 73,453,987. Receipts (`eth_getTransactionReceipt`): all four status 1; gas used 63,624 (deploy), 6,180,776 (activate), 478,164 (initialize), 197,613 (Morpho `createMarket`).

Morpho Blue market priced by this instance: loan USDG, collateral AAPL, oracle this instance, AdaptiveCurveIrm `0x2BD3d5965B26B51814AC95127B2b80dD6CcC0fa1`, LLTV 62.5%, market id `0x3d9b0c04e374f7b50fa7a635393d2ecae23f45289e4e23f83793a6a611010918`, created in tx `0x6ac7a5f03bd021d0b3cdc0d28ba76a4b8f43a1840cb7372fb9c95091df8ba09f`. Nothing is supplied or borrowed in it (Morpho `market()` for this id, block 73,460,757).

First reads: the workflow's read-back at 22:50 UTC Saturday (feed silent since Friday 19:49 UTC) returned `ONCHAIN_TWAP`, answer $340.29441121 inside the band, median liquidity 6.68e17. The server recorder's first row for this instance (`status/10min.md`, 23:00 UTC): `ONCHAIN_TWAP` $340.3284, the feed's $341.4532 print 27.2 hours old. The superseded instance read $340.1359 in the same minute, about 0.057% lower: its per-share division by AAPL's multiplier (1.00057).

## Inputs

The same for both instances, AAPL on Robinhood Chain mainnet (4663):

| input | value |
|---|---|
| feed (Chainlink AAPL/USD) | `0x6B22A786bAa607d76728168703a39Ea9C99f2cD0` |
| pool (Uniswap v3 AAPL/USDG 0.05%) | `0xaae0d815ee56e4092a5e5c2911e676fea50b2d6d`, the primary and only pool: observation cardinality 1,801 = window + 1 when measured on 2026-09-23 and 3,000 in the deploy preflights on 2026-09-25 and 2026-09-26, so it cannot be made unobservable. The 0.30% and 1% tiers are not configured: a standby is read only while the primary cannot be observed, and a primary keeping 1,801 observations cannot be made unobservable. |
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

## Superseded — do not integrate

`0x69190621e300cd2bc4cbb80777b517691ee80f65`, superseded on 2026-09-27 by `0x88b628472e595725178cc3e5e2ec70ada67f80f0`. Reason: it treats Chainlink's answer as a price per share, while Chainlink's Robinhood feeds price one token of raw balance, so its `price()` counts AAPL's multiplier twice and its pool answers are divided by it (README "Units", REVIEWS.md 2026-09-27). It has no owner and no upgrade, so it stays on chain as it is; its Morpho market holds no supply.

Deployed 2026-09-25 22:56 UTC by the manual `deploy` workflow from source commit `38ec3f9`, built reproducibly in `offchainlabs/cargo-stylus-base:0.10.9` (wasm sha256 `66848ef80af973d90ce715568723021ada8e2647cf0bef2d460d95e498acac2b`). The workflow read every configured field back and found them equal to the inputs. Receipts: all four status 1; gas used 63,660 (deploy), 6,412,290 (activate), 489,663 (initialize), 197,601 (Morpho `createMarket`).

Its Morpho Blue market: loan USDG, collateral AAPL, LLTV 62.5%, market id `0x516829adb04c8351c09747755eed21f4514370e2df11a73c89054fef7de316bc`, created in tx `0xe7cc78e6bce947d8b1d95f5853d9ce0f11b30fdb2bc730fc69c2d085ebb65419`. Nothing is supplied or borrowed in it (Morpho `market()`, block 73,460,757).

First read after deployment (`scripts/probe.py`, 2026-09-25 22:57 UTC, a Friday after the US close): `LIVE_FEED`, answer $341.4532 = the feed's 19:49 UTC print; primary pool $341.0896 per share, median liquidity 8.56e17. A server read it every 10 minutes until 2026-09-26 23:00 UTC: `status/10min-0x6919-superseded.md`.

The first failed attempt sent nothing: the node refused the deploy transaction because cargo-stylus capped the fee at the base fee it had just read (35,854,000 < 36,026,000 wei). `scripts/deploy.sh` now caps at twice the current base fee.
