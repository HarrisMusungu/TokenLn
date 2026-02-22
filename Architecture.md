# TokenLn Architecture

## Overview

TokenLn is a compiler. Not a text compressor, not a RAG system, not a context manager. A compiler.

It takes one representation (verbose CLI output) and transforms it into another (minimal deviation IR), preserving 100% of semantically relevant information while eliminating everything else.

## The Compiler Pipeline

```
Raw CLI Output
      │
      ▼
┌─────────────────────────────────┐
│  Stage 1: Lexer                 │
│  src/lexers/                    │
│                                 │
│  Tokenizes raw output into      │
│  typed tokens. One lexer per    │
│  tool (cargo, pytest, jest).    │
└──────────────┬──────────────────┘
               │  Vec<Token>
               ▼
┌─────────────────────────────────┐
│  Stage 2: Parser                │
│  src/parsers/                   │
│                                 │
│  Builds semantic tree from      │
│  tokens. Understands structure  │
│  of test suites, build graphs,  │
│  diff hunks.                    │
└──────────────┬──────────────────┘
               │  SemanticNode
               ▼
┌─────────────────────────────────┐
│  Stage 3: Semantic Analyzer     │
│  src/analysis/                  │
│                                 │
│  Compares actual behavior to    │
│  expected behavior. Queries     │
│  codebase for type info, call   │
│  graphs, contracts. Scores      │
│  confidence from evidence.      │
└──────────────┬──────────────────┘
               │  Vec<Deviation>
               ▼
┌─────────────────────────────────┐
│  Stage 4: IR Generator          │
│  src/ir.rs                      │
│                                 │
│  Converts raw deviations into   │
│  canonical Deviation IR.        │
│  Language-agnostic.             │
│  Tool-agnostic. LLM-agnostic.   │
└──────────────┬──────────────────┘
               │  DeviationReport
               ▼
┌─────────────────────────────────┐
│  Stage 5: Optimizer             │
│  src/optimizer.rs               │
│                                 │
│  Removes non-deviations.        │
│  Deduplicates by root cause.    │
│  Ranks by severity.             │
│  Groups related deviations.     │
│  Adds low-confidence fallback.  │
└──────────────┬──────────────────┘
               │  DeviationReport (optimized)
               ▼
┌─────────────────────────────────┐
│  Stage 6: Emitter               │
│  src/emitters/                  │
│                                 │
│  Renders for specific LLM.      │
│  One emitter per target         │
│  (generic, claude, ollama,      │
│   codex, copilot).              │
└──────────────┬──────────────────┘
               │  String (LLM context)
               ▼
         LLM Agent
```

## The Deviation IR

The core primitive. Defined in `src/ir.rs`.

```rust
struct Deviation {
    kind: DeviationKind,       // test, type, build, runtime, behavioral
    expected: Expectation,     // what should have happened
    actual: Behavior,          // what did happen
    location: Location,        // where in the codebase
    trace: ExecutionTrace,     // how we got here
    confidence: f32,           // how sure are we
    summary: String,           // one-line human readable
}
```

**Key property**: The IR is the same regardless of source language or target LLM. A Python pytest failure and a Rust cargo test failure produce the same IR structure. A deviation report for Claude looks the same as one for Copilot (before final emission).

## Adding a New Language Frontend

To support a new test runner:

1. Implement `Lexer` in `src/lexers/your_tool.rs`
2. Implement `Parser` in `src/parsers/your_tool.rs`
3. Implement `SemanticAnalyzer` in `src/analysis/your_tool.rs`
4. Register in `src/main.rs`

The IR, Optimizer, and Emitters require no changes.

## Adding a New LLM Target

To emit for a new LLM agent:

1. Implement `Emitter` in `src/emitters/your_llm.rs`
2. Register in `src/main.rs`

The entire pipeline up to Stage 5 requires no changes.

## Command Proxy Layer (Implemented)

TokenLn now includes a transparent command proxy path:

```bash
tokenln proxy run --target claude -- pytest -q
tokenln proxy install --target claude --dir .tokenln/bin
```

Design:
- `proxy run` executes the requested command, then routes output through the compiler pipeline only for supported frontends.
- Supported compiler routes: `cargo test`, `cargo build`, `go test`, `pytest`, `jest`.
- Zero-deviation output uses compact success summaries by default (token control while preserving expected tool-style status lines).
- Optional compatibility override: `--success-output passthrough` returns full raw output for successful runs.
- Full-fidelity artifacts are still preserved on disk per run (`.tokenln/runs/run-*/raw_output.txt`, `report.ir.json`, `meta.txt`), with `.tokenln/runs/latest.txt` pointing to the newest run.
- `tokenln proxy last --raw|--ir` provides fast retrieval without increasing default agent context.
- Unsupported commands/subcommands remain passthrough by default, with policy guardrails blocking broad exploration patterns (root `find`, recursive root `ls/tree`, massive `cat`) unless `--allow-broad` is set.
- Proxy guardrail settings are repo-local and loaded from the nearest `.tokenln/policy.toml` in the working-directory ancestry.
- `proxy install` writes PATH shims (`cargo`, `go`, `pytest`, `jest`) that call back into TokenLn.

## Phase 2: Context OS (In Progress)

Goal: optimize for `fixes_per_token`, not just token reduction.

Current prototype:
- `tokenln query --budget <n> [--emit-json]` reads latest run artifacts and emits a deterministic budget-bounded context packet.
- `tokenln expand dN --view evidence|trace|full --budget <n>` expands one deviation with artifact-backed evidence slices.
- `tokenln compare --latest --previous [--emit-json]` reports new/resolved/persistent deviations between runs.
- `tokenln replay <run-id> [--emit-json] [--target ...]` re-executes the stored command, compiles fresh output, and classifies deviations as fixed / still_failing / new.
- `tokenln proxy run` now defaults to delta-first output when a previous comparable run exists (same source).
- direct runner commands (`cargo/go/pytest/jest`) support `--delta` for artifact-backed run-to-run diffs.
- `tokenln fixed dN` records fix outcomes and feeds novelty deprioritization in subsequent `query/expand`.
- `tokenln repo log <path>` shows per-file git commit history (`git log --follow`).

### Core Idea

Phase 1 compiles logs into a compact deviation report.
Phase 2 adds an interactive context protocol so the agent receives only the minimum required context first, then requests more detail incrementally.

Think of it as:
- Phase 1: `compile`
- Phase 2: `compile + budget + queryable evidence`

### Runtime Flow

```
Real command execution
      │
      ▼
Deviation IR + artifacts (raw_output, report.ir.json, meta)
      │
      ▼
Context Budget Controller
      │
      ▼
Context Packet (budget-bound, prioritized deviations)
      │
      ▼
Agent chooses one of:
  - patch based on packet
  - expand specific deviation evidence
  - compare with previous run
```

### Phase 2 Protocol (CLI Surface)

Implemented commands:

```bash
tokenln query --budget 400 --target claude
tokenln expand <deviation_id> --view evidence --budget 180
tokenln compare --latest --previous
tokenln replay <run-id> [--emit-json] [--target claude]
tokenln repo log src/main.rs --limit 20
```

Behavior:
- `query`: returns a budget-bounded packet ranked by root-cause utility.
- `expand`: returns additional slices for one deviation (trace, snippets, raw lines, confidence proof).
- `compare`: reports what is new/resolved/regressed between two runs.
- `replay`: re-executes the stored command from `meta.txt`, compiles fresh deviation output, and classifies each as `fixed`, `still_failing`, or `new`. Verdict values: `all_fixed`, `partial_fix`, `fixed_with_regression`, `regression`, `no_change`.
- `repo log`: runs `git log --follow --date=short -n <limit>` for a single file and emits structured commit entries.

### Context Packet Contract

```rust
struct ContextPacket {
    packet_id: String,
    run_id: String,
    budget_tokens: u32,
    used_tokens: u32,
    source: String,
    objective: String,              // e.g. "fix test failures"
    deviations: Vec<DeviationSlice>,
    expansion_hints: Vec<ExpansionHint>,
    unresolved_count: u32,
}

struct DeviationSlice {
    id: String,
    summary: String,
    expected: String,
    actual: String,
    location: String,
    confidence: f32,
    novelty_score: f32,             // new vs repeated from prior runs
    utility_score: f32,             // ranking score used by budget controller
    evidence_refs: Vec<EvidenceRef>,// proof-carrying references into artifacts
}

struct EvidenceRef {
    artifact: String,               // raw_output.txt or report.ir.json
    line_start: u32,
    line_end: u32,
    hash: String,                   // deterministic anti-hallucination proof pointer
}
```

### Budget Controller (Deterministic)

Packet assembly algorithm:

1. Score each deviation by `severity * confidence * novelty * fixability`.
2. Reserve minimum budget per deviation for top-K root causes.
3. Greedy-pack remaining budget using highest marginal utility per token.
4. Emit `expansion_hints` for anything truncated.

Result: two runs with same inputs produce the same packet ordering and truncation points.

### Groundbreaking Differentiators

1. Proof-carrying deviations:
each summary line maps to exact artifact spans + hash.
2. Causal memory graph:
store root-cause identity across runs; avoid re-sending old resolved noise.
3. Delta-first agent loop:
every new run answers "what changed since the last fix attempt?"
4. Adaptive budgeting:
learn per-repo/agent defaults from observed fix success.

### Safety and Quality Guardrails

- If confidence is low, budget controller auto-allocates more evidence.
- If parser uncertainty is high, emit raw excerpt pointers with explicit uncertainty flags.
- Never fabricate omitted context: omitted sections are represented as expansion hints.
- All summaries remain reproducible from saved artifacts.

### Implementation Order (Concrete)

Sprint 1: Queryable packets ✓
- `tokenln query` for latest run artifact.
- Deterministic scoring + budget packing.
- Packet JSON and human-readable target-specific render.

Sprint 2: Expansion + delta loops ✓
- `tokenln expand <deviation_id>`.
- `tokenln compare` across runs.
- Root-cause identity and novelty scoring.

Sprint 3: Replay + repo intelligence ✓
- `tokenln replay <run-id>` iterative fix loop (fixed/still_failing/new classification).
- `tokenln repo log` per-file git history.
- Two-phase `repo_query`: Phase 1 = symbol index (definition/reference queries), Phase 2 = content search; results merged with Phase 1 precedence.
- Rule-based `QueryIntent` classifier routing queries to optimal search strategy.
- Language-aware symbol index (Rust, Python, Go, TypeScript/JavaScript) — no external crate dependency.
- Cross-platform stable FNV-1a hashing (replaces `DefaultHasher`).
- Code-aware token estimation (per-symbol + per-punctuation charging).
- Adversarial fixture coverage: ANSI codes, multi-failure interleaving, unicode test names, empty/truncated input, deduplication.

Sprint 4: Adaptive policies (planned)
- Repository-local budget profiles.
- Telemetry for `fixes_per_token`, `turns_to_fix`, `expansion_rate`.

### Success Criteria

Phase 2 is considered successful when:
- Median context tokens per successful fix drop by 40%+ from Phase 1.
- Median turns-to-fix drop by 20%+.
- 95%+ of emitted claims can be traced to artifact evidence refs.
- Repeated failure runs show stable root-cause IDs and deterministic ordering.

## MCP Server (Implemented)

TokenLn now exposes a JSON-RPC 2.0 stdio server:

```
tokenln serve --dir .tokenln/runs --fix-log .tokenln/fix_log.jsonl
```

Current MCP tools (11):
- `analyze` (compile raw output into IR)
- `query` (budget-bounded context packet)
- `expand` (deviation-specific evidence expansion)
- `compare` (run deltas)
- `fixed` (record fix feedback)
- `last` (retrieve latest run artifacts)
- `repo_query` (budget-bounded repository context packet with two-phase symbol+content search)
- `repo_search` (rg-style repository search with fallback)
- `repo_read` (safe bounded file reads)
- `repo_tree` (compact repository structure map)
- `repo_log` (git commit history for a file via `git log --follow`)
- Repo tool limits/guardrails are configurable via `<repo-root>/.tokenln/policy.toml`.

## Local Daemon (Future)

A persistent daemon mode (`serve --daemon`) over local sockets remains a future Phase 3 target for:
- Filesystem watching (inotify / FSEvents)
- Incremental indexing (only re-analyze changed files)
- Pre-built deviation context before agent queries
- Long-lived state and repository-local policies

## File Structure

```
src/
├── main.rs           CLI entry point
├── proxy.rs          Command classification + wrapper helpers (binary module)
├── context.rs        Context packet builder + budgeting (Phase 2 prototype)
├── fixlog.rs         Fix feedback loop persistence (`tokenln fixed`)
├── mcp.rs            MCP JSON-RPC stdio server (`tokenln serve`) — 11 tools
├── policy.rs         Guardrails for broad exploration + bounded repo requests
├── repo.rs           Repo context primitives (`repo query/search/read/tree/log`)
├── symbol_index.rs   Language-aware in-process symbol scanner (Rust/Python/Go/TS/JS)
├── query_intent.rs   Rule-based query intent classifier (FindDefinition/FindReferences/…)
├── ir.rs             Deviation IR types
├── pipeline.rs       Trait definitions for all stages
├── lib.rs            Module exports
├── postprocess.rs    Confidence fallback enrichment
├── lexers/
│   ├── mod.rs
│   ├── cargo_test.rs   ← Implemented (Phase 1)
│   ├── cargo_build.rs  ← Implemented (Phase 1)
│   ├── pytest.rs      ← Implemented (Phase 1)
│   ├── jest.rs        ← Implemented (Phase 1)
│   └── go_test.rs     ← Implemented (Phase 1)
├── parsers/
│   ├── mod.rs
│   ├── cargo_test.rs  ← Implemented (Phase 1)
│   ├── cargo_build.rs ← Implemented (Phase 1)
│   ├── pytest.rs      ← Implemented (Phase 1)
│   ├── jest.rs        ← Implemented (Phase 1)
│   └── go_test.rs     ← Implemented (Phase 1)
├── analysis/
│   ├── mod.rs
│   ├── cargo_test.rs  ← Implemented (Phase 1)
│   ├── cargo_build.rs ← Implemented (Phase 1)
│   ├── pytest.rs      ← Implemented (Phase 1)
│   ├── jest.rs        ← Implemented (Phase 1)
│   └── go_test.rs     ← Implemented (Phase 1)
├── optimizer.rs       ← Implemented (basic dedupe/ranking)
└── emitters/
    ├── mod.rs
    ├── generic.rs     ← Implemented (Phase 1)
    ├── claude.rs      ← Implemented (Phase 1)
    ├── ollama.rs      ← Implemented (Phase 1)
    ├── codex.rs       ← Implemented (Phase 1)
    └── copilot.rs     ← Implemented (Phase 1)

tests/
├── fixtures/
│   ├── cargo_test/        Raw cargo test outputs (incl. ANSI, unicode, multi-failure)
│   ├── cargo_build/       Raw cargo build outputs (incl. adversarial cases)
│   ├── pytest/            Raw pytest outputs
│   ├── jest/              Raw jest outputs
│   ├── go_test/           Raw go test outputs
│   └── expected_ir/       Golden IR snapshots (Phase 1)
├── adversarial_pipeline.rs Behavioral adversarial coverage (ANSI, unicode, empty, dedup…)
├── cargo_test_pipeline.rs  Integration coverage for cargo test frontend
├── cargo_build_pipeline.rs Integration coverage for cargo build frontend
├── pytest_pipeline.rs      Integration coverage for pytest frontend
├── jest_pipeline.rs        Integration coverage for jest frontend
└── go_test_pipeline.rs     Integration coverage for go test frontend

docs/
├── IR_SPEC.md        Deviation IR specification
├── BENCHMARKS.md     Experiment results
├── EXPERIMENT_PROTOCOL.md Validation protocol and success criteria
└── ROADMAP.md        Detailed roadmap

scripts/
├── refresh_ir_snapshots.sh  Regenerates Phase 1 golden IR fixtures
├── benchmark_phase1.sh      Generates repeatable Phase 1 benchmark report
├── run_validation_experiment.sh Generates hypothesis-validation report + CSVs
├── fill_manual_trial_case.sh Auto-fills trial CSV rows with measured/estimated values
└── run_ci.sh                Strict CI chain with stage-based failure summaries

Makefile               test/snapshot/benchmark/experiment/check/ci/demo shortcuts
```

## Design Decisions

### Why a Compiler, Not a Compressor?

Compressors reduce tokens while preserving (most) information.
Compilers transform representation while preserving all semantics.

The difference matters: a compressor of test output might keep 30% of lines.
A compiler of test output keeps 100% of deviation information and 0% of non-deviation noise.

These are different goals. Compression is lossy. Compilation is semantic.

### Why an IR?

Without an IR, every new tool requires changes to every LLM emitter.
N tools × M LLMs = N×M implementations.

With an IR:
N tools × 1 IR + 1 IR × M LLMs = N+M implementations.

This is the same reason LLVM uses an IR. Same logic, same solution.

### Why Local?

Privacy. Your code never leaves your machine.
Latency. No network round trips.
Cost. No API calls for indexing.

The current MCP server is local-first (stdio). A future daemon can stay local-first with optional sync metadata only.
