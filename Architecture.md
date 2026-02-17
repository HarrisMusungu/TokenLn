# devc Architecture

## Overview

devc is a compiler. Not a text compressor, not a RAG system, not a context manager. A compiler.

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
│  graphs, contracts.             │
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
└──────────────┬──────────────────┘
               │  DeviationReport (optimized)
               ▼
┌─────────────────────────────────┐
│  Stage 6: Emitter               │
│  src/emitters/                  │
│                                 │
│  Renders for specific LLM.      │
│  One emitter per target         │
│  (claude, copilot, generic).    │
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

## The Local Server (Future)

Phase 3 adds a persistent local server:

```
devc serve --daemon
```

This provides:
- Filesystem watching (inotify / FSEvents)
- Incremental indexing (only re-analyze changed files)
- Pre-built deviation context (before LLM is invoked)
- State tracking across queries

Server communicates via Unix socket. CLI tools query the server instead of running the pipeline on each invocation.

## File Structure

```
src/
├── main.rs           CLI entry point
├── ir.rs             Deviation IR types
├── pipeline.rs       Trait definitions for all stages
├── lexers/
│   ├── mod.rs
│   ├── cargo_test.rs  ← Implemented
│   ├── pytest.rs      ← Planned
│   ├── jest.rs        ← Planned
│   └── go_test.rs     ← Planned
├── parsers/
│   ├── mod.rs
│   └── cargo_test.rs  ← Planned
├── analysis/
│   ├── mod.rs
│   └── cargo_test.rs  ← Planned
├── optimizer.rs       ← Planned
└── emitters/
    ├── mod.rs
    ├── claude.rs      ← Planned
    ├── copilot.rs     ← Planned
    └── generic.rs     ← Planned

tests/
├── fixtures/
│   ├── benchmark_bugs.md  50 real bugs for validation
│   ├── cargo_test/        Raw cargo test outputs
│   └── expected_ir/       Expected deviation IR outputs
└── integration/
    └── hypothesis_test.rs The core experiment

docs/
├── IR_SPEC.md        Deviation IR specification
├── BENCHMARKS.md     Experiment results
└── ROADMAP.md        Detailed roadmap
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

The server (Phase 3) is local-first. Cloud sync is optional and ships no code, only indexes.