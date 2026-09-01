#!/usr/bin/env bash
# Turn a directory of `capture` frames into an annotated GIF.
#
# `examples/capture.rs` writes frame0000.png … ; this is the other half of that pipeline, kept as a
# script rather than as remembered ffmpeg flags so that two GIFs taken a week apart are encoded
# identically and therefore actually comparable. A palette or dither change between runs would show up
# as a visual difference that no code change caused.
#
#   tools/gif.sh <frames-dir> <out.gif> "<caption>"
#
# The caption is burned in with ImageMagick because this ffmpeg build has no `drawtext` filter.
#
# LEGEND=audit (default) burns in the colour key `capture.rs --tint audit` applies, so the two must
# be changed together. LEGEND=none omits it, which is what the demo-tinted clips want: a key naming
# colours that are not in the picture is worse than no key at all.
set -euo pipefail

FRAMES=${1:?usage: gif.sh <frames-dir> <out.gif> "<caption>"}
OUT=${2:?usage: gif.sh <frames-dir> <out.gif> "<caption>"}
CAPTION=${3:-}

LEGEND=${LEGEND:-audit}
# **No default font path, deliberately.** These were `/System/Library/Fonts/Supplemental/Arial.ttf`,
# which exists on exactly one of the machines this repo is built on and fails on the others — and it
# fails in `magick`, after ffmpeg has already spent the entire two-pass encode. Name both files at the
# call site; `docs/DEMOS.md`'s recipe carries the paths for the host it was last run on. Arial and
# Liberation Sans are metrically compatible, so substituting one for the other does not move the
# caption or the legend.
FONT=${FONT:?set FONT to a regular-weight .ttf — see docs/DEMOS.md}
BOLD=${BOLD:?set BOLD to a bold-weight .ttf — see docs/DEMOS.md}
WIDTH=${WIDTH:-640}
FPS=${FPS:-30}
# **Keep every Nth frame.** A GIF stores whole frames, so its size is very nearly linear in frame
# count — and a clip that has to run long enough to *show* something (a wound bleeding until it clots
# takes six seconds) is otherwise several times the size of the short ones for no extra information.
# `STRIDE=2` with `FPS` left alone halves the file and plays at half speed; pair it with a doubled
# `FPS` to keep real-time speed, which is what `docs/DEMOS.md`'s carnage recipe does.
STRIDE=${STRIDE:-1}

[ -d "$FRAMES" ] || { echo "gif.sh: no such directory: $FRAMES" >&2; exit 1; }
count=$(find "$FRAMES" -name 'frame*.png' | wc -l | tr -d ' ')
[ "$count" -gt 0 ] || { echo "gif.sh: no frame*.png in $FRAMES" >&2; exit 1; }
for f in "$FONT" "$BOLD"; do
    [ -f "$f" ] || { echo "gif.sh: no such font file: $f" >&2; exit 1; }
done

tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT

# Two-pass palette: one palette for the whole clip, so colours do not shift frame to frame.
# `select` runs before `scale` so the dropped frames are never resized, and `setpts` re-times what is
# left — without it ffmpeg keeps the original timestamps and the gap shows as a stutter.
decimate=""
if [ "$STRIDE" -gt 1 ]; then
    decimate="select='not(mod(n\,${STRIDE}))',setpts=N/FRAME_RATE/TB,"
fi
ffmpeg -y -loglevel error -framerate "$FPS" -i "$FRAMES/frame%04d.png" \
  -vf "${decimate}scale=${WIDTH}:-1:flags=lanczos,split[a][b];[a]palettegen=max_colors=128[p];[b][p]paletteuse=dither=bayer:bayer_scale=3" \
  "$tmp/raw.gif"

# Caption top-left, and the audit legend bottom-left when it describes what is on screen.
case "$LEGEND" in
  audit)
    magick "$tmp/raw.gif" -coalesce \
      -font "$BOLD" -pointsize 15 -fill white \
      -gravity NorthWest -annotate +12+10 "$CAPTION" \
      -font "$FONT" -pointsize 13 \
      -gravity SouthWest \
      -fill '#3d9e5c' -annotate +12+56 '●  watertight + manifold' \
      -fill '#e6a61f' -annotate +12+36 '●  watertight, non-manifold' \
      -fill '#d92eb8' -annotate +12+16 '●  open cut edges' \
      -layers optimize "$OUT"
    ;;
  none)
    magick "$tmp/raw.gif" -coalesce \
      -font "$BOLD" -pointsize 15 -fill white \
      -gravity NorthWest -annotate +12+10 "$CAPTION" \
      -layers optimize "$OUT"
    ;;
  *)
    echo "gif.sh: unknown LEGEND=$LEGEND (use 'audit' or 'none')" >&2; exit 1
    ;;
esac

echo "gif.sh: $count frames -> $OUT ($(du -h "$OUT" | cut -f1))"
