use crate::ir::{
    Behavior, Deviation, DeviationKind, DeviationReport, ExecutionTrace, Expectation, Location,
};
use crate::pipeline::{ParsedFailure, SemanticAnalyzer};

pub struct CargoBuildAnalyzer;

impl SemanticAnalyzer for CargoBuildAnalyzer {
    fn analyze(&self, parsed_failures: &[ParsedFailure]) -> DeviationReport {
        let deviations = parsed_failures.iter().map(to_deviation).collect::<Vec<_>>();
        DeviationReport::new("cargo build", deviations)
    }
}

fn to_deviation(failure: &ParsedFailure) -> Deviation {
    let error_code = failure.build_error_code.clone();

    let expected = match error_code.as_deref() {
        Some(code) => format!("compilation should satisfy rustc {code}"),
        None => "compilation should succeed".to_string(),
    };

    let primary_message = failure
        .panic_message
        .clone()
        .unwrap_or_else(|| "build failed".to_string());
    let actual = build_actual_description(failure, &primary_message);

    let summary = match (error_code.as_deref(), failure.file.as_deref(), failure.line) {
        (Some(code), Some(file), Some(line)) => format!("build error {code} at {file}:{line}"),
        (None, Some(file), Some(line)) => format!("build error at {file}:{line}"),
        (Some(code), _, _) => format!("build error {code}"),
        (None, _, _) => "build error".to_string(),
    };

    let mut trace = if failure.trace.is_empty() {
        vec!["cargo build".to_string()]
    } else {
        failure.trace.clone()
    };
    if let Some(code) = error_code.as_ref() {
        trace.push(format!("rustc {code}"));
    }

    let (confidence, confidence_reasons) = score_confidence(failure, &trace);

    Deviation {
        kind: DeviationKind::Build,
        expected: Expectation {
            description: expected,
        },
        actual: Behavior {
            description: actual,
        },
        location: Location {
            file: failure.file.clone(),
            line: failure.line,
            column: failure.column,
            symbol: error_code.clone().or(Some("rustc".to_string())),
        },
        trace: ExecutionTrace { frames: trace },
        confidence,
        confidence_reasons,
        raw_excerpt: None,
        summary,
        group_id: None,
        is_root_cause: None,
    }
}

fn score_confidence(failure: &ParsedFailure, trace: &[String]) -> (f32, Vec<String>) {
    let mut score = 0.25_f32;
    let mut reasons = vec!["base:+0.25".to_string()];

    if failure.build_error_code.is_some() {
        score += 0.25;
        reasons.push("error_code:+0.25".to_string());
    }

    if failure.file.is_some() && failure.line.is_some() && failure.column.is_some() {
        score += 0.25;
        reasons.push("precise_location:+0.25".to_string());
    } else if failure.file.is_some() && failure.line.is_some() {
        score += 0.2;
        reasons.push("coarse_location:+0.20".to_string());
    }

    if failure.panic_message.is_some() {
        score += 0.1;
        reasons.push("primary_message:+0.10".to_string());
    }

    if failure.build_help.is_some() {
        score += 0.05;
        reasons.push("help_text:+0.05".to_string());
    }

    if failure.build_explain_code.is_some() {
        score += 0.05;
        reasons.push("explain_code:+0.05".to_string());
    }

    if failure.build_source_snippet.is_some() && failure.build_caret_marker.is_some() {
        score += 0.1;
        reasons.push("code_span_pair:+0.10".to_string());
    } else if failure.build_source_snippet.is_some() || failure.build_caret_marker.is_some() {
        score += 0.05;
        reasons.push("partial_code_span:+0.05".to_string());
    }

    if let (Some(location_line), Some(source_line)) =
        (failure.line.as_ref(), failure.build_source_line.as_ref())
    {
        if location_line == source_line {
            score += 0.05;
            reasons.push("line_alignment:+0.05".to_string());
        } else {
            score -= 0.1;
            reasons.push("line_mismatch:-0.10".to_string());
        }
    }

    if let (Some(code), Some(explain_code)) = (
        failure.build_error_code.as_ref(),
        failure.build_explain_code.as_ref(),
    ) {
        if code == explain_code {
            score += 0.05;
            reasons.push("code_alignment:+0.05".to_string());
        } else {
            score -= 0.2;
            reasons.push("code_mismatch:-0.20".to_string());
        }
    }

    if trace.len() >= 2 {
        score += 0.05;
        reasons.push("trace_depth:+0.05".to_string());
    }

    let clamped = score.clamp(0.2, 0.99);
    if (clamped - score).abs() > f32::EPSILON {
        reasons.push(format!("clamped:{clamped:.2}"));
    }

    (round_confidence(clamped), reasons)
}

fn build_actual_description(failure: &ParsedFailure, primary_message: &str) -> String {
    let mut segments = vec![primary_message.to_string()];

    if let Some(help) = failure.build_help.as_ref() {
        segments.push(format!("help: {help}"));
    }

    if let Some(snippet) = failure.build_source_snippet.as_ref() {
        segments.push(format!("snippet: {snippet}"));
    }

    if let Some(marker) = failure.build_caret_marker.as_ref() {
        segments.push(format!("marker: {marker}"));
    }

    segments.join("; ")
}

fn round_confidence(value: f32) -> f32 {
    (value * 100.0).round() / 100.0
}
