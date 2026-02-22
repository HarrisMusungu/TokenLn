use crate::ir::{Deviation, DeviationReport};
use crate::pipeline::Emitter;

pub struct GenericEmitter;

impl Emitter for GenericEmitter {
    fn emit(&self, report: &DeviationReport) -> String {
        if report.deviations.is_empty() {
            return "NO_DEVIATIONS".to_string();
        }

        report
            .deviations
            .iter()
            .map(render_deviation)
            .collect::<Vec<_>>()
            .join("\n\n")
    }
}

fn render_deviation(deviation: &Deviation) -> String {
    format!(
        "DEVIATION {{
  kind: {:?}
  expected: {}
  actual:   {}
  location: {}
  trace:    {}
  confidence: {:.2}
  confidence_reasons: {}
{}{}
  summary:  {}
}}",
        deviation.kind,
        deviation.expected.description,
        deviation.actual.description,
        format_location(deviation),
        deviation.trace.frames.join(" -> "),
        deviation.confidence,
        if deviation.confidence_reasons.is_empty() {
            "none".to_string()
        } else {
            deviation.confidence_reasons.join(", ")
        },
        format_raw_excerpt(deviation),
        format_causal_role(deviation),
        deviation.summary
    )
}

fn format_causal_role(deviation: &Deviation) -> String {
    match (deviation.group_id.as_ref(), deviation.is_root_cause) {
        (Some(group_id), Some(true)) => {
            format!("  causal_role: root cause [{}]\n", group_id)
        }
        (Some(group_id), Some(false)) => {
            format!("  causal_role: cascade [{}]\n", group_id)
        }
        _ => String::new(),
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

fn format_raw_excerpt(deviation: &Deviation) -> String {
    match deviation.raw_excerpt.as_ref() {
        Some(excerpt) => format!("  raw_excerpt: |\n{}", indent_excerpt(excerpt, "    ")),
        None => "  raw_excerpt: none".to_string(),
    }
}

fn indent_excerpt(text: &str, indent: &str) -> String {
    text.lines()
        .map(|line| format!("{indent}{line}"))
        .collect::<Vec<_>>()
        .join("\n")
}
