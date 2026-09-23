# Demo video

The narration and its Korean gloss live in one place, `video/script.json`
(8 slides, about 2:20). This file only describes how the video is made and
what has to be true before the final cut.

## Pipeline

| step | tool | output |
|---|---|---|
| slides | `video/make_slides.py` (Refero "Linear" mood, headless Edge) | `video/out/slide-*.png` |
| narration | `video/make_narration.sh` (edge-tts `en-US-AndrewNeural`, run where edge-tts is installed) | `video/narration/SS-LL.mp3`, one per line |
| assembly | `video/make_video.py` (ffmpeg; English line and Korean gloss in one subtitle event) | `video/afterhours-demo.mp4` |

## Before the final cut

- The AAPL instance is live on Robinhood Chain mainnet and listed in `DEPLOYMENTS.md`.
  Slide 7 and the narration line "deployed on Robinhood Chain mainnet" are only true then.
- Slide 6 is a real capture of `scripts/probe.py --watch` on a weekend (the feed silent,
  the oracle in ONCHAIN_TWAP). Until then it shows a labelled placeholder.
- Every number on screen matches the current README, DESIGN and CI results
  (tests, e2e assertions, gas, asset tiers).
