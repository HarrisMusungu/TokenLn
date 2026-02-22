use tokenln::analysis::pytest::PytestAnalyzer;
use tokenln::lexers::pytest::PytestLexer;
use tokenln::optimizer::BasicOptimizer;
use tokenln::parsers::pytest::PytestParser;
use tokenln::pipeline::{Lexer, Optimizer, Parser, SemanticAnalyzer};
use tokenln::postprocess::apply_low_confidence_fallback;

#[test]
fn compiles_pytest_failure_into_one_deviation() {
    let raw = include_str!("fixtures/pytest/assertion_failure.txt");

    let lexer = PytestLexer;
    let parser = PytestParser;
    let analyzer = PytestAnalyzer;
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
        Some("tests/test_auth.py")
    );
    assert_eq!(deviation.location.line, Some(27));
    assert!(
        deviation.expected.description.contains("401"),
        "expected assertion side should be preserved"
    );
    assert!(
        deviation.actual.description.contains("403"),
        "actual assertion side should be preserved"
    );
    assert!(
        !deviation.confidence_reasons.is_empty(),
        "confidence reasons should be populated"
    );
    assert!(
        deviation.confidence >= 0.85,
        "canonical pytest assertion fixture should remain high confidence"
    );
    assert!(
        deviation.raw_excerpt.is_none(),
        "high-confidence deviations should not trigger fallback excerpt"
    );

    let actual_json = format!(
        "{}\n",
        serde_json::to_string_pretty(&report).expect("report should serialize to JSON")
    );
    let expected_json = include_str!("fixtures/expected_ir/pytest_assertion_failure.ir.json");
    assert_eq!(
        actual_json, expected_json,
        "IR snapshot mismatch for pytest assertion failure fixture"
    );
}
