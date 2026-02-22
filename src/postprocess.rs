use crate::ir::{Deviation, DeviationReport};

pub const LOW_CONFIDENCE_THRESHOLD: f32 = 0.85;
const RAW_EXCERPT_WINDOW_BEFORE: usize = 2;
const RAW_EXCERPT_WINDOW_AFTER: usize = 3;
const RAW_EXCERPT_MAX_CHARS: usize = 700;

pub fn apply_low_confidence_fallback(report: &mut DeviationReport, raw_output: &str) {
    let lines = raw_output.lines().collect::<Vec<_>>();
    for deviation in &mut report.deviations {
        if deviation.confidence >= LOW_CONFIDENCE_THRESHOLD {
            continue;
        }

        if deviation.raw_excerpt.is_none() {
            deviation.raw_excerpt = extract_relevant_excerpt(deviation, &lines);
        }

        deviation.confidence_reasons.push(format!(
            "low_confidence_fallback:<{LOW_CONFIDENCE_THRESHOLD:.2}"
        ));
    }
}

fn extract_relevant_excerpt(deviation: &Deviation, lines: &[&str]) -> Option<String> {
    if lines.is_empty() {
        return None;
    }

    let mut anchors = Vec::new();
    if let (Some(file), Some(line), Some(column)) = (
        deviation.location.file.as_ref(),
        deviation.location.line,
        deviation.location.column,
    ) {
        anchors.push(format!("{file}:{line}:{column}"));
    }
    if let (Some(file), Some(line)) = (deviation.location.file.as_ref(), deviation.location.line) {
        anchors.push(format!("{file}:{line}"));
    }
    if let Some(symbol) = deviation.location.symbol.as_ref() {
        anchors.push(symbol.clone());
    }

    let anchor_idx = anchors
        .iter()
        .find_map(|anchor| lines.iter().position(|line| line.contains(anchor)));

    let (start, end) = match anchor_idx {
        Some(idx) => (
            idx.saturating_sub(RAW_EXCERPT_WINDOW_BEFORE),
            (idx + RAW_EXCERPT_WINDOW_AFTER + 1).min(lines.len()),
        ),
        None => (0, lines.len().min(RAW_EXCERPT_WINDOW_AFTER + 1)),
    };

    let mut excerpt = lines[start..end].join("\n");
    if excerpt.len() > RAW_EXCERPT_MAX_CHARS {
        excerpt.truncate(RAW_EXCERPT_MAX_CHARS);
        excerpt.push_str("...");
    }

    if excerpt.trim().is_empty() {
        None
    } else {
        Some(excerpt)
    }
}

#[cfg(test)]
mod tests {
    use crate::ir::{
        Behavior, Deviation, DeviationKind, DeviationReport, ExecutionTrace, Expectation, Location,
    };

    use super::apply_low_confidence_fallback;

    #[test]
    fn applies_raw_excerpt_to_low_confidence_deviation() {
        let mut report = DeviationReport {
            schema_version: "0.1".to_string(),
            source: "cargo test".to_string(),
            deviations: vec![Deviation {
                kind: DeviationKind::Test,
                expected: Expectation {
                    description: "x".to_string(),
                },
                actual: Behavior {
                    description: "y".to_string(),
                },
                location: Location {
                    file: Some("src/main.rs".to_string()),
                    line: Some(12),
                    column: Some(5),
                    symbol: Some("tests::panic_case".to_string()),
                },
                trace: ExecutionTrace {
                    frames: vec!["cargo test".to_string()],
                },
                confidence: 0.5,
                confidence_reasons: vec!["base:+0.25".to_string()],
                raw_excerpt: None,
                summary: "panic".to_string(),
                group_id: None,
                is_root_cause: None,
            }],
        };

        let raw = "\
---- tests::panic_case stdout ----
thread 'tests::panic_case' panicked at src/main.rs:12:5: boom";
        apply_low_confidence_fallback(&mut report, raw);

        let deviation = &report.deviations[0];
        assert!(deviation.raw_excerpt.is_some());
        assert!(deviation
            .confidence_reasons
            .iter()
            .any(|reason| reason.starts_with("low_confidence_fallback")));
    }
}
