# Deviation IR Spec (`schema_version: "0.1"`)

This document defines the current JSON contract emitted by `tokenln --emit-ir`.

## Top-Level Shape

```json
{
  "schema_version": "0.1",
  "source": "cargo test | cargo build | go test | pytest | jest",
  "deviations": []
}
```

Fields:
- `schema_version` (`string`): Current schema identifier.
- `source` (`string`): Frontend that produced this report.
- `deviations` (`Deviation[]`): Zero or more normalized deviation records.

## Deviation Object

```json
{
  "kind": "test | build | type | runtime | behavioral",
  "expected": { "description": "..." },
  "actual": { "description": "..." },
  "location": {
    "file": "optional path",
    "line": 1,
    "column": 1,
    "symbol": "optional symbol/test id"
  },
  "trace": { "frames": ["..."] },
  "confidence": 0.95,
  "confidence_reasons": ["..."],
  "raw_excerpt": "optional fallback snippet",
  "summary": "one-line summary",
  "group_id": "optional causal group id",
  "is_root_cause": true
}
```

Field notes:
- `confidence` is normalized to two decimals and clamped to `[0.20, 0.99]`.
- `confidence_reasons` explains the evidence and penalties used in scoring.
- `raw_excerpt` is omitted unless low-confidence fallback enrichment is applied.
- `group_id` is optional and is assigned when multiple deviations share a file.
- `is_root_cause` is optional and marks root (`true`) vs cascade (`false`) within a group.

## Stability Rules

- `schema_version` changes only for breaking JSON contract changes.
- New optional fields may be added without bumping major schema version.
- Existing required fields must not be removed in `0.x` patch updates.

## Current Frontends

- `cargo test`
- `cargo build`
- `go test`
- `pytest`
- `jest`
