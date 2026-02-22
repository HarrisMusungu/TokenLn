use tokenln::analysis::cargo_test::CargoTestAnalyzer;
use tokenln::lexers::cargo_test::CargoTestLexer;
use tokenln::optimizer::BasicOptimizer;
use tokenln::parsers::cargo_test::CargoTestParser;
use tokenln::pipeline::{Lexer, Optimizer, Parser, SemanticAnalyzer};
use tokenln::postprocess::apply_low_confidence_fallback;

#[test]
fn compiles_cargo_test_failure_into_one_deviation() {
    let raw = include_str!("fixtures/cargo_test/assertion_failure.txt");

    let lexer = CargoTestLexer;
    let parser = CargoTestParser;
    let analyzer = CargoTestAnalyzer;
    let optimizer = BasicOptimizer;

    let tokens = lexer.lex(raw);
    let parsed = parser.parse(&tokens);
    let report = analyzer.analyze(&parsed);
    let mut report = optimizer.optimize(report);
    apply_low_confidence_fallback(&mut report, raw);

    assert_eq!(report.deviations.len(), 1);
    let deviation = &report.deviations[0];
    assert_eq!(
        deviation.location.file.as_deref(),
        Some("src/auth.rs"),
        "file location should be preserved"
    );
    assert_eq!(
        deviation.location.line,
        Some(89),
        "line location should be preserved"
    );
    assert!(
        deviation.expected.description.contains("401"),
        "expected side should be captured"
    );
    assert!(
        deviation.actual.description.contains("403"),
        "actual side should be captured"
    );
    assert!(
        !deviation.confidence_reasons.is_empty(),
        "confidence reasons should be populated"
    );

    let actual_json = format!(
        "{}\n",
        serde_json::to_string_pretty(&report).expect("report should serialize to JSON")
    );
    let expected_json = include_str!("fixtures/expected_ir/assertion_failure.ir.json");
    assert_eq!(
        actual_json, expected_json,
        "IR snapshot mismatch for assertion failure fixture"
    );
}

#[test]
fn compiles_quoted_panic_format_into_deviation() {
    let raw = include_str!("fixtures/cargo_test/panic_quoted_format.txt");

    let lexer = CargoTestLexer;
    let parser = CargoTestParser;
    let analyzer = CargoTestAnalyzer;
    let optimizer = BasicOptimizer;

    let tokens = lexer.lex(raw);
    let parsed = parser.parse(&tokens);
    let report = analyzer.analyze(&parsed);
    let mut report = optimizer.optimize(report);
    apply_low_confidence_fallback(&mut report, raw);

    assert_eq!(report.deviations.len(), 1);
    let deviation = &report.deviations[0];
    assert_eq!(deviation.location.file.as_deref(), Some("src/main.rs"));
    assert_eq!(deviation.location.line, Some(12));
    assert!(
        deviation
            .actual
            .description
            .contains("called `Option::unwrap()` on a `None` value"),
        "panic-only failures should preserve the panic reason in actual behavior"
    );
    assert!(
        !deviation.confidence_reasons.is_empty(),
        "confidence reasons should be populated"
    );
    assert!(
        deviation
            .confidence_reasons
            .iter()
            .any(|reason| reason.starts_with("low_confidence_fallback")),
        "low-confidence cases should include fallback reason"
    );
    assert!(
        deviation
            .raw_excerpt
            .as_ref()
            .is_some_and(|raw| raw.contains("tests::panic_case")),
        "low-confidence cases should include a raw excerpt"
    );

    let actual_json = format!(
        "{}\n",
        serde_json::to_string_pretty(&report).expect("report should serialize to JSON")
    );
    let expected_json = include_str!("fixtures/expected_ir/panic_quoted_format.ir.json");
    assert_eq!(
        actual_json, expected_json,
        "IR snapshot mismatch for panic quoted format fixture"
    );
}
