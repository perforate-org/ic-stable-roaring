#!/usr/bin/env bash
# Sweep compile-time `JOURNAL_CAP_SLOTS` and run canbench against this crate.
#
# `canbench` loads `canbench.yml`; its build_cmd runs `cargo build`, which reads
# `JOURNAL_CAP_SLOTS` when this script exports it (see `build.rs`, which generates layout constants in Cargo `OUT_DIR`).
#
# Usage:
#   ./scripts/sweep_journal_cap_canbench.sh
#   ./scripts/sweep_journal_cap_canbench.sh bench_roaring_reopen_journal_prefix_small bench_roaring_checkpoint_after_full_journal
#   CANBENCH_EXTRA_OPTS='--persist' ./scripts/sweep_journal_cap_canbench.sh
#
# Each positional argument is a literal substring passed to `canbench`; it is not a regular expression.
# With no arguments, every benchmark runs once per capacity. `--persist` rewrites
# `canbench_results.yml` after every benchmark/capacity pair, so rely on the saved logs for sweeps.
#
# Environment:
#   JOURNAL_CAP_SWEEP_SLOTS — space-separated list (default: several sizes including 1024 multiples).
#   CANBENCH_EXTRA_OPTS — extra canbench flags (space-separated), e.g. `--persist`.

set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

: "${JOURNAL_CAP_SWEEP_SLOTS:=1024 2048 3072 4096 5120 6144 8192 12288 16384}"
: "${CANBENCH_EXTRA_OPTS:=}"

RUN_ID="$(date +%Y%m%d_%H%M%S)"
OUT_DIR="$ROOT/tmp/canbench_journal_sweep/$RUN_ID"
mkdir -p "$OUT_DIR"

{
  echo "run_id=$RUN_ID"
  echo "journal_cap_slots_list=$JOURNAL_CAP_SWEEP_SLOTS"
  echo "extra_canbench_opts=$CANBENCH_EXTRA_OPTS"
  echo "canbench_literal_patterns=$*"
} >"$OUT_DIR/SUMMARY.meta"

echo "Sweep output -> $OUT_DIR"

FAIL=0

for SLOTS in $JOURNAL_CAP_SWEEP_SLOTS; do
  LOG="$OUT_DIR/cap_${SLOTS}.log"
  echo "===== JOURNAL_CAP_SLOTS=${SLOTS} =====" >"$LOG"

  export JOURNAL_CAP_SLOTS="$SLOTS"

  base=(--less-verbose)
  if [[ -n "${CANBENCH_EXTRA_OPTS}" ]]; then
    read -r -a user_extra <<<"$CANBENCH_EXTRA_OPTS"
    base+=("${user_extra[@]}")
  fi

  patterns=("$@")
  if [[ "${#patterns[@]}" -eq 0 ]]; then
    patterns=("")
  fi

  for pattern in "${patterns[@]}"; do
    if [[ -n "$pattern" ]]; then
      echo "--- canbench substring=$pattern ---" >>"$LOG"
      command=(canbench "$pattern" "${base[@]}")
    else
      echo "--- canbench all benchmarks ---" >>"$LOG"
      command=(canbench "${base[@]}")
    fi

    if ! "${command[@]}" 2>&1 | tee -a "$LOG"; then
      FAIL=1
      echo "canbench failed JOURNAL_CAP_SLOTS=$SLOTS pattern=${pattern:-all} (log: $LOG)" >>"$OUT_DIR/SUMMARY.meta"
    fi
  done

  echo >>"$LOG"
done

if [[ "$FAIL" -ne 0 ]]; then
  echo "[sweep] finished with one or more failures"
  exit 1
fi

echo "[sweep] done"
