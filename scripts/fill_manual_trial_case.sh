#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cargo_bin="${CARGO_BIN:-cargo}"
cases_file="$repo_root/docs/experiment/cases.tsv"
manual_file="$repo_root/docs/experiment/manual_trials.csv"
case_id="pytest_assertion"
agent="claude-code"
target="claude"
query_budget=140
expand_budget=140
auto_expand="if-needed"
fix_success_baseline=1
fix_success_tokenln=1

usage() {
  cat <<'USAGE'
Usage: fill_manual_trial_case.sh [options]

Options:
  --case <id>                   Case id from docs/experiment/cases.tsv (default: pytest_assertion)
  --agent <name>                Agent label for manual CSV (default: claude-code)
  --target <target>             TokenLn target emitter (default: claude)
  --query-budget <n>            Query budget (default: 140)
  --expand-budget <n>           Expand budget (default: 140)
  --auto-expand <mode>          never|if-needed|always (default: if-needed)
  --manual-file <path>          Manual trials CSV path
  --cases-file <path>           Case manifest path
  --fix-success-baseline <0|1>  Baseline fix_success value (default: 1)
  --fix-success-tokenln <0|1>   TokenLn fix_success value (default: 1)
  --cargo-bin <path>            Cargo binary override
  -h, --help                    Show this help
USAGE
}

while [ $# -gt 0 ]; do
  case "$1" in
    --case)
      case_id="$2"
      shift 2
      ;;
    --agent)
      agent="$2"
      shift 2
      ;;
    --target)
      target="$2"
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
    --auto-expand)
      auto_expand="$2"
      shift 2
      ;;
    --manual-file)
      manual_file="$2"
      shift 2
      ;;
    --cases-file)
      cases_file="$2"
      shift 2
      ;;
    --fix-success-baseline)
      fix_success_baseline="$2"
      shift 2
      ;;
    --fix-success-tokenln)
      fix_success_tokenln="$2"
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

if ! [[ "$fix_success_baseline" =~ ^[01]$ && "$fix_success_tokenln" =~ ^[01]$ ]]; then
  echo "error: fix_success values must be 0 or 1" >&2
  exit 1
fi

if ! [[ "$auto_expand" =~ ^(never|if-needed|always)$ ]]; then
  echo "error: --auto-expand must be one of never|if-needed|always" >&2
  exit 1
fi

if ! command -v "$cargo_bin" >/dev/null 2>&1; then
  if [ "$cargo_bin" = "cargo" ] && [ -x "$HOME/.cargo/bin/cargo" ]; then
    cargo_bin="$HOME/.cargo/bin/cargo"
  else
    echo "error: cargo binary '$cargo_bin' not found" >&2
    exit 1
  fi
fi

if [ ! -f "$cases_file" ]; then
  echo "error: cases file '$cases_file' does not exist" >&2
  exit 1
fi

case_row="$(awk -F'\t' -v id="$case_id" 'NR > 1 && $1 == id { print $0; exit }' "$cases_file")"
if [ -z "$case_row" ]; then
  echo "error: case '$case_id' not found in '$cases_file'" >&2
  exit 1
fi

IFS=$'\t' read -r _ command_path fixture_path objective <<<"$case_row"
fixture_abs="$repo_root/$fixture_path"
if [ ! -f "$fixture_abs" ]; then
  echo "error: fixture '$fixture_path' does not exist" >&2
  exit 1
fi

tokenln_cmd=("$cargo_bin" run --quiet --manifest-path "$repo_root/Cargo.toml" --)

run_with_timer() {
  local __outvar="$1"
  local __msvar="$2"
  shift 2
  local start_ns
  local end_ns
  local output
  start_ns="$(date +%s%N)"
  output="$("$@")"
  end_ns="$(date +%s%N)"
  printf -v "$__outvar" '%s' "$output"
  printf -v "$__msvar" '%s' "$(((end_ns - start_ns) / 1000000))"
}

estimate_tokens() {
  local text="$1"
  local chars
  chars="$(printf "%s" "$text" | wc -c | awk '{print $1}')"
  echo $(((chars + 3) / 4))
}

ms_to_seconds() {
  local ms="$1"
  awk -v ms="$ms" 'BEGIN { printf "%.3f", ms / 1000.0 }'
}

extract_json_number() {
  local key="$1"
  awk -v key="$key" '
    $0 ~ "\"" key "\"" {
      line = $0
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

run_proxy_case() {
  local command_path="$1"
  local fixture_abs="$2"
  case "$command_path" in
    cargo:test)
      "${tokenln_cmd[@]}" proxy run --from-file "$fixture_abs" --target "$target" -- cargo test
      ;;
    cargo:build)
      "${tokenln_cmd[@]}" proxy run --from-file "$fixture_abs" --target "$target" -- cargo build
      ;;
    go:test)
      "${tokenln_cmd[@]}" proxy run --from-file "$fixture_abs" --target "$target" -- go test
      ;;
    pytest)
      "${tokenln_cmd[@]}" proxy run --from-file "$fixture_abs" --target "$target" -- pytest
      ;;
    jest)
      "${tokenln_cmd[@]}" proxy run --from-file "$fixture_abs" --target "$target" -- jest
      ;;
    *)
      echo "error: unsupported command_path '$command_path'" >&2
      exit 1
      ;;
  esac
}

run_query_text() {
  local run_dir="$1"
  "${tokenln_cmd[@]}" query --run "$run_dir" --budget "$query_budget" --target "$target" --objective "$objective"
}

run_query_json() {
  local run_dir="$1"
  "${tokenln_cmd[@]}" query --emit-json --run "$run_dir" --budget "$query_budget" --objective "$objective"
}

run_expand_text() {
  local run_dir="$1"
  local deviation_id="$2"
  "${tokenln_cmd[@]}" expand "$deviation_id" --run "$run_dir" --view evidence --budget "$expand_budget" --target "$target" --objective "$objective"
}

run_expand_json() {
  local run_dir="$1"
  local deviation_id="$2"
  "${tokenln_cmd[@]}" expand "$deviation_id" --emit-json --run "$run_dir" --view evidence --budget "$expand_budget" --objective "$objective"
}

run_with_timer baseline_output baseline_ms cat "$fixture_abs"

run_with_timer proxy_output proxy_ms run_proxy_case "$command_path" "$fixture_abs"
run_dir="$("${tokenln_cmd[@]}" proxy last)"
run_with_timer query_output query_ms run_query_text "$run_dir"
query_json="$(run_query_json "$run_dir")"

deviation_count="$(printf "%s\n" "$query_json" | extract_json_number "unresolved_count")"
query_used_tokens="$(printf "%s\n" "$query_json" | extract_json_number "used_tokens")"
[ -z "$deviation_count" ] && deviation_count=0
[ -z "$query_used_tokens" ] && query_used_tokens=0

expand_output=""
expand_ms=0
expand_used_tokens=0
expand_needed=0
if [ "$deviation_count" -gt 0 ]; then
  case "$auto_expand" in
    always)
      expand_needed=1
      ;;
    if-needed)
      if ! printf "%s\n" "$query_json" | grep -q '"expansion_hints": \[\]'; then
        expand_needed=1
      fi
      ;;
    never)
      expand_needed=0
      ;;
  esac
fi

if [ "$expand_needed" -eq 1 ]; then
  deviation_id="$(printf "%s\n" "$query_json" | extract_first_deviation_id)"
  [ -z "$deviation_id" ] && deviation_id="d1"
  run_with_timer expand_output expand_ms run_expand_text "$run_dir" "$deviation_id"
  expand_json="$(run_expand_json "$run_dir" "$deviation_id")"
  expand_used_tokens="$(printf "%s\n" "$expand_json" | extract_json_number "used_tokens")"
  [ -z "$expand_used_tokens" ] && expand_used_tokens=0
fi

baseline_turns=1
tokenln_turns=2
if [ "$expand_needed" -eq 1 ]; then
  tokenln_turns=3
fi

baseline_time_sec="$(ms_to_seconds "$baseline_ms")"
tokenln_time_sec="$(ms_to_seconds "$((proxy_ms + query_ms + expand_ms))")"

baseline_tokens_in="$(estimate_tokens "$baseline_output")"
tokenln_tokens_in="$(estimate_tokens "$proxy_output
$query_output
$expand_output")"

baseline_row="$case_id,$agent,baseline,$fix_success_baseline,$baseline_turns,$baseline_time_sec,$baseline_tokens_in,auto-filled fixture-only tokens_est"
tokenln_note="auto-filled proxy+query tokens_est query_used=${query_used_tokens}/${query_budget}"
if [ "$expand_needed" -eq 1 ]; then
  tokenln_note="auto-filled proxy+query+expand tokens_est query_used=${query_used_tokens}/${query_budget} expand_used=${expand_used_tokens}/${expand_budget}"
fi
tokenln_row="$case_id,$agent,tokenln,$fix_success_tokenln,$tokenln_turns,$tokenln_time_sec,$tokenln_tokens_in,$tokenln_note"

mkdir -p "$(dirname "$manual_file")"
if [ ! -f "$manual_file" ]; then
  cat >"$manual_file" <<'CSV'
case_id,agent,mode,fix_success,turns_to_fix,time_to_fix_sec,tokens_in,notes
# mode: baseline|tokenln
# fill one row per trial, keep numeric fields plain numbers
CSV
fi

tmp_file="$(mktemp)"
awk -F',' -v row1="$baseline_row" -v row2="$tokenln_row" -v case_id="$case_id" -v agent="$agent" '
  BEGIN {
    row1_written = 0
    row2_written = 0
  }
  {
    if ($1 == case_id && $2 == agent && $3 == "baseline") {
      if (!row1_written) {
        print row1
        row1_written = 1
      }
      next
    }
    if ($1 == case_id && $2 == agent && $3 == "tokenln") {
      if (!row2_written) {
        print row2
        row2_written = 1
      }
      next
    }
    print $0
  }
  END {
    if (!row1_written) {
      print row1
    }
    if (!row2_written) {
      print row2
    }
  }
' "$manual_file" >"$tmp_file"
mv "$tmp_file" "$manual_file"

cat <<SUMMARY
filled rows for case '$case_id' in '${manual_file#$repo_root/}':
  baseline -> turns=$baseline_turns time_sec=$baseline_time_sec tokens_in=$baseline_tokens_in
  tokenln  -> turns=$tokenln_turns time_sec=$tokenln_time_sec tokens_in=$tokenln_tokens_in
  query_used_tokens=$query_used_tokens/$query_budget
  expand_used_tokens=$expand_used_tokens/$expand_budget (expand_needed=$expand_needed)
SUMMARY
