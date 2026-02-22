use tokenln::analysis::jest::JestAnalyzer;
use tokenln::lexers::jest::JestLexer;
use tokenln::optimizer::BasicOptimizer;
use tokenln::parsers::jest::JestParser;
use tokenln::pipeline::{Lexer, Optimizer, Parser, SemanticAnalyzer};
use tokenln::postprocess::apply_low_confidence_fallback;

#[test]
fn compiles_jest_failure_into_one_deviation() {
    let raw = include_str!("fixtures/jest/assertion_failure.txt");

    let lexer = JestLexer;
    let parser = JestParser;
    let analyzer = JestAnalyzer;
    let optimizer = BasicOptimizer;

    let tokens = lexer.lex(raw);
    let parsed = parser.parse(&tokens);
    let report = analyzer.analyze(&parsed);
    let mut report = optimizer.optimize(report);
    apply_low_confidence_fallback(&mut report, raw);

    assert_eq!(report.deviations.len(), 1);
    let deviation = &report.deviations[0];
    assert_eq!(deviation.location.file.as_deref(), Some("src/auth.test.ts"));
    assert_eq!(deviation.location.line, Some(14));
    assert_eq!(deviation.location.column, Some(23));
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
        deviation.confidence >= 0.95,
        "canonical jest assertion fixture should remain high confidence"
    );
    assert!(
        deviation.raw_excerpt.is_none(),
        "high-confidence deviations should not trigger fallback excerpt"
    );

    let actual_json = format!(
        "{}\n",
        serde_json::to_string_pretty(&report).expect("report should serialize to JSON")
    );
    let expected_json = include_str!("fixtures/expected_ir/jest_assertion_failure.ir.json");
    assert_eq!(
        actual_json, expected_json,
        "IR snapshot mismatch for jest assertion failure fixture"
    );
}
