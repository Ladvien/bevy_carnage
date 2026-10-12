#!/usr/bin/env bash
# Render every visual surface of the crate in one command, so a QA round is one invocation.
#
#   FONT=/System/Library/Fonts/Supplemental/Arial.ttf tools/qa.sh <out-dir> [name ...]
#
# What it does, in order:
#   1. builds every example (`cargo build --release -p bevy_carnage --examples`);
#   2. runs the headless recorders, each into `<out>/<name>/frameNNNN.png`, log in `<out>/<name>.log`;
#   3. runs the terminal examples, logs in `<out>/<name>.log` (`cross_section_strip` also writes a PNG);
#   4. runs each windowed example under `examples/common/autopilot.rs`, which makes the example press
#      its own keys and screenshot itself (`CARNAGE_AUTOPILOT`, `CARNAGE_AUTOPILOT_OUT`);
#   5. builds contact sheets into `<out>/sheets/`;
#   6. writes `<out>/warnings.txt`, `<out>/digests.txt` and `<out>/missing.txt`.
#
# **Names.** With no `name`, everything runs. With one or more, only those run — so a later round can
# re-render just the examples a fix touched, into the same directory. `capture` is accepted as a name
# for the pair `capture_demo` + `capture_audit`. A name that matches nothing is an error, checked
# before anything is built. The three summary files are rebuilt from the logs in `<out>`, so after a
# partial run they cover every log there; `missing.txt` covers only what this run executed.
#
# **No screen is needed for any of it.** A scripted windowed example opens no window: the autopilot
# swaps winit for a plain loop and renders every camera into an off-screen image, at a fixed 1/60 s
# per frame (see its module header). That is what used to make whole runs come back black when a
# window was covered or the screen locked. A round therefore runs on a build host with its lid shut.
#
# **Phases.** `QA_PHASES` picks from `build recorders terminal windowed sheets` (default: all of
# them), `QA_BIN` points at a directory of prebuilt example binaries, and `QA_KEEP_MISSING=1`
# appends to an existing `missing.txt` instead of truncating it. Sheets need ImageMagick, which the
# build host may not have: run `build recorders terminal windowed` there, `rsync` the out-dir, and
# run `QA_PHASES=sheets QA_KEEP_MISSING=1 tools/qa.sh <out-dir>` where `magick` is.
#
# `QA_DRY=1` prints every command instead of running it, which is how this script is tested.
#
# macOS ships bash 3.2: no associative arrays, no `mapfile`, no `${var,,}`. Hence the `case`
# functions and the space-delimited name lists below.
set -euo pipefail

OUT=${1:?usage: qa.sh <out-dir> [name ...]}
shift

# **No default font path, deliberately** — same reasoning as `gif.sh`: a path that exists on one
# host fails on the others, and here it would fail in `magick` only after every example has run.
FONT=${FONT:?set FONT to a regular-weight .ttf — see docs/DEMOS.md}
[ -f "$FONT" ] || { echo "qa.sh: no such font file: $FONT" >&2; exit 1; }

# The out-dir may be relative to where the caller stood; everything below runs from the crate dir.
case "$OUT" in
  /*) ;;
  *) OUT=$PWD/$OUT ;;
esac
cd "$(dirname "$0")/.."

DRY=0
[ "${QA_DRY:-}" = 1 ] && DRY=1

PHASES=${QA_PHASES:-"build recorders terminal windowed sheets"}
for p in $PHASES; do
  case "$p" in
    build | recorders | terminal | windowed | sheets) ;;
    *) echo "qa.sh: unknown QA_PHASES entry '$p' (build recorders terminal windowed sheets)" >&2; exit 1 ;;
  esac
done
phase() {
  case " $PHASES " in *" $1 "*) return 0 ;; esac
  return 1
}

say() { printf '%s\n' "$*"; }
die() { echo "qa.sh: $*" >&2; exit 1; }

# --- the roster -----------------------------------------------------------------------------------

RECORDERS="capture_demo capture_audit capture_sever capture_holes capture_carnage capture_ribbons capture_pooling capture_entrails capture_flesh capture_preset capture_gore capture_family"
TERMINAL="fracture_cube canvas_digest dry_timeline flay_digest fracture_modes_bar laceration_curve rod_determinism stain_sweep cross_section_strip"
WINDOWED="bullet_holes carnage carnage_web cross_section drying entrails explode fault_modes flaymap_peel flesh fracture_modes fragment_energy gore_tier laceration_open pattern_classes pooling preset rheology ribbons sever spill stain_morphology viscera_spill wetmap_paint"

# The example a recorder name runs. `capture` is run twice with different flags; every other recorder
# is its own example.
rec_example() {
  case "$1" in
    capture_demo|capture_audit) echo capture ;;
    *) echo "$1" ;;
  esac
}

# Flags beyond `--out`. Same as `docs/DEMOS.md`'s "Regenerating these" recipe.
rec_args() {
  case "$1" in
    capture_demo) echo "--tint demo --width 720 --height 512 --soften 0.5" ;;
    capture_audit) echo "--tint audit --width 720 --height 512 --soften 0.25" ;;
    *) echo ;;
  esac
}

# **The one place the key lists live.** Space-separated `KeyCode` variant names; `Name*n` holds the
# key for `n` frames. An empty list means an auto-play demo that needs no input. Each list was
# checked against the example's own `just_pressed` / `pressed` calls; a polled `pressed()` key
# (arrows, `[`/`]` on rheology, `G` on viscera_spill, `F` on gore_tier) needs a hold, and a hold must
# stay under the 130-frame spacing so it ends before the next press. Blows are cumulative until `R`,
# so the strike demos reset between blows or the later ones report "nothing left to break here".
# **A repeat shot needs the aim moved first**: an identical bore reaches no proxy cell and changes
# nothing, so `bullet_holes` and `pooling` step the aim between shots. The aim is not reset by `R`;
# at the examples' 1.1 m/s, `ArrowRight*17` from `sever`'s (0, 0.25) lands on the right arm and
# `ArrowLeft*10 ArrowDown*47` after it on the right leg.
keys_for() {
  case "$1" in
    bullet_holes) echo "Space KeyK ArrowLeft*5 Space BracketRight ArrowRight*10 Space KeyJ ArrowDown*5 Space KeyF ArrowUp*10 Space KeyR" ;;
    carnage) echo "Digit6 KeyR Digit1 KeyR Digit2 KeyR Digit3 KeyR Digit4 KeyR Digit5 KeyR" ;;
    carnage_web) echo "Digit1 Digit2 Digit3 Digit4 Digit5 KeyH KeyR" ;;
    cross_section) echo ;;
    drying) echo "ArrowRight*120 KeyH Space ArrowRight*110 KeyH" ;;
    entrails) echo "Digit1 KeyR Digit2 KeyR Digit3 KeyR Digit4 KeyR Digit5 KeyR" ;;
    explode) echo "Space Space" ;;
    fault_modes) echo "Digit1 Digit2 Digit3 Digit4 KeyT BracketRight KeyR" ;;
    flaymap_peel) echo ;;
    flesh) echo ;;
    fracture_modes) echo "Space Space Space KeyR" ;;
    fragment_energy) echo "BracketRight BracketRight Period KeyT KeyT KeyH" ;;
    gore_tier) echo "Space Digit1 Space Digit2 Space Digit3 Space Digit4 Space KeyA KeyF*90" ;;
    laceration_open) echo "Space" ;;
    pattern_classes) echo "Digit1 Digit2 Digit3 Digit4 Digit5 Digit5 Digit6 KeyC" ;;
    pooling) echo "Space ArrowLeft*4 Space ArrowRight*8 Space ArrowDown*4 Space KeyR" ;;
    preset) echo "Digit1 Digit2 Digit3 Digit4 Digit1 Digit1 KeyR" ;;
    rheology) echo "Space ArrowRight*60 Space BracketRight*100 Space BracketRight*50 Space" ;;
    ribbons) echo "Digit1 KeyR Digit4 KeyR Digit2 KeyR Digit3 KeyR Digit5 KeyR" ;;
    sever) echo "Digit1 KeyR Digit2 KeyR Digit3 KeyR Digit4 KeyR Digit5 KeyR KeyG KeyT KeyR ArrowRight*17 Digit3 KeyR ArrowLeft*10 ArrowDown*47 Digit3 KeyR" ;;
    spill) echo "Space Space KeyR" ;;
    stain_morphology) echo "ArrowRight ArrowUp KeyR KeyR KeyB" ;;
    viscera_spill) echo "Space KeyG*90 KeyT KeyR" ;;
    wetmap_paint) echo "Space KeyG Space KeyD" ;;
    *) die "no key list for windowed example '$1'" ;;
  esac
}

# Build the `CARNAGE_AUTOPILOT` string for a key list (grammar: `examples/common/autopilot.rs`).
#   With keys: `30:shot`, then key i (1-based) is pressed at f = 40 + 130*(i-1), with shots at f+30
#              and f+120, and `exit` at the last press + 150.
#   No keys:   shots at 30 120 240 360 480 600 720, `exit` at 760.
# Tokens come out in frame order. `Name*n` becomes `hold=Name*n`; any other `Name` is `key=Name`.
script_for() {
  local s="30:shot" i=0 f k last=0
  if [ $# -eq 0 ]; then
    for f in 120 240 360 480 600 720; do
      s="$s $f:shot"
    done
    echo "$s 760:exit"
    return 0
  fi
  for k in "$@"; do
    f=$((40 + 130 * i))
    case "$k" in
      *\**) s="$s $f:hold=$k" ;;
      *) s="$s $f:key=$k" ;;
    esac
    s="$s $((f + 30)):shot $((f + 120)):shot"
    last=$f
    i=$((i + 1))
  done
  echo "$s $((last + 150)):exit"
}

# --- name selection -------------------------------------------------------------------------------

WANT=" "
for a in "$@"; do
  case " $RECORDERS $TERMINAL $WINDOWED capture " in
    *" $a "*) WANT="$WANT$a " ;;
    *) die "unknown name '$a' — recorders: $RECORDERS; terminal: $TERMINAL; windowed: $WINDOWED" ;;
  esac
done

selected() {
  [ "$WANT" = " " ] && return 0
  case "$WANT" in *" $1 "*) return 0 ;; esac
  case "$1" in
    capture_demo|capture_audit) case "$WANT" in *" capture "*) return 0 ;; esac ;;
  esac
  return 1
}

# --- running --------------------------------------------------------------------------------------

# Run the built binaries directly rather than through `cargo run`: killing `cargo` on a timeout would
# leave the example it spawned running. `cargo run` also sets `CARGO_MANIFEST_DIR`, which Bevy's
# asset root reads at runtime, so set it here to the crate dir (this script's cwd).
if [ -n "${QA_BIN:-}" ]; then
  BIN=$QA_BIN
else
  TARGET=${CARGO_TARGET_DIR:-}
  if [ -z "$TARGET" ]; then
    TARGET=$(cargo metadata --format-version 1 --no-deps 2>/dev/null \
      | sed -n 's/.*"target_directory":"\([^"]*\)".*/\1/p') || true
  fi
  [ -n "$TARGET" ] || die "cannot find the cargo target directory; set CARGO_TARGET_DIR or QA_BIN"
  BIN=$TARGET/release/examples
fi
export CARGO_MANIFEST_DIR=$PWD

# The `required-features` examples (`vfx`: carnage/pooling/ribbons and their recorders; `flesh`:
# capture_flesh/capture_preset/preset/carnage_web/flesh) are all in the crate's `default` features, so a
# plain `--examples` builds every one. Cargo silently skips an example whose features are unmet, so
# `need_bin` below refuses to continue rather than let one vanish from the round.
BUILD="cargo build --release -p bevy_carnage --examples"

TIMEOUT=${QA_TIMEOUT:-180}

# `timeout` is not on a stock macOS, so this is a watchdog in plain bash. It signals the child
# directly, which is why the binary is run directly. Exit status 124 means it timed out, as GNU
# `timeout` does. Before killing a run that outlived its limit, macOS's `sample` records where every
# thread was stuck into `<out>/<name>.hang.txt`: a hang that is gone on the next run can only be
# diagnosed from the run that had it.
run_limited() {
  local secs=$1 flag finished pid wd rc
  shift
  flag=$(mktemp)
  finished="$flag.finished"
  "$@" &
  pid=$!
  (
    sleep "$secs"
    # **The run may have ended while this slept.** To stop this subshell the parent kills its
    # `sleep` (bash would not act on the subshell's own TERM until that child returned), and bash
    # then runs the next line before the TERM lands. Without this check that next line reported a
    # timeout for a run that had already exited, and the subshell died before sampling anything.
    # That race was every "exit hang" `AG-045` and `AG-056` chased: a run that finished in seconds,
    # logged as hitting the 180 s timeout, with no hang file.
    [ -e "$finished" ] && exit 0
    echo timeout >"$flag"
    # The process state first, then the sample, and sample's own output too, so a real hang leaves
    # its reason in the file even if the process refuses to be sampled.
    if command -v sample >/dev/null 2>&1 && [ -n "${HANG_FILE:-}" ]; then
      { ps -o pid,stat,etime,command -p "$pid"; sample "$pid" 2 -file "$HANG_FILE.sample"; } >"$HANG_FILE" 2>&1 || true
    fi
    kill -TERM "$pid" 2>/dev/null || true
    sleep 5
    kill -KILL "$pid" 2>/dev/null || true
  ) >/dev/null 2>&1 &
  wd=$!
  rc=0
  wait "$pid" || rc=$?
  : >"$finished"
  pkill -P "$wd" 2>/dev/null || true
  kill "$wd" 2>/dev/null || true
  wait "$wd" 2>/dev/null || true
  if [ -s "$flag" ]; then rc=124; fi
  rm -f "$flag" "$finished"
  return "$rc"
}

need_bin() {
  [ -x "$BIN/$1" ] || die "no built binary $BIN/$1 — the build step skipped it (required-features?)"
}

MISSING=$OUT/missing.txt
FAILED=0

# run_logged <name> <timeout-secs|0> <command ...>: tee everything to <out>/<name>.log and, on a
# non-zero exit, say so in the log and in missing.txt without aborting the round.
run_logged() {
  local name=$1 secs=$2 rc=0
  shift 2
  set +e
  if [ "$secs" -gt 0 ]; then
    HANG_FILE="$OUT/$name.hang.txt" run_limited "$secs" "$@" 2>&1 | tee "$OUT/$name.log"
  else
    "$@" 2>&1 | tee "$OUT/$name.log"
  fi
  rc=${PIPESTATUS[0]}
  set -e
  if [ "$rc" -ne 0 ]; then
    FAILED=$((FAILED + 1))
    if [ "$rc" -eq 124 ]; then
      echo "qa.sh: $name hit the ${secs}s timeout" | tee -a "$OUT/$name.log" >>"$MISSING"
    else
      echo "qa.sh: $name exited with status $rc" | tee -a "$OUT/$name.log" >>"$MISSING"
    fi
  fi
}

# --- contact sheets -------------------------------------------------------------------------------

# Recorders: every 10th frame, 12 frames per sheet.
sheets_recorder() {
  local name=$1 dir=$OUT/$1 frames i n=0 page=1 sheet
  phase sheets || return 0
  local picked=()
  if [ "$DRY" -eq 1 ]; then
    say "+ magick montage -font \"$FONT\" -label '%t' <every 10th of $dir/frame*.png, 12 per sheet> -tile 4x3 -geometry 320x240+4+4 $OUT/sheets/${name}_NN.png"
    return 0
  fi
  frames=("$dir"/frame*.png)
  [ -e "${frames[0]}" ] || { echo "qa.sh: no frames in $dir; no sheet for $name" >>"$MISSING"; return 0; }
  i=0
  while [ "$i" -lt "${#frames[@]}" ]; do
    picked+=("${frames[$i]}")
    n=$((n + 1))
    if [ "$n" -eq 12 ]; then
      sheet=$(printf '%s/sheets/%s_%02d.png' "$OUT" "$name" "$page")
      magick montage -font "$FONT" -label '%t' "${picked[@]}" -tile 4x3 -geometry 320x240+4+4 "$sheet"
      picked=()
      n=0
      page=$((page + 1))
    fi
    i=$((i + 10))
  done
  if [ "$n" -gt 0 ]; then
    sheet=$(printf '%s/sheets/%s_%02d.png' "$OUT" "$name" "$page")
    magick montage -font "$FONT" -label '%t' "${picked[@]}" -tile 4x3 -geometry 320x240+4+4 "$sheet"
  fi
}

# Windowed: every screenshot, 6 per sheet (`<name>.png` when one sheet is enough).
sheets_windowed() {
  local name=$1 dir=$OUT/$1 frames i n=0 page=1 sheet total pages
  local picked=()
  phase sheets || return 0
  if [ "$DRY" -eq 1 ]; then
    say "+ magick montage -font \"$FONT\" -label '%t' $dir/f*.png -tile 3x2 -geometry 480x+4+4 $OUT/sheets/$name.png   # ${name}_NN.png past 6 shots"
    return 0
  fi
  frames=("$dir"/f*.png)
  [ -e "${frames[0]}" ] || return 0 # already reported in missing.txt by the shot count
  total=${#frames[@]}
  pages=$(((total + 5) / 6))
  for i in "${frames[@]}"; do
    picked+=("$i")
    n=$((n + 1))
    if [ "$n" -eq 6 ]; then
      if [ "$pages" -eq 1 ]; then sheet=$OUT/sheets/$name.png; else sheet=$(printf '%s/sheets/%s_%02d.png' "$OUT" "$name" "$page"); fi
      magick montage -font "$FONT" -label '%t' "${picked[@]}" -tile 3x2 -geometry 480x+4+4 "$sheet"
      picked=()
      n=0
      page=$((page + 1))
    fi
  done
  if [ "$n" -gt 0 ]; then
    if [ "$pages" -eq 1 ]; then sheet=$OUT/sheets/$name.png; else sheet=$(printf '%s/sheets/%s_%02d.png' "$OUT" "$name" "$page"); fi
    magick montage -font "$FONT" -label '%t' "${picked[@]}" -tile 3x2 -geometry 480x+4+4 "$sheet"
  fi
}

# --- go -------------------------------------------------------------------------------------------

if [ "$DRY" -eq 1 ]; then
  say "# QA_DRY=1: printing commands, running none. out=$OUT cwd=$PWD phases=$PHASES"
  say "+ mkdir -p $OUT $OUT/sheets"
  if [ -z "${QA_KEEP_MISSING:-}" ]; then say "+ : > $MISSING"; fi
  if phase build; then say "+ $BUILD"; fi
else
  mkdir -p "$OUT" "$OUT/sheets"
  if [ -z "${QA_KEEP_MISSING:-}" ]; then : >"$MISSING"; fi
  if phase sheets; then
    command -v magick >/dev/null || die "ImageMagick (magick) is required for the sheets phase"
  fi
  if phase build; then $BUILD; fi
fi

# 2. Recorders.
for name in $RECORDERS; do
  selected "$name" || continue
  ex=$(rec_example "$name")
  args=$(rec_args "$name")
  if [ "$DRY" -eq 1 ]; then
    if phase recorders; then say "+ $BIN/$ex --out $OUT/$name $args 2>&1 | tee $OUT/$name.log"; fi
    sheets_recorder "$name"
    continue
  fi
  if phase recorders; then
    need_bin "$ex"
    rm -f "$OUT/$name"/frame*.png
    # Unquoted on purpose: `$args` is a flag list with no glob characters.
    # shellcheck disable=SC2086
    run_logged "$name" 0 "$BIN/$ex" --out "$OUT/$name" $args
  fi
  sheets_recorder "$name"
done

# 3. Terminal examples.
for name in $TERMINAL; do
  selected "$name" || continue
  if [ "$name" = cross_section_strip ]; then
    if [ "$DRY" -eq 1 ]; then
      if phase terminal; then say "+ $BIN/$name --ppm $OUT/$name.ppm 2>&1 | tee $OUT/$name.log"; fi
      if phase sheets; then say "+ magick $OUT/$name.ppm $OUT/$name.png"; fi
      continue
    fi
    if phase terminal; then
      need_bin "$name"
      run_logged "$name" 0 "$BIN/$name" --ppm "$OUT/$name.ppm"
    fi
    if phase sheets; then
      if [ -f "$OUT/$name.ppm" ]; then
        magick "$OUT/$name.ppm" "$OUT/$name.png"
      else
        echo "qa.sh: $name wrote no $OUT/$name.ppm" >>"$MISSING"
      fi
    fi
    continue
  fi
  if [ "$DRY" -eq 1 ]; then
    if phase terminal; then say "+ $BIN/$name 2>&1 | tee $OUT/$name.log"; fi
    continue
  fi
  if phase terminal; then
    need_bin "$name"
    run_logged "$name" 0 "$BIN/$name"
  fi
done

# 4. Windowed examples, scripted and off screen.
for name in $WINDOWED; do
  selected "$name" || continue
  IFS=' ' read -r -a KEYS <<<"$(keys_for "$name")"
  # `${KEYS[@]+...}`: an empty array is "unbound" under `set -u` on bash 3.2.
  script=$(script_for ${KEYS[@]+"${KEYS[@]}"})
  shots=$(printf '%s\n' "$script" | tr ' ' '\n' | grep -c ':shot$' || true)
  if [ "$DRY" -eq 1 ]; then
    if phase windowed; then
      say "+ CARNAGE_AUTOPILOT=\"$script\" CARNAGE_AUTOPILOT_OUT=$OUT/$name timeout $TIMEOUT $BIN/$name 2>&1 | tee $OUT/$name.log   # expects $shots shots"
    fi
    sheets_windowed "$name"
    continue
  fi
  if phase windowed; then
    need_bin "$name"
    mkdir -p "$OUT/$name"
    rm -f "$OUT/$name"/f*.png
    run_logged "$name" "$TIMEOUT" env CARNAGE_AUTOPILOT="$script" CARNAGE_AUTOPILOT_OUT="$OUT/$name" "$BIN/$name"
    got=$(find "$OUT/$name" -maxdepth 1 -name 'f*.png' | wc -l | tr -d ' ')
    if [ "$got" -lt "$shots" ]; then
      echo "qa.sh: $name produced $got of $shots screenshots (script: $script)" >>"$MISSING"
    fi
  fi
  sheets_windowed "$name"
done

# 6. Summaries. All three files exist after a real run even when empty.
summarise() {
  local pattern=$1 dest=$2 exclude=${3:-}
  local logs=("$OUT"/*.log)
  : >"$dest"
  [ -e "${logs[0]}" ] || return 0
  if [ -n "$exclude" ]; then
    grep -H "$pattern" "${logs[@]}" | grep -v "$exclude" >>"$dest" || true
  else
    grep -H "$pattern" "${logs[@]}" >>"$dest" || true
  fi
}

# What counts as a digest line. Most recorders print `digest=`. `capture_gore` and `capture_family`
# print one named hash per clip on a `<name>: frames=N ...` line instead, so that shape is matched too.
DIGEST_RE='digest|^[a-z_]+: frames=[0-9]+ [a-z_]+=[0-9a-f]{16}'

if [ "$DRY" -eq 1 ]; then
  say "+ grep -HE 'WARN|ERROR|panicked' $OUT/*.log | grep -v 'Skipped event Destroyed for unknown winit Window Id' > $OUT/warnings.txt"
  say "+ grep -HiE '$DIGEST_RE' $OUT/*.log > $OUT/digests.txt"
  say "# missing.txt: any windowed example with fewer f*.png than its script's shot count, or that hit the ${TIMEOUT}s timeout; any run exiting non-zero"
  exit 0
fi

summarise 'WARN\|ERROR\|panicked' "$OUT/warnings.txt" 'Skipped event Destroyed for unknown winit Window Id'
# `-i` is the one flag here that differs from the pattern style above, so spell it out.
{ grep -HiE "$DIGEST_RE" "$OUT"/*.log || true; } >"$OUT/digests.txt"

echo "qa.sh: done -> $OUT"
echo "qa.sh: $(wc -l <"$OUT/warnings.txt" | tr -d ' ') warning lines, $(wc -l <"$OUT/digests.txt" | tr -d ' ') digest lines, $(wc -l <"$MISSING" | tr -d ' ') missing/failed lines"
