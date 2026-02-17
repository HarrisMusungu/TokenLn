# devc — Dev Compiler

[![License: MIT](https://img.shields.io/badge/License-MIT-yellow.svg)](https://opensource.org/licenses/MIT)
[![Status: Experimental](https://img.shields.io/badge/Status-Experimental-orange.svg)]()

**A compiler that transforms runtime behavior into minimal, precise LLM context.**

devc sits between your development environment and your LLM agent. Instead of dumping verbose CLI output into your context window, it compiles runtime behavior into a universal **Deviation IR** — the exact points where reality diverges from expectation. Nothing more.

```
cargo test (200 lines, ~15,000 tokens)         devc cargo test (4 lines, ~50 tokens)
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

That's what devc produces.

---

## How It Works

devc is a **compiler pipeline**, not a text compressor:

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
Python/pytest    → devc → Deviation IR → Claude Code
Rust/cargo test  → devc → Deviation IR → Copilot
TypeScript/jest  → devc → Deviation IR → Codex
Go/go test       → devc → Deviation IR → Cursor
```

---

## Token Savings (Real, Not Aspirational)

> **Honest disclaimer**: devc eliminates *diagnostic* tokens (what's wrong). The LLM still needs some *repair* tokens (the relevant source lines to fix it). Real-world savings: **80-90%**, not 99%.

| Operation | Standard | devc | Savings |
|-----------|----------|------|---------|
| `cargo test` (1 failure / 127 tests) | ~15,000 | ~500 | -97% |
| `pytest` (3 failures / 200 tests) | ~20,000 | ~800 | -96% |
| `jest` (2 failures / 89 tests) | ~12,000 | ~600 | -95% |
| `git diff` (complex refactor) | ~8,000 | ~1,200 | -85% |
| Build errors (5 errors) | ~3,000 | ~400 | -87% |

*Estimates based on medium-sized projects. Actual savings vary.*

---

## Installation

```bash
# Quick install (Linux/macOS)
curl -fsSL https://raw.githubusercontent.com/your-org/devc/master/install.sh | sh

# Verify
devc --version
devc status
```

### From source

```bash
cargo install --git https://github.com/your-org/devc
```

---

## Quick Start

```bash
# Initialize for your LLM agent
devc init --claude    # Claude Code
devc init --copilot   # GitHub Copilot
devc init --generic   # Any agent via stdout

# Run commands through the compiler
devc cargo test
devc pytest
devc jest
devc go test ./...

# See what the IR looks like
devc cargo test --emit-ir

# Check token savings
devc stats
```

---

## Commands

### Test Runners

```bash
devc cargo test              # Rust — failures only with deviation IR
devc cargo test --emit-ir    # Show raw IR before LLM emission
devc pytest                  # Python — assertion deviations
devc jest                    # JavaScript/TypeScript
devc go test ./...           # Go
devc vitest run              # Vite/TypeScript
devc rspec                   # Ruby
```

### Build Tools

```bash
devc cargo build             # Rust build errors as deviations
devc npm run build           # JS/TS build failures
devc tsc                     # TypeScript type errors
devc next build              # Next.js
```

### Git

```bash
devc git diff                # Semantic diff (what changed, not how)
devc git status              # Compact, structured status
devc git log -n 10           # One-line semantic commits
```

### Analysis

```bash
devc stats                   # Token savings summary
devc stats --graph           # ASCII graph of last 30 days
devc stats --history         # Recent command history
devc ir show <command>       # Show IR for last command
devc ir diff                 # Deviation diff between two runs
```

---

## The Deviation IR

The core primitive of devc. Language-agnostic, tool-agnostic, LLM-agnostic.

```rust
// Every deviation looks like this:
Deviation {
    // What was expected (from type system, tests, contracts)
    expected: Expectation,

    // What actually happened (from runtime, output, traces)
    actual: Behavior,

    // Where the divergence occurs
    location: Location,

    // How we got here
    trace: ExecutionPath,

    // How confident we are in this deviation
    confidence: f32,
}
```

**Examples:**

```
// Test assertion failure
DEVIATION [test]
  expected: status_code == 401
  actual:   status_code == 403
  location: auth.rs:89
  trace:    test_auth_invalid → validate_token → token_expired
  confidence: 0.99

// Type error
DEVIATION [type]
  expected: String
  actual:   &str
  location: parser.rs:142
  context:  fn parse_input(s: String) called with &str
  confidence: 1.0

// Build error
DEVIATION [build]
  expected: field `user_id` exists on struct `Session`
  actual:   field `user_id` not found
  location: session.rs:67
  hint:     renamed to `account_id` in session.rs:12
  confidence: 0.95
```

---

## Integrations

### Claude Code

```bash
devc init --claude
# Installs hook to ~/.claude/settings.json
# All Bash commands automatically compiled before reaching Claude
```

### Generic (any agent)

```bash
# Pipe devc output to any LLM agent
devc cargo test | your-llm-agent
```

### MCP Server

```bash
# Run as MCP server for direct LLM integration
devc serve --mcp
```

---

## Architecture

```
┌──────────────────────────────────────────────────┐
│                 LOCAL SERVER                      │
│                                                  │
│  Watchers                                        │
│  ├── Filesystem (inotify / FSEvents)             │
│  ├── Terminal (PTY capture)                      │
│  ├── Git (hook integration)                      │
│  └── Test runner (output capture)               │
│                     │                            │
│  Compiler Pipeline                               │
│  ├── Lexer           parse raw output            │
│  ├── Parser          build semantic tree         │
│  ├── Semantic        actual vs expected          │
│  ├── IR Gen          pure deviation              │
│  ├── Optimizer       remove non-deviations       │
│  └── LLM Emit        minimal context             │
│                     │                            │
│  State (minimal RAM)                             │
│  ├── Call graph      structural, ~5MB            │
│  ├── Type contracts  expectations, ~2MB          │
│  └── Deviation log   ring buffer, bounded        │
└──────────────────────────────────────────────────┘
                     │
             ~500 tokens/query
             (not 15,000)
                     │
┌──────────────────────────────────────────────────┐
│              ANY LLM AGENT                        │
│  Claude Code / Copilot / Codex / Cursor / etc.   │
│                                                  │
│  Receives:  pure deviation context               │
│  Knows:     exactly where reality broke          │
│  Acts:      immediately, correctly               │
└──────────────────────────────────────────────────┘
```

---

## Status & Roadmap

> ⚠️ **This project is in early experimental phase.** The core hypothesis is being validated.

### Phase 1: Prove the Hypothesis *(current)*

The one question that matters:

> *"If you show an LLM only the deviation between expected and actual behavior, can it fix bugs with dramatically fewer tokens AND equal or better accuracy?"*

We're testing this with 50 real Rust bugs, comparing:
- Raw `cargo test` output
- `rtk` compressed output  
- `devc` deviation IR

**If equal accuracy at 80%+ fewer tokens → we build the rest.**

### Phase 2: Define the IR

Make the Deviation IR expressive enough for real bugs, constrained enough to be tractable. Language-agnostic, tool-agnostic, LLM-agnostic.

### Phase 3: Local Server

Persistent local process. Watches filesystem, terminal, git. Pre-builds deviation context before the LLM asks. Only after IR is validated.

### Phase 4: Language Frontends

Compiler passes for each language/tool. Community-extensible like LLVM passes.

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

| | rtk | devc |
|---|---|---|
| **Approach** | Text compression | Semantic compilation |
| **Information loss** | Yes (heuristic) | No (deviation-only) |
| **Language support** | Any (heuristic) | Explicit passes per language |
| **LLM agent support** | Any (stdout) | Any (IR + emitter) |
| **Token savings** | 60-90% | 80-95% (realistic) |
| **Latency** | None (inline) | ~10ms (local server) |
| **Granularity** | Lossy | Lossless on deviations |

devc is **rtk's successor**, not its replacement. rtk compresses the haystack. devc shows you the needle.

---

## Contributing

Contributions welcome. The most valuable contributions right now:

- **New language frontends** (Lexer + Parser for new test runners/build tools)
- **IR improvements** (better deviation representation for edge cases)
- **Benchmark bugs** (real bugs to validate the hypothesis)
- **LLM emitters** (optimized output for different agents)

See [CONTRIBUTING.md](CONTRIBUTING.md) and [ARCHITECTURE.md](ARCHITECTURE.md).

**For external contributors**: PRs undergo automated security review. See [SECURITY.md](SECURITY.md).

---

## License

MIT — see [LICENSE](LICENSE).

---

## About

devc is built on one insight:

**Developers don't debug by reading code. They trace execution and find where actual behavior diverges from expected behavior.**

Current LLM tools ignore this. They dump everything and hope. devc compiles the signal from the noise.

The compiler analogy is precise. A compiler transforms what humans write (verbose, redundant) into what machines execute (precise, minimal). devc transforms what machines produce (verbose, noisy) into what LLMs reason about (precise, minimal).

Same problem. Inverted.