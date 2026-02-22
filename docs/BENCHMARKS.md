# Phase 1 Benchmarks

_Generated on 2026-02-18 20:15:04Z_

| Case | Raw words | Emitter words | IR words | Emitter savings | Confidence | Fallback |
|---|---:|---:|---:|---:|---:|---|
| `cargo_test_assertion` | 59 | 40 | 68 | 32.2% | 0.99 | no |
| `cargo_test_panic_low_conf` | 52 | 72 | 98 | -38.5% | 0.80 | yes |
| `cargo_build_missing_symbol` | 59 | 69 | 97 | -16.9% | 0.99 | no |
| `cargo_build_conflicting_evidence` | 54 | 87 | 114 | -61.1% | 0.80 | yes |
| `go_test_assertion` | 13 | 39 | 67 | -200.0% | 0.95 | no |
| `pytest_assertion` | 59 | 38 | 65 | 35.6% | 0.95 | no |
| `jest_assertion` | 54 | 47 | 78 | 13.0% | 0.99 | no |

## Confidence Calibration

| Bucket | Range | Cases | Avg confidence | Fallback rate |
|---|---|---:|---:|---:|
| `low` | <0.85 | 2 | 0.80 | 100.0% |
| `medium` | 0.85-0.95 | 0 | 0.00 | 0.0% |
| `high` | >=0.95 | 5 | 0.97 | 0.0% |

Notes:
- Word counts are a lightweight proxy for token usage in this Phase 1 harness.
- `Fallback=yes` means confidence was below threshold and raw excerpt enrichment was applied.
- Small fixtures can show negative savings; benchmark focus is confidence behavior and pipeline stability.
- As of 2026-02-22, internal token estimation uses a code-aware algorithm (1 token per 4 alphanumeric chars + 1 token per punctuation symbol) rather than `chars/4`. This improves accuracy for code-heavy context packets but does not affect the Phase 1 word-count benchmarks above. Re-run `make benchmark` after any pipeline change to refresh.
