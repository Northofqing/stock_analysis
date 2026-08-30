#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$repo_root"

scope="${1:-all}"
failed=0

reject() {
  local label="$1"
  local pattern="$2"
  shift 2
  local existing=()
  local path
  for path in "$@"; do
    if [[ -e "$path" ]]; then
      existing+=("$path")
    fi
  done
  if ((${#existing[@]} > 0)) && rg -n "$pattern" "${existing[@]}"; then
    printf 'forbidden %s remains\n' "$label" >&2
    failed=1
  fi
}

if [[ "$scope" == "manifest" || "$scope" == "all" ]]; then
  reject "Cargo dependency" '(^|[^[:alnum:]_])magic-(tdx-rs|market-core|market-router|market-composition|eastmoney-rs|ths-rs|sina-rs|cninfo-rs|tencent-rs|cls-rs|jin10-rs|thepaper-rs|exchange-rs|baidu-rs)([^[:alnum:]_]|$)' Cargo.toml Cargo.lock
  reject "upstream Git source" 'magic-market-data-rs\.git' Cargo.toml Cargo.lock build.rs build_support tests
  reject "gateway feature" 'magic-gateway' Cargo.toml
  reject "TDX lock attestation" 'MAGIC_TDX_DEPENDENCY_REVISION|locked_magic_tdx_revision|magic_tdx_lock' build.rs build_support tests src/data_gateway/benchmark.rs
fi

if [[ "$scope" == "source" || "$scope" == "all" ]]; then
  reject "upstream Rust path" '\b(magic_tdx_rs|magic_market_core|magic_market_router|magic_market_composition|magic_eastmoney_rs|magic_ths_rs|magic_sina_rs|magic_cninfo_rs|magic_tencent_rs|magic_cls_rs|magic_jin10_rs|magic_thepaper_rs|magic_exchange_rs|magic_baidu_rs)\b' src tests build.rs build_support
  reject "gateway cfg" 'cfg\([^\n]*feature[[:space:]]*=[[:space:]]*"magic-gateway"' src tests build.rs
fi

if [[ "$scope" == "targets" || "$scope" == "all" ]]; then
  for path in \
    src/grpc_server \
    src/bin/grpc_market_server.rs \
    src/bin/friday_full_replay.rs \
    src/bin/hbars_probe.rs \
    src/bin/rq_probe.rs \
    src/bin/selection_live_probe.rs \
    src/bin/t0_lib_probe.rs \
    src/bin/t0_minute_probe.rs \
    src/bin/t0_replay.rs \
    src/bin/tdx_5min_probe.rs \
    src/bin/tdx_raw_probe.rs \
    src/bin/tdx_server_probe.rs \
    src/bin/tencent_quote_probe.rs \
    src/bin/virtual_pnl.rs
  do
    if [[ -e "$path" ]]; then
      printf 'provider-only target remains: %s\n' "$path" >&2
      failed=1
    fi
  done
fi

exit "$failed"
