use tokenln::analysis::cargo_build::CargoBuildAnalyzer;
use tokenln::lexers::cargo_build::CargoBuildLexer;
use tokenln::optimizer::BasicOptimizer;
use tokenln::parsers::cargo_build::CargoBuildParser;
use tokenln::pipeline::{Lexer, Optimizer, Parser, SemanticAnalyzer};
use tokenln::postprocess::apply_low_confidence_fallback;

#[test]
fn compiles_cargo_build_error_into_one_deviation() {
    let raw = include_str!("fixtures/cargo_build/missing_symbol.txt");

    let lexer = CargoBuildLexer;
    let parser = CargoBuildParser;
    let analyzer = CargoBuildAnalyzer;
    let optimizer = BasicOptimizer;

    let tokens = lexer.lex(raw);
    let parsed = parser.parse(&tokens);
    let report = analyzer.analyze(&parsed);
    let mut report = optimizer.optimize(report);
    apply_low_confidence_fallback(&mut report, raw);

    assert_eq!(report.deviations.len(), 1);
    let deviation = &report.deviations[0];
    assert_eq!(deviation.location.file.as_deref(), Some("src/main.rs"));
    assert_eq!(deviation.location.line, Some(4));
    assert_eq!(deviation.location.column, Some(20));
    assert!(
        deviation.summary.contains("E0425"),
        "build summary should include rustc error code"
    );
    assert!(
        deviation
            .actual
            .description
            .contains("cannot find value `x`"),
        "actual behavior should capture build error text"
    );
    assert!(
        deviation.actual.description.contains("similar name"),
        "help text should be included in actual behavior"
    );
    assert!(
        deviation.actual.description.contains("snippet:"),
        "code-span snippet should be included in actual behavior"
    );
    assert!(
        deviation.actual.description.contains("marker: ^"),
        "caret marker should be included in actual behavior"
    );
    assert!(
        !deviation.confidence_reasons.is_empty(),
        "confidence reasons should be populated"
    );

    let actual_json = format!(
        "{}\n",
        serde_json::to_string_pretty(&report).expect("report should serialize to JSON")
    );
    let expected_json = include_str!("fixtures/expected_ir/cargo_build_missing_symbol.ir.json");
    assert_eq!(
        actual_json, expected_json,
        "IR snapshot mismatch for cargo build missing symbol fixture"
    );
}

#[test]
fn penalizes_conflicting_build_evidence_and_adds_fallback() {
    let raw = include_str!("fixtures/cargo_build/conflicting_evidence.txt");

    let lexer = CargoBuildLexer;
    let parser = CargoBuildParser;
    let analyzer = CargoBuildAnalyzer;
    let optimizer = BasicOptimizer;

    let tokens = lexer.lex(raw);
    let parsed = parser.parse(&tokens);
    let report = analyzer.analyze(&parsed);
    let mut report = optimizer.optimize(report);
    apply_low_confidence_fallback(&mut report, raw);

    assert_eq!(report.deviations.len(), 1);
    let deviation = &report.deviations[0];
    assert!(
        deviation.confidence < 0.85,
        "conflicting evidence should lower confidence below fallback threshold"
    );
    assert!(
        deviation
            .confidence_reasons
            .iter()
            .any(|reason| reason == "line_mismatch:-0.10"),
        "line mismatch reason should be recorded"
    );
    assert!(
        deviation
            .confidence_reasons
            .iter()
            .any(|reason| reason == "code_mismatch:-0.20"),
        "code mismatch reason should be recorded"
    );
    assert!(
        deviation
            .confidence_reasons
            .iter()
            .any(|reason| reason.starts_with("low_confidence_fallback")),
        "low-confidence fallback reason should be recorded"
    );
    assert!(
        deviation
            .raw_excerpt
            .as_ref()
            .is_some_and(|excerpt| excerpt.contains("src/main.rs:4:20")),
        "fallback excerpt should include the anchored location context"
    );

    let actual_json = format!(
        "{}\n",
        serde_json::to_string_pretty(&report).expect("report should serialize to JSON")
    );
    let expected_json =
        include_str!("fixtures/expected_ir/cargo_build_conflicting_evidence.ir.json");
    assert_eq!(
        actual_json, expected_json,
        "IR snapshot mismatch for cargo build conflicting evidence fixture"
    );
}
