# -*- coding: utf-8 -*-
"""Assemble the demo video: slides (video/out/slide-N.png) + narration.

Narration: Windows built-in TTS (System.Speech, en-US voice) as the draft
track; replace video/narration/N.wav with recorded audio for the final cut.
Video: ffmpeg, one still per slide held for the narration length + 0.8 s,
1920x1080, H.264 + AAC. Run make_slides.py first.

    python video/make_video.py            # tts (if missing) + assemble
    python video/make_video.py --tts      # regenerate narration wavs
"""
import io
import os
import subprocess
import sys
import wave
from pathlib import Path

sys.stdout = io.TextIOWrapper(sys.stdout.buffer, encoding="utf-8", errors="replace")
HERE = Path(__file__).resolve().parent
OUT = HERE / "out"
NARR = HERE / "narration"
MASTER_LIB = Path(".")  # ffmpeg locator
sys.path.insert(0, str(MASTER_LIB))
from _lib.ffmpeg import find_ffmpeg  # noqa: E402

VOICE = "Microsoft Zira Desktop"

# Narration per slide (VIDEO.md). Keep sentences short: TTS pacing.
SCRIPT = [
    "Robinhood Chain trades tokenized US stocks around the clock. Their Chainlink price feeds don't.",
    "We measured it on mainnet. Every weekend the Apple feed goes silent for fifty-two hours; seventy-six on a holiday weekend. Weeknight gaps are not closures; the price just did not move half a percent.",
    "While it sleeps, the token keeps trading. Four to five million dollars of Apple alone, every weekend, in tens of thousands of swaps, at prices that move. And no contract can read that price.",
    "The one live lending market copes by accepting a five-day-old price. Its own verified code says why. The pool's time-weighted price is already a trusted component there; nobody points it at the stock.",
    "AfterHours sits in front of the feed. Fresh feed: pass it through. Silent feed: the pool's thirty-minute average, refused if the pool is thin over the window, bounded to a circuit-breaker band around the last print. Corporate action pause: refuse. A print older than five days: refuse, the feed is gone, not closed. Same interface as Chainlink, plus Morpho's.",
    "This is Saturday. Chainlink's last print is thirty-six hours old. AfterHours is answering from live trades, inside the band, and says which mode it is in.",
    "It is a Rust contract on Arbitrum Stylus, deployed on Robinhood Chain mainnet, no owner, no upgrade. Thirty-six unit tests, tick math checked against sixty-digit reference values, an adversarial review folded in, and every number in the docs backed by a script you can run.",
    "Lending that can liquidate on Saturday. Automation that runs seven days. One answer to what a stock is worth while the exchange is closed. AfterHours.",
]


def tts(text, wav):
    ps = (
        "Add-Type -AssemblyName System.Speech; "
        "$s = New-Object System.Speech.Synthesis.SpeechSynthesizer; "
        f"$s.SelectVoice('{VOICE}'); $s.Rate = 0; "
        f"$s.SetOutputToWaveFile('{wav}'); "
        "$s.Speak([Console]::In.ReadToEnd()); $s.Dispose()"
    )
    subprocess.run(["powershell", "-NoProfile", "-Command", ps], input=text.encode("utf-8"),
                   check=True, capture_output=True, timeout=120)


def wav_seconds(path):
    with wave.open(str(path), "rb") as w:
        return w.getnframes() / float(w.getframerate())


def main():
    ffmpeg = find_ffmpeg()
    NARR.mkdir(exist_ok=True)
    regen = "--tts" in sys.argv
    segments = []
    for i, text in enumerate(SCRIPT, 1):
        png = OUT / f"slide-{i}.png"
        if not png.exists():
            sys.exit(f"missing {png}; run make_slides.py")
        wav = NARR / f"{i}.wav"
        if regen or not wav.exists():
            tts(text, wav)
        dur = wav_seconds(wav) + 0.8
        seg = OUT / f"seg-{i}.mp4"
        subprocess.run([ffmpeg, "-y", "-loglevel", "error", "-loop", "1", "-framerate", "30",
                        "-i", str(png), "-i", str(wav), "-t", f"{dur:.2f}",
                        "-c:v", "libx264", "-pix_fmt", "yuv420p", "-preset", "medium", "-crf", "18",
                        "-c:a", "aac", "-b:a", "160k", "-ar", "48000", "-shortest", str(seg)],
                       check=True)
        segments.append(seg)
        print(f"  slide {i}: {dur:5.1f}s")
    lst = OUT / "segments.txt"
    lst.write_text("".join(f"file '{s.name}'\n" for s in segments), encoding="utf-8")
    final = HERE / "afterhours-demo.mp4"
    subprocess.run([ffmpeg, "-y", "-loglevel", "error", "-f", "concat", "-safe", "0",
                    "-i", str(lst), "-c", "copy", str(final)], check=True, cwd=str(OUT))
    total = sum(wav_seconds(NARR / f"{i}.wav") + 0.8 for i in range(1, len(SCRIPT) + 1))
    print(f"wrote {final} ({total:.0f} s, {final.stat().st_size // 1024} KB)")


if __name__ == "__main__":
    main()
