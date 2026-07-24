#!/bin/sh
set -eu

ROOT=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
cd "$ROOT"
git config core.hooksPath .githooks
echo "Git hooks enabled via .githooks"
