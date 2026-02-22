use std::cmp::Ordering;
use std::collections::{HashMap, HashSet};

use crate::ir::DeviationReport;
use crate::pipeline::Optimizer;

pub struct BasicOptimizer;

impl Optimizer for BasicOptimizer {
    fn optimize(&self, mut report: DeviationReport) -> DeviationReport {
        // Deduplicate by (summary, file, line).
        let mut seen = HashSet::new();
        report.deviations.retain(|deviation| {
            seen.insert((
                deviation.summary.clone(),
                deviation.location.file.clone(),
                deviation.location.line,
            ))
        });

        // Sort by confidence descending before grouping so root-cause election is stable.
        report.deviations.sort_by(|a, b| {
            b.confidence
                .partial_cmp(&a.confidence)
                .unwrap_or(Ordering::Equal)
        });

        // Assign causal groups for deviations that share a source file.
        assign_causal_groups(&mut report);

        report
    }
}

/// Groups deviations that share the same source file.
///
/// Within each group of 2+ deviations, the one with the lowest line number is
/// elected the root cause (`is_root_cause = true`). The others are marked as
/// likely cascading failures (`is_root_cause = false`). All members receive a
/// stable `group_id` derived from the file path.
///
/// After grouping the list is re-sorted so that root causes precede their
/// cascades, preserving relative confidence ordering within each role.
fn assign_causal_groups(report: &mut DeviationReport) {
    // Map file → list of deviation indices that live in that file.
    let mut file_groups: HashMap<String, Vec<usize>> = HashMap::new();
    for (idx, deviation) in report.deviations.iter().enumerate() {
        if let Some(file) = deviation.location.file.as_ref() {
            file_groups.entry(file.clone()).or_default().push(idx);
        }
    }

    for (file, indices) in &file_groups {
        if indices.len() < 2 {
            continue;
        }

        let group_id = stable_group_id(file);

        // The root cause is the deviation with the smallest line number.
        // If line numbers are absent, treat the first (highest-confidence)
        // deviation as the root cause.
        let root_idx = *indices
            .iter()
            .min_by_key(|&&idx| report.deviations[idx].location.line.unwrap_or(u32::MAX))
            .expect("indices is non-empty");

        for &idx in indices {
            report.deviations[idx].group_id = Some(group_id.clone());
            report.deviations[idx].is_root_cause = Some(idx == root_idx);
        }
    }

    // Re-sort: within groups, root causes come before cascades.
    // Across groups (and ungrouped), maintain confidence descending order.
    report.deviations.sort_by(|a, b| {
        // Primary: root causes sort before cascades.
        let a_root = a.is_root_cause.unwrap_or(true);
        let b_root = b.is_root_cause.unwrap_or(true);
        match (a_root, b_root) {
            (true, false) => Ordering::Less,
            (false, true) => Ordering::Greater,
            _ => {
                // Secondary: higher confidence first.
                b.confidence
                    .partial_cmp(&a.confidence)
                    .unwrap_or(Ordering::Equal)
            }
        }
    });
}

/// Produces a short, stable identifier for a causal group derived from the file path.
/// Uses FNV-1a (stable across Rust versions/platforms) to keep group IDs human-readable.
fn stable_group_id(file: &str) -> String {
    const FNV_OFFSET: u64 = 14695981039346656037;
    const FNV_PRIME: u64 = 1099511628211;
    let hash = b"group"
        .iter()
        .chain(file.bytes().collect::<Vec<_>>().iter())
        .fold(FNV_OFFSET, |acc, &b| (acc ^ b as u64).wrapping_mul(FNV_PRIME));
    // Keep it short: first 6 hex chars.
    format!("grp-{:06x}", hash & 0xFFFFFF)
}

#[cfg(test)]
mod tests {
    use crate::ir::{
        Behavior, Deviation, DeviationKind, DeviationReport, ExecutionTrace, Expectation, Location,
    };
    use crate::pipeline::Optimizer;

    use super::BasicOptimizer;

    fn make_deviation(file: &str, line: u32, confidence: f32) -> Deviation {
        Deviation {
            kind: DeviationKind::Test,
            expected: Expectation {
                description: "expected".to_string(),
            },
            actual: Behavior {
                description: "actual".to_string(),
            },
            location: Location {
                file: Some(file.to_string()),
                line: Some(line),
                column: None,
                symbol: None,
            },
            trace: ExecutionTrace { frames: vec![] },
            confidence,
            confidence_reasons: vec![],
            raw_excerpt: None,
            summary: format!("{file}:{line}"),
            group_id: None,
            is_root_cause: None,
        }
    }

    #[test]
    fn groups_deviations_from_same_file() {
        let report = DeviationReport::new(
            "cargo test",
            vec![
                make_deviation("src/auth.rs", 40, 0.80),
                make_deviation("src/auth.rs", 10, 0.75),
                make_deviation("src/other.rs", 5, 0.90),
            ],
        );

        let optimized = BasicOptimizer.optimize(report);

        let auth_deviations: Vec<_> = optimized
            .deviations
            .iter()
            .filter(|d| d.location.file.as_deref() == Some("src/auth.rs"))
            .collect();

        assert_eq!(auth_deviations.len(), 2);
        // Both share a group_id.
        assert_eq!(
            auth_deviations[0].group_id, auth_deviations[1].group_id,
            "same-file deviations should share group_id"
        );
        assert!(
            auth_deviations[0].group_id.is_some(),
            "group_id should be assigned"
        );

        // Line 10 is the root cause (lower line number).
        let root = auth_deviations
            .iter()
            .find(|d| d.is_root_cause == Some(true))
            .expect("root cause should be elected");
        assert_eq!(root.location.line, Some(10));

        // The other.rs deviation should NOT be grouped.
        let other = optimized
            .deviations
            .iter()
            .find(|d| d.location.file.as_deref() == Some("src/other.rs"))
            .expect("other deviation should be present");
        assert!(
            other.group_id.is_none(),
            "single-file deviation should not be grouped"
        );
    }

    #[test]
    fn root_causes_sort_before_cascades() {
        let report = DeviationReport::new(
            "cargo test",
            vec![
                make_deviation("src/lib.rs", 50, 0.95), // would be cascade (higher line)
                make_deviation("src/lib.rs", 5, 0.70),  // root cause (lower line, lower conf)
            ],
        );

        let optimized = BasicOptimizer.optimize(report);
        assert_eq!(optimized.deviations.len(), 2);

        // Root cause (line 5) should appear first despite lower confidence.
        assert_eq!(
            optimized.deviations[0].is_root_cause,
            Some(true),
            "root cause should sort first"
        );
        assert_eq!(optimized.deviations[0].location.line, Some(5));
    }

    #[test]
    fn deduplicates_identical_deviations() {
        let dup = make_deviation("src/lib.rs", 10, 0.80);
        let report = DeviationReport::new("pytest", vec![dup.clone(), dup]);
        let optimized = BasicOptimizer.optimize(report);
        assert_eq!(optimized.deviations.len(), 1);
    }
}
