/// Adversarial pipeline tests: edge cases the happy-path fixtures don't cover.
///
/// These are behavioural assertions (no golden JSON snapshots) — they verify
/// that the pipeline never panics and degrades gracefully under hostile input.
use tokenln::analysis::cargo_test::CargoTestAnalyzer;
use tokenln::lexers::cargo_test::CargoTestLexer;
use tokenln::optimizer::BasicOptimizer;
use tokenln::parsers::cargo_test::CargoTestParser;
use tokenln::pipeline::{Lexer, Optimizer, Parser, SemanticAnalyzer};
use tokenln::postprocess::apply_low_confidence_fallback;

fn run_cargo_test_pipeline(raw: &str) -> tokenln::ir::DeviationReport {
    let tokens = CargoTestLexer.lex(raw);
    let parsed = CargoTestParser.parse(&tokens);
    let report = CargoTestAnalyzer.analyze(&parsed);
    let mut report = BasicOptimizer.optimize(report);
    apply_low_confidence_fallback(&mut report, raw);
    report
}

// ─── ANSI escape codes ───────────────────────────────────────────────────────

/// Cargo `--color=always` wraps "ok" / "FAILED" and the summary line in ANSI
/// colour codes. The failure *body* (thread panic, assertions) is plain text.
/// The pipeline must still extract the deviation correctly.
#[test]
fn handles_ansi_color_codes_in_result_lines() {
    let raw = include_str!("fixtures/cargo_test/ansi_output.txt");
    let report = run_cargo_test_pipeline(raw);

    assert_eq!(
        report.deviations.len(),
        1,
        "should extract exactly one deviation despite ANSI codes"
    );
    let dev = &report.deviations[0];
    assert_eq!(dev.location.file.as_deref(), Some("src/lib.rs"));
    assert_eq!(dev.location.line, Some(42));
    assert_eq!(dev.location.column, Some(5));
    assert!(
        dev.actual.description.contains("assertion"),
        "assertion message should be present"
    );
}

// ─── Multi-failure interleaving ───────────────────────────────────────────────

/// Two failing tests in *different* source files. The parser should produce
/// two independent deviations; the optimizer should NOT group them (different
/// files → no causal group). Each deviation must carry the correct file.
#[test]
fn handles_multiple_interleaved_failures() {
    let raw = include_str!("fixtures/cargo_test/multi_failure.txt");
    let report = run_cargo_test_pipeline(raw);

    assert_eq!(
        report.deviations.len(),
        2,
        "should extract two deviations"
    );

    let files: Vec<_> = report
        .deviations
        .iter()
        .filter_map(|d| d.location.file.as_deref())
        .collect();
    assert!(files.contains(&"src/beta.rs"), "beta.rs deviation expected");
    assert!(files.contains(&"src/delta.rs"), "delta.rs deviation expected");

    // Different files → optimizer must NOT assign a group_id to either.
    for dev in &report.deviations {
        assert!(
            dev.group_id.is_none(),
            "deviations in different files must not share a group_id"
        );
    }
}

// ─── Unicode test names ───────────────────────────────────────────────────────

/// Test names with non-ASCII characters (é, ë, ï, etc.) must round-trip
/// through the pipeline without panicking or garbling the name.
#[test]
fn handles_unicode_test_names() {
    let raw = include_str!("fixtures/cargo_test/unicode_test_name.txt");
    let report = run_cargo_test_pipeline(raw);

    assert_eq!(
        report.deviations.len(),
        1,
        "should extract one deviation for unicode-named test"
    );
    let dev = &report.deviations[0];
    assert_eq!(dev.location.file.as_deref(), Some("src/unicode.rs"));
    assert_eq!(dev.location.line, Some(7));
    // Summary must contain the unicode test name.
    assert!(
        dev.summary.contains("vérifié_tëst"),
        "summary should preserve unicode test name, got: {}",
        dev.summary
    );
}

// ─── Empty input ─────────────────────────────────────────────────────────────

/// An empty input string must not panic; it should produce zero deviations.
#[test]
fn empty_input_produces_no_deviations() {
    let report = run_cargo_test_pipeline("");
    assert_eq!(
        report.deviations.len(),
        0,
        "empty input must produce zero deviations"
    );
}

// ─── Truncated output ────────────────────────────────────────────────────────

/// Output that ends abruptly mid-failure (simulating a truncated pipe or
/// buffer overflow). The pipeline must not panic; it should either produce
/// a partial deviation or zero deviations — never a hard crash.
#[test]
fn truncated_output_does_not_panic() {
    let raw = "\
running 2 tests
test tests::ok_test ... ok
test tests::trunc_test ... FAILED

failures:

---- tests::trunc_test stdout ----
thread 'tests::trunc_test' panicked at src/trunc.rs:5:9:
assertion `left ==";
    // Must not panic; deviations may or may not be populated.
    let report = run_cargo_test_pipeline(raw);
    // If a deviation was produced, it must reference the right file.
    for dev in &report.deviations {
        if let Some(file) = &dev.location.file {
            assert_eq!(file, "src/trunc.rs");
        }
    }
}

// ─── Only-whitespace input ───────────────────────────────────────────────────

#[test]
fn whitespace_only_input_produces_no_deviations() {
    let report = run_cargo_test_pipeline("   \n\t\n  ");
    assert_eq!(report.deviations.len(), 0);
}

// ─── Repeated identical failures (deduplication) ────────────────────────────

/// If the same failure appears twice in the output (e.g. copy-pasted log),
/// the optimizer must deduplicate them to a single deviation.
#[test]
fn deduplicates_repeated_identical_failure_blocks() {
    let block = "\
---- tests::dup_fail stdout ----
thread 'tests::dup_fail' panicked at src/dup.rs:10:5:
assertion `left == right` failed
  left: 1
 right: 2
";
    let raw = format!("{block}{block}test result: FAILED. 0 passed; 2 failed;");
    let report = run_cargo_test_pipeline(&raw);

    assert_eq!(
        report.deviations.len(),
        1,
        "identical duplicate failures must be deduplicated to one deviation"
    );
}
