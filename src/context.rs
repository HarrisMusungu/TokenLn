use std::collections::HashSet;

use serde::{Deserialize, Serialize};

use crate::ir::{Deviation, DeviationKind, DeviationReport};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContextPacket {
    pub packet_id: String,
    pub run_id: String,
    pub budget_tokens: u32,
    pub used_tokens: u32,
    pub source: String,
    pub objective: String,
    pub deviations: Vec<DeviationSlice>,
    pub expansion_hints: Vec<ExpansionHint>,
    pub unresolved_count: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeviationSlice {
    pub id: String,
    pub summary: String,
    pub expected: String,
    pub actual: String,
    pub location: String,
    pub confidence: f32,
    pub novelty_score: f32,
    pub utility_score: f32,
    pub evidence_refs: Vec<EvidenceRef>,
    /// Present when this deviation was previously recorded as fixed via `tokenln fixed`.
    /// Indicates it was deprioritized in budget allocation.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fix_hint: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExpansionHint {
    pub deviation_id: String,
    pub reason: String,
    pub estimated_tokens: u32,
    pub hint: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EvidenceRef {
    pub artifact: String,
    pub line_start: u32,
    pub line_end: u32,
    pub hash: String,
}

pub struct BuildPacketOptions<'a> {
    pub run_id: &'a str,
    pub source: &'a str,
    pub objective: &'a str,
    pub budget_tokens: u32,
    pub report: &'a DeviationReport,
    pub raw_output: &'a str,
    pub report_artifact: &'a str,
    pub previous_signatures: &'a HashSet<String>,
    /// Signatures of deviations previously recorded as fixed via `tokenln fixed`.
    /// Deviations matching these signatures are strongly deprioritized (novelty_score = 0.10).
    pub fixed_signatures: &'a HashSet<String>,
}

pub fn build_context_packet(options: BuildPacketOptions<'_>) -> ContextPacket {
    let packet_id = format!("pkt-{}-{}", options.run_id, options.budget_tokens);

    let mut candidates = options
        .report
        .deviations
        .iter()
        .enumerate()
        .map(|(idx, deviation)| {
            let id = format!("d{}", idx + 1);
            let signature = deviation_signature(deviation);

            // Previously fixed deviations get a very low novelty score (0.10) so they
            // sink to the bottom of budget allocation. Seen-before (not fixed) deviations
            // get 0.55. Brand-new deviations get 1.0.
            let (novelty_score, fix_hint) = if options.fixed_signatures.contains(&signature) {
                (0.10_f32, Some("previously recorded as fixed".to_string()))
            } else if options.previous_signatures.contains(&signature) {
                (0.55_f32, None)
            } else {
                (1.0_f32, None)
            };

            let utility_score = score_utility(deviation, novelty_score);
            let evidence_refs =
                build_evidence_refs(deviation, options.raw_output, options.report_artifact);
            let slice = DeviationSlice {
                id: id.clone(),
                summary: deviation.summary.clone(),
                expected: deviation.expected.description.clone(),
                actual: deviation.actual.description.clone(),
                location: format_location(deviation),
                confidence: deviation.confidence,
                novelty_score: round2(novelty_score),
                utility_score,
                evidence_refs,
                fix_hint,
            };
            let estimated_tokens = estimate_slice_tokens(&slice);
            (slice, estimated_tokens)
        })
        .collect::<Vec<_>>();

    candidates.sort_by(|(left, _), (right, _)| {
        right
            .utility_score
            .partial_cmp(&left.utility_score)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| left.id.cmp(&right.id))
    });

    let mut deviations = Vec::new();
    let mut expansion_hints = Vec::new();
    let mut used_tokens = estimate_overhead_tokens(options.run_id, options.objective);

    for (slice, estimated_tokens) in candidates {
        if used_tokens + estimated_tokens <= options.budget_tokens {
            used_tokens += estimated_tokens;
            deviations.push(slice);
            continue;
        }

        expansion_hints.push(ExpansionHint {
            deviation_id: slice.id.clone(),
            reason: "budget_exhausted".to_string(),
            estimated_tokens,
            hint: format!("tokenln expand {} --view evidence --budget 180", slice.id),
        });
    }

    if deviations.is_empty() && !options.report.deviations.is_empty() {
        let deviation = &options.report.deviations[0];
        let sig = deviation_signature(deviation);
        let (novelty_score, fix_hint) = if options.fixed_signatures.contains(&sig) {
            (0.10_f32, Some("previously recorded as fixed".to_string()))
        } else if options.previous_signatures.contains(&sig) {
            (0.55_f32, None)
        } else {
            (1.0_f32, None)
        };
        let minimal = DeviationSlice {
            id: "d1".to_string(),
            summary: deviation.summary.clone(),
            expected: "<omitted: run tokenln expand d1 --view evidence>".to_string(),
            actual: "<omitted: run tokenln expand d1 --view evidence>".to_string(),
            location: format_location(deviation),
            confidence: deviation.confidence,
            novelty_score: round2(novelty_score),
            utility_score: score_utility(deviation, novelty_score),
            evidence_refs: build_evidence_refs(
                deviation,
                options.raw_output,
                options.report_artifact,
            ),
            fix_hint,
        };
        used_tokens += estimate_slice_tokens(&minimal);
        deviations.push(minimal);
    }

    ContextPacket {
        packet_id,
        run_id: options.run_id.to_string(),
        budget_tokens: options.budget_tokens,
        used_tokens,
        source: options.source.to_string(),
        objective: options.objective.to_string(),
        deviations,
        expansion_hints,
        unresolved_count: options.report.deviations.len() as u32,
    }
}

pub fn deviation_signature(deviation: &Deviation) -> String {
    let mut signature = String::new();
    signature.push_str(&format!("{:?}|", deviation.kind));
    signature.push_str(&deviation.summary);
    signature.push('|');
    signature.push_str(deviation.location.file.as_deref().unwrap_or("unknown"));
    signature.push('|');
    signature.push_str(
        &deviation
            .location
            .line
            .map(|line| line.to_string())
            .unwrap_or_else(|| "?".to_string()),
    );
    signature.push('|');
    signature.push_str(
        deviation
            .location
            .symbol
            .as_deref()
            .unwrap_or("unknown_symbol"),
    );
    signature
}

fn score_utility(deviation: &Deviation, novelty_score: f32) -> f32 {
    let severity = severity_weight(deviation.kind.clone());
    let fixability = if deviation.location.file.is_some() && deviation.location.line.is_some() {
        1.0
    } else {
        0.75
    };
    round2(deviation.confidence * severity * novelty_score * fixability)
}

fn severity_weight(kind: DeviationKind) -> f32 {
    match kind {
        DeviationKind::Runtime => 1.0,
        DeviationKind::Type => 0.95,
        DeviationKind::Build => 0.9,
        DeviationKind::Test => 0.85,
        DeviationKind::Behavioral => 0.8,
    }
}

fn estimate_slice_tokens(slice: &DeviationSlice) -> u32 {
    let evidence_cost = (slice.evidence_refs.len() as u32) * 8;
    let text_cost = estimate_tokens(&format!(
        "{} {} {} {} {:.2} {:.2}",
        slice.summary,
        slice.expected,
        slice.actual,
        slice.location,
        slice.confidence,
        slice.utility_score
    ));
    text_cost + evidence_cost
}

fn estimate_overhead_tokens(run_id: &str, objective: &str) -> u32 {
    estimate_tokens(&format!("{run_id} {objective}")) + 24
}

fn estimate_tokens(text: &str) -> u32 {
    // Code-aware estimation: alphanumeric runs are ~1 token per 4 chars;
    // punctuation/operators are each ~1 token; whitespace is absorbed.
    if text.is_empty() {
        return 1;
    }
    let mut tokens = 0u32;
    let mut word_chars = 0u32;
    for ch in text.chars() {
        if ch.is_alphanumeric() || ch == '_' {
            word_chars += 1;
            if word_chars == 4 {
                tokens += 1;
                word_chars = 0;
            }
        } else {
            if word_chars > 0 {
                tokens += 1;
                word_chars = 0;
            }
            if !ch.is_whitespace() {
                tokens += 1;
            }
        }
    }
    if word_chars > 0 {
        tokens += 1;
    }
    tokens.max(1)
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

fn build_evidence_refs(
    deviation: &Deviation,
    raw_output: &str,
    report_artifact: &str,
) -> Vec<EvidenceRef> {
    let mut refs = Vec::new();

    let report_lines = report_artifact.lines().collect::<Vec<_>>();
    if !report_lines.is_empty() {
        let report_idx = report_lines
            .iter()
            .position(|line| line.contains(&deviation.summary))
            .unwrap_or(0);
        let line = report_lines[report_idx];
        refs.push(EvidenceRef {
            artifact: "report.ir.json".to_string(),
            line_start: (report_idx + 1) as u32,
            line_end: (report_idx + 1) as u32,
            hash: stable_hash_hex(line),
        });
    }

    let raw_lines = raw_output.lines().collect::<Vec<_>>();
    if raw_lines.is_empty() {
        return refs;
    }

    let patterns = build_anchor_patterns(deviation);
    let idx = patterns
        .iter()
        .find_map(|pattern| raw_lines.iter().position(|line| line.contains(pattern)))
        .unwrap_or(0);

    let start = idx.saturating_sub(2);
    let end = (idx + 3).min(raw_lines.len());
    let excerpt = raw_lines[start..end].join("\n");

    refs.push(EvidenceRef {
        artifact: "raw_output.txt".to_string(),
        line_start: (start + 1) as u32,
        line_end: end as u32,
        hash: stable_hash_hex(&excerpt),
    });

    refs
}

fn build_anchor_patterns(deviation: &Deviation) -> Vec<String> {
    let mut patterns = Vec::new();
    if let (Some(file), Some(line), Some(column)) = (
        deviation.location.file.as_ref(),
        deviation.location.line,
        deviation.location.column,
    ) {
        patterns.push(format!("{file}:{line}:{column}"));
    }
    if let (Some(file), Some(line)) = (deviation.location.file.as_ref(), deviation.location.line) {
        patterns.push(format!("{file}:{line}"));
    }
    if let Some(symbol) = deviation.location.symbol.as_ref() {
        patterns.push(symbol.clone());
    }
    patterns.push(deviation.summary.clone());
    patterns
}

fn stable_hash_hex(input: &str) -> String {
    // FNV-1a 64-bit — stable across all Rust versions and platforms.
    // Two independent hashes give 128-bit proof pointer without extra deps.
    const FNV_OFFSET: u64 = 14695981039346656037;
    const FNV_PRIME: u64 = 1099511628211;
    let hash_a = input
        .bytes()
        .fold(FNV_OFFSET, |acc, b| (acc ^ b as u64).wrapping_mul(FNV_PRIME));
    let hash_b = input
        .bytes()
        .chain(b"tokenln".iter().copied())
        .fold(FNV_OFFSET, |acc, b| (acc ^ b as u64).wrapping_mul(FNV_PRIME));
    format!("{hash_a:016x}{hash_b:016x}")
}

fn round2(value: f32) -> f32 {
    (value * 100.0).round() / 100.0
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use crate::ir::{
        Behavior, Deviation, DeviationKind, DeviationReport, ExecutionTrace, Expectation, Location,
    };

    use super::{build_context_packet, deviation_signature, BuildPacketOptions};

    fn sample_report() -> DeviationReport {
        DeviationReport::new(
            "pytest",
            vec![
                Deviation {
                    kind: DeviationKind::Test,
                    expected: Expectation {
                        description: "expected 401".to_string(),
                    },
                    actual: Behavior {
                        description: "actual 403".to_string(),
                    },
                    location: Location {
                        file: Some("tests/test_auth.py".to_string()),
                        line: Some(27),
                        column: Some(1),
                        symbol: Some("tests/test_auth.py::test_auth_invalid_token".to_string()),
                    },
                    trace: ExecutionTrace {
                        frames: vec!["pytest".to_string()],
                    },
                    confidence: 0.95,
                    confidence_reasons: vec![],
                    raw_excerpt: None,
                    summary: "tests/test_auth.py::test_auth_invalid_token failed".to_string(),
                    group_id: None,
                    is_root_cause: None,
                },
                Deviation {
                    kind: DeviationKind::Build,
                    expected: Expectation {
                        description: "type should resolve".to_string(),
                    },
                    actual: Behavior {
                        description: "unresolved symbol".to_string(),
                    },
                    location: Location {
                        file: Some("src/main.rs".to_string()),
                        line: Some(10),
                        column: Some(5),
                        symbol: Some("main".to_string()),
                    },
                    trace: ExecutionTrace {
                        frames: vec!["cargo build".to_string()],
                    },
                    confidence: 0.7,
                    confidence_reasons: vec![],
                    raw_excerpt: None,
                    summary: "build failed at src/main.rs:10".to_string(),
                    group_id: None,
                    is_root_cause: None,
                },
            ],
        )
    }

    #[test]
    fn builds_budget_bounded_packet() {
        let report = sample_report();
        let packet = build_context_packet(BuildPacketOptions {
            run_id: "run-1",
            source: "pytest",
            objective: "fix failures",
            budget_tokens: 110,
            report: &report,
            raw_output: "tests/test_auth.py::test_auth_invalid_token\nsrc/main.rs:10:5",
            report_artifact: "{}\n",
            previous_signatures: &HashSet::new(),
            fixed_signatures: &HashSet::new(),
        });

        assert!(!packet.deviations.is_empty());
        assert_eq!(packet.run_id, "run-1");
        assert_eq!(packet.unresolved_count, 2);
    }

    #[test]
    fn novelty_score_drops_for_repeated_deviation() {
        let report = sample_report();
        let mut prev = HashSet::new();
        prev.insert(deviation_signature(&report.deviations[0]));

        let packet = build_context_packet(BuildPacketOptions {
            run_id: "run-2",
            source: "pytest",
            objective: "fix failures",
            budget_tokens: 500,
            report: &report,
            raw_output: "tests/test_auth.py::test_auth_invalid_token",
            report_artifact: "{}\n",
            previous_signatures: &prev,
            fixed_signatures: &HashSet::new(),
        });

        let repeated = packet
            .deviations
            .iter()
            .find(|slice| slice.summary.contains("invalid_token"))
            .expect("expected repeated deviation to be present");
        assert!(repeated.novelty_score < 1.0);
    }

    #[test]
    fn fixed_deviation_gets_very_low_novelty_and_fix_hint() {
        let report = sample_report();
        let mut fixed = HashSet::new();
        fixed.insert(deviation_signature(&report.deviations[0]));

        let packet = build_context_packet(BuildPacketOptions {
            run_id: "run-3",
            source: "pytest",
            objective: "fix failures",
            budget_tokens: 500,
            report: &report,
            raw_output: "",
            report_artifact: "{}",
            previous_signatures: &HashSet::new(),
            fixed_signatures: &fixed,
        });

        let fixed_slice = packet
            .deviations
            .iter()
            .find(|slice| slice.summary.contains("invalid_token"))
            .expect("fixed deviation should still appear in packet");

        assert!(
            fixed_slice.novelty_score <= 0.10,
            "fixed deviation novelty should be ≤0.10, got {}",
            fixed_slice.novelty_score
        );
        assert!(
            fixed_slice.fix_hint.is_some(),
            "fix_hint should be set for fixed deviations"
        );
    }
}
