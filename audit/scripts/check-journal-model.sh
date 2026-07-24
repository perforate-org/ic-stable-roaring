#!/usr/bin/env bash
set -euo pipefail

readonly SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
readonly AUDIT_DIR="$(cd "${SCRIPT_DIR}/.." && pwd)"
readonly ROOT_DIR="$(cd "${AUDIT_DIR}/.." && pwd)"
readonly MANIFEST="${AUDIT_DIR}/Audit/Generated/JournalCore/source-manifest"

expected="$(
  {
    echo "aeneas d71d2e3"
    echo "charon 0.1.223"
    echo "src/journal.rs $(git -C "${ROOT_DIR}" hash-object src/journal.rs)"
    echo "audit/aeneas/Cargo.toml $(git -C "${ROOT_DIR}" hash-object audit/aeneas/Cargo.toml)"
    echo "audit/aeneas/lib.rs $(git -C "${ROOT_DIR}" hash-object audit/aeneas/lib.rs)"
  }
)"
actual="$(<"${MANIFEST}")"

if [[ "${actual}" != "${expected}" ]]; then
  echo "generated journal model is stale" >&2
  echo "run audit/scripts/regenerate-journal-model.sh with the pinned Nix tools" >&2
  exit 1
fi

echo "generated journal model matches its Rust and harness inputs"
