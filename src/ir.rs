use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeviationReport {
    pub schema_version: String,
    pub source: String,
    pub deviations: Vec<Deviation>,
}

impl DeviationReport {
    pub fn new(source: impl Into<String>, deviations: Vec<Deviation>) -> Self {
        Self {
            schema_version: "0.1".to_string(),
            source: source.into(),
            deviations,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Deviation {
    pub kind: DeviationKind,
    pub expected: Expectation,
    pub actual: Behavior,
    pub location: Location,
    pub trace: ExecutionTrace,
    pub confidence: f32,
    #[serde(default)]
    pub confidence_reasons: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub raw_excerpt: Option<String>,
    pub summary: String,
    /// Assigned by the optimizer when multiple deviations share the same source file.
    /// Deviations in the same group likely share a root cause.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub group_id: Option<String>,
    /// True if this deviation is the inferred root cause of its group (earliest line in file).
    /// False if it is likely a cascading failure from the root cause.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub is_root_cause: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DeviationKind {
    Test,
    Type,
    Build,
    Runtime,
    Behavioral,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Expectation {
    pub description: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Behavior {
    pub description: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Location {
    pub file: Option<String>,
    pub line: Option<u32>,
    pub column: Option<u32>,
    pub symbol: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExecutionTrace {
    pub frames: Vec<String>,
}
