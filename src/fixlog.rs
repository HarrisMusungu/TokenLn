use std::collections::HashSet;
use std::fs::{self, OpenOptions};
use std::io::{BufRead, BufReader, Write};
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

/// A single entry in the fix log recording that a specific deviation was fixed.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FixLogEntry {
    /// Unix timestamp in milliseconds when the fix was recorded.
    pub timestamp_ms: u64,
    /// Stable signature of the deviation that was fixed (from `deviation_signature()`).
    pub deviation_signature: String,
    /// Source tool (e.g. "pytest", "cargo test").
    pub source: String,
    /// Run ID from which the deviation was taken.
    pub run_id: String,
    /// Optional user-provided note describing what was fixed.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

impl FixLogEntry {
    pub fn new(
        deviation_signature: impl Into<String>,
        source: impl Into<String>,
        run_id: impl Into<String>,
        note: Option<String>,
    ) -> Self {
        let timestamp_ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0);
        Self {
            timestamp_ms,
            deviation_signature: deviation_signature.into(),
            source: source.into(),
            run_id: run_id.into(),
            note,
        }
    }
}

/// Appends a fix log entry to the newline-delimited JSON file at `fix_log_path`.
/// Creates the file and any parent directories if they do not exist.
pub fn record_fix(fix_log_path: &Path, entry: &FixLogEntry) -> Result<(), String> {
    if let Some(parent) = fix_log_path.parent() {
        fs::create_dir_all(parent).map_err(|err| {
            format!(
                "failed to create fix log directory '{}': {err}",
                parent.display()
            )
        })?;
    }

    let json = serde_json::to_string(entry)
        .map_err(|err| format!("failed to serialize fix log entry: {err}"))?;

    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(fix_log_path)
        .map_err(|err| format!("failed to open fix log '{}': {err}", fix_log_path.display()))?;

    writeln!(file, "{json}").map_err(|err| {
        format!(
            "failed to write fix log entry to '{}': {err}",
            fix_log_path.display()
        )
    })?;

    Ok(())
}

/// Loads all deviation signatures from the fix log at `fix_log_path`.
/// Returns an empty set if the file does not exist or cannot be read.
/// Malformed lines are silently skipped.
pub fn load_fix_signatures(fix_log_path: &Path) -> HashSet<String> {
    let file = match fs::File::open(fix_log_path) {
        Ok(f) => f,
        Err(_) => return HashSet::new(),
    };

    BufReader::new(file)
        .lines()
        .filter_map(Result::ok)
        .filter(|line| !line.trim().is_empty())
        .filter_map(|line| {
            serde_json::from_str::<FixLogEntry>(&line)
                .ok()
                .map(|entry| entry.deviation_signature)
        })
        .collect()
}

/// Returns all entries from the fix log, oldest first.
/// Returns an empty vec if the file does not exist or cannot be read.
pub fn load_fix_log(fix_log_path: &Path) -> Vec<FixLogEntry> {
    let file = match fs::File::open(fix_log_path) {
        Ok(f) => f,
        Err(_) => return Vec::new(),
    };

    BufReader::new(file)
        .lines()
        .filter_map(Result::ok)
        .filter(|line| !line.trim().is_empty())
        .filter_map(|line| serde_json::from_str::<FixLogEntry>(&line).ok())
        .collect()
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::{load_fix_log, load_fix_signatures, record_fix, FixLogEntry};

    fn tmp_log_path(name: &str) -> PathBuf {
        PathBuf::from(format!("/tmp/tokenln-fixlog-{name}.jsonl"))
    }

    #[test]
    fn round_trips_fix_log_entries() {
        let path = tmp_log_path("round-trip");
        let _ = std::fs::remove_file(&path);

        let entry = FixLogEntry::new("sig-abc|file.py|10|sym", "pytest", "run-001", None);
        record_fix(&path, &entry).expect("record should succeed");

        let sigs = load_fix_signatures(&path);
        assert!(
            sigs.contains("sig-abc|file.py|10|sym"),
            "recorded signature should be present"
        );

        let entries = load_fix_log(&path);
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].source, "pytest");
        assert_eq!(entries[0].run_id, "run-001");
    }

    #[test]
    fn returns_empty_set_for_missing_file() {
        let sigs = load_fix_signatures(std::path::Path::new("/tmp/no-such-fixlog.jsonl"));
        assert!(sigs.is_empty());
    }

    #[test]
    fn appends_multiple_entries() {
        let path = tmp_log_path("multi");
        let _ = std::fs::remove_file(&path);

        for i in 0..3u32 {
            let entry = FixLogEntry::new(
                format!("sig-{i}"),
                "cargo test",
                format!("run-{i}"),
                Some(format!("note {i}")),
            );
            record_fix(&path, &entry).expect("record should succeed");
        }

        let sigs = load_fix_signatures(&path);
        assert_eq!(sigs.len(), 3);
        for i in 0..3u32 {
            assert!(sigs.contains(&format!("sig-{i}")));
        }
    }
}
