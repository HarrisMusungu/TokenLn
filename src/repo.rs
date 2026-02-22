use std::collections::{HashMap, HashSet};
use std::ffi::OsStr;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use serde::Serialize;
use serde_json::Value;

use crate::query_intent::{classify, QueryIntent};
use crate::symbol_index::{index_directory, search_symbols, IndexOptions};

const DEFAULT_EXCLUDED_DIRS: &[&str] = &[".git", "target", ".tokenln", "node_modules"];
const MAX_FALLBACK_FILE_BYTES: u64 = 1_000_000;

#[derive(Debug, Clone)]
pub struct RepoSearchOptions<'a> {
    pub repo_root: &'a Path,
    pub scope: Option<&'a Path>,
    pub query: &'a str,
    pub glob: Option<&'a str>,
    pub max_results: usize,
    pub ignore_case: bool,
    pub fixed_strings: bool,
}

#[derive(Debug, Clone)]
pub struct RepoReadOptions<'a> {
    pub repo_root: &'a Path,
    pub path: &'a Path,
    pub start_line: u32,
    pub end_line: Option<u32>,
    pub max_chars: usize,
}

#[derive(Debug, Clone)]
pub struct RepoTreeOptions<'a> {
    pub repo_root: &'a Path,
    pub scope: Option<&'a Path>,
    pub max_depth: u32,
    pub max_entries: usize,
}

#[derive(Debug, Clone)]
pub struct RepoQueryOptions<'a> {
    pub repo_root: &'a Path,
    pub scope: Option<&'a Path>,
    pub objective: &'a str,
    pub budget_tokens: u32,
    pub max_findings: usize,
    pub max_hints: usize,
}

#[derive(Debug, Clone, Serialize)]
pub struct RepoSearchResult {
    pub backend: String,
    pub query: String,
    pub root: String,
    pub scope: String,
    pub returned: usize,
    pub truncated: bool,
    pub matches: Vec<RepoMatch>,
}

#[derive(Debug, Clone, Serialize)]
pub struct RepoMatch {
    pub path: String,
    pub line: u32,
    pub column: u32,
    pub snippet: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct RepoReadResult {
    pub root: String,
    pub path: String,
    pub start_line: u32,
    pub end_line: u32,
    pub total_lines: u32,
    pub truncated: bool,
    pub content: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct RepoTreeResult {
    pub root: String,
    pub scope: String,
    pub max_depth: u32,
    pub max_entries: usize,
    pub returned: usize,
    pub truncated: bool,
    pub entries: Vec<RepoTreeEntry>,
}

#[derive(Debug, Clone, Serialize)]
pub struct RepoTreeEntry {
    pub path: String,
    pub kind: String,
    pub depth: u32,
}

#[derive(Debug, Clone, Serialize)]
pub struct RepoContextPacket {
    pub packet_id: String,
    pub root: String,
    pub scope: String,
    pub objective: String,
    pub budget_tokens: u32,
    pub used_tokens: u32,
    pub findings: Vec<RepoContextFinding>,
    pub expansion_hints: Vec<RepoContextHint>,
    pub omitted_hints: usize,
}

#[derive(Debug, Clone, Serialize)]
pub struct RepoContextFinding {
    pub id: String,
    pub summary: String,
    pub path: String,
    pub line: Option<u32>,
    pub relevance_score: f32,
    pub snippet: String,
    pub read_hint: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct RepoContextHint {
    pub finding_id: String,
    pub reason: String,
    pub estimated_tokens: u32,
    pub hint: String,
}

#[derive(Debug, Clone)]
pub struct RepoLogOptions<'a> {
    pub repo_root: &'a Path,
    /// File path relative to `repo_root` to query git log for.
    pub path: &'a Path,
    /// Maximum number of commits to return.
    pub limit: usize,
}

#[derive(Debug, Clone, Serialize)]
pub struct RepoLogResult {
    pub root: String,
    pub path: String,
    pub returned: usize,
    pub entries: Vec<RepoLogEntry>,
}

#[derive(Debug, Clone, Serialize)]
pub struct RepoLogEntry {
    /// Short commit hash.
    pub hash: String,
    /// Commit date in ISO-8601 format (author date).
    pub date: String,
    /// One-line commit subject.
    pub subject: String,
}

pub fn search_repo(options: RepoSearchOptions<'_>) -> Result<RepoSearchResult, String> {
    if options.query.trim().is_empty() {
        return Err("repo search: query must not be empty".to_string());
    }

    let max_results = options.max_results.max(1);
    let root = canonicalize_existing_dir(options.repo_root, "repo_root")?;
    let scope = resolve_scope(&root, options.scope)?;

    match search_with_rg(&root, &scope, &options, max_results) {
        Ok(result) => Ok(result),
        Err(RgSearchError::NotAvailable) => {
            Ok(search_with_fallback(&root, &scope, &options, max_results))
        }
        Err(RgSearchError::Failed(message)) => Err(message),
    }
}

pub fn read_repo_file(options: RepoReadOptions<'_>) -> Result<RepoReadResult, String> {
    let root = canonicalize_existing_dir(options.repo_root, "repo_root")?;
    let file_path = resolve_within_root(&root, options.path)?;
    if !file_path.is_file() {
        return Err(format!(
            "repo read: '{}' is not a readable file",
            file_path.display()
        ));
    }

    let text = fs::read_to_string(&file_path)
        .map_err(|err| format!("repo read: failed to read '{}': {err}", file_path.display()))?;
    let lines = text.lines().collect::<Vec<_>>();
    let total_lines = lines.len() as u32;

    let start_line = options.start_line.max(1);
    let default_end = start_line.saturating_add(199);
    let end_line = options.end_line.unwrap_or(default_end).max(start_line);

    let start_idx = (start_line as usize).saturating_sub(1).min(lines.len());
    let end_idx = (end_line as usize).min(lines.len());
    let mut content = if start_idx >= end_idx {
        String::new()
    } else {
        lines[start_idx..end_idx].join("\n")
    };

    let max_chars = options.max_chars.max(40);
    let mut truncated = false;
    if content.chars().count() > max_chars {
        truncated = true;
        content = truncate_chars(&content, max_chars.saturating_sub(32));
        content.push_str("\n...[truncated]");
    }

    Ok(RepoReadResult {
        root: root.display().to_string(),
        path: relative_display(&root, &file_path),
        start_line,
        end_line: end_idx as u32,
        total_lines,
        truncated,
        content,
    })
}

pub fn tree_repo(options: RepoTreeOptions<'_>) -> Result<RepoTreeResult, String> {
    let root = canonicalize_existing_dir(options.repo_root, "repo_root")?;
    let scope = resolve_scope(&root, options.scope)?;
    let mut entries = Vec::new();
    let mut truncated = false;
    let max_entries = options.max_entries.max(1);
    collect_tree_entries(
        &root,
        &scope,
        0,
        options.max_depth,
        max_entries,
        &mut entries,
        &mut truncated,
    )?;

    Ok(RepoTreeResult {
        root: root.display().to_string(),
        scope: relative_display(&root, &scope),
        max_depth: options.max_depth,
        max_entries,
        returned: entries.len(),
        truncated,
        entries,
    })
}

pub fn query_repo_context(options: RepoQueryOptions<'_>) -> Result<RepoContextPacket, String> {
    if options.objective.trim().is_empty() {
        return Err("repo query: objective must not be empty".to_string());
    }

    let root = canonicalize_existing_dir(options.repo_root, "repo_root")?;
    let scope = resolve_scope(&root, options.scope)?;
    let objective = options.objective.trim();
    let max_findings = options.max_findings.max(1);
    let max_hints = options.max_hints;
    let budget_tokens = options.budget_tokens.max(80);
    let hint_budget_tokens = budget_tokens.saturating_div(4).max(24);
    let objective_lower = objective.to_lowercase();
    let objective_terms = extract_objective_terms(objective);

    // Classify intent for two-phase search routing.
    let intent = classify(objective);

    // Phase 1: symbol-index lookup (fast, high-precision for definition/reference queries).
    let phase1 = gather_symbol_candidates(
        &root,
        &scope,
        &objective_lower,
        &objective_terms,
        &intent,
        max_findings.saturating_mul(2),
    );

    // Phase 2: content search (existing keyword/rg-based approach).
    let phase2 = gather_match_candidates(
        &root,
        &scope,
        objective,
        &objective_lower,
        &objective_terms,
        max_findings.saturating_mul(4),
    );

    // Merge both phases; Phase 1 results take precedence.
    let mut candidates = merge_candidates(phase1, phase2, max_findings.saturating_mul(4));

    if candidates.is_empty() {
        candidates = fallback_structure_candidates(
            &root,
            &scope,
            &objective_terms,
            max_findings.saturating_mul(2),
        );
    }

    let packet_id = format!(
        "repo-pkt-{:016x}",
        stable_hash_u64(&format!("{}|{}", objective, scope.display()))
    );
    let mut used_tokens = estimate_tokens(&(objective.to_string() + &packet_id)) + 24;
    let mut hint_tokens_used = 0_u32;
    let mut findings = Vec::new();
    let mut expansion_hints = Vec::new();
    let mut omitted_hints = 0_usize;

    for (idx, candidate) in candidates.iter().enumerate() {
        let id = format!("f{}", idx + 1);
        let snippet = truncate_chars(&candidate.snippet, 220);
        let summary = if let Some(line) = candidate.line {
            format!(
                "{}:{} is likely relevant to '{}'",
                candidate.path, line, objective
            )
        } else {
            format!(
                "{} is a likely structural anchor for '{}'",
                candidate.path, objective
            )
        };
        let read_hint = build_repo_read_hint(&candidate.path, candidate.line);

        let finding = RepoContextFinding {
            id: id.clone(),
            summary,
            path: candidate.path.clone(),
            line: candidate.line,
            relevance_score: round2(candidate.score),
            snippet,
            read_hint: read_hint.clone(),
        };
        let estimated_tokens = estimate_finding_tokens(&finding);

        let over_count_limit = findings.len() >= max_findings;
        let over_budget_limit = used_tokens + estimated_tokens > budget_tokens;
        if !over_count_limit && !over_budget_limit {
            used_tokens += estimated_tokens;
            findings.push(finding);
        } else {
            let reason = match (over_count_limit, over_budget_limit) {
                (true, true) => "budget_and_count_limit",
                (true, false) => "count_limit",
                (false, true) => "budget_limit",
                (false, false) => "filtered",
            };
            let hint = RepoContextHint {
                finding_id: id,
                reason: reason.to_string(),
                estimated_tokens,
                hint: read_hint,
            };
            let hint_tokens = estimate_hint_tokens(&hint);
            let can_include_hint = max_hints > 0
                && expansion_hints.len() < max_hints
                && hint_tokens_used + hint_tokens <= hint_budget_tokens
                && used_tokens + hint_tokens <= budget_tokens;
            if can_include_hint {
                hint_tokens_used += hint_tokens;
                used_tokens += hint_tokens;
                expansion_hints.push(hint);
            } else {
                omitted_hints += 1;
            }
        }
    }

    if findings.is_empty() && !candidates.is_empty() {
        let candidate = &candidates[0];
        let snippet = truncate_chars(&candidate.snippet, 120);
        let summary = if let Some(line) = candidate.line {
            format!("{}:{} relevant to '{}'", candidate.path, line, objective)
        } else {
            format!("{} relevant to '{}'", candidate.path, objective)
        };
        let read_hint = build_repo_read_hint(&candidate.path, candidate.line);

        let minimal = RepoContextFinding {
            id: "f1".to_string(),
            summary,
            path: candidate.path.clone(),
            line: candidate.line,
            relevance_score: round2(candidate.score),
            snippet,
            read_hint,
        };
        used_tokens += estimate_finding_tokens(&minimal);
        findings.push(minimal);
    }

    Ok(RepoContextPacket {
        packet_id,
        root: root.display().to_string(),
        scope: relative_display(&root, &scope),
        objective: objective.to_string(),
        budget_tokens,
        used_tokens: used_tokens.min(budget_tokens),
        findings,
        expansion_hints,
        omitted_hints,
    })
}

#[derive(Debug, Clone)]
struct CandidateMatch {
    path: String,
    line: Option<u32>,
    snippet: String,
    score: f32,
}

fn gather_match_candidates(
    root: &Path,
    scope: &Path,
    objective: &str,
    objective_lower: &str,
    terms: &[String],
    max_candidates: usize,
) -> Vec<CandidateMatch> {
    let per_query_limit = max_candidates.clamp(10, 80);
    let mut query_plan = Vec::new();
    query_plan.push(objective.to_string());
    for term in terms.iter().take(4) {
        if !query_plan.contains(term) {
            query_plan.push(term.clone());
        }
    }

    let mut dedup: HashMap<String, CandidateMatch> = HashMap::new();
    for query in query_plan {
        let search = search_repo(RepoSearchOptions {
            repo_root: root,
            scope: Some(scope),
            query: &query,
            glob: None,
            max_results: per_query_limit,
            ignore_case: true,
            fixed_strings: true,
        });
        let Ok(result) = search else {
            continue;
        };

        for entry in result.matches {
            let score = relevance_score(&entry.path, &entry.snippet, objective_lower, terms);
            let key = format!("{}:{}", entry.path, entry.line);
            let candidate = CandidateMatch {
                path: entry.path,
                line: Some(entry.line),
                snippet: entry.snippet,
                score,
            };
            match dedup.get(&key) {
                Some(existing) if existing.score >= candidate.score => {}
                _ => {
                    dedup.insert(key, candidate);
                }
            }
        }
    }

    let mut values = dedup.into_values().collect::<Vec<_>>();
    values.sort_by(|left, right| {
        right
            .score
            .partial_cmp(&left.score)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| left.path.cmp(&right.path))
            .then_with(|| left.line.cmp(&right.line))
    });

    // Bias toward path diversity so packets do not collapse into one hot file.
    let mut selected = Vec::new();
    let mut used_paths = HashSet::new();
    for entry in &values {
        if selected.len() >= max_candidates {
            break;
        }
        if used_paths.insert(entry.path.clone()) {
            selected.push(entry.clone());
        }
    }
    for entry in values {
        if selected.len() >= max_candidates {
            break;
        }
        if !selected
            .iter()
            .any(|item| item.path == entry.path && item.line == entry.line)
        {
            selected.push(entry);
        }
    }
    selected
}

// ─── Phase 1: symbol-index candidates ────────────────────────────────────────

/// Build high-confidence candidates from the symbol index.
///
/// For `FindPattern` and `RecentChanges` queries, returns empty (content search
/// or git log is more appropriate).  For all other intents, searches the symbol
/// index for each objective term and builds `CandidateMatch` records enriched
/// with up to 4 lines of source context around the definition.
fn gather_symbol_candidates(
    root: &Path,
    scope: &Path,
    objective_lower: &str,
    terms: &[String],
    intent: &QueryIntent,
    max_candidates: usize,
) -> Vec<CandidateMatch> {
    match intent {
        QueryIntent::FindPattern | QueryIntent::RecentChanges => return Vec::new(),
        _ => {}
    }

    let max_index = max_candidates.saturating_mul(10).clamp(100, 4000);
    let symbols = index_directory(IndexOptions {
        repo_root: root,
        scope: Some(scope),
        max_symbols: max_index,
    });

    let mut dedup: HashMap<String, CandidateMatch> = HashMap::new();

    for term in terms.iter().take(6) {
        let hits = search_symbols(&symbols, term, max_candidates);
        for sym in hits {
            let key = format!("{}:{}", sym.path, sym.line);
            if dedup.contains_key(&key) {
                continue;
            }
            let snippet = read_symbol_snippet(root, sym);
            let score = symbol_relevance_score(&sym.name, &sym.path, objective_lower, terms);
            dedup.insert(
                key,
                CandidateMatch {
                    path: sym.path.clone(),
                    line: Some(sym.line),
                    snippet,
                    score,
                },
            );
            if dedup.len() >= max_candidates {
                break;
            }
        }
        if dedup.len() >= max_candidates {
            break;
        }
    }

    dedup.into_values().collect()
}

/// Read up to 4 lines around a symbol definition to use as the snippet.
fn read_symbol_snippet(
    root: &Path,
    sym: &crate::symbol_index::IndexedSymbol,
) -> String {
    let path = root.join(&sym.path);
    let Ok(text) = fs::read_to_string(&path) else {
        return format!("{} {} (line {})", sym.kind.as_str(), sym.name, sym.line);
    };
    let lines: Vec<&str> = text.lines().collect();
    let start = (sym.line as usize).saturating_sub(1);
    if start >= lines.len() {
        return format!("{} {} (line {})", sym.kind.as_str(), sym.name, sym.line);
    }
    let end = (start + 4).min(lines.len());
    lines[start..end].join("\n")
}

/// Score a symbol hit against the query terms.
fn symbol_relevance_score(
    name: &str,
    path: &str,
    objective_lower: &str,
    terms: &[String],
) -> f32 {
    let name_lower = name.to_lowercase();
    let path_lower = path.to_lowercase();
    let mut score = 0.60_f32;

    for term in terms {
        if name_lower == *term {
            score += 0.35;
        } else if name_lower.starts_with(term.as_str()) || name_lower.contains(term.as_str()) {
            score += 0.18;
        }
        if path_lower.contains(term.as_str()) {
            score += 0.07;
        }
    }
    if !objective_lower.is_empty()
        && objective_lower.len() <= 120
        && name_lower.contains(objective_lower)
    {
        score += 0.20;
    }

    score.clamp(0.05, 0.99)
}

/// Merge Phase-1 (symbol index) and Phase-2 (content search) candidates.
/// Phase-1 hits take precedence when the same path:line appears in both.
fn merge_candidates(
    phase1: Vec<CandidateMatch>,
    phase2: Vec<CandidateMatch>,
    max_candidates: usize,
) -> Vec<CandidateMatch> {
    let mut dedup: HashMap<String, CandidateMatch> = HashMap::new();

    for c in phase1 {
        let key = format!("{}:{:?}", c.path, c.line);
        dedup.insert(key, c);
    }
    for c in phase2 {
        let key = format!("{}:{:?}", c.path, c.line);
        dedup.entry(key).or_insert(c);
    }

    let mut values: Vec<CandidateMatch> = dedup.into_values().collect();
    values.sort_by(|a, b| {
        b.score
            .partial_cmp(&a.score)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| a.path.cmp(&b.path))
            .then_with(|| a.line.cmp(&b.line))
    });
    values.truncate(max_candidates);
    values
}

fn fallback_structure_candidates(
    root: &Path,
    scope: &Path,
    terms: &[String],
    max_candidates: usize,
) -> Vec<CandidateMatch> {
    let tree = match tree_repo(RepoTreeOptions {
        repo_root: root,
        scope: Some(scope),
        max_depth: 2,
        max_entries: max_candidates.saturating_mul(2).max(20),
    }) {
        Ok(value) => value,
        Err(_) => return Vec::new(),
    };

    let mut entries = tree
        .entries
        .into_iter()
        .map(|entry| {
            let mut score = 0.15_f32;
            let lower = entry.path.to_lowercase();
            for term in terms {
                if lower.contains(term) {
                    score += 0.18;
                }
            }
            if entry.kind == "dir" {
                score -= 0.05;
            }
            CandidateMatch {
                path: entry.path.clone(),
                line: None,
                snippet: format!("[{}] {}", entry.kind, entry.path),
                score: score.clamp(0.05, 0.95),
            }
        })
        .collect::<Vec<_>>();

    entries.sort_by(|left, right| {
        right
            .score
            .partial_cmp(&left.score)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| left.path.cmp(&right.path))
    });
    entries.truncate(max_candidates);
    entries
}

fn extract_objective_terms(objective: &str) -> Vec<String> {
    let stopwords = [
        "the",
        "a",
        "an",
        "to",
        "for",
        "of",
        "in",
        "on",
        "and",
        "or",
        "is",
        "are",
        "with",
        "what",
        "where",
        "how",
        "why",
        "fix",
        "find",
        "understand",
        "explain",
        "this",
        "that",
        "from",
        "into",
    ];
    let stopset = stopwords.iter().copied().collect::<HashSet<_>>();
    let mut seen = HashSet::new();
    let mut terms = Vec::new();
    for raw in objective
        .split(|ch: char| !ch.is_ascii_alphanumeric() && ch != '_' && ch != '-')
        .filter(|part| !part.is_empty())
    {
        let term = raw.to_lowercase();
        if term.len() < 3 || stopset.contains(term.as_str()) || !seen.insert(term.clone()) {
            continue;
        }
        terms.push(term);
    }
    terms.sort_by(|left, right| right.len().cmp(&left.len()).then_with(|| left.cmp(right)));
    terms
}

fn relevance_score(path: &str, snippet: &str, objective_lower: &str, terms: &[String]) -> f32 {
    let path_lower = path.to_lowercase();
    let snippet_lower = snippet.to_lowercase();
    let mut score = 0.20_f32;

    if !objective_lower.is_empty()
        && objective_lower.len() <= 120
        && (snippet_lower.contains(objective_lower) || path_lower.contains(objective_lower))
    {
        score += 0.35;
    }

    for term in terms.iter().take(6) {
        if snippet_lower.contains(term) {
            score += 0.10;
        }
        if path_lower.contains(term) {
            score += 0.07;
        }
    }

    if path_lower.ends_with(".rs")
        || path_lower.ends_with(".py")
        || path_lower.ends_with(".ts")
        || path_lower.ends_with(".js")
        || path_lower.ends_with(".go")
    {
        score += 0.05;
    }

    score.clamp(0.05, 0.99)
}

fn build_repo_read_hint(path: &str, line: Option<u32>) -> String {
    match line {
        Some(line_number) => {
            let start = line_number.saturating_sub(20).max(1);
            let end = line_number.saturating_add(20);
            format!(
                "repo read {} --start-line {} --end-line {}",
                path, start, end
            )
        }
        None => format!("repo read {} --start-line 1 --end-line 200", path),
    }
}

fn estimate_finding_tokens(finding: &RepoContextFinding) -> u32 {
    estimate_tokens(&format!(
        "{} {} {} {} {}",
        finding.summary, finding.path, finding.snippet, finding.relevance_score, finding.read_hint
    )) + 12
}

fn estimate_hint_tokens(hint: &RepoContextHint) -> u32 {
    estimate_tokens(&format!(
        "{} {} {} {}",
        hint.finding_id, hint.reason, hint.estimated_tokens, hint.hint
    )) + 8
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

fn stable_hash_u64(value: &str) -> u64 {
    // FNV-1a 64-bit — stable across all Rust versions and platforms.
    const FNV_OFFSET: u64 = 14695981039346656037;
    const FNV_PRIME: u64 = 1099511628211;
    b"repo-query"
        .iter()
        .chain(value.bytes().collect::<Vec<_>>().iter())
        .fold(FNV_OFFSET, |acc, &b| (acc ^ b as u64).wrapping_mul(FNV_PRIME))
}

fn round2(value: f32) -> f32 {
    (value * 100.0).round() / 100.0
}

enum RgSearchError {
    NotAvailable,
    Failed(String),
}

fn search_with_rg(
    root: &Path,
    scope: &Path,
    options: &RepoSearchOptions<'_>,
    max_results: usize,
) -> Result<RepoSearchResult, RgSearchError> {
    let mut command = Command::new("rg");
    command.current_dir(root);
    command.arg("--json");
    command.arg("--line-number");
    command.arg("--column");
    if options.ignore_case {
        command.arg("--ignore-case");
    }
    if options.fixed_strings {
        command.arg("--fixed-strings");
    }
    if let Some(glob) = options.glob {
        command.arg("--glob");
        command.arg(glob);
    }
    command.arg(options.query);
    command.arg(relative_scope(root, scope));

    let output = command.output().map_err(|err| {
        if err.kind() == std::io::ErrorKind::NotFound {
            RgSearchError::NotAvailable
        } else {
            RgSearchError::Failed(format!("repo search: failed to execute rg: {err}"))
        }
    })?;

    let code = output.status.code().unwrap_or(1);
    if code != 0 && code != 1 {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        return Err(RgSearchError::Failed(format!(
            "repo search: rg failed with exit code {code}: {stderr}"
        )));
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let mut matches = Vec::new();
    let mut truncated = false;

    for line in stdout.lines() {
        let value: Value = match serde_json::from_str(line) {
            Ok(v) => v,
            Err(_) => continue,
        };
        if value.get("type").and_then(Value::as_str) != Some("match") {
            continue;
        }
        let data = match value.get("data") {
            Some(v) => v,
            None => continue,
        };
        let path = data
            .get("path")
            .and_then(|v| v.get("text"))
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string();
        let line_number = data.get("line_number").and_then(Value::as_u64).unwrap_or(0) as u32;
        let column = data
            .get("submatches")
            .and_then(Value::as_array)
            .and_then(|items| items.first())
            .and_then(|item| item.get("start"))
            .and_then(Value::as_u64)
            .map(|n| n as u32 + 1)
            .unwrap_or(1);
        let snippet = data
            .get("lines")
            .and_then(|v| v.get("text"))
            .and_then(Value::as_str)
            .unwrap_or("")
            .trim_end_matches('\n')
            .to_string();

        matches.push(RepoMatch {
            path,
            line: line_number,
            column,
            snippet,
        });
        if matches.len() >= max_results {
            truncated = true;
            break;
        }
    }

    Ok(RepoSearchResult {
        backend: "rg".to_string(),
        query: options.query.to_string(),
        root: root.display().to_string(),
        scope: relative_display(root, scope),
        returned: matches.len(),
        truncated,
        matches,
    })
}

fn search_with_fallback(
    root: &Path,
    scope: &Path,
    options: &RepoSearchOptions<'_>,
    max_results: usize,
) -> RepoSearchResult {
    let needle = if options.ignore_case {
        options.query.to_lowercase()
    } else {
        options.query.to_string()
    };

    let mut matches = Vec::new();
    let mut truncated = false;
    let _ = fallback_collect_matches(
        root,
        scope,
        &needle,
        options.ignore_case,
        max_results,
        &mut matches,
        &mut truncated,
    );

    RepoSearchResult {
        backend: "fallback".to_string(),
        query: options.query.to_string(),
        root: root.display().to_string(),
        scope: relative_display(root, scope),
        returned: matches.len(),
        truncated,
        matches,
    }
}

#[allow(clippy::too_many_arguments)]
fn fallback_collect_matches(
    root: &Path,
    dir: &Path,
    needle: &str,
    ignore_case: bool,
    max_results: usize,
    matches: &mut Vec<RepoMatch>,
    truncated: &mut bool,
) -> Result<(), String> {
    if matches.len() >= max_results {
        *truncated = true;
        return Ok(());
    }

    let mut entries = fs::read_dir(dir)
        .map_err(|err| format!("repo search: failed to read '{}': {err}", dir.display()))?
        .filter_map(Result::ok)
        .collect::<Vec<_>>();
    entries.sort_by_key(|entry| entry.file_name());

    for entry in entries {
        if matches.len() >= max_results {
            *truncated = true;
            return Ok(());
        }

        let path = entry.path();
        let file_name = entry.file_name();
        if should_skip_name(&file_name) {
            continue;
        }

        let Ok(metadata) = entry.metadata() else {
            continue;
        };
        if metadata.is_dir() {
            fallback_collect_matches(
                root,
                &path,
                needle,
                ignore_case,
                max_results,
                matches,
                truncated,
            )?;
            continue;
        }
        if !metadata.is_file() || metadata.len() > MAX_FALLBACK_FILE_BYTES {
            continue;
        }
        let Ok(text) = fs::read_to_string(&path) else {
            continue;
        };
        for (idx, line) in text.lines().enumerate() {
            let haystack = if ignore_case {
                line.to_lowercase()
            } else {
                line.to_string()
            };
            if let Some(pos) = haystack.find(needle) {
                matches.push(RepoMatch {
                    path: relative_display(root, &path),
                    line: (idx + 1) as u32,
                    column: (pos + 1) as u32,
                    snippet: line.to_string(),
                });
                if matches.len() >= max_results {
                    *truncated = true;
                    return Ok(());
                }
            }
        }
    }

    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn collect_tree_entries(
    root: &Path,
    dir: &Path,
    depth: u32,
    max_depth: u32,
    max_entries: usize,
    entries: &mut Vec<RepoTreeEntry>,
    truncated: &mut bool,
) -> Result<(), String> {
    if entries.len() >= max_entries {
        *truncated = true;
        return Ok(());
    }
    if depth > max_depth {
        return Ok(());
    }

    let mut children = fs::read_dir(dir)
        .map_err(|err| format!("repo tree: failed to read '{}': {err}", dir.display()))?
        .filter_map(Result::ok)
        .collect::<Vec<_>>();
    children.sort_by_key(|entry| entry.file_name());

    for child in children {
        if entries.len() >= max_entries {
            *truncated = true;
            return Ok(());
        }

        let path = child.path();
        let name = child.file_name();
        if should_skip_name(&name) {
            continue;
        }
        let Ok(metadata) = child.metadata() else {
            continue;
        };
        let kind = if metadata.is_dir() { "dir" } else { "file" };
        entries.push(RepoTreeEntry {
            path: relative_display(root, &path),
            kind: kind.to_string(),
            depth,
        });

        if metadata.is_dir() && depth < max_depth {
            collect_tree_entries(
                root,
                &path,
                depth + 1,
                max_depth,
                max_entries,
                entries,
                truncated,
            )?;
        }
    }

    Ok(())
}

fn canonicalize_existing_dir(path: &Path, label: &str) -> Result<PathBuf, String> {
    let canonical = path
        .canonicalize()
        .map_err(|err| format!("{label}: failed to resolve '{}': {err}", path.display()))?;
    if !canonical.is_dir() {
        return Err(format!(
            "{label}: '{}' is not a directory",
            canonical.display()
        ));
    }
    Ok(canonical)
}

fn resolve_scope(root: &Path, scope: Option<&Path>) -> Result<PathBuf, String> {
    match scope {
        Some(path) => resolve_within_root(root, path),
        None => Ok(root.to_path_buf()),
    }
}

fn resolve_within_root(root: &Path, target: &Path) -> Result<PathBuf, String> {
    let joined = if target.is_absolute() {
        target.to_path_buf()
    } else {
        root.join(target)
    };
    let canonical = joined
        .canonicalize()
        .map_err(|err| format!("path resolution failed for '{}': {err}", joined.display()))?;
    if !canonical.starts_with(root) {
        return Err(format!(
            "path '{}' escapes repo root '{}'",
            canonical.display(),
            root.display()
        ));
    }
    Ok(canonical)
}

fn should_skip_name(name: &OsStr) -> bool {
    let Some(value) = name.to_str() else {
        return false;
    };
    DEFAULT_EXCLUDED_DIRS.contains(&value)
}

fn relative_scope(root: &Path, scope: &Path) -> String {
    if scope == root {
        ".".to_string()
    } else {
        relative_display(root, scope)
    }
}

fn relative_display(root: &Path, path: &Path) -> String {
    path.strip_prefix(root)
        .unwrap_or(path)
        .display()
        .to_string()
}

fn truncate_chars(text: &str, max_chars: usize) -> String {
    text.chars().take(max_chars).collect::<String>()
}

// ─── repo_log ─────────────────────────────────────────────────────────────────

/// Return recent git commit history for a single file.
///
/// Runs `git log --follow --date=short --format="%h %ad %s" -n <limit> -- <path>`.
/// Returns an empty result (not an error) when `git` is unavailable or the repo
/// has no history for the given path.
pub fn log_repo_file(options: RepoLogOptions<'_>) -> Result<RepoLogResult, String> {
    let root = canonicalize_existing_dir(options.repo_root, "repo_root")?;
    let limit = options.limit.max(1).min(200);

    // Resolve the path relative to the root.
    let abs_path = if options.path.is_absolute() {
        options.path.to_path_buf()
    } else {
        root.join(options.path)
    };
    let display_path = options.path.display().to_string();

    let mut cmd = Command::new("git");
    cmd.current_dir(&root);
    cmd.args([
        "log",
        "--follow",
        "--date=short",
        "--format=%h %ad %s",
        &format!("-n{limit}"),
        "--",
        &abs_path.display().to_string(),
    ]);

    let output = match cmd.output() {
        Ok(o) => o,
        Err(_) => {
            // git not available → return empty result, not an error.
            return Ok(RepoLogResult {
                root: root.display().to_string(),
                path: display_path,
                returned: 0,
                entries: Vec::new(),
            });
        }
    };

    if !output.status.success() && output.status.code() != Some(0) {
        // Non-zero exit can mean no git repo or no commits — treat as empty.
        return Ok(RepoLogResult {
            root: root.display().to_string(),
            path: display_path,
            returned: 0,
            entries: Vec::new(),
        });
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let entries: Vec<RepoLogEntry> = stdout
        .lines()
        .filter_map(|line| parse_git_log_line(line.trim()))
        .collect();

    let returned = entries.len();
    Ok(RepoLogResult {
        root: root.display().to_string(),
        path: display_path,
        returned,
        entries,
    })
}

/// Parse a single line from `git log --format="%h %ad %s"`.
/// Expected format: `<hash> <date> <subject>`
fn parse_git_log_line(line: &str) -> Option<RepoLogEntry> {
    if line.is_empty() {
        return None;
    }
    let mut parts = line.splitn(3, ' ');
    let hash = parts.next()?.to_string();
    let date = parts.next()?.to_string();
    let subject = parts.next().unwrap_or("").trim().to_string();
    if hash.is_empty() {
        return None;
    }
    Some(RepoLogEntry { hash, date, subject })
}

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};

    use super::{
        query_repo_context, read_repo_file, search_repo, tree_repo, RepoQueryOptions,
        RepoReadOptions, RepoSearchOptions, RepoTreeOptions,
    };

    fn fixture_root() -> PathBuf {
        PathBuf::from(".")
            .canonicalize()
            .expect("repo root should canonicalize")
    }

    #[test]
    fn reads_line_window_from_repo_file() {
        let root = fixture_root();
        let result = read_repo_file(RepoReadOptions {
            repo_root: &root,
            path: Path::new("Cargo.toml"),
            start_line: 1,
            end_line: Some(3),
            max_chars: 10_000,
        })
        .expect("read should succeed");

        assert!(result.path.ends_with("Cargo.toml"));
        assert!(result.total_lines >= 3);
        assert!(result.content.contains("[package]"));
    }

    #[test]
    fn tree_lists_source_entries() {
        let root = fixture_root();
        let result = tree_repo(RepoTreeOptions {
            repo_root: &root,
            scope: Some(Path::new("src")),
            max_depth: 1,
            max_entries: 200,
        })
        .expect("tree should succeed");

        assert!(result
            .entries
            .iter()
            .any(|entry| entry.path == "src/main.rs"));
    }

    #[test]
    fn fallback_search_finds_query_when_rg_missing_or_unavailable() {
        let root = fixture_root();
        let result = search_repo(RepoSearchOptions {
            repo_root: &root,
            scope: Some(Path::new("src")),
            query: "enum Commands",
            glob: None,
            max_results: 20,
            ignore_case: false,
            fixed_strings: true,
        })
        .expect("search should succeed");

        assert!(
            result
                .matches
                .iter()
                .any(|entry| entry.path.ends_with("src/main.rs")),
            "search should include src/main.rs"
        );
    }

    #[test]
    fn builds_budgeted_repo_query_packet() {
        let root = fixture_root();
        let packet = query_repo_context(RepoQueryOptions {
            repo_root: &root,
            scope: Some(Path::new("src")),
            objective: "understand command routing and repo tools",
            budget_tokens: 220,
            max_findings: 4,
            max_hints: 3,
        })
        .expect("repo query should succeed");

        assert_eq!(packet.scope, "src");
        assert!(packet.used_tokens <= packet.budget_tokens);
        assert!(
            !packet.findings.is_empty(),
            "packet should contain findings"
        );
        assert!(packet.expansion_hints.len() <= 3);
    }
}
