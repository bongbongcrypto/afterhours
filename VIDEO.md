# Demo video

The narration and its Korean gloss live in one place, `video/script.json`
(9 slides, 33 lines, about three minutes). The submitted cut carries English
subtitles only; the Korean gloss is for the owner's review copy. This file only describes how the video is made and
what has to stay true when it is cut again.

## Pipeline

| step | tool | output |
|---|---|---|
| page captures | `video/capture_page.py` (headless Chrome over the DevTools protocol, stdlib): the published live page after its chain reads land, at the top, at the band gauge and at the stocks table | `video/out/page-1..3.png` |
| slides | `video/make_slides.py` (Refero "Linear" mood, headless Edge). Slide 6 draws the weekend from `status/10min.md` and looks every figure up in `status/reopen_check.txt` before drawing it; its strip shows `video/probe.txt`. The tour frames put the page captures under a one-line caption | `video/out/slide-1..8.png`, `video/out/tour-1..3.png` |
| narration | `video/make_narration.sh` (edge-tts `en-US-AndrewNeural`, run where edge-tts is installed: `REMOTE=<ssh host> bash video/make_narration.sh`) | `video/narration/SS-LL.mp3`, one per line |
| assembly | `video/make_video.py` (ffmpeg; English subtitles; `--gloss` adds the Korean line under each). A script slide with `shots` shows one tour frame per narration line | `video/afterhours-demo.mp4` (`.gloss.mp4` with `--gloss`) |

## Rules for any cut

- Slide 6 shows only real reads of the current instance
  `0x88b628472e595725178cc3e5e2ec70ada67f80f0` (DEPLOYMENTS.md): the server's
  10-minute record of the 09-26/27 weekend up to Monday's first print, and one
  `scripts/probe.py --oracle 0x88b628472e595725178cc3e5e2ec70ada67f80f0` read saved
  to `video/probe.txt`, taken while the feed is past `liveMaxAge` and the oracle
  answers in ONCHAIN_TWAP (the final cut: 2026-09-30 01:36:31 UTC, the feed 6.0 h
  old, ONCHAIN_TWAP $330.1716 inside the band). Without `probe.txt` the strip says
  the read is pending.
  The first instance (`0x69190621…0f65`) treated the feed as a price per share and is
  superseded; neither its probe output nor `status/10min-0x6919-superseded.md` is
  shown.
- The narration's weekend figures are the ones `reopen_check.py` printed, spoken
  in words: AfterHours' last answer 0.19% below Monday's first print, Friday's
  print 0.3% above it.
- Every number on screen matches the current README, DESIGN and CI results
  (73 unit and property tests, 90 e2e assertions, mainnet gas, 14 stocks that meet
  the bar and 18 that pass `initialize`, eleven independent review rounds and one
  self-review).
- The tour (slide 7) shows the published page, captured the day the cut is
  made, with the current instance in its header; it is never a mock-up.
- The Morpho figures on slide 4 are the `morpho_markets.py --borrows-since
  2026-09-16` output kept in `status/morpho-2026-09-30.txt`; the weekend
  flows on slide 3 are `weekend_swaps.py` for the four September weekends.
- A changed line gets the english-slop pass and a Korean gloss in `script.json`;
  its narration clip and the assembled video are rendered again.
