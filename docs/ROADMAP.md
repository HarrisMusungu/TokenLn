# TokenLn Roadmap

## Phase 1 (Current)

Status: complete.

Completed:
- `cargo test` frontend (lexer/parser/analyzer)
- `cargo build` frontend (lexer/parser/analyzer)
- `go test` frontend (lexer/parser/analyzer)
- `pytest` frontend (lexer/parser/analyzer)
- `jest` frontend (lexer/parser/analyzer)
- Deviation IR emission (`schema_version: "0.1"`)
- Multi-emitter output targets (`generic`, `claude`, `ollama`, `codex`, `copilot`)
- Command proxy layer (`proxy run`, `proxy install`, passthrough fallback for unsupported commands) with broad-command guardrails + override
- Confidence scoring with explainable `confidence_reasons`
- Low-confidence fallback (`raw_excerpt`)
- Golden IR snapshots + integration tests
- Adversarial fixture suite: ANSI color codes, multi-failure interleaving, unicode test names, empty/truncated input, deduplication (`tests/adversarial_pipeline.rs`)
- Benchmark harness (`docs/BENCHMARKS.md`)
- Validation harness + protocol scaffold (`scripts/run_validation_experiment.sh`, `docs/EXPERIMENT_PROTOCOL.md`)
- Strict stage-based CI script (`scripts/run_ci.sh`)

## Phase 2

Goal: Context OS protocol + adaptive context budgeting.

Implemented:
- `tokenln query --budget <n>` budget-bounded context packets
- `tokenln expand <deviation_id>` targeted evidence expansion
- deterministic utility scoring + budget controller
- `tokenln compare --latest --previous` delta-first diagnostics
- `tokenln replay <run-id>` iterative fix loop — re-executes stored command, classifies deviations as fixed/still_failing/new, emits verdict (`all_fixed`, `partial_fix`, `fixed_with_regression`, `regression`, `no_change`)
- delta-first proxy output default for iterative runs
- `tokenln serve` MCP JSON-RPC stdio server (11 tools)
- `tokenln fixed` feedback loop for deprioritizing previously fixed signatures
- repo context primitives (`tokenln repo query/search/read/tree/log`, MCP `repo_query/repo_search/repo_read/repo_tree/repo_log`)
- `tokenln repo log` per-file git commit history (`git log --follow`)
- two-phase `repo_query`: Phase 1 = in-process symbol index (Rust/Python/Go/TypeScript/JavaScript); Phase 2 = content search
- rule-based `QueryIntent` classifier (`FindDefinition`, `FindReferences`, `Understand`, `FindPattern`, `RecentChanges`, `General`)
- policy enforcement for bounded repo context requests (query/search/read/tree caps)
- repo-local policy config (`.tokenln/policy.toml`) for guardrail and limit customization
- cross-platform stable FNV-1a hashing (replaces `DefaultHasher`)
- code-aware token estimation (per-4-alnum-chars + per-punctuation)
- full-fidelity artifacts as proof source for all emitted claims

Remaining:
- root-cause identity graph + novelty scoring across runs (partial — novelty score present, graph not persisted)
- telemetry: `fixes_per_token`, `turns_to_fix`, `expansion_rate`

Stretch:
- Additional frontend variants (`vitest`, `rspec`, `next build`)
- Richer deduplication/root-cause grouping in optimizer
- Configurable confidence thresholds

## Phase 3

Goal: local daemon for incremental analysis.

Planned:
- `tokenln serve --daemon`
- filesystem watch and incremental indexing
- cached context over local socket
