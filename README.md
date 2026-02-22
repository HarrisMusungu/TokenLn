# TokenLn — Dev Compiler

[![License: MIT](https://img.shields.io/badge/License-MIT-yellow.svg)](https://opensource.org/licenses/MIT)
[![Status: Experimental](https://img.shields.io/badge/Status-Experimental-orange.svg)](docs/ROADMAP.md)

**A compiler that transforms runtime behavior into minimal, precise LLM context.**

TokenLn sits between your development environment and your LLM agent. Instead of dumping verbose CLI output into your context window, it compiles runtime behavior into a universal **Deviation IR** — the exact points where reality diverges from expectation. Nothing more.

```
cargo test (200 lines, ~15,000 tokens)         tokenln cargo test (4 lines, ~50 tokens)
─────────────────────────────────             ─────────────────────────────────────
running 127 tests                             DEVIATION {
test_auth_login ... ok                          expected: validate_token() → 401
test_auth_logout ... ok                         actual:   validate_token() → 403
test_auth_invalid_token ... FAILED              location: auth.rs:89
  assertion failed: left == right               trace:    request → middleware → auth
  left: 403, right: 401               →        confidence: 0.97
  at tests/auth.rs:127                        }
test_payment_charge ... ok
[123 more lines...]
```

Same information. One direction.

---

## Current Prototype (Implemented)

The repository currently ships a working Rust scaffold with this command path:

```bash
tokenln cargo test --from-file tests/fixtures/cargo_test/assertion_failure.txt
tokenln cargo test --from-file tests/fixtures/cargo_test/assertion_failure.txt --emit-ir
tokenln cargo build --from-file tests/fixtures/cargo_build/missing_symbol.txt
tokenln cargo build --from-file tests/fixtures/cargo_build/missing_symbol.txt --emit-ir
tokenln go test --from-file tests/fixtures/go_test/assertion_failure.txt
tokenln go test --from-file tests/fixtures/go_test/assertion_failure.txt --emit-ir
tokenln pytest --from-file tests/fixtures/pytest/assertion_failure.txt
tokenln pytest --from-file tests/fixtures/pytest/assertion_failure.txt --emit-ir
tokenln jest --from-file tests/fixtures/jest/assertion_failure.txt
tokenln jest --from-file tests/fixtures/jest/assertion_failure.txt --emit-ir
tokenln pytest --from-file tests/fixtures/pytest/assertion_failure.txt --target claude
tokenln pytest --from-file tests/fixtures/pytest/assertion_failure.txt --target ollama
tokenln pytest --from-file tests/fixtures/pytest/assertion_failure.txt --target codex
tokenln pytest --from-file tests/fixtures/pytest/assertion_failure.txt --target copilot
tokenln proxy run --from-file tests/fixtures/pytest/assertion_failure.txt --target claude -- pytest
tokenln proxy run --target claude -- pytest -q
tokenln proxy run --target claude --no-delta -- pytest -q
tokenln proxy run --allow-broad --target claude -- find .
tokenln proxy install --target claude --dir .tokenln/bin
tokenln proxy last
tokenln proxy last --raw
tokenln proxy last --ir
tokenln query --budget 400 --target claude
tokenln query --emit-json --budget 300
tokenln expand d1 --view evidence --budget 180
tokenln expand d1 --view full --budget 240 --target claude
tokenln compare --latest --previous --target claude
tokenln compare --emit-json
tokenln serve --dir .tokenln/runs --fix-log .tokenln/fix_log.jsonl
tokenln fixed d1 --note "what changed"
tokenln repo query "understand command routing" --path src --budget 260 --max-hints 4
tokenln cargo test --from-file tests/fixtures/cargo_test/assertion_failure.txt --delta
tokenln repo search "enum Commands" --path src --glob "*.rs"
tokenln repo read src/main.rs --start-line 1 --end-line 80
tokenln repo tree --path src --max-depth 2
tokenln repo log src/main.rs --limit 20
tokenln replay <run-id> --target claude
```

Current support is intentionally narrow:
- `cargo test` failure parsing
- Rust panic line variants:
  - `panicked at src/file.rs:line:col:`
  - `panicked at 'message', src/file.rs:line:col`
  - `panicked at src/file.rs:line:col: message`
- Assertion mismatch shape (`left`/`right`) extraction
- Panic reason preservation in `actual` behavior (for panic-only failures)
- `cargo build` error parsing (`error[...]`, `--> file:line:col`, `help:`)
- `cargo build` code-span capture (`N | source` and caret marker lines)
- `go test` assertion failure parsing (`--- FAIL:`, `file:line: expected X, got Y`)
- `pytest` assertion failure parsing (`FAILED file::test - ...`, `E assert ...`, `file:line: AssertionError`)
- `jest` assertion failure parsing (`● suite > test`, `Expected:`, `Received:`, stack location)
- Evidence-weighted confidence scoring (identity, location, assertion/code evidence)
- `confidence_reasons` attached to each deviation for explainable scoring
- Low-confidence fallback excerpt (`raw_excerpt`) automatically included
- Adversarial fixture coverage for conflicting evidence penalties
- Deviation IR generation (`schema_version: "0.1"`)
- Golden IR snapshot tests for fixture stability
- Emitters: `generic` (default), `claude`, `ollama`, `codex`, `copilot` (`--target <name>`)
- Command proxy runner: `tokenln proxy run -- <command...>` with broad-command guardrails (override with `--allow-broad`)
- PATH shim installer: `tokenln proxy install --target <llm> --dir .tokenln/bin`
- Delta-first proxy output (new/resolved/persistent) when a previous comparable run exists (`--no-delta` to disable)
- Direct-command delta mode via `--delta` on `cargo/go/pytest/jest` runner commands
- Phase 2 query packet prototype: `tokenln query --budget <n>`
- Phase 2 expansion prototype: `tokenln expand dN --view evidence|trace|full --budget <n>`
- Phase 2 run delta prototype: `tokenln compare --latest --previous`
- Causal grouping (`group_id`, `is_root_cause`) in optimizer + all emitters
- Fix feedback loop (`tokenln fixed`) with novelty deprioritization in `query/expand`
- MCP JSON-RPC stdio server (`tokenln serve`) with 11 tools: `analyze`, `query`, `expand`, `compare`, `fixed`, `last`, `repo_query`, `repo_search`, `repo_read`, `repo_tree`, `repo_log`
- Repo context primitives for non-debug tasks: `tokenln repo query|search|read|tree|log` with budget-bounded findings/hints and omitted-hint accounting
- Two-phase `repo_query`: Phase 1 = in-process symbol index (Rust/Python/Go/TypeScript/JavaScript); Phase 2 = content search; merged with Phase 1 precedence for definition/reference queries
- Rule-based query intent classifier routes queries to optimal search strategy (`FindDefinition`, `FindReferences`, `Understand`, `FindPattern`, `RecentChanges`, `General`)
- `tokenln replay <run-id>` re-executes the stored command and classifies deviations as fixed/still_failing/new; verdict: `all_fixed`, `partial_fix`, `fixed_with_regression`, `regression`, `no_change`
- `tokenln repo log <path>` per-file git commit history via `git log --follow`
- Cross-platform stable FNV-1a hashing in all hash-dependent modules (replaces non-deterministic `DefaultHasher`)
- Code-aware token estimation (per-4-alnum-chars + per-punctuation symbol) in all budget calculations
- Adversarial fixture coverage: ANSI color codes, multi-failure interleaving, unicode test names, empty/truncated input, deduplication
- Repo-local policy config via `.tokenln/policy.toml` for guardrails and request caps

Everything else in this README is roadmap unless explicitly marked as implemented.
Detailed references:
- `docs/IR_SPEC.md` for the concrete IR contract
- `docs/ROADMAP.md` for phased implementation milestones
- `Architecture.md` for the Phase 2 Context OS protocol and implementation order
- `docs/EXPERIMENT_PROTOCOL.md` for the validation method and success criteria

### Refresh Golden IR Snapshots

```bash
./scripts/refresh_ir_snapshots.sh
```

Scripts auto-detect `$HOME/.cargo/bin/cargo` when `cargo` is not on PATH.
You can still override explicitly:

```bash
CARGO_BIN="$HOME/.cargo/bin/cargo" ./scripts/refresh_ir_snapshots.sh
```

### Common Dev Commands

```bash
make test
make snapshots
make benchmark
make experiment
make fill-trial
make check
make ci
make demo-test
make demo-build
make demo-go-test
make demo-pytest
make demo-jest
make demo-claude
make demo-ollama
make demo-codex
make demo-copilot
make demo-proxy-pytest
make demo-query
make demo-expand
make demo-compare
```

`make benchmark` updates `docs/BENCHMARKS.md` with per-case metrics and a confidence calibration table.
`make experiment` generates validation outputs in `docs/experiment/results/`.
`make fill-trial` auto-fills one baseline/tokenln row pair in `docs/experiment/manual_trials.csv` using scripted token/time estimates.
`make ci` runs the strict CI chain and prints `CI_FAILURE stage=...` on failure.

### Validation Harness

Run the hypothesis harness:

```bash
./scripts/run_validation_experiment.sh
```

It generates:
- `docs/experiment/results/auto_metrics.csv`
- `docs/experiment/results/VALIDATION_REPORT.md`

Manual trial logging:
- Fill `docs/experiment/manual_trials.csv` with real agent runs (`baseline` vs `tokenln`).
- Re-run the script to compute success-rate and turns/time/token summaries per mode.

Auto-fill helper (estimated metrics):

```bash
./scripts/fill_manual_trial_case.sh --case pytest_assertion --agent claude-code
```

This helper estimates `tokens_in` from output length and measures command runtime locally.
Replace with real agent telemetry values when available.

### Transparent Command Proxy (Implemented)

TokenLn now supports wrapper-style command interception inspired by `rtk`, but with Deviation IR routing for supported runners.

1. Install proxy shims:

```bash
cargo run -- proxy install --target claude --dir .tokenln/bin
```

2. Prepend shim directory to `PATH` before launching your agent:

```bash
export PATH="$(pwd)/.tokenln/bin:$PATH"
```

3. Use your agent normally (`cargo`, `go`, `pytest`, `jest` commands stay the same).

Proxy behavior:
- `cargo test`, `cargo build`, `go test`, `pytest`, `jest`: compiled into TokenLn output when deviations exist.
- Zero-deviation case defaults to compact success summaries (cargo/pytest/jest/go familiar result lines + `tokenln: no deviations detected`) to cut context tokens.
- Delta-first mode is enabled by default: after at least one prior run for the same source, `proxy run` emits run deltas (`new`, `resolved`, `persistent`) instead of repeating full reports.
- Use `--no-delta` to force legacy full report / compact-success behavior.
- Use `--success-output passthrough` if you need full raw logs for successful runs.
- Full-fidelity sidecar artifacts are saved by default per analyzed run (`raw_output.txt`, `report.ir.json`, `meta.txt`) and the latest pointer is tracked at `.tokenln/runs/latest.txt`.
- Inspect latest artifacts with `tokenln proxy last`, `tokenln proxy last --raw`, or `tokenln proxy last --ir`.
- Budget-bounded context packet query (`tokenln query --budget <n>`) with utility ranking and novelty scoring from previous run signatures.
- Targeted deviation expansion (`tokenln expand dN --view evidence|trace|full --budget <n>`) from artifact-backed evidence refs.
- All other commands/subcommands: passthrough unchanged (raw stdout/stderr and exit code preserved).

Example packet query:

```bash
tokenln query --budget 400 --target claude
tokenln query --emit-json --budget 220
tokenln expand d1 --view evidence --budget 180
tokenln expand d1 --view full --budget 260 --target claude
tokenln compare --latest --previous --target claude
```

---

## The Problem

Every LLM coding agent (Claude Code, Copilot, Codex, Cursor) has the same issue:

```
To help you → needs to understand your code
To understand your code → loads it into context
Loading context → costs tokens
Tokens → cost money and hit limits fast
```

Tools like [rtk](https://github.com/rtk-ai/rtk) compress output to reduce tokens. That helps. But it's still the wrong mental model.

**The problem isn't that too many tokens are sent. It's that the wrong tokens are sent.**

When your test fails, the LLM doesn't need:
- The 124 tests that passed
- Build progress bars
- Dependency resolution logs
- Redundant file paths

It needs exactly one thing: **where does reality diverge from expectation?**

That's what TokenLn produces.

---

## How It Works

TokenLn is a **compiler pipeline**, not a text compressor:

```
Raw CLI Output
      │
      ▼
┌─────────────┐
│    Lexer    │  Tokenizes raw output into typed structures
└──────┬──────┘
       │
       ▼
┌─────────────┐
│   Parser    │  Builds semantic tree of what happened
└──────┬──────┘
       │
       ▼
┌─────────────┐
│  Semantic   │  Compares actual behavior to expected
│  Analysis   │  (types, test assertions, contracts)
└──────┬──────┘
       │
       ▼
┌─────────────┐
│  IR Gen     │  Pure deviation representation
└──────┬──────┘
       │
       ▼
┌─────────────┐
│  Optimizer  │  Removes non-deviations, deduplicates by root cause
└──────┬──────┘
       │
       ▼
┌─────────────┐
│  LLM Emit   │  Minimal, actionable context for any LLM agent
└─────────────┘
```

The magic is the **Intermediate Representation**. Like LLVM IR works for any source language and any target platform, the Deviation IR works for any language, tool, or LLM agent:

```
Python/pytest    → TokenLn → Deviation IR → Claude Code
Rust/cargo test  → TokenLn → Deviation IR → Copilot
TypeScript/jest  → TokenLn → Deviation IR → Codex
Go/go test       → TokenLn → Deviation IR → Cursor
```

---

## Token Savings (Real, Not Aspirational)

> **Honest disclaimer**: TokenLn eliminates *diagnostic* tokens (what's wrong). The LLM still needs some *repair* tokens (the relevant source lines to fix it). Real-world savings: **80-90%**, not 99%.

| Operation | Standard | TokenLn | Savings |
|-----------|----------|------|---------|
| `cargo test` (1 failure / 127 tests) | ~15,000 | ~500 | -97% |
| `pytest` (3 failures / 200 tests) | ~20,000 | ~800 | -96% |
| `jest` (2 failures / 89 tests) | ~12,000 | ~600 | -95% |
| `cargo build` (5 compiler errors) | ~3,000 | ~400 | -87% |

*Estimates based on medium-sized projects. Actual savings vary.*

---

## Installation

```bash
# Build from source
cargo build --release
./target/release/tokenln --help
```

---

## Quick Start

```bash
# 1) Compile one failure output into deviation context
tokenln pytest --from-file tests/fixtures/pytest/assertion_failure.txt --target claude

# 2) Run through proxy (captures artifacts)
tokenln proxy run --from-file tests/fixtures/pytest/assertion_failure.txt --target claude -- pytest

# 3) Query a budget-bounded packet and expand only if needed
tokenln query --budget 200 --target claude
tokenln expand d1 --view evidence --budget 180 --target claude

# 4) Compare run deltas
tokenln compare --latest --previous --target claude

# 5) Mark a deviation fixed (feedback loop)
tokenln fixed d1 --note "returned 401 for expired token"
```

---

## Command Reference

### Frontend Commands

```bash
tokenln cargo test [--emit-ir] [--target claude|ollama|codex|copilot|generic] [--delta]
tokenln cargo build [--emit-ir] [--target ...] [--delta]
tokenln go test [--emit-ir] [--target ...] [--delta]
tokenln pytest [--emit-ir] [--target ...] [--delta]
tokenln jest [--emit-ir] [--target ...] [--delta]
```

Notes:
- `--delta` (direct commands) persists artifacts and emits `new/resolved/persistent` buckets against the previous same-source run.
- `--artifacts-dir` is available on direct commands when using `--delta`.

### Proxy Commands

```bash
tokenln proxy run --target claude -- pytest -q
tokenln proxy run --no-delta --target claude -- pytest -q
tokenln proxy install --target claude --dir .tokenln/bin
tokenln proxy last
tokenln proxy last --raw
tokenln proxy last --ir
```

Notes:
- `proxy run` defaults to delta-first output when a previous comparable run exists.
- Use `--no-delta` to force full legacy brief behavior.
- Broad repo-exploration passthrough commands are blocked by policy (`find .`, recursive `ls/tree`, massive `cat`); use `--allow-broad` to bypass.
- Policy is auto-loaded from the nearest `.tokenln/policy.toml` in the current working directory ancestry.

### Context OS Commands

```bash
tokenln query --budget 400 --target claude
tokenln query --emit-json --budget 300
tokenln expand d1 --view evidence --budget 180 --target claude
tokenln expand d1 --view full --budget 240 --target claude
tokenln compare --latest --previous --target claude
tokenln compare --emit-json
tokenln replay <run-id> --target claude
tokenln replay <run-id> --emit-json
```

Notes:
- `replay` loads `<run-id>/meta.txt` to recover the original command and frontend, re-executes it, compiles fresh deviation output, and classifies each deviation as `fixed`, `still_failing`, or `new`.
- Verdict field values: `all_fixed`, `partial_fix`, `fixed_with_regression`, `regression`, `no_change`.

### MCP + Fix Log Commands

```bash
tokenln serve --dir .tokenln/runs --fix-log .tokenln/fix_log.jsonl --repo-root .
tokenln fixed d1 --note "what changed"
```

### Repo Context Commands

```bash
tokenln repo query "understand auth flow and token validation" --path src --budget 300
tokenln repo query "where should I add telemetry?" --budget 260 --max-findings 6 --max-hints 4 --target claude
tokenln repo search "validate_token" --path src --glob "*.rs"
tokenln repo search "auth middleware" --ignore-case --fixed-strings
tokenln repo read src/main.rs --start-line 1 --end-line 120
tokenln repo tree --path src --max-depth 2 --max-entries 200
tokenln repo log src/main.rs --limit 20
tokenln repo log src/repo.rs --emit-json
```

Notes:
- `repo query` uses a two-phase strategy: Phase 1 indexes symbols in-process (fast, precise for definitions); Phase 2 does content search (broader). A `QueryIntent` classifier automatically picks the best strategy.
- `repo log` runs `git log --follow --date=short` for a single file and returns structured commit entries (`hash`, `date`, `subject`).
- `repo query` budgets both findings and hints; when more hints exist than fit, output includes an omitted-hints count instead of dumping everything.
- Repo guardrails enforce bounded requests:
  - `repo query`: `budget <= 900`, `max-findings <= 16`, `max-hints <= 12`
  - `repo search`: `max-results <= 120`
  - `repo read`: line span `<= 400`, `max-chars <= 12000`
  - `repo tree`: `max-depth <= 4`, `max-entries <= 400`
- For `repo` commands and MCP repo tools, policy is loaded from `<repo-root>/.tokenln/policy.toml`.

### Policy Config (`.tokenln/policy.toml`)

This repository includes a starter policy at `.tokenln/policy.toml`.

```toml
[limits]
repo_query_budget = 600
repo_query_max_findings = 10
repo_query_max_hints = 8
repo_search_max_results = 80
repo_read_max_chars = 8000
repo_read_max_span_lines = 240
repo_tree_max_depth = 3
repo_tree_max_entries = 260
proxy_cat_max_files = 4
proxy_cat_max_file_bytes = 200000

[proxy]
block_broad_find = true
block_recursive_ls_root = true
block_tree_root_without_depth = true
block_tree_root_depth_exceeded = true
block_massive_cat = true
```

---

## The Deviation IR

The core primitive is language-agnostic, tool-agnostic, and LLM-target-agnostic.

```rust
Deviation {
    kind: DeviationKind,
    expected: Expectation,
    actual: Behavior,
    location: Location,
    trace: ExecutionTrace,
    confidence: f32,
    confidence_reasons: Vec<String>,
    raw_excerpt: Option<String>,
    summary: String,
    group_id: Option<String>,      // optional causal group id
    is_root_cause: Option<bool>,   // optional root/cascade marker
}
```

For canonical schema details, see `docs/IR_SPEC.md`.
For pipeline and protocol details, see `Architecture.md`.

---

## Status & Roadmap

> ⚠️ **This project is in early experimental phase.** The core hypothesis is being validated.

Implemented today:
- Phase 1 compiler pipeline for `cargo test`, `cargo build`, `go test`, `pytest`, `jest`.
- Proxy mode with artifact capture and delta-first output.
- Phase 2 packet commands: `query`, `expand`, `compare`, `replay`.
- Causal grouping (`group_id`, `is_root_cause`) in optimizer + emitters.
- Fix feedback loop via `tokenln fixed`.
- MCP JSON-RPC stdio server via `tokenln serve` (11 tools, including `repo_log`).
- Two-phase repo query with in-process symbol index + query intent classifier.
- Per-file git history via `tokenln repo log` / MCP `repo_log`.
- Cross-platform FNV-1a hashing and code-aware token estimation.
- Adversarial fixture suite (ANSI, unicode, multi-failure, empty, dedup).

Next priorities:
- Telemetry-backed validation (`fixes_per_token`, `turns_to_fix`, `expansion_rate`).
- Better causal/root-cause identity across iterations.
- Additional frontends where benchmark data justifies it.

For detailed milestones, see `docs/ROADMAP.md` and `docs/EXPERIMENT_PROTOCOL.md`.

---

## What We Don't Know Yet

Being honest about open problems:

```
1. Implicit expectations
   Test assertions cover ~20% of what developers actually expect.
   The other 80% lives in their head. We don't have this yet.

2. Execution tracing without running code
   Static call graphs ≠ runtime execution paths.
   Dynamic dispatch, async boundaries, conditionals break static analysis.

3. Cross-system deviations
   frontend → backend → database traces.
   Highest-value bugs. Hardest to capture.

4. When the expectation itself is wrong
   The test might be the bug, not the code.
   How do we know the difference?
```

These aren't footnotes. They're the actual hard problems. We're working on them.

---

## Comparison

| | rtk | TokenLn |
|---|---|---|
| **Approach** | Text compression | Semantic compilation |
| **Information loss** | Yes (heuristic) | No (deviation-only) |
| **Language support** | Any (heuristic) | Explicit passes per language |
| **LLM agent support** | Any (stdout) | Any (emitters + MCP tools) |
| **Token savings goal** | 60-90% | 80-95% (hypothesis under validation) |
| **Runtime mode** | Inline CLI | Inline CLI + stdio MCP server |
| **Granularity** | Lossy | Lossless on deviations |

TokenLn is complementary to `rtk`: compression and semantic deviation extraction can be used together.

---

## Contributing

Contributions welcome. The most valuable contributions right now:

- **New language frontends** (Lexer + Parser for new test runners/build tools)
- **IR improvements** (better deviation representation for edge cases)
- **Benchmark bugs** (real bugs to validate the hypothesis)
- **LLM emitters** (optimized output for different agents)

Before submitting changes, run:

```bash
make ci
make experiment
```

Primary design docs:
- `Architecture.md`
- `docs/IR_SPEC.md`
- `docs/ROADMAP.md`
- `docs/EXPERIMENT_PROTOCOL.md`

---

## License

MIT — see [LICENSE](LICENSE).

---

## About

TokenLn is built on one insight:

**Developers don't debug by reading code. They trace execution and find where actual behavior diverges from expected behavior.**

Current LLM tools ignore this. They dump everything and hope. TokenLn compiles the signal from the noise.

The compiler analogy is precise. A compiler transforms what humans write (verbose, redundant) into what machines execute (precise, minimal). TokenLn transforms what machines produce (verbose, noisy) into what LLMs reason about (precise, minimal).

Same problem. Inverted.
