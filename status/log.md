# AfterHours status log (hourly, UTC)

Units: the AAPL rows up to 2026-09-26 19:51 come from the first instance (`0x69190621e300cd2bc4cbb80777b517691ee80f65`), which treated the feed as a price per share and is superseded (REVIEWS.md, 2026-09-27). Its LIVE_FEED answers are the feed's own print and are unchanged under the corrected model; its ONCHAIN_TWAP answers are the pool's price divided by the token's uiMultiplier (1.00057 for AAPL), 0.057% under the per-token price the current contract gives. The rows after that come from the current instance (`0x88b628472e595725178cc3e5e2ec70ada67f80f0`, deployed 2026-09-26 22:50 UTC); the status job no longer probes the first one.

| time | asset | session | AfterHours | Chainlink | feed age | pool | liquidity |
|---|---|---|---|---|---|---|---|
| 2026-09-26 01:33 | AAPL | LIVE_FEED | 341.4532 | 341.4532 | 5.7 |  |  |
| 2026-09-26 07:41 | AAPL | ONCHAIN_TWAP | 340.7146 | 341.4532 | 11.9 | 0xaae0d815 | 8.64e+17 |
| 2026-09-26 13:05 | AAPL | ONCHAIN_TWAP | 340.2039 | 341.4532 | 17.3 | 0xaae0d815 | 6.86e+17 |
| 2026-09-26 17:10 | AAPL | ONCHAIN_TWAP | 340.3400 | 341.4532 | 21.4 | 0xaae0d815 | 1.06e+18 |
| 2026-09-26 19:51 | AAPL | ONCHAIN_TWAP | 340.2379 | 341.4532 | 24.0 | 0xaae0d815 | 8.38e+17 |
