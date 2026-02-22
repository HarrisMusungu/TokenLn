#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cargo_bin="${CARGO_BIN:-cargo}"
output_file="${1:-$repo_root/docs/BENCHMARKS.md}"

if ! command -v "$cargo_bin" >/dev/null 2>&1; then
  if [ "$cargo_bin" = "cargo" ] && [ -x "$HOME/.cargo/bin/cargo" ]; then
    cargo_bin="$HOME/.cargo/bin/cargo"
  else
    echo "error: cargo binary '$cargo_bin' was not found in PATH" >&2
    echo "hint: set CARGO_BIN=/absolute/path/to/cargo if needed" >&2
    exit 1
  fi
fi

cases=(
  "cargo:test|tests/fixtures/cargo_test/assertion_failure.txt|cargo_test_assertion"
  "cargo:test|tests/fixtures/cargo_test/panic_quoted_format.txt|cargo_test_panic_low_conf"
  "cargo:build|tests/fixtures/cargo_build/missing_symbol.txt|cargo_build_missing_symbol"
  "cargo:build|tests/fixtures/cargo_build/conflicting_evidence.txt|cargo_build_conflicting_evidence"
  "go:test|tests/fixtures/go_test/assertion_failure.txt|go_test_assertion"
  "pytest|tests/fixtures/pytest/assertion_failure.txt|pytest_assertion"
  "jest|tests/fixtures/jest/assertion_failure.txt|jest_assertion"
)

mkdir -p "$(dirname "$output_file")"

{
  echo "# Phase 1 Benchmarks"
  echo
  echo "_Generated on $(date -u '+%Y-%m-%d %H:%M:%SZ')_"
  echo
  echo "| Case | Raw words | Emitter words | IR words | Emitter savings | Confidence | Fallback |"
  echo "|---|---:|---:|---:|---:|---:|---|"

  metrics_rows=""

  for case in "${cases[@]}"; do
    IFS='|' read -r command_path fixture_path label <<<"$case"
    fixture_abs="$repo_root/$fixture_path"

    raw_output="$(cat "$fixture_abs")"
    raw_words="$(printf "%s" "$raw_output" | wc -w | awk '{print $1}')"

    case "$command_path" in
      "cargo:test")
        emitter_output="$("$cargo_bin" run --quiet --manifest-path "$repo_root/Cargo.toml" -- cargo test --from-file "$fixture_abs")"
        ir_output="$("$cargo_bin" run --quiet --manifest-path "$repo_root/Cargo.toml" -- cargo test --from-file "$fixture_abs" --emit-ir)"
        ;;
      "cargo:build")
        emitter_output="$("$cargo_bin" run --quiet --manifest-path "$repo_root/Cargo.toml" -- cargo build --from-file "$fixture_abs")"
        ir_output="$("$cargo_bin" run --quiet --manifest-path "$repo_root/Cargo.toml" -- cargo build --from-file "$fixture_abs" --emit-ir)"
        ;;
      "go:test")
        emitter_output="$("$cargo_bin" run --quiet --manifest-path "$repo_root/Cargo.toml" -- go test --from-file "$fixture_abs")"
        ir_output="$("$cargo_bin" run --quiet --manifest-path "$repo_root/Cargo.toml" -- go test --from-file "$fixture_abs" --emit-ir)"
        ;;
      "pytest")
        emitter_output="$("$cargo_bin" run --quiet --manifest-path "$repo_root/Cargo.toml" -- pytest --from-file "$fixture_abs")"
        ir_output="$("$cargo_bin" run --quiet --manifest-path "$repo_root/Cargo.toml" -- pytest --from-file "$fixture_abs" --emit-ir)"
        ;;
      "jest")
        emitter_output="$("$cargo_bin" run --quiet --manifest-path "$repo_root/Cargo.toml" -- jest --from-file "$fixture_abs")"
        ir_output="$("$cargo_bin" run --quiet --manifest-path "$repo_root/Cargo.toml" -- jest --from-file "$fixture_abs" --emit-ir)"
        ;;
      *)
        echo "error: unsupported command path '$command_path'" >&2
        exit 1
        ;;
    esac
    emitter_words="$(printf "%s" "$emitter_output" | wc -w | awk '{print $1}')"
    ir_words="$(printf "%s" "$ir_output" | wc -w | awk '{print $1}')"

    savings="$(awk -v raw="$raw_words" -v emit="$emitter_words" 'BEGIN { if (raw == 0) { printf "0.0%%"; } else { printf "%.1f%%", ((raw - emit) * 100.0) / raw; } }')"
    confidence_raw="$(printf "%s\n" "$ir_output" | grep -m1 '"confidence"' | sed -E 's/.*: ([0-9.]+),?/\1/')"
    confidence="$(awk -v c="$confidence_raw" 'BEGIN { printf "%.2f", c + 0 }')"

    fallback="no"
    if printf "%s\n" "$ir_output" | grep -q '"raw_excerpt"'; then
      fallback="yes"
    fi

    echo "| \`$label\` | $raw_words | $emitter_words | $ir_words | $savings | $confidence | $fallback |"
    metrics_rows+="${label}\t${confidence}\t${fallback}\n"
  done

  echo
  echo "## Confidence Calibration"
  echo
  echo "| Bucket | Range | Cases | Avg confidence | Fallback rate |"
  echo "|---|---|---:|---:|---:|"
  printf "%b" "$metrics_rows" | awk -F'\t' '
    BEGIN {
      order[1] = "low";
      order[2] = "medium";
      order[3] = "high";
      range["low"] = "<0.85";
      range["medium"] = "0.85-0.95";
      range["high"] = ">=0.95";
    }
    NF >= 3 {
      confidence = $2 + 0;
      fallback = ($3 == "yes") ? 1 : 0;
      if (confidence < 0.85) {
        bucket = "low";
      } else if (confidence < 0.95) {
        bucket = "medium";
      } else {
        bucket = "high";
      }
      count[bucket] += 1;
      sum[bucket] += confidence;
      fallbacks[bucket] += fallback;
    }
    END {
      for (i = 1; i <= 3; i++) {
        bucket = order[i];
        cases = count[bucket] + 0;
        avg = (cases == 0) ? 0 : sum[bucket] / cases;
        rate = (cases == 0) ? 0 : (fallbacks[bucket] * 100.0) / cases;
        printf "| `%s` | %s | %d | %.2f | %.1f%% |\n", bucket, range[bucket], cases, avg, rate;
      }
    }
  '

  echo
  echo "Notes:"
  echo "- Word counts are a lightweight proxy for token usage in this Phase 1 harness."
  echo "- \`Fallback=yes\` means confidence was below threshold and raw excerpt enrichment was applied."
  echo "- Small fixtures can show negative savings; benchmark focus is confidence behavior and pipeline stability."
} > "$output_file"

echo "wrote benchmark report: ${output_file#$repo_root/}"
