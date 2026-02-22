# TokenLn Validation Protocol

## Goal

Test the core hypothesis:

> TokenLn achieves equal fix success with materially fewer context tokens than raw CLI output.

Primary decision metric:
- `fix_success` parity between baseline and TokenLn modes.

Secondary metrics:
- `tokens_in` reduction
- `turns_to_fix` reduction
- `time_to_fix_sec` reduction

## Trial Modes

- `baseline`: agent sees raw command output.
- `tokenln`: agent uses TokenLn packet flow (`query` + `expand`).

## Dataset

Case manifest:
- `docs/experiment/cases.tsv`

Each row defines:
- `case_id`
- `command_path`
- `fixture_path`
- `objective`

## Running The Harness

```bash
./scripts/run_validation_experiment.sh
```

Outputs:
- `docs/experiment/results/auto_metrics.csv`
- `docs/experiment/results/VALIDATION_REPORT.md`

Manual trial sheet:
- `docs/experiment/manual_trials.csv`

## Manual Trial Procedure

For each `case_id` and each target agent:

1. Run baseline trial.
2. Record `fix_success`, `turns_to_fix`, `time_to_fix_sec`, `tokens_in`.
3. Run TokenLn trial.
4. Record same fields.
5. Repeat across at least 2-3 seeds/prompts if possible.

Notes:
- Keep prompts and constraints constant between modes.
- Prefer deterministic repo state for each trial.
- Mark failed or abandoned trials explicitly.

## Interpreting Results

Groundbreaking threshold (suggested):
- TokenLn `fix_success` is not worse than baseline by more than 5%.
- TokenLn reduces `tokens_in` by 60%+ median.
- TokenLn reduces `turns_to_fix` by 15%+ median.

If success drops materially, prioritize semantic analysis accuracy before optimizing compression behavior.
