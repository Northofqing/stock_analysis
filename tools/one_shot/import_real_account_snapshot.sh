#!/usr/bin/env bash
set -euo pipefail

if [[ $# -ne 2 ]]; then
  echo "usage: $0 <database-path> <ignored-evidence-json>" >&2
  exit 2
fi

cargo run --quiet --bin import_real_account_snapshot -- \
  --database "$1" \
  --evidence "$2"
