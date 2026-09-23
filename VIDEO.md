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

- Slide 6 only ever shows a real read. After deployment: `scripts/probe.py --watch`
  output saved to `video/probe.txt`, ideally on the weekend of 09-26 (the feed silent,
  the oracle in ONCHAIN_TWAP). Before deployment: a dated capture of the live page,
  which runs the contract's rules on mainnet data and says it is not deployed. With
  neither, the slide says the capture is pending and shows no numbers.
- Once the AAPL instance is live and listed in `DEPLOYMENTS.md`, slide 7's narration
  says so again ("deployed on Robinhood Chain mainnet"); until then it does not.
- Every number on screen matches the current README, DESIGN and CI results
  (tests, e2e assertions, gas, asset tiers).
