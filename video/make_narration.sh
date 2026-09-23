#!/usr/bin/env bash
# Synthesise the narration on a remote host with edge-tts, one mp3 per script line
# (video/script.json -> video/narration/SS-LL.mp3). Same voice as the earlier
# demo videos (en-US-AndrewNeural, -4%), so they sound like one shop. Nothing
# is installed locally; the script only ships text out and audio back.
#
#   REMOTE=<ssh host with edge-tts> bash video/make_narration.sh
set -euo pipefail
REMOTE="${REMOTE:?set REMOTE to an ssh host that has edge-tts installed}"
HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
OUT="$HERE/narration"
# Windows python cannot open /c/... paths; hand it a native path.
WHERE="$(cygpath -w "$HERE" 2>/dev/null || echo "$HERE")"

python - "$WHERE\script.json" > "$HERE/narration.lines.txt" <<'PY'
import json, sys
s = json.load(open(sys.argv[1], encoding="utf-8"))
for si, slide in enumerate(s["slides"], 1):
    for li, (en, _ko) in enumerate(slide["lines"], 1):
        print("%02d-%02d\t%s" % (si, li, en))
PY
voice=$(python -c "import json,sys;print(json.load(open(sys.argv[1],encoding='utf-8'))['voice'])" "$WHERE\script.json")
rate=$(python -c "import json,sys;print(json.load(open(sys.argv[1],encoding='utf-8'))['rate'])" "$WHERE\script.json")

echo "sending $(wc -l < "$HERE/narration.lines.txt") lines to $REMOTE ($voice $rate)"
scp -q "$HERE/narration.lines.txt" "$REMOTE:~/afterhours-narration.txt"
ssh "$REMOTE" VOICE="$voice" RATE="$rate" bash -s <<'REMOTE_SCRIPT'
set -euo pipefail
rm -rf ~/afterhours-narration && mkdir -p ~/afterhours-narration
n=0
while IFS=$'\t' read -r id line; do
  [ -z "$line" ] && continue
  n=$((n + 1))
  ~/.local/bin/edge-tts --voice "$VOICE" --rate="$RATE" --text "$line" \
    --write-media ~/afterhours-narration/"$id".mp3 >/dev/null 2>&1
done < ~/afterhours-narration.txt
echo "  synthesised $n lines"
REMOTE_SCRIPT

rm -rf "$OUT" && mkdir -p "$OUT"
scp -q "$REMOTE:~/afterhours-narration/*.mp3" "$OUT/"
bad=0
for f in "$OUT"/*.mp3; do
  size=$(wc -c < "$f")
  if [ "$size" -lt 2000 ]; then echo "  EMPTY: $f ($size bytes)"; bad=1; fi
done
[ "$bad" = 0 ] || { echo "some lines came back empty"; exit 1; }
echo "wrote $(ls "$OUT"/*.mp3 | wc -l) files to video/narration/"
