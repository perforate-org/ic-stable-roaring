#!/usr/bin/env bash
set -euo pipefail

readonly AENEAS_REV="d71d2e3"
readonly CHARON_VERSION="0.1.223"
readonly SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
readonly AUDIT_DIR="$(cd "${SCRIPT_DIR}/.." && pwd)"
readonly ROOT_DIR="$(cd "${AUDIT_DIR}/.." && pwd)"

actual_aeneas="$(aeneas -version)"
if [[ "${actual_aeneas}" != "aeneas ${AENEAS_REV}" ]]; then
  echo "expected aeneas ${AENEAS_REV}, found ${actual_aeneas}" >&2
  exit 1
fi

actual_charon="$(charon version)"
if [[ "${actual_charon}" != "${CHARON_VERSION}" ]]; then
  echo "expected charon ${CHARON_VERSION}, found ${actual_charon}" >&2
  exit 1
fi

tmp_dir="$(mktemp -d)"
trap 'rm -rf "${tmp_dir}"' EXIT

(
  cd "${AUDIT_DIR}/aeneas"
  charon cargo --preset=aeneas --dest-file="${tmp_dir}/journal.llbc"
)

aeneas \
  -backend lean \
  "${tmp_dir}/journal.llbc" \
  -dest "${AUDIT_DIR}" \
  -subdir /Audit/Generated/JournalCore \
  -split-files \
  -namespace Audit.Generated.JournalCore \
  -no-progress-bar

{
  echo "aeneas ${AENEAS_REV}"
  echo "charon ${CHARON_VERSION}"
  echo "src/journal.rs $(git -C "${ROOT_DIR}" hash-object src/journal.rs)"
  echo "audit/aeneas/Cargo.toml $(git -C "${ROOT_DIR}" hash-object audit/aeneas/Cargo.toml)"
  echo "audit/aeneas/lib.rs $(git -C "${ROOT_DIR}" hash-object audit/aeneas/lib.rs)"
} > "${AUDIT_DIR}/Audit/Generated/JournalCore/source-manifest"

echo "regenerated ${ROOT_DIR}/audit/Audit/Generated/JournalCore"
