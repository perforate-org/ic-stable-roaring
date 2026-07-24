#!/bin/sh
set -eu

# Git hooks may run with a minimal PATH (for example when invoked by an IDE or
# an SSH session), so include the standard Rust and Lean tool locations.
export PATH="${HOME}/.cargo/bin:${HOME}/.elan/bin:/opt/homebrew/bin:/usr/local/bin:${PATH}"

ROOT=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
cd "$ROOT"

echo "== Rust format =="
cargo +1.90.0 fmt --check
echo "== Rust tests =="
cargo +1.90.0 test --locked --all-features
echo "== Rust clippy =="
cargo +1.90.0 clippy --locked --all-targets --all-features -- -D warnings
echo "== Rust docs =="
RUSTDOCFLAGS=-Dwarnings cargo +1.90.0 doc --locked --no-deps --all-features
echo "== Layout matrix =="
scripts/test_layout_matrix.sh
echo "== Journal model freshness =="
audit/scripts/check-journal-model.sh
echo "== Lean audit =="
(cd audit && lake build)
if rg -n '\b(sorry|axiom)\b|UNDERSPECIFIED|True := by|: True' audit --glob '*.lean'; then
    echo "Lean audit contains a prohibited proof placeholder or vacuous claim" >&2
    exit 1
fi
echo "== Diff check =="
git diff --check
echo "CI audit passed"
