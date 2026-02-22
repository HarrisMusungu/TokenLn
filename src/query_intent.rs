//! Rule-based query intent classifier.
//!
//! Maps free-form query text to a `QueryIntent` variant, which the repo query
//! layer uses to choose the best search strategy.  No ML or external deps —
//! classification is purely keyword/pattern based.

/// The inferred purpose of a repo query.
#[derive(Debug, Clone, Eq, PartialEq)]
pub enum QueryIntent {
    /// User wants to find where a symbol (function, struct, class, …) is defined.
    /// Example: "where is build_context_packet defined"
    FindDefinition,
    /// User wants all call sites or usages of a symbol.
    /// Example: "who calls process_event"
    FindReferences,
    /// User wants a conceptual understanding of a module, feature, or flow.
    /// Example: "how does the MCP server handle requests"
    Understand,
    /// User wants to find all occurrences of a text pattern or error message.
    /// Example: "find TODO comments" or "error: unresolved import"
    FindPattern,
    /// User wants recent changes to a file or symbol (git history).
    /// Example: "what changed in repo.rs" or "recent commits to optimizer"
    RecentChanges,
    /// Catch-all for queries that don't match any specific pattern.
    General,
}

impl QueryIntent {
    pub fn as_str(&self) -> &'static str {
        match self {
            QueryIntent::FindDefinition => "find_definition",
            QueryIntent::FindReferences => "find_references",
            QueryIntent::Understand => "understand",
            QueryIntent::FindPattern => "find_pattern",
            QueryIntent::RecentChanges => "recent_changes",
            QueryIntent::General => "general",
        }
    }
}

/// Classify the user's query into a `QueryIntent`.
///
/// Rule priority (first match wins):
/// 1. `FindDefinition` — "where is", "definition of", "defined in", "locate fn/struct/class/…"
/// 2. `FindReferences` — "who calls", "callers of", "references to", "uses of", "usages of"
/// 3. `Understand` — "how does", "how is", "explain", "understand", "overview of", "what does"
/// 4. `RecentChanges` — "recent", "changed", "history", "git log", "last modified", "what changed"
/// 5. `FindPattern` — "find all", "search for", "occurrences of", "grep", "pattern"
/// 6. `General` — everything else
pub fn classify(query: &str) -> QueryIntent {
    let lower = query.to_lowercase();
    let words: Vec<&str> = lower
        .split(|c: char| !c.is_alphanumeric() && c != '_')
        .filter(|w| !w.is_empty())
        .collect();

    // ── FindDefinition ────────────────────────────────────────────────────────
    if contains_phrase(&lower, &["where is", "where's"])
        || contains_phrase(&lower, &["definition of", "defined in", "defined at"])
        || contains_phrase(&lower, &["locate the", "find the definition"])
        || (contains_word(&words, "where") && contains_word(&words, "defined"))
        || (contains_word(&words, "show") && contains_word(&words, "definition"))
        || has_symbol_kind_prefix(&lower)
    {
        return QueryIntent::FindDefinition;
    }

    // ── FindReferences ────────────────────────────────────────────────────────
    if contains_phrase(&lower, &["who calls", "callers of", "all callers"])
        || contains_phrase(&lower, &["references to", "reference to"])
        || contains_phrase(&lower, &["uses of", "usages of", "usage of"])
        || contains_phrase(&lower, &["calls to", "call sites"])
        || (contains_word(&words, "who") && contains_word(&words, "calls"))
        || (contains_word(&words, "find") && contains_word(&words, "callers"))
    {
        return QueryIntent::FindReferences;
    }

    // ── Understand ────────────────────────────────────────────────────────────
    if contains_phrase(&lower, &["how does", "how is", "how do"])
        || contains_phrase(&lower, &["explain", "understand", "overview"])
        || contains_phrase(&lower, &["what does", "what is", "what are"])
        || contains_phrase(&lower, &["describe", "show me how", "walk me through"])
        || contains_word(&words, "architecture")
        || contains_word(&words, "design")
    {
        return QueryIntent::Understand;
    }

    // ── RecentChanges ─────────────────────────────────────────────────────────
    if contains_phrase(&lower, &["recent changes", "recently changed", "what changed"])
        || contains_phrase(&lower, &["git log", "git history", "commit history"])
        || contains_phrase(&lower, &["last modified", "last changed", "last updated"])
        || contains_word(&words, "changelog")
        || (contains_word(&words, "recent") && contains_word(&words, "commits"))
        || (contains_word(&words, "history") && !lower.contains("history of"))
    {
        return QueryIntent::RecentChanges;
    }

    // ── FindPattern ───────────────────────────────────────────────────────────
    if contains_phrase(&lower, &["find all", "search for", "look for"])
        || contains_phrase(&lower, &["occurrences of", "all occurrences"])
        || contains_phrase(&lower, &["every place", "every file"])
        || contains_word(&words, "grep")
        || contains_word(&words, "pattern")
        || contains_word(&words, "todo")
        || contains_word(&words, "fixme")
        || contains_word(&words, "hack")
    {
        return QueryIntent::FindPattern;
    }

    QueryIntent::General
}

// ─── Helpers ──────────────────────────────────────────────────────────────────

/// True if any of the given phrases appear as a substring in `text`.
fn contains_phrase(text: &str, phrases: &[&str]) -> bool {
    phrases.iter().any(|p| text.contains(p))
}

/// True if `word` is present in the tokenised word list.
fn contains_word(words: &[&str], word: &str) -> bool {
    words.contains(&word)
}

/// True if the query starts with a symbol-kind keyword like "fn", "struct",
/// "class", "def", "function", "method", "enum", "interface", "trait".
fn has_symbol_kind_prefix(lower: &str) -> bool {
    const KINDS: &[&str] = &[
        "fn ", "struct ", "class ", "def ", "function ", "method ", "enum ",
        "interface ", "trait ", "type ", "const ", "var ",
    ];
    KINDS.iter().any(|k| lower.starts_with(k))
}

// ─── Unit tests ───────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classifies_definition_queries() {
        assert_eq!(classify("where is build_context_packet defined"), QueryIntent::FindDefinition);
        assert_eq!(classify("definition of BasicOptimizer"), QueryIntent::FindDefinition);
        assert_eq!(classify("fn stable_hash_hex"), QueryIntent::FindDefinition);
        assert_eq!(classify("struct DeviationReport"), QueryIntent::FindDefinition);
    }

    #[test]
    fn classifies_reference_queries() {
        assert_eq!(classify("who calls query_repo_context"), QueryIntent::FindReferences);
        assert_eq!(classify("all callers of deviation_signature"), QueryIntent::FindReferences);
        assert_eq!(classify("references to build_context_packet"), QueryIntent::FindReferences);
        assert_eq!(classify("usages of the ContextPacket type"), QueryIntent::FindReferences);
    }

    #[test]
    fn classifies_understand_queries() {
        assert_eq!(classify("how does the MCP server handle requests"), QueryIntent::Understand);
        assert_eq!(classify("explain the deviation pipeline"), QueryIntent::Understand);
        assert_eq!(classify("what does the optimizer do"), QueryIntent::Understand);
        assert_eq!(classify("overview of the repo query system"), QueryIntent::Understand);
    }

    #[test]
    fn classifies_recent_changes_queries() {
        assert_eq!(classify("what changed in repo.rs"), QueryIntent::RecentChanges);
        assert_eq!(classify("recent commits to the mcp module"), QueryIntent::RecentChanges);
        assert_eq!(classify("git log for optimizer.rs"), QueryIntent::RecentChanges);
    }

    #[test]
    fn classifies_find_pattern_queries() {
        assert_eq!(classify("find all TODO comments"), QueryIntent::FindPattern);
        assert_eq!(classify("grep for unwrap calls"), QueryIntent::FindPattern);
        assert_eq!(classify("search for DefaultHasher occurrences"), QueryIntent::FindPattern);
    }

    #[test]
    fn classifies_general_queries() {
        assert_eq!(classify("cargo build failing"), QueryIntent::General);
        assert_eq!(classify("context packet budget"), QueryIntent::General);
        assert_eq!(classify(""), QueryIntent::General);
    }
}
