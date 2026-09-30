# -*- coding: utf-8 -*-
"""Assemble the demo video: slides + neural narration + burned-in subtitles.

Inputs
  video/out/slide-N.png        from make_slides.py (N counts the script's slides
                               that have no "shots")
  video/out/tour-N.png         from make_slides.py, for a script slide with
                               "shots": one image per line, cut at each line
  video/script.json            one source for narration lines and Korean gloss
  video/narration/SS-LL.mp3    from make_narration.sh (edge-tts on a remote host)

Each slide is held for the sum of its line durations plus gaps plus a tail.
Subtitles are one ASS event per line, timed to the synthesised audio's real
length, not to a slot, so a caption never outlives its sentence: English
(44px, white) in the submitted cut; with --gloss the Korean gloss (34px,
grey) goes under it, for the owner's review copy. ffmpeg only.

    python video/make_video.py            # video/afterhours-demo.mp4
    python video/make_video.py --gloss    # video/afterhours-demo.gloss.mp4
"""
import io
import json
import os
import shutil
import subprocess
import sys
from pathlib import Path

sys.stdout = io.TextIOWrapper(sys.stdout.buffer, encoding="utf-8", errors="replace")
HERE = Path(__file__).resolve().parent
OUT = HERE / "out"
NARR = HERE / "narration"


def find_ffmpeg():
    """ffmpeg from $FFMPEG, else from PATH."""
    exe = os.environ.get("FFMPEG") or shutil.which("ffmpeg")
    if not exe:
        sys.exit("ffmpeg not found: install it or set FFMPEG to its full path")
    return exe


GAP = 0.35      # seconds between lines
LEAD = 0.6      # silence before the first line of a slide
TAIL = 1.0      # hold after the last line
MAX_EN = 70     # characters per subtitle row before wrapping (44px on 1920)
MAX_KO = 40

ASS_HEADER = """[Script Info]
ScriptType: v4.00+
PlayResX: 1920
PlayResY: 1080
WrapStyle: 2

[V4+ Styles]
Format: Name, Fontname, Fontsize, PrimaryColour, SecondaryColour, OutlineColour, BackColour, Bold, Italic, Underline, StrikeOut, ScaleX, ScaleY, Spacing, Angle, BorderStyle, Outline, Shadow, Alignment, MarginL, MarginR, MarginV, Encoding
Style: En,Malgun Gothic,44,&H00FFFFFF,&H00FFFFFF,&H00000000,&H80000000,0,0,0,0,100,100,0,0,1,2,0,2,120,120,48,1
Style: Ko,Malgun Gothic,34,&H00A8ADB5,&H00A8ADB5,&H00000000,&H80000000,0,0,0,0,100,100,0,0,1,2,0,2,120,120,48,1

[Events]
Format: Layer, Start, End, Style, Name, MarginL, MarginR, MarginV, Effect, Text
"""


def probe_seconds(ffprobe, path):
    out = subprocess.run([ffprobe, "-v", "error", "-show_entries", "format=duration",
                          "-of", "default=nw=1:nk=1", str(path)],
                         capture_output=True, text=True, check=True).stdout.strip()
    return float(out)


def ass_time(t):
    h = int(t // 3600)
    m = int(t % 3600 // 60)
    s = t % 60
    return f"{h}:{m:02d}:{s:05.2f}"


def wrap(text, width):
    words = text.split(" ")
    rows, cur = [], ""
    for w in words:
        if cur and len(cur) + 1 + len(w) > width:
            rows.append(cur)
            cur = w
        else:
            cur = (cur + " " + w).strip()
    if cur:
        rows.append(cur)
    return "\\N".join(rows)


def esc(text):
    return text.replace("{", "(").replace("}", ")")


def main():
    gloss = "--gloss" in sys.argv
    ffmpeg = find_ffmpeg()
    sibling = Path(ffmpeg).with_name("ffprobe" + Path(ffmpeg).suffix)
    ffprobe = str(sibling) if sibling.exists() else (shutil.which("ffprobe") or "ffprobe")
    script = json.load(io.open(HERE / "script.json", encoding="utf-8"))
    segments = []
    total = 0.0
    plain = 0
    for si, slide in enumerate(script["slides"], 1):
        if slide.get("shots"):
            pngs = [OUT / s for s in slide["shots"]]
            if len(pngs) != len(slide["lines"]):
                sys.exit(f"slide {si}: {len(pngs)} shots for {len(slide['lines'])} lines")
        else:
            plain += 1
            pngs = [OUT / f"slide-{plain}.png"]
        for png in pngs:
            if not png.exists():
                sys.exit(f"missing {png}; run make_slides.py")
        # narration track for this slide: lead, lines separated by gaps, tail
        clips, events = [], []
        t = LEAD
        for li, (en, ko) in enumerate(slide["lines"], 1):
            mp3 = NARR / f"{si:02d}-{li:02d}.mp3"
            if not mp3.exists():
                sys.exit(f"missing {mp3}; run make_narration.sh")
            dur = probe_seconds(ffprobe, mp3)
            clips.append((mp3, t))
            events.append((t, t + dur, en, ko))
            t += dur + GAP
        length = t - GAP + TAIL
        # mix: each clip delayed to its start (adelay), summed with normalize=0
        inputs, filters = [], []
        for k, (mp3, start) in enumerate(clips):
            inputs += ["-i", str(mp3)]
            filters.append(f"[{k + len(pngs)}:a]adelay={int(start * 1000)}|{int(start * 1000)}[a{k}]")
        mix = "".join(f"[a{k}]" for k in range(len(clips)))
        filters.append(f"{mix}amix=inputs={len(clips)}:normalize=0:duration=longest,apad=whole_dur={length:.3f}[aout]")
        ass = OUT / f"sub-{si}.ass"
        with io.open(ass, "w", encoding="utf-8-sig", newline="\n") as f:
            f.write(ASS_HEADER)
            for start, end, en, ko in events:
                # One event, two styles: two events at the same time collide and
                # libass stacks the second above the first (Korean ended up on top).
                text = esc(wrap(en, MAX_EN))
                if gloss:
                    text += "\\N{\\rKo}" + esc(wrap(ko, MAX_KO))
                f.write(f"Dialogue: 0,{ass_time(start)},{ass_time(end)},En,,0,0,0,,{text}\n")
        ass_arg = str(ass).replace("\\", "/").replace(":", "\\:")
        seg = OUT / f"seg-{si}.mp4"
        # video: one still, or one still per line, each held from its line's start to the next
        cuts = [0.0] + [start for start, _, _, _ in events[1:]] + [length]
        vin, vlab = [], []
        for k, png in enumerate(pngs):
            hold = (cuts[k + 1] - cuts[k]) if len(pngs) > 1 else length
            vin += ["-loop", "1", "-framerate", "30", "-t", f"{hold:.3f}", "-i", str(png)]
            vlab.append(f"[{k}:v]")
        filters.append("".join(vlab) + f"concat=n={len(pngs)}:v=1:a=0,ass='{ass_arg}'[vout]")
        cmd = [ffmpeg, "-y", "-loglevel", "error"] + vin + inputs + [
            "-filter_complex", ";".join(filters),
            "-map", "[vout]", "-map", "[aout]", "-t", f"{length:.3f}",
            "-c:v", "libx264", "-pix_fmt", "yuv420p", "-preset", "medium", "-crf", "18",
            "-c:a", "aac", "-b:a", "160k", "-ar", "48000", str(seg)]
        subprocess.run(cmd, check=True)
        segments.append(seg)
        total += length
        print(f"  slide {si}: {len(clips)} lines, {length:5.1f}s")
    lst = OUT / "segments.txt"
    lst.write_text("".join(f"file '{s.name}'\n" for s in segments), encoding="utf-8")
    final = HERE / ("afterhours-demo.gloss.mp4" if gloss else "afterhours-demo.mp4")
    subprocess.run([ffmpeg, "-y", "-loglevel", "error", "-f", "concat", "-safe", "0",
                    "-i", str(lst), "-c", "copy", str(final)], check=True, cwd=str(OUT))
    print(f"wrote {final} ({total:.0f} s, {final.stat().st_size // 1024} KB)")


if __name__ == "__main__":
    main()
