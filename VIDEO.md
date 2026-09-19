# Demo video — script (target 2:30)

Recorded over the 2026-09-26/27 weekend so the feed is genuinely silent on camera.

| t | on screen | voice |
|---|---|---|
| 0:00 | Title card: AfterHours — a 24/7 price for tokenized stocks on Robinhood Chain | Robinhood Chain trades tokenized US stocks around the clock. Their Chainlink price feeds don't. |
| 0:10 | `feed_cadence.py` output: the 52.2 h and 76.2 h gaps highlighted | We measured it on mainnet. Every weekend the AAPL feed goes silent for 52 hours; 76 on a holiday weekend. |
| 0:25 | `weekend_swaps.py` table: 26,331 swaps / $4.17M, 44,338 / $5.14M | While it sleeps, the token keeps trading: four to five million dollars of AAPL alone, every weekend, at prices that move. |
| 0:40 | Blockscout: `PareMorphoOracle` source, `MIN_FEED_AGE = 5 days` and its comment | The one live lending market copes by accepting a five-day-old price. Its own code says why. |
| 0:55 | Diagram from DESIGN.md | AfterHours sits in front of the feed. Fresh feed: pass it through. Silent feed: the pool's 30-minute average, refused if the pool is thin, bounded to a circuit-breaker band around the last print. Corporate action pause: refuse. Same interface as Chainlink, plus Morpho's. |
| 1:20 | terminal: `probe.py --watch` live on Saturday — Chainlink line "printed Fri 19:5x, 40 h ago", AfterHours line `ONCHAIN_TWAP answer $…` ticking | This is now, Saturday. Chainlink's last print is forty hours old. AfterHours is answering from live trades, inside the band, and says which mode it is in. |
| 1:50 | Blockscout contract page (Stylus, verified) + `cast call state()` | It is a Rust contract on Arbitrum Stylus, deployed on Robinhood Chain mainnet, no owner, no upgrade. |
| 2:05 | GitHub: tests green, CI, DESIGN.md | Twenty-seven unit tests, tick math checked against 60-digit reference values, every number in the docs backed by a script you can run. |
| 2:20 | Closing card: what it unlocks (weekend liquidations at sane LTVs, 24/7 price-aware contracts) | Lending that can liquidate on Saturday. Perps and automation that run seven days. One answer to what a stock is worth while the exchange is closed. AfterHours. |

Production: slides are `video/slides/*.html` rendered to PNG; terminal segments are
real captures of `probe.py`; narration is recorded separately; assembled with ffmpeg.
