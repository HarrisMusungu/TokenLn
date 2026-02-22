use crate::ir::{Deviation, DeviationReport};
use crate::pipeline::Emitter;

pub struct OllamaEmitter;

impl Emitter for OllamaEmitter {
    fn emit(&self, report: &DeviationReport) -> String {
        if report.deviations.is_empty() {
            return "OLLAMA_CONTEXT\nstatus: no_deviations".to_string();
        }

        let mut lines = Vec::new();
        lines.push("OLLAMA_CONTEXT".to_string());
        lines.push(format!("source: {}", report.source));
        lines.push(format!("deviation_count: {}", report.deviations.len()));

        for (idx, deviation) in report.deviations.iter().enumerate() {
            lines.push(render_deviation(idx + 1, deviation));
        }

        lines.join("\n")
    }
}

fn render_deviation(index: usize, deviation: &Deviation) -> String {
    let mut lines = Vec::new();
    lines.push(format!("deviation_{}:", index));
    lines.push(format!("  kind: {:?}", deviation.kind));
    lines.push(format!("  summary: {}", deviation.summary));
    lines.push(format!("  expected: {}", deviation.expected.description));
    lines.push(format!("  actual: {}", deviation.actual.description));
    lines.push(format!("  location: {}", format_location(deviation)));
    lines.push(format!("  trace: {}", deviation.trace.frames.join(" -> ")));
    lines.push(format!("  confidence: {:.2}", deviation.confidence));
    lines.push(format!(
        "  confidence_reasons: {}",
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
        lines.push("  raw_excerpt: |".to_string());
        lines.extend(
            excerpt
                .lines()
                .map(|line| format!("    {line}"))
                .collect::<Vec<_>>(),
        );
    } else {
        lines.push("  raw_excerpt: none".to_string());
    }

    lines.join("\n")
}

fn format_causal_role(deviation: &Deviation) -> Option<String> {
    match (deviation.group_id.as_ref(), deviation.is_root_cause) {
        (Some(group_id), Some(true)) => Some(format!("  causal_role: root_cause [{}]", group_id)),
        (Some(group_id), Some(false)) => Some(format!("  causal_role: cascade [{}]", group_id)),
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
    use super::OllamaEmitter;
    use crate::ir::{
        Behavior, Deviation, DeviationKind, DeviationReport, ExecutionTrace, Expectation, Location,
    };
    use crate::pipeline::Emitter;

    #[test]
    fn emits_ollama_context() {
        let report = DeviationReport::new(
            "go test",
            vec![Deviation {
                kind: DeviationKind::Test,
                expected: Expectation {
                    description: "assertion right side should be 401".to_string(),
                },
                actual: Behavior {
                    description: "assertion produced 403".to_string(),
                },
                location: Location {
                    file: Some("auth_test.go".to_string()),
                    line: Some(42),
                    column: Some(1),
                    symbol: Some("TestValidateToken".to_string()),
                },
                trace: ExecutionTrace {
                    frames: vec!["go test".to_string(), "TestValidateToken".to_string()],
                },
                confidence: 0.95,
                confidence_reasons: vec!["assertion_pair:+0.30".to_string()],
                raw_excerpt: None,
                summary: "TestValidateToken failed at auth_test.go:42".to_string(),
                group_id: None,
                is_root_cause: None,
            }],
        );

        let output = OllamaEmitter.emit(&report);
        assert!(output.contains("OLLAMA_CONTEXT"));
        assert!(output.contains("deviation_1:"));
        assert!(output.contains("confidence: 0.95"));
    }
}
