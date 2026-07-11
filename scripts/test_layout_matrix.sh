#!/usr/bin/env bash
set -euo pipefail

repo_root=$(cd "$(dirname "$0")/.." && pwd)
target_root=$(mktemp -d "${TMPDIR:-/tmp}/ic-stable-roaring-layout-matrix.XXXXXX")
trap 'rm -rf -- "$target_root"' EXIT

run_case() {
    local name=$1
    local expected_chunk=$2
    shift 2
    local target_dir="$target_root/$name"

    env CARGO_TARGET_DIR="$target_dir" "$@" cargo test --manifest-path "$repo_root/Cargo.toml" --all-features

    local layout
    layout=$(find "$target_dir/debug/build" -path '*/out/journal_layout.rs' -print -quit)
    test -n "$layout"
    rg -Fqx "pub const JOURNAL_READ_CHUNK_BYTES: usize = $expected_chunk;" "$layout"
}

run_case default 5120
run_case cap-1 5 JOURNAL_CAP_SLOTS=1
run_case cap-7-min-chunk 5 JOURNAL_CAP_SLOTS=7 JOURNAL_READ_CHUNK_TARGET=5 JOURNAL_READ_CHUNK_MAX=5
run_case cap-6-divisor-fallback 15 JOURNAL_CAP_SLOTS=6 JOURNAL_READ_CHUNK_TARGET=25 JOURNAL_READ_CHUNK_MAX=25
