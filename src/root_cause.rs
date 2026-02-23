//! Persistent root-cause identity graph.
//!
//! Tracks how many times each deviation signature has been observed across all
//! recorded runs, whether it has been resolved, and whether it has regressed
//! (re-appeared after being marked resolved).  This drives logarithmic novelty
//! decay in the context packet budget controller.
//!
//! Storage: `<tokenln-dir>/root_cause_graph.jsonl` — newline-delimited JSON,
//! one [`RootCauseRecord`] per unique deviation signature.  The file is
//! rewritten in full on every [`RootCauseStore::save`] call; at typical repo
//! scale (hundreds of unique signatures) this is negligible.

use std::collections::{HashMap, HashSet};
use std::fs::{self, OpenOptions};
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// Persisted metadata for a single root-cause identity.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RootCauseRecord {
    /// Stable deviation signature produced by `deviation_signature()`.
    pub signature: String,
    /// Run ID in which this signature was first observed.
    pub first_seen_run: String,
    /// Run ID in which this signature was most recently observed.
    pub last_seen_run: String,
    /// Total number of runs in which this signature appeared.
    pub occurrence_count: u32,
    /// True after `mark_resolved()` is called and before the signature reappears.
    pub is_resolved: bool,
    /// True when the signature reappeared after being marked resolved.
    pub is_regression: bool,
}

/// In-memory root-cause store backed by a JSONL file.
pub struct RootCauseStore {
    records: HashMap<String, RootCauseRecord>,
    path: PathBuf,
}

impl RootCauseStore {
    /// Load the store from `path`.  Missing or unreadable file → empty store.
    pub fn load(path: &Path) -> Self {
        let records = load_records(path);
        Self {
            records,
            path: path.to_path_buf(),
        }
    }

    /// Rewrite the JSONL file with the current in-memory state.
    pub fn save(&self) -> Result<(), String> {
        if let Some(parent) = self.path.parent() {
            fs::create_dir_all(parent).map_err(|err| {
                format!(
                    "failed to create root cause graph directory '{}': {err}",
                    parent.display()
                )
            })?;
        }

        let mut file = OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(true)
            .open(&self.path)
            .map_err(|err| {
                format!(
                    "failed to open root cause graph '{}': {err}",
                    self.path.display()
                )
            })?;

        for record in self.records.values() {
            let json = serde_json::to_string(record)
                .map_err(|err| format!("failed to serialize root cause record: {err}"))?;
            writeln!(file, "{json}")
                .map_err(|err| format!("failed to write root cause record: {err}"))?;
        }

        Ok(())
    }

    /// Update the store with deviation signatures observed in a new run.
    ///
    /// - Increments `occurrence_count` for each known signature.
    /// - Creates a new record for previously unseen signatures.
    /// - Sets `is_regression = true` when a resolved signature reappears.
    pub fn record_run(&mut self, run_id: &str, signatures: &[String]) {
        for sig in signatures {
            match self.records.get_mut(sig) {
                Some(rec) => {
                    rec.occurrence_count += 1;
                    rec.last_seen_run = run_id.to_string();
                    if rec.is_resolved {
                        rec.is_resolved = false;
                        rec.is_regression = true;
                    }
                }
                None => {
                    self.records.insert(
                        sig.clone(),
                        RootCauseRecord {
                            signature: sig.clone(),
                            first_seen_run: run_id.to_string(),
                            last_seen_run: run_id.to_string(),
                            occurrence_count: 1,
                            is_resolved: false,
                            is_regression: false,
                        },
                    );
                }
            }
        }
    }

    /// Mark a deviation signature as resolved (i.e. the user ran `tokenln fixed`).
    /// No-op if the signature is not yet tracked.
    pub fn mark_resolved(&mut self, signature: &str) {
        if let Some(rec) = self.records.get_mut(signature) {
            rec.is_resolved = true;
            rec.is_regression = false;
        }
    }

    /// Returns a snapshot map of `signature → occurrence_count` for all records.
    pub fn occurrence_counts(&self) -> HashMap<String, u32> {
        self.records
            .iter()
            .map(|(sig, rec)| (sig.clone(), rec.occurrence_count))
            .collect()
    }

    /// Returns the set of signatures that were resolved but have since reappeared.
    pub fn regression_signatures(&self) -> HashSet<String> {
        self.records
            .values()
            .filter(|rec| rec.is_regression)
            .map(|rec| rec.signature.clone())
            .collect()
    }
}

// ─── Helpers ──────────────────────────────────────────────────────────────────

fn load_records(path: &Path) -> HashMap<String, RootCauseRecord> {
    let file = match fs::File::open(path) {
        Ok(f) => f,
        Err(_) => return HashMap::new(),
    };

    BufReader::new(file)
        .lines()
        .filter_map(Result::ok)
        .filter(|line| !line.trim().is_empty())
        .filter_map(|line| serde_json::from_str::<RootCauseRecord>(&line).ok())
        .map(|rec| (rec.signature.clone(), rec))
        .collect()
}

// ─── Unit tests ───────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn tmp_path(name: &str) -> PathBuf {
        PathBuf::from(format!("/tmp/tokenln-rg-{name}.jsonl"))
    }

    #[test]
    fn new_signature_gets_count_one() {
        let path = tmp_path("new-sig");
        let _ = fs::remove_file(&path);
        let mut store = RootCauseStore::load(&path);
        store.record_run("run-1", &["sig-a".to_string()]);
        let counts = store.occurrence_counts();
        assert_eq!(counts.get("sig-a").copied(), Some(1));
        assert!(store.regression_signatures().is_empty());
    }

    #[test]
    fn repeated_signature_increments_count() {
        let path = tmp_path("repeat");
        let _ = fs::remove_file(&path);
        let mut store = RootCauseStore::load(&path);
        store.record_run("run-1", &["sig-b".to_string()]);
        store.record_run("run-2", &["sig-b".to_string()]);
        store.record_run("run-3", &["sig-b".to_string()]);
        let counts = store.occurrence_counts();
        assert_eq!(counts.get("sig-b").copied(), Some(3));
    }

    #[test]
    fn resolved_then_reappeared_is_regression() {
        let path = tmp_path("regression");
        let _ = fs::remove_file(&path);
        let mut store = RootCauseStore::load(&path);
        store.record_run("run-1", &["sig-c".to_string()]);
        store.mark_resolved("sig-c");
        // Not a regression yet — just resolved.
        assert!(store.regression_signatures().is_empty());
        // Reappears in run-2 → regression.
        store.record_run("run-2", &["sig-c".to_string()]);
        assert!(store.regression_signatures().contains("sig-c"));
    }

    #[test]
    fn round_trips_across_save_and_load() {
        let path = tmp_path("round-trip");
        let _ = fs::remove_file(&path);
        let mut store = RootCauseStore::load(&path);
        store.record_run("run-1", &["sig-d".to_string(), "sig-e".to_string()]);
        store.record_run("run-2", &["sig-d".to_string()]);
        store.save().expect("save should succeed");

        let store2 = RootCauseStore::load(&path);
        let counts = store2.occurrence_counts();
        assert_eq!(counts.get("sig-d").copied(), Some(2));
        assert_eq!(counts.get("sig-e").copied(), Some(1));
    }

    #[test]
    fn missing_file_returns_empty_store() {
        let store = RootCauseStore::load(Path::new("/tmp/no-such-rg-tokenln.jsonl"));
        assert!(store.occurrence_counts().is_empty());
    }
}
