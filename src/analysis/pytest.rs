use crate::ir::{
    Behavior, Deviation, DeviationKind, DeviationReport, ExecutionTrace, Expectation, Location,
};
use crate::pipeline::{ParsedFailure, SemanticAnalyzer};

pub struct PytestAnalyzer;

impl SemanticAnalyzer for PytestAnalyzer {
    fn analyze(&self, parsed_failures: &[ParsedFailure]) -> DeviationReport {
        let deviations = parsed_failures.iter().map(to_deviation).collect::<Vec<_>>();
        DeviationReport::new("pytest", deviations)
    }
}

fn to_deviation(failure: &ParsedFailure) -> Deviation {
    let test_name = failure
        .test_name
        .clone()
        .unwrap_or_else(|| "unknown_test".to_string());

    let expected = failure
        .assertion_right
        .as_ref()
        .map(|right| format!("assertion right side should be {right}"))
        .unwrap_or_else(|| "test assertion should pass".to_string());

    let actual = match (
        failure.assertion_left.as_ref(),
        failure.panic_message.as_ref(),
    ) {
        (Some(left), _) => format!("assertion produced {left}"),
        (None, Some(message)) => format!("pytest reported failure: {message}"),
        (None, None) => "pytest reported failure".to_string(),
    };

    let summary = match (failure.file.as_ref(), failure.line) {
        (Some(file), Some(line)) => format!("{test_name} failed at {file}:{line}"),
        _ => format!("{test_name} failed"),
    };

    let trace = if failure.trace.is_empty() {
        vec!["pytest".to_string(), test_name.clone()]
    } else {
        failure.trace.clone()
    };

    let (confidence, confidence_reasons) = score_confidence(failure, &trace);

    Deviation {
        kind: DeviationKind::Test,
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
            symbol: Some(test_name),
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
    let mut score = 0.2_f32;
    let mut reasons = vec!["base:+0.20".to_string()];

    if failure.test_name.is_some() {
        score += 0.15;
        reasons.push("test_identity:+0.15".to_string());
    }

    if failure.file.is_some() && failure.line.is_some() {
        score += 0.2;
        reasons.push("coarse_location:+0.20".to_string());
    } else if failure.file.is_some() {
        score += 0.1;
        reasons.push("file_only_location:+0.10".to_string());
    }

    if failure.assertion_left.is_some() && failure.assertion_right.is_some() {
        score += 0.3;
        reasons.push("assertion_pair:+0.30".to_string());
    } else if failure.panic_message.is_some() {
        score += 0.15;
        reasons.push("failure_message:+0.15".to_string());
    }

    if trace.len() >= 2 {
        score += 0.05;
        reasons.push("trace_depth:+0.05".to_string());
    }

    if failure
        .panic_message
        .as_ref()
        .is_some_and(|msg| msg.contains("AssertionError"))
    {
        score += 0.05;
        reasons.push("assertion_error_marker:+0.05".to_string());
    }

    let clamped = score.clamp(0.2, 0.99);
    if (clamped - score).abs() > f32::EPSILON {
        reasons.push(format!("clamped:{clamped:.2}"));
    }

    (round_confidence(clamped), reasons)
}

fn round_confidence(value: f32) -> f32 {
    (value * 100.0).round() / 100.0
}
