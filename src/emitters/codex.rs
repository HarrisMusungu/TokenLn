use crate::ir::{Deviation, DeviationReport};
use crate::pipeline::Emitter;

pub struct CodexEmitter;

impl Emitter for CodexEmitter {
    fn emit(&self, report: &DeviationReport) -> String {
        if report.deviations.is_empty() {
            return "CODEX_BRIEF\nstatus: no_deviations".to_string();
        }

        let mut sections = Vec::new();
        sections.push("CODEX_BRIEF".to_string());
        sections.push(format!("source: {}", report.source));
        sections.push(format!("deviation_count: {}", report.deviations.len()));

        for (idx, deviation) in report.deviations.iter().enumerate() {
            sections.push(render_deviation(idx + 1, deviation));
        }

        sections.join("\n\n")
    }
}

fn render_deviation(index: usize, deviation: &Deviation) -> String {
    let mut lines = Vec::new();
    lines.push(format!("[deviation {}]", index));
    lines.push(format!("kind={:?}", deviation.kind));
    lines.push(format!("summary={}", deviation.summary));
    lines.push(format!("expected={}", deviation.expected.description));
    lines.push(format!("actual={}", deviation.actual.description));
    lines.push(format!("location={}", format_location(deviation)));
    lines.push(format!("trace={}", deviation.trace.frames.join(" -> ")));
    lines.push(format!("confidence={:.2}", deviation.confidence));
    lines.push(format!(
        "confidence_reasons={}",
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
        lines.push("raw_excerpt<<EOF".to_string());
        lines.push(excerpt.to_string());
        lines.push("EOF".to_string());
    } else {
        lines.push("raw_excerpt=none".to_string());
    }

    lines.join("\n")
}

fn format_causal_role(deviation: &Deviation) -> Option<String> {
    match (deviation.group_id.as_ref(), deviation.is_root_cause) {
        (Some(group_id), Some(true)) => Some(format!("causal_role=root_cause [{}]", group_id)),
        (Some(group_id), Some(false)) => Some(format!("causal_role=cascade [{}]", group_id)),
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
    use super::CodexEmitter;
    use crate::ir::{
        Behavior, Deviation, DeviationKind, DeviationReport, ExecutionTrace, Expectation, Location,
    };
    use crate::pipeline::Emitter;

    #[test]
    fn emits_codex_brief() {
        let report = DeviationReport::new(
            "jest",
            vec![Deviation {
                kind: DeviationKind::Test,
                expected: Expectation {
                    description: "assertion right side should be 401".to_string(),
                },
                actual: Behavior {
                    description: "assertion produced 403".to_string(),
                },
                location: Location {
                    file: Some("src/auth.test.ts".to_string()),
                    line: Some(14),
                    column: Some(23),
                    symbol: Some("auth > rejects expired token".to_string()),
                },
                trace: ExecutionTrace {
                    frames: vec![
                        "jest".to_string(),
                        "auth > rejects expired token".to_string(),
                    ],
                },
                confidence: 0.99,
                confidence_reasons: vec!["assertion_pair:+0.30".to_string()],
                raw_excerpt: None,
                summary: "auth > rejects expired token failed at src/auth.test.ts:14".to_string(),
                group_id: None,
                is_root_cause: None,
            }],
        );

        let output = CodexEmitter.emit(&report);
        assert!(output.contains("CODEX_BRIEF"));
        assert!(output.contains("[deviation 1]"));
        assert!(output.contains("confidence=0.99"));
    }
}
