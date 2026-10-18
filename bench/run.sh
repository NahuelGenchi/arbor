#!/usr/bin/env bash
# Arbor baseline suite: re-measures the M numbers of OPTIMIZATION.md on this PC and writes one Markdown table to
# results/<date>_<host>.md (plus a .json with the same rows and a .log of the run).
#   ./run.sh                     everything (about 4½ minutes on the M machine)
#   ONLY="boot search" ./run.sh  only these sections;  SKIP="storage" ./run.sh  all but these
# Sections: memstate boot runtimes cells pycell search heap transcripts formats storage arbor screen, and toolkits,
# mission and goals only when ONLY names them (ONLY=toolkits: about four minutes a toolkit, see toolkits/README.md;
# ONLY=mission: Mission Control, about fifteen minutes; ONLY=goals: the five goals over a whole stretch of work, about
# 45 minutes, which writes its builds to a temp folder on disk, GOALS_TMP)
# Needs no root. Temp files live in one mktemp dir that is removed on exit, also on Ctrl-C.
set -uo pipefail

BENCH_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
export BENCH_DIR
# shellcheck source=config.sh
. "$BENCH_DIR/config.sh"
export PROJECTS_DIR NAME_SEARCH_ROOT NAME_QUERY CODE_ROOT CODE_QUERIES DEDUP_ROOT FORK_TREE TRANSCRIPTS_DIR \
       O200K_FILE LISTING_DIR FORMATS_LOCALE GIT_LOG_REPO GIT_LOG_REF HEAP_JSONL HEAP_JSONL_BYTES DOCKER_IMAGE INITRD \
       ARBOR_BIN OUTLINE_REF OUTLINE_FILES SCREEN_STILL_S FRAMES_S FRAMES_CELLS FRAMES_SPIKE_MIB TOOLKITS \
       TOOLKITS_PREBUILT TOOLKITS_LAUNCHES TOOLKITS_STILL_S TOOLKITS_SETTLE_S FLUTTER FLUTTER_PUB_CACHE MISSION_BIN \
       MISSION_ROOMS GOALS_REF GOALS_PARTS GOALS_RUNS GOALS_STILL_S GOALS_STILL_BLOCKS
export GOALS_TMP="${GOALS_TMP:-$RESULTS_DIR}"
[ -d "$HOME/.cargo/bin" ] && ! command -v rustc >/dev/null && PATH="$PATH:$HOME/.cargo/bin"

WORK="$(mktemp -d "$BENCH_TMP/arbor-bench.XXXXXX")" || { echo "cannot create a temp dir in $BENCH_TMP" >&2; exit 1; }
export BENCH_WORK="$WORK" BENCH_RESULTS="$WORK/results.jsonl"
mkdir -p "$WORK/logs" "$RESULTS_DIR"
CHILD=
cleanup() {
  [ -n "$CHILD" ] && kill -TERM "$CHILD" 2>/dev/null && wait "$CHILD" 2>/dev/null
  chmod -R u+rwX "$WORK" 2>/dev/null   # overlay work dirs are created without permissions
  rm -rf "$WORK"
}
trap cleanup EXIT
trap 'echo " interrupted" >&2; exit 130' INT TERM

STAMP="$(date +%Y-%m-%d_%H%M)_$(hostname -s)"
OUT="$RESULTS_DIR/$STAMP.md"
LOG="$RESULTS_DIR/$STAMP.log"
PY=(python3 -I -B)   # -B: no __pycache__ in bench/

selected() {
  local s=" ${ONLY:-} " k=" ${SKIP:-} "
  { [ -z "${ONLY:-}" ] || [[ "$s" == *" $1 "* ]]; } && [[ "$k" != *" $1 "* ]]
}

SKIPPED=
section() {   # name timeout-seconds command...
  local name=$1 limit=$2 t0 rc ms status
  shift 2
  if ! selected "$name"; then SKIPPED="$SKIPPED $name"; return 0; fi
  printf '%-12s' "$name" >&2
  t0=$(date +%s%N)
  # In the background so Ctrl-C reaches the trap at once; timeout passes the TERM on to the whole section.
  timeout --kill-after=10 "$limit" "$@" >"$WORK/logs/$name.log" 2>&1 &
  CHILD=$!
  wait "$CHILD"
  rc=$?
  CHILD=
  ms=$(( ($(date +%s%N) - t0) / 1000000 ))
  case $rc in
    0) status=ok ;;
    124|137) status="timed out after $limit s" ;;
    *) status="failed (exit $rc): $(tail -n 3 "$WORK/logs/$name.log" | tr -s '[:space:]' ' ')" ;;
  esac
  if [ $rc -ne 0 ]; then
    "${PY[@]}" -c 'import json, sys; print(json.dumps({"key": "section." + sys.argv[1], "value": None, "err": sys.argv[2][:300]}))' \
      "$name" "$status" >>"$BENCH_RESULTS"
  fi
  printf '%4d.%d s  %s\n' $(( ms / 1000 )) $(( ms % 1000 / 100 )) "$status" >&2
  { echo "=== $name (exit $rc, $ms ms)"; cat "$WORK/logs/$name.log"; } >>"$WORK/run.log"
}

START=$(date +%s)
echo "Arbor baseline -> $OUT" >&2
"${PY[@]}" "$BENCH_DIR/lib/sysinfo.py" >"$WORK/meta.json" 2>>"$WORK/run.log"

S="$BENCH_DIR/sections"
section memstate     60 "${PY[@]}" "$S/memstate.py"
section boot        120 "${PY[@]}" "$S/boot.py"
section runtimes    300 "${PY[@]}" "$S/runtimes.py"
section cells       180 "${PY[@]}" "$S/cells.py"
section pycell      120 "${PY[@]}" "$S/pycell.py"
section search      600 "${PY[@]}" "$S/search.py"
section heap        300 "${PY[@]}" "$S/heap.py"
section transcripts 600 node "$S/transcripts.js"
section formats     120 "${PY[@]}" "$S/formats.py"
section storage     900 "${PY[@]}" "$S/storage.py"
section arbor       900 "${PY[@]}" "$S/arbor.py"
section screen      300 "${PY[@]}" "$S/screen.py"
if [[ " ${ONLY:-} " == *" toolkits "* ]]; then
  section toolkits 5400 "${PY[@]}" "$S/toolkits.py"
else
  SKIPPED="$SKIPPED toolkits"
fi
if [[ " ${ONLY:-} " == *" mission "* ]]; then
  section mission 3600 "${PY[@]}" "$S/mission.py"
else
  SKIPPED="$SKIPPED mission"
fi
if [[ " ${ONLY:-} " == *" goals "* ]]; then
  section goals 5400 "${PY[@]}" "$S/goals.py"
else
  SKIPPED="$SKIPPED goals"
fi

export BENCH_DURATION_S=$(( $(date +%s) - START )) BENCH_SKIPPED="${SKIPPED# }"
"${PY[@]}" "$BENCH_DIR/lib/report.py" "$BENCH_RESULTS" "$WORK/meta.json" "$OUT" >&2
cp "$WORK/run.log" "$LOG" 2>/dev/null
echo "report: $OUT  (log: $LOG, $BENCH_DURATION_S s)" >&2
