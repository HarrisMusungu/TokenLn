#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cargo_bin="${CARGO_BIN:-cargo}"
cases_file="$repo_root/docs/experiment/cases.tsv"
manual_file="$repo_root/docs/experiment/manual_trials.csv"
output_dir="$repo_root/docs/experiment/results"
query_budget=400
expand_budget=220

usage() {
  cat <<'USAGE'
Usage: run_validation_experiment.sh [options]

Options:
  --cases <file>          TSV case manifest (default: docs/experiment/cases.tsv)
  --manual <file>         Manual trials CSV (default: docs/experiment/manual_trials.csv)
  --output-dir <dir>      Output directory (default: docs/experiment/results)
  --query-budget <n>      Token budget for tokenln query (default: 400)
  --expand-budget <n>     Token budget for tokenln expand (default: 220)
  --cargo-bin <path>      Cargo binary override
  -h, --help              Show this help
USAGE
}

while [ $# -gt 0 ]; do
  case "$1" in
    --cases)
      cases_file="$2"
      shift 2
      ;;
    --manual)
      manual_file="$2"
      shift 2
      ;;
    --output-dir)
      output_dir="$2"
      shift 2
      ;;
    --query-budget)
      query_budget="$2"
      shift 2
      ;;
    --expand-budget)
      expand_budget="$2"
      shift 2
      ;;
    --cargo-bin)
      cargo_bin="$2"
      shift 2
      ;;
    -h|--help)
      usage
      exit 0
      ;;
    *)
      echo "error: unknown argument '$1'" >&2
      usage >&2
      exit 1
      ;;
  esac
done

if ! command -v "$cargo_bin" >/dev/null 2>&1; then
  if [ "$cargo_bin" = "cargo" ] && [ -x "$HOME/.cargo/bin/cargo" ]; then
    cargo_bin="$HOME/.cargo/bin/cargo"
  else
    echo "error: cargo binary '$cargo_bin' was not found in PATH" >&2
    echo "hint: set CARGO_BIN=/absolute/path/to/cargo or pass --cargo-bin" >&2
    exit 1
  fi
fi

if [ ! -f "$cases_file" ]; then
  echo "error: cases file '$cases_file' does not exist" >&2
  exit 1
fi

mkdir -p "$output_dir"
mkdir -p "$(dirname "$manual_file")"

if [ ! -f "$manual_file" ]; then
  cat >"$manual_file" <<'CSV'
case_id,agent,mode,fix_success,turns_to_fix,time_to_fix_sec,tokens_in,notes
# mode: baseline|tokenln
# fill one row per trial, keep numeric fields plain numbers
CSV
fi

auto_csv="$output_dir/auto_metrics.csv"
report_md="$output_dir/VALIDATION_REPORT.md"
manifest_used="$output_dir/cases.used.tsv"

tokenln_cmd=("$cargo_bin" run --quiet --manifest-path "$repo_root/Cargo.toml" --)

run_tokenln_brief() {
  local command_path="$1"
  local fixture_abs="$2"
  case "$command_path" in
    cargo:test)
      "${tokenln_cmd[@]}" cargo test --from-file "$fixture_abs" --target generic
      ;;
    cargo:build)
      "${tokenln_cmd[@]}" cargo build --from-file "$fixture_abs" --target generic
      ;;
    go:test)
      "${tokenln_cmd[@]}" go test --from-file "$fixture_abs" --target generic
      ;;
    pytest)
      "${tokenln_cmd[@]}" pytest --from-file "$fixture_abs" --target generic
      ;;
    jest)
      "${tokenln_cmd[@]}" jest --from-file "$fixture_abs" --target generic
      ;;
    *)
      echo "error: unsupported command_path '$command_path'" >&2
      exit 1
      ;;
  esac
}

run_proxy_capture() {
  local command_path="$1"
  local fixture_abs="$2"
  case "$command_path" in
    cargo:test)
      "${tokenln_cmd[@]}" proxy run --from-file "$fixture_abs" --target generic -- cargo test
      ;;
    cargo:build)
      "${tokenln_cmd[@]}" proxy run --from-file "$fixture_abs" --target generic -- cargo build
      ;;
    go:test)
      "${tokenln_cmd[@]}" proxy run --from-file "$fixture_abs" --target generic -- go test
      ;;
    pytest)
      "${tokenln_cmd[@]}" proxy run --from-file "$fixture_abs" --target generic -- pytest
      ;;
    jest)
      "${tokenln_cmd[@]}" proxy run --from-file "$fixture_abs" --target generic -- jest
      ;;
    *)
      echo "error: unsupported command_path '$command_path'" >&2
      exit 1
      ;;
  esac
}

extract_json_number() {
  local key="$1"
  awk -v key="$key" '
    $0 ~ "\"" key "\"" {
      line=$0
      sub(/.*: /, "", line)
      gsub(/[^0-9.]/, "", line)
      if (line != "") {
        print line
        exit
      }
    }
  '
}

extract_first_deviation_id() {
  awk -F'"' '/"id": "d[0-9]+"/ { print $4; exit }'
}

calculate_savings_pct() {
  local baseline_words="$1"
  local tokenln_words="$2"
  awk -v base="$baseline_words" -v opt="$tokenln_words" 'BEGIN {
    if (base == 0) { printf "0.0"; exit }
    printf "%.1f", ((base - opt) * 100.0) / base
  }'
}

word_count() {
  wc -w | awk '{print $1}'
}

echo -e "case_id\tcommand_path\tfixture_path\tobjective" >"$manifest_used"
echo "case_id,command_path,baseline_words,brief_words,query_words,expand_words,total_tokenln_words,savings_pct,deviation_count,query_used_tokens,expand_used_tokens,query_budget,expand_budget,truncated" >"$auto_csv"

while IFS=$'\t' read -r case_id command_path fixture_path objective; do
  if [ "$case_id" = "case_id" ] || [ -z "$case_id" ]; then
    continue
  fi
  if [[ "$case_id" =~ ^# ]]; then
    continue
  fi

  fixture_abs="$repo_root/$fixture_path"
  if [ ! -f "$fixture_abs" ]; then
    echo "warning: skipping '$case_id' because fixture '$fixture_path' does not exist" >&2
    continue
  fi

  printf "running case: %s\n" "$case_id"
  printf "%s\t%s\t%s\t%s\n" "$case_id" "$command_path" "$fixture_path" "$objective" >>"$manifest_used"

  raw_output="$(cat "$fixture_abs")"
  baseline_words="$(printf "%s" "$raw_output" | word_count)"

  brief_output="$(run_tokenln_brief "$command_path" "$fixture_abs")"
  brief_words="$(printf "%s" "$brief_output" | word_count)"

  run_proxy_capture "$command_path" "$fixture_abs" >/dev/null
  run_dir="$("${tokenln_cmd[@]}" proxy last)"

  query_json="$("${tokenln_cmd[@]}" query --emit-json --run "$run_dir" --budget "$query_budget" --objective "$objective")"
  query_words="$(printf "%s" "$query_json" | word_count)"
  query_used_tokens="$(printf "%s\n" "$query_json" | extract_json_number "used_tokens")"
  deviation_count="$(printf "%s\n" "$query_json" | extract_json_number "unresolved_count")"
  [ -z "$query_used_tokens" ] && query_used_tokens=0
  [ -z "$deviation_count" ] && deviation_count=0

  expand_words=0
  expand_used_tokens=0
  truncated="no"
  if [ "$deviation_count" -gt 0 ]; then
    deviation_id="$(printf "%s\n" "$query_json" | extract_first_deviation_id)"
    [ -z "$deviation_id" ] && deviation_id="d1"
    expand_json="$("${tokenln_cmd[@]}" expand "$deviation_id" --emit-json --view full --run "$run_dir" --budget "$expand_budget" --objective "$objective")"
    expand_words="$(printf "%s" "$expand_json" | word_count)"
    expand_used_tokens="$(printf "%s\n" "$expand_json" | extract_json_number "used_tokens")"
    [ -z "$expand_used_tokens" ] && expand_used_tokens=0
    if printf "%s\n" "$expand_json" | grep -q '\[truncated;'; then
      truncated="yes"
    fi
  fi

  total_tokenln_words=$((query_words + expand_words))
  savings_pct="$(calculate_savings_pct "$baseline_words" "$total_tokenln_words")"

  echo "$case_id,$command_path,$baseline_words,$brief_words,$query_words,$expand_words,$total_tokenln_words,$savings_pct,$deviation_count,$query_used_tokens,$expand_used_tokens,$query_budget,$expand_budget,$truncated" >>"$auto_csv"
done <"$cases_file"

{
  echo "# Validation Report"
  echo
  echo "_Generated on $(date -u '+%Y-%m-%d %H:%M:%SZ')_"
  echo
  echo "## Experiment Setup"
  echo
  echo "- Manifest: \`${cases_file#$repo_root/}\`"
  echo "- Auto metrics: \`${auto_csv#$repo_root/}\`"
  echo "- Manual trials: \`${manual_file#$repo_root/}\`"
  echo "- Query budget: \`${query_budget}\`"
  echo "- Expand budget: \`${expand_budget}\`"
  echo
  echo "## Auto Metrics"
  echo
  echo "| Case | Baseline words | Brief words | Query words | Expand words | Total TokenLn words | Savings vs baseline | Deviations | Truncated |"
  echo "|---|---:|---:|---:|---:|---:|---:|---:|---|"
  tail -n +2 "$auto_csv" | awk -F',' '{ printf "| `%s` | %s | %s | %s | %s | %s | %s%% | %s | %s |\n", $1, $3, $4, $5, $6, $7, $8, $9, $14 }'
  echo
  echo "## Auto Summary"
  echo
  awk -F',' '
    NR == 1 { next }
    {
      n += 1
      base += $3
      opt += $7
      brief += $4
      trunc += ($14 == "yes") ? 1 : 0
    }
    END {
      if (n == 0) {
        print "- No rows were produced."
      } else {
        savings = (base == 0) ? 0 : ((base - opt) * 100.0 / base)
        brief_savings = (base == 0) ? 0 : ((base - brief) * 100.0 / base)
        printf "- Cases: `%d`\n", n
        printf "- Aggregate baseline words: `%d`\n", base
        printf "- Aggregate TokenLn words (query+expand): `%d`\n", opt
        printf "- Aggregate brief words: `%d`\n", brief
        printf "- Aggregate savings (query+expand): `%.1f%%`\n", savings
        printf "- Aggregate savings (brief only): `%.1f%%`\n", brief_savings
        printf "- Truncated expansions: `%d` (%s)\n", trunc, (n == 0 ? "0.0%" : sprintf("%.1f%%", trunc * 100.0 / n))
      }
    }
  ' "$auto_csv"
  echo
  echo "## Manual Trial Summary"
  echo
  echo "_Fill \`${manual_file#$repo_root/}\` and rerun this script to populate this section._"
  echo
  if awk -F',' '
    NR == 1 { next }
    $1 ~ /^#/ { next }
    ($4 == "0" || $4 == "1") && $5 ~ /^[0-9]+([.][0-9]+)?$/ && $6 ~ /^[0-9]+([.][0-9]+)?$/ && $7 ~ /^[0-9]+([.][0-9]+)?$/ { found = 1 }
    END { exit found ? 0 : 1 }
  ' "$manual_file"; then
    echo "| Mode | Trials | Success rate | Avg turns | Avg time (s) | Avg tokens |"
    echo "|---|---:|---:|---:|---:|---:|"
    awk -F',' '
      NR == 1 { next }
      $1 ~ /^#/ { next }
      ($4 == "0" || $4 == "1") && $5 ~ /^[0-9]+([.][0-9]+)?$/ && $6 ~ /^[0-9]+([.][0-9]+)?$/ && $7 ~ /^[0-9]+([.][0-9]+)?$/ {
        mode = $3
        count[mode] += 1
        success[mode] += $4
        turns[mode] += $5
        time_s[mode] += $6
        tokens[mode] += $7
      }
      END {
        modes[1] = "baseline"
        modes[2] = "tokenln"
        for (i = 1; i <= 2; i++) {
          mode = modes[i]
          c = count[mode] + 0
          if (c == 0) {
            printf "| `%s` | 0 | 0.0%% | 0.0 | 0.0 | 0.0 |\n", mode
            continue
          }
          success_rate = (success[mode] * 100.0) / c
          avg_turns = turns[mode] / c
          avg_time = time_s[mode] / c
          avg_tokens = tokens[mode] / c
          printf "| `%s` | %d | %.1f%% | %.2f | %.2f | %.1f |\n", mode, c, success_rate, avg_turns, avg_time, avg_tokens
        }
      }
    ' "$manual_file"
  else
    echo "No complete manual trials found yet."
  fi
  echo
  echo "## Notes"
  echo
  echo "- This harness treats word counts as a lightweight token proxy."
  echo "- Real token counts should be copied from agent telemetry into \`manual_trials.csv\`."
  echo "- The decision metric is \`fix_success\` parity with lower \`tokens_in\` and fewer turns."
} >"$report_md"

echo "wrote auto metrics: ${auto_csv#$repo_root/}"
echo "wrote validation report: ${report_md#$repo_root/}"
echo "manual trials file: ${manual_file#$repo_root/}"
