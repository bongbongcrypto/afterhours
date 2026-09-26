# Demo video

The narration and its Korean gloss live in one place, `video/script.json`
(8 slides, about 2:20). This file only describes how the video is made and
what has to be true before the final cut.

## Pipeline

| step | tool | output |
|---|---|---|
| slides | `video/make_slides.py` (Refero "Linear" mood, headless Edge); `--capture-page <url>` screenshots the live page for slide 6 | `video/out/slide-*.png` |
| narration | `video/make_narration.sh` (edge-tts `en-US-AndrewNeural`, run where edge-tts is installed) | `video/narration/SS-LL.mp3`, one per line |
| assembly | `video/make_video.py` (ffmpeg; English line and Korean gloss in one subtitle event) | `video/afterhours-demo.mp4` |

## Before the final cut

- Slide 6 only ever shows a real read. The AAPL instance has been live on Robinhood
  Chain mainnet since 2026-09-25 22:56 UTC (`DEPLOYMENTS.md`), so the final cut uses
  `scripts/probe.py --oracle 0x69190621e300cd2bc4cbb80777b517691ee80f65` output saved
  to `video/probe.txt`, read while the feed is silent and the oracle is in
  ONCHAIN_TWAP (the weekend of 09-26; `status/10min.md` has a read every 10 minutes).
  The live-page capture branch of `make_slides.py` was written for the days before
  the deployment and its caption says so, so it is not for the final cut. With
  neither, the slide says the capture is pending and shows no numbers.
- Slide 7's narration says the instance is deployed on Robinhood Chain mainnet.
  `video/script.json` does not say it yet: the line and its audio are added before
  the final cut.
- Every number on screen matches the current README, DESIGN and CI results
  (tests, e2e assertions, gas, asset tiers).
