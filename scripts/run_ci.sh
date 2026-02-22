#!/usr/bin/env bash
set -euo pipefail

cargo_bin="${CARGO_BIN:-cargo}"
current_stage="init"

if ! command -v "$cargo_bin" >/dev/null 2>&1; then
  if [ "$cargo_bin" = "cargo" ] && [ -x "$HOME/.cargo/bin/cargo" ]; then
    cargo_bin="$HOME/.cargo/bin/cargo"
  else
    echo "CI_FAILURE stage=init reason='cargo not found' cargo_bin='$cargo_bin'" >&2
    exit 1
  fi
fi

on_error() {
  local exit_code=$?
  echo "CI_FAILURE stage=${current_stage} exit_code=${exit_code}" >&2
  exit "$exit_code"
}
trap on_error ERR

run_stage() {
  local stage_name="$1"
  shift
  current_stage="$stage_name"
  echo "==> [$stage_name]"
  "$@"
}

run_stage "test" "$cargo_bin" test
run_stage "snapshots" env CARGO_BIN="$cargo_bin" ./scripts/refresh_ir_snapshots.sh
run_stage "benchmark" env CARGO_BIN="$cargo_bin" ./scripts/benchmark_phase1.sh

echo "CI_SUCCESS stages=test,snapshots,benchmark"
