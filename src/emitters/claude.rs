use crate::ir::{Deviation, DeviationReport};
use crate::pipeline::Emitter;

pub struct ClaudeEmitter;

impl Emitter for ClaudeEmitter {
    fn emit(&self, report: &DeviationReport) -> String {
        if report.deviations.is_empty() {
            return "No deviations detected.".to_string();
        }

        let mut sections = Vec::new();
        sections.push("# TokenLn Deviation Brief".to_string());
        sections.push(format!("Source: {}", report.source));
        sections.push(format!("Total deviations: {}", report.deviations.len()));

        for (idx, deviation) in report.deviations.iter().enumerate() {
            sections.push(render_deviation(idx + 1, deviation));
        }

        sections.join("\n\n")
    }
}

fn render_deviation(index: usize, deviation: &Deviation) -> String {
    let mut lines = Vec::new();
    lines.push(format!("## Deviation {}: {:?}", index, deviation.kind));
    lines.push(format!("Summary: {}", deviation.summary));
    lines.push(format!("Expected: {}", deviation.expected.description));
    lines.push(format!("Actual: {}", deviation.actual.description));
    lines.push(format!("Location: {}", format_location(deviation)));
    lines.push(format!("Trace: {}", deviation.trace.frames.join(" -> ")));
    lines.push(format!("Confidence: {:.2}", deviation.confidence));
    lines.push(format!(
        "Confidence reasons: {}",
        if deviation.confidence_reasons.is_empty() {
            "none".to_string()
        } else {
            deviation.confidence_reasons.join(", ")
        }
    ));

    if let Some(causal_role) = format_causal_role(deviation) {
        lines.push(causal_role);
    }

    if let Some(excerpt) = deviation.raw_excerpt.as_ref() {
        lines.push("Raw excerpt:".to_string());
        lines.push("```text".to_string());
        lines.push(excerpt.to_string());
        lines.push("```".to_string());
    }

    lines.join("\n")
}

fn format_causal_role(deviation: &Deviation) -> Option<String> {
    match (deviation.group_id.as_ref(), deviation.is_root_cause) {
        (Some(group_id), Some(true)) => Some(format!(
            "Causal role: **root cause** (group `{}`)",
            group_id
        )),
        (Some(group_id), Some(false)) => {
            Some(format!("Causal role: cascade (group `{}`)", group_id))
        }
        _ => None,
    }
}

fn format_location(deviation: &Deviation) -> String {
    let file = deviation.location.file.as_deref().unwrap_or("unknown");
    let line = deviation
        .location
        .line
        .map(|line| line.to_string())
        .unwrap_or_else(|| "?".to_string());
    let column = deviation
        .location
        .column
        .map(|column| column.to_string())
        .unwrap_or_else(|| "?".to_string());
    format!("{file}:{line}:{column}")
}

#[cfg(test)]
mod tests {
    use super::ClaudeEmitter;
    use crate::ir::{
        Behavior, Deviation, DeviationKind, DeviationReport, ExecutionTrace, Expectation, Location,
    };
    use crate::pipeline::Emitter;

    #[test]
    fn emits_markdown_brief() {
        let report = DeviationReport::new(
            "pytest",
            vec![Deviation {
                kind: DeviationKind::Test,
                expected: Expectation {
                    description: "assertion right side should be 401".to_string(),
                },
                actual: Behavior {
                    description: "assertion produced 403".to_string(),
                },
                location: Location {
                    file: Some("tests/test_auth.py".to_string()),
                    line: Some(27),
                    column: Some(1),
                    symbol: Some("tests/test_auth.py::test_auth_invalid_token".to_string()),
                },
                trace: ExecutionTrace {
                    frames: vec![
                        "pytest".to_string(),
                        "tests/test_auth.py::test_auth_invalid_token".to_string(),
                    ],
                },
                confidence: 0.95,
                confidence_reasons: vec!["assertion_pair:+0.30".to_string()],
                raw_excerpt: None,
                summary:
                    "tests/test_auth.py::test_auth_invalid_token failed at tests/test_auth.py:27"
                        .to_string(),
                group_id: None,
                is_root_cause: None,
            }],
        );

        let output = ClaudeEmitter.emit(&report);
        assert!(output.contains("# TokenLn Deviation Brief"));
        assert!(output.contains("## Deviation 1: Test"));
        assert!(output.contains("Confidence: 0.95"));
    }
}
