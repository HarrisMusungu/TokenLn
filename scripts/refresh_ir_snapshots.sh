#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cargo_bin="${CARGO_BIN:-cargo}"

if ! command -v "$cargo_bin" >/dev/null 2>&1; then
  if [ "$cargo_bin" = "cargo" ] && [ -x "$HOME/.cargo/bin/cargo" ]; then
    cargo_bin="$HOME/.cargo/bin/cargo"
  else
    echo "error: cargo binary '$cargo_bin' was not found in PATH" >&2
    echo "hint: set CARGO_BIN=/absolute/path/to/cargo if needed" >&2
    exit 1
  fi
fi

refresh_snapshot() {
  local command_path="$1"
  local fixture_path="$2"
  local output_path="$3"

  case "$command_path" in
    "cargo:test")
      "$cargo_bin" run --quiet --manifest-path "$repo_root/Cargo.toml" -- \
        cargo test --from-file "$fixture_path" --emit-ir > "$output_path"
      ;;
    "cargo:build")
      "$cargo_bin" run --quiet --manifest-path "$repo_root/Cargo.toml" -- \
        cargo build --from-file "$fixture_path" --emit-ir > "$output_path"
      ;;
    "go:test")
      "$cargo_bin" run --quiet --manifest-path "$repo_root/Cargo.toml" -- \
        go test --from-file "$fixture_path" --emit-ir > "$output_path"
      ;;
    "pytest")
      "$cargo_bin" run --quiet --manifest-path "$repo_root/Cargo.toml" -- \
        pytest --from-file "$fixture_path" --emit-ir > "$output_path"
      ;;
    "jest")
      "$cargo_bin" run --quiet --manifest-path "$repo_root/Cargo.toml" -- \
        jest --from-file "$fixture_path" --emit-ir > "$output_path"
      ;;
    *)
      echo "error: unsupported command path '$command_path'" >&2
      exit 1
      ;;
  esac
}

refresh_snapshot \
  cargo:test \
  "$repo_root/tests/fixtures/cargo_test/assertion_failure.txt" \
  "$repo_root/tests/fixtures/expected_ir/assertion_failure.ir.json"

refresh_snapshot \
  cargo:test \
  "$repo_root/tests/fixtures/cargo_test/panic_quoted_format.txt" \
  "$repo_root/tests/fixtures/expected_ir/panic_quoted_format.ir.json"

refresh_snapshot \
  cargo:build \
  "$repo_root/tests/fixtures/cargo_build/missing_symbol.txt" \
  "$repo_root/tests/fixtures/expected_ir/cargo_build_missing_symbol.ir.json"

refresh_snapshot \
  cargo:build \
  "$repo_root/tests/fixtures/cargo_build/conflicting_evidence.txt" \
  "$repo_root/tests/fixtures/expected_ir/cargo_build_conflicting_evidence.ir.json"

refresh_snapshot \
  go:test \
  "$repo_root/tests/fixtures/go_test/assertion_failure.txt" \
  "$repo_root/tests/fixtures/expected_ir/go_test_assertion_failure.ir.json"

refresh_snapshot \
  pytest \
  "$repo_root/tests/fixtures/pytest/assertion_failure.txt" \
  "$repo_root/tests/fixtures/expected_ir/pytest_assertion_failure.ir.json"

refresh_snapshot \
  jest \
  "$repo_root/tests/fixtures/jest/assertion_failure.txt" \
  "$repo_root/tests/fixtures/expected_ir/jest_assertion_failure.ir.json"

echo "refreshed IR snapshots in tests/fixtures/expected_ir/"
