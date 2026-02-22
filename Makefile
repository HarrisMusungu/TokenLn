CARGO ?= $(shell if command -v cargo >/dev/null 2>&1; then command -v cargo; elif [ -x "$$HOME/.cargo/bin/cargo" ]; then printf "%s" "$$HOME/.cargo/bin/cargo"; else printf "%s" cargo; fi)

.PHONY: test snapshots benchmark experiment fill-trial check ci demo-test demo-build demo-go-test demo-pytest demo-jest demo-claude demo-ollama demo-codex demo-copilot demo-proxy-pytest demo-query demo-expand demo-compare

test:
	$(CARGO) test

snapshots:
	CARGO_BIN="$(CARGO)" ./scripts/refresh_ir_snapshots.sh

benchmark:
	CARGO_BIN="$(CARGO)" ./scripts/benchmark_phase1.sh

experiment:
	CARGO_BIN="$(CARGO)" ./scripts/run_validation_experiment.sh

fill-trial:
	CARGO_BIN="$(CARGO)" ./scripts/fill_manual_trial_case.sh --case pytest_assertion --agent claude-code

check: test snapshots benchmark

ci:
	CARGO_BIN="$(CARGO)" ./scripts/run_ci.sh

demo-test:
	$(CARGO) run -- cargo test --from-file tests/fixtures/cargo_test/assertion_failure.txt

demo-build:
	$(CARGO) run -- cargo build --from-file tests/fixtures/cargo_build/missing_symbol.txt

demo-go-test:
	$(CARGO) run -- go test --from-file tests/fixtures/go_test/assertion_failure.txt

demo-pytest:
	$(CARGO) run -- pytest --from-file tests/fixtures/pytest/assertion_failure.txt

demo-jest:
	$(CARGO) run -- jest --from-file tests/fixtures/jest/assertion_failure.txt

demo-claude:
	$(CARGO) run -- pytest --from-file tests/fixtures/pytest/assertion_failure.txt --target claude

demo-ollama:
	$(CARGO) run -- pytest --from-file tests/fixtures/pytest/assertion_failure.txt --target ollama

demo-codex:
	$(CARGO) run -- pytest --from-file tests/fixtures/pytest/assertion_failure.txt --target codex

demo-copilot:
	$(CARGO) run -- pytest --from-file tests/fixtures/pytest/assertion_failure.txt --target copilot

demo-proxy-pytest:
	$(CARGO) run -- proxy run --from-file tests/fixtures/pytest/assertion_failure.txt --target claude -- pytest

demo-query:
	$(CARGO) run -- query --budget 200 --target claude

demo-expand:
	$(CARGO) run -- expand d1 --view full --budget 240 --target claude

demo-compare:
	$(CARGO) run -- compare --latest --previous --target claude
