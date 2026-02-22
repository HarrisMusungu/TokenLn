use std::fs;
use std::path::{Path, PathBuf};

pub const POLICY_FILE_RELATIVE_PATH: &str = ".tokenln/policy.toml";
pub const MAX_REPO_QUERY_BUDGET: u32 = 900;
pub const MAX_REPO_QUERY_FINDINGS: usize = 16;
pub const MAX_REPO_QUERY_HINTS: usize = 12;
pub const MAX_REPO_SEARCH_RESULTS: usize = 120;
pub const MAX_REPO_READ_CHARS: usize = 12_000;
pub const MAX_REPO_READ_SPAN_LINES: u32 = 400;
pub const MAX_REPO_TREE_DEPTH: u32 = 4;
pub const MAX_REPO_TREE_ENTRIES: usize = 400;
const MAX_PROXY_CAT_FILES: usize = 3;
const MAX_PROXY_CAT_FILE_BYTES: u64 = 120_000;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PolicySettings {
    pub repo_query_budget: u32,
    pub repo_query_max_findings: usize,
    pub repo_query_max_hints: usize,
    pub repo_search_max_results: usize,
    pub repo_read_max_chars: usize,
    pub repo_read_max_span_lines: u32,
    pub repo_tree_max_depth: u32,
    pub repo_tree_max_entries: usize,
    pub proxy_cat_max_files: usize,
    pub proxy_cat_max_file_bytes: u64,
    pub block_broad_find: bool,
    pub block_recursive_ls_root: bool,
    pub block_tree_root_without_depth: bool,
    pub block_tree_root_depth_exceeded: bool,
    pub block_massive_cat: bool,
}

impl Default for PolicySettings {
    fn default() -> Self {
        Self {
            repo_query_budget: MAX_REPO_QUERY_BUDGET,
            repo_query_max_findings: MAX_REPO_QUERY_FINDINGS,
            repo_query_max_hints: MAX_REPO_QUERY_HINTS,
            repo_search_max_results: MAX_REPO_SEARCH_RESULTS,
            repo_read_max_chars: MAX_REPO_READ_CHARS,
            repo_read_max_span_lines: MAX_REPO_READ_SPAN_LINES,
            repo_tree_max_depth: MAX_REPO_TREE_DEPTH,
            repo_tree_max_entries: MAX_REPO_TREE_ENTRIES,
            proxy_cat_max_files: MAX_PROXY_CAT_FILES,
            proxy_cat_max_file_bytes: MAX_PROXY_CAT_FILE_BYTES,
            block_broad_find: true,
            block_recursive_ls_root: true,
            block_tree_root_without_depth: true,
            block_tree_root_depth_exceeded: true,
            block_massive_cat: true,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PolicyViolation {
    pub code: &'static str,
    pub message: String,
    pub suggestions: Vec<String>,
}

impl PolicyViolation {
    pub fn render(&self) -> String {
        let mut lines = Vec::new();
        lines.push(format!("tokenln policy [{}]: {}", self.code, self.message));
        if !self.suggestions.is_empty() {
            lines.push("Try one of:".to_string());
            for suggestion in &self.suggestions {
                lines.push(format!("  - {suggestion}"));
            }
        }
        lines.join("\n")
    }
}

#[derive(Debug, Copy, Clone, Eq, PartialEq)]
enum PolicySection {
    Limits,
    Proxy,
    Unknown,
}

pub fn load_policy_for_repo_root(repo_root: &Path) -> Result<PolicySettings, String> {
    load_policy_from_path(&repo_root.join(POLICY_FILE_RELATIVE_PATH))
}

pub fn load_policy_for_working_dir(start_dir: &Path) -> Result<PolicySettings, String> {
    let mut current = start_dir
        .canonicalize()
        .unwrap_or_else(|_| start_dir.to_path_buf());

    loop {
        let candidate = current.join(POLICY_FILE_RELATIVE_PATH);
        if candidate.is_file() {
            return load_policy_from_path(&candidate);
        }
        if !current.pop() {
            return Ok(PolicySettings::default());
        }
    }
}

fn load_policy_from_path(path: &Path) -> Result<PolicySettings, String> {
    if !path.is_file() {
        return Ok(PolicySettings::default());
    }

    let content = fs::read_to_string(path).map_err(|err| {
        format!(
            "failed to read policy file '{}': {err}",
            path.as_os_str().to_string_lossy()
        )
    })?;
    parse_policy_content(&content, path)
}

fn parse_policy_content(content: &str, path: &Path) -> Result<PolicySettings, String> {
    let mut settings = PolicySettings::default();
    let mut section = PolicySection::Unknown;

    for (line_index, raw_line) in content.lines().enumerate() {
        let line_number = line_index + 1;
        let line = strip_comments(raw_line).trim();
        if line.is_empty() {
            continue;
        }

        if line.starts_with('[') && line.ends_with(']') {
            let header = line[1..line.len() - 1].trim();
            section = match header {
                "limits" => PolicySection::Limits,
                "proxy" => PolicySection::Proxy,
                _ => PolicySection::Unknown,
            };
            continue;
        }

        let (key, value) = line.split_once('=').ok_or_else(|| {
            format!(
                "invalid policy entry at {}:{} (expected key = value)",
                path.display(),
                line_number
            )
        })?;
        let key = key.trim();
        let value = value.trim();

        match section {
            PolicySection::Limits => {
                apply_limit_value(&mut settings, key, value, path, line_number)?
            }
            PolicySection::Proxy => {
                apply_proxy_value(&mut settings, key, value, path, line_number)?
            }
            PolicySection::Unknown => {}
        }
    }

    Ok(settings)
}

fn apply_limit_value(
    settings: &mut PolicySettings,
    key: &str,
    value: &str,
    path: &Path,
    line_number: usize,
) -> Result<(), String> {
    match key {
        "repo_query_budget" => {
            settings.repo_query_budget = parse_u32(value, path, line_number)?.max(1)
        }
        "repo_query_max_findings" => {
            settings.repo_query_max_findings = parse_usize(value, path, line_number)?.max(1)
        }
        "repo_query_max_hints" => {
            settings.repo_query_max_hints = parse_usize(value, path, line_number)?
        }
        "repo_search_max_results" => {
            settings.repo_search_max_results = parse_usize(value, path, line_number)?.max(1)
        }
        "repo_read_max_chars" => {
            settings.repo_read_max_chars = parse_usize(value, path, line_number)?.max(200)
        }
        "repo_read_max_span_lines" => {
            settings.repo_read_max_span_lines = parse_u32(value, path, line_number)?.max(1)
        }
        "repo_tree_max_depth" => {
            settings.repo_tree_max_depth = parse_u32(value, path, line_number)?.max(1)
        }
        "repo_tree_max_entries" => {
            settings.repo_tree_max_entries = parse_usize(value, path, line_number)?.max(1)
        }
        "proxy_cat_max_files" => {
            settings.proxy_cat_max_files = parse_usize(value, path, line_number)?.max(1)
        }
        "proxy_cat_max_file_bytes" => {
            settings.proxy_cat_max_file_bytes = parse_u64(value, path, line_number)?.max(1024)
        }
        _ => {
            return Err(format!(
                "unknown limits key '{}' at {}:{}",
                key,
                path.display(),
                line_number
            ));
        }
    }

    Ok(())
}

fn apply_proxy_value(
    settings: &mut PolicySettings,
    key: &str,
    value: &str,
    path: &Path,
    line_number: usize,
) -> Result<(), String> {
    match key {
        "block_broad_find" => settings.block_broad_find = parse_bool(value, path, line_number)?,
        "block_recursive_ls_root" => {
            settings.block_recursive_ls_root = parse_bool(value, path, line_number)?
        }
        "block_tree_root_without_depth" => {
            settings.block_tree_root_without_depth = parse_bool(value, path, line_number)?
        }
        "block_tree_root_depth_exceeded" => {
            settings.block_tree_root_depth_exceeded = parse_bool(value, path, line_number)?
        }
        "block_massive_cat" => settings.block_massive_cat = parse_bool(value, path, line_number)?,
        _ => {
            return Err(format!(
                "unknown proxy key '{}' at {}:{}",
                key,
                path.display(),
                line_number
            ));
        }
    }

    Ok(())
}

fn strip_comments(line: &str) -> &str {
    line.split('#').next().unwrap_or(line)
}

fn parse_bool(value: &str, path: &Path, line_number: usize) -> Result<bool, String> {
    match value {
        "true" => Ok(true),
        "false" => Ok(false),
        _ => Err(format!(
            "invalid boolean '{}' at {}:{}",
            value,
            path.display(),
            line_number
        )),
    }
}

fn parse_u32(value: &str, path: &Path, line_number: usize) -> Result<u32, String> {
    value.parse::<u32>().map_err(|_| {
        format!(
            "invalid numeric value '{}' at {}:{}",
            value,
            path.display(),
            line_number
        )
    })
}

fn parse_u64(value: &str, path: &Path, line_number: usize) -> Result<u64, String> {
    value.parse::<u64>().map_err(|_| {
        format!(
            "invalid numeric value '{}' at {}:{}",
            value,
            path.display(),
            line_number
        )
    })
}

fn parse_usize(value: &str, path: &Path, line_number: usize) -> Result<usize, String> {
    value.parse::<usize>().map_err(|_| {
        format!(
            "invalid numeric value '{}' at {}:{}",
            value,
            path.display(),
            line_number
        )
    })
}

pub fn evaluate_proxy_command(program: &str, args: &[String]) -> Option<PolicyViolation> {
    evaluate_proxy_command_with_policy(&PolicySettings::default(), program, args)
}

pub fn evaluate_proxy_command_with_policy(
    policy: &PolicySettings,
    program: &str,
    args: &[String],
) -> Option<PolicyViolation> {
    let binary = Path::new(program)
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or(program);

    match binary {
        "find" if policy.block_broad_find => detect_broad_find(args),
        "ls" if policy.block_recursive_ls_root => detect_recursive_ls(args),
        "tree" => detect_broad_tree(args, policy),
        "cat" if policy.block_massive_cat => detect_massive_cat(args, policy),
        _ => None,
    }
}

pub fn validate_repo_query_request(
    budget_tokens: u32,
    max_findings: usize,
    max_hints: usize,
) -> Result<(), PolicyViolation> {
    validate_repo_query_request_with_policy(
        &PolicySettings::default(),
        budget_tokens,
        max_findings,
        max_hints,
    )
}

pub fn validate_repo_query_request_with_policy(
    policy: &PolicySettings,
    budget_tokens: u32,
    max_findings: usize,
    max_hints: usize,
) -> Result<(), PolicyViolation> {
    if budget_tokens > policy.repo_query_budget {
        return Err(PolicyViolation {
            code: "repo_query_budget_too_large",
            message: format!(
                "repo_query budget {} exceeds limit {}",
                budget_tokens, policy.repo_query_budget
            ),
            suggestions: vec![
                format!("Use budget <= {}.", policy.repo_query_budget),
                "Increase focus by narrowing `path` (for example: `src`).".to_string(),
            ],
        });
    }

    if max_findings > policy.repo_query_max_findings {
        return Err(PolicyViolation {
            code: "repo_query_findings_too_large",
            message: format!(
                "repo_query max_findings {} exceeds limit {}",
                max_findings, policy.repo_query_max_findings
            ),
            suggestions: vec![format!(
                "Use max_findings <= {}.",
                policy.repo_query_max_findings
            )],
        });
    }

    if max_hints > policy.repo_query_max_hints {
        return Err(PolicyViolation {
            code: "repo_query_hints_too_large",
            message: format!(
                "repo_query max_hints {} exceeds limit {}",
                max_hints, policy.repo_query_max_hints
            ),
            suggestions: vec![format!("Use max_hints <= {}.", policy.repo_query_max_hints)],
        });
    }

    Ok(())
}

pub fn validate_repo_search_request(
    query: &str,
    max_results: usize,
    fixed_strings: bool,
    has_scope: bool,
    has_glob: bool,
) -> Result<(), PolicyViolation> {
    validate_repo_search_request_with_policy(
        &PolicySettings::default(),
        query,
        max_results,
        fixed_strings,
        has_scope,
        has_glob,
    )
}

pub fn validate_repo_search_request_with_policy(
    policy: &PolicySettings,
    query: &str,
    max_results: usize,
    fixed_strings: bool,
    has_scope: bool,
    has_glob: bool,
) -> Result<(), PolicyViolation> {
    if max_results > policy.repo_search_max_results {
        return Err(PolicyViolation {
            code: "repo_search_results_too_large",
            message: format!(
                "repo_search max_results {} exceeds limit {}",
                max_results, policy.repo_search_max_results
            ),
            suggestions: vec![
                format!("Use max_results <= {}.", policy.repo_search_max_results),
                "Narrow search with `--path` and `--glob`.".to_string(),
            ],
        });
    }

    if !fixed_strings && !has_scope && !has_glob && is_broad_regex(query) {
        return Err(PolicyViolation {
            code: "repo_search_broad_regex",
            message: "repo_search received a broad regex without path/glob constraints".to_string(),
            suggestions: vec![
                "Use a specific term or fixed_strings=true.".to_string(),
                "Add `path` (for example: `src`) and `glob` (for example: `*.rs`).".to_string(),
            ],
        });
    }

    Ok(())
}

pub fn validate_repo_read_request(
    start_line: u32,
    end_line: Option<u32>,
    max_chars: usize,
) -> Result<(), PolicyViolation> {
    validate_repo_read_request_with_policy(
        &PolicySettings::default(),
        start_line,
        end_line,
        max_chars,
    )
}

pub fn validate_repo_read_request_with_policy(
    policy: &PolicySettings,
    start_line: u32,
    end_line: Option<u32>,
    max_chars: usize,
) -> Result<(), PolicyViolation> {
    let start = start_line.max(1);
    let end = end_line.unwrap_or(start.saturating_add(199)).max(start);
    let span = end.saturating_sub(start).saturating_add(1);

    if span > policy.repo_read_max_span_lines {
        return Err(PolicyViolation {
            code: "repo_read_span_too_large",
            message: format!(
                "repo_read line span {} exceeds limit {}",
                span, policy.repo_read_max_span_lines
            ),
            suggestions: vec![
                format!("Use a line window <= {}.", policy.repo_read_max_span_lines),
                "Fetch the exact area first with `repo_search`.".to_string(),
            ],
        });
    }

    if max_chars > policy.repo_read_max_chars {
        return Err(PolicyViolation {
            code: "repo_read_chars_too_large",
            message: format!(
                "repo_read max_chars {} exceeds limit {}",
                max_chars, policy.repo_read_max_chars
            ),
            suggestions: vec![format!("Use max_chars <= {}.", policy.repo_read_max_chars)],
        });
    }

    Ok(())
}

pub fn validate_repo_tree_request(
    max_depth: u32,
    max_entries: usize,
) -> Result<(), PolicyViolation> {
    validate_repo_tree_request_with_policy(&PolicySettings::default(), max_depth, max_entries)
}

pub fn validate_repo_tree_request_with_policy(
    policy: &PolicySettings,
    max_depth: u32,
    max_entries: usize,
) -> Result<(), PolicyViolation> {
    if max_depth > policy.repo_tree_max_depth {
        return Err(PolicyViolation {
            code: "repo_tree_depth_too_large",
            message: format!(
                "repo_tree max_depth {} exceeds limit {}",
                max_depth, policy.repo_tree_max_depth
            ),
            suggestions: vec![format!("Use max_depth <= {}.", policy.repo_tree_max_depth)],
        });
    }

    if max_entries > policy.repo_tree_max_entries {
        return Err(PolicyViolation {
            code: "repo_tree_entries_too_large",
            message: format!(
                "repo_tree max_entries {} exceeds limit {}",
                max_entries, policy.repo_tree_max_entries
            ),
            suggestions: vec![format!(
                "Use max_entries <= {}.",
                policy.repo_tree_max_entries
            )],
        });
    }

    Ok(())
}

fn detect_broad_find(args: &[String]) -> Option<PolicyViolation> {
    let root_scope = first_positional(args).map(is_root_scope).unwrap_or(true);
    if !root_scope {
        return None;
    }

    let has_depth_limit = has_option(args, "-maxdepth");
    let has_name_filter = has_any_option(
        args,
        &["-name", "-iname", "-path", "-ipath", "-regex", "-iregex"],
    );
    if has_depth_limit || has_name_filter {
        return None;
    }

    Some(PolicyViolation {
        code: "broad_find",
        message: "blocked broad `find` command over repository root".to_string(),
        suggestions: vec![
            "tokenln repo query \"find where X is implemented\" --path src --budget 260"
                .to_string(),
            "tokenln repo search \"symbol_or_text\" --path src --glob \"*.rs\"".to_string(),
        ],
    })
}

fn detect_recursive_ls(args: &[String]) -> Option<PolicyViolation> {
    let has_recursive = args
        .iter()
        .any(|arg| arg == "--recursive" || (arg.starts_with('-') && arg.contains('R')));
    if !has_recursive {
        return None;
    }

    let root_scope = positional_args(args)
        .first()
        .map(|arg| is_root_scope(arg))
        .unwrap_or(true);
    if !root_scope {
        return None;
    }

    Some(PolicyViolation {
        code: "recursive_ls_root",
        message: "blocked recursive `ls` over repository root".to_string(),
        suggestions: vec![
            "tokenln repo tree --path src --max-depth 2 --max-entries 200".to_string(),
            "tokenln repo search \"needle\" --path src --glob \"*.rs\"".to_string(),
        ],
    })
}

fn detect_broad_tree(args: &[String], policy: &PolicySettings) -> Option<PolicyViolation> {
    let positional = positional_args(args);
    let root_scope = positional
        .first()
        .map(|arg| is_root_scope(arg))
        .unwrap_or(true);
    if !root_scope {
        return None;
    }

    let depth = parse_tree_depth(args);
    if depth.is_none() && policy.block_tree_root_without_depth {
        return Some(PolicyViolation {
            code: "tree_without_depth",
            message: "blocked `tree` at repository root without depth limit".to_string(),
            suggestions: vec![
                "tokenln repo tree --path src --max-depth 2 --max-entries 200".to_string(),
            ],
        });
    }

    match depth {
        Some(value)
            if value > policy.repo_tree_max_depth && policy.block_tree_root_depth_exceeded =>
        {
            Some(PolicyViolation {
                code: "tree_depth_too_large",
                message: format!(
                    "blocked `tree` depth {} at repository root (limit {})",
                    value, policy.repo_tree_max_depth
                ),
                suggestions: vec![format!(
                    "Use tree depth <= {} or tokenln repo tree --max-depth 2.",
                    policy.repo_tree_max_depth
                )],
            })
        }
        _ => None,
    }
}

fn detect_massive_cat(args: &[String], policy: &PolicySettings) -> Option<PolicyViolation> {
    let files = positional_args(args);
    if files.is_empty() {
        return None;
    }

    if files.len() > policy.proxy_cat_max_files {
        return Some(PolicyViolation {
            code: "cat_too_many_files",
            message: format!(
                "blocked `cat` across {} files (limit {})",
                files.len(),
                policy.proxy_cat_max_files
            ),
            suggestions: vec![
                "tokenln repo search \"target text\" --path src --glob \"*.rs\"".to_string(),
                "tokenln repo read <path> --start-line <n> --end-line <m>".to_string(),
            ],
        });
    }

    for file in files {
        if has_glob_wildcards(file) {
            return Some(PolicyViolation {
                code: "cat_glob_pattern",
                message: "blocked `cat` using wildcard pattern".to_string(),
                suggestions: vec![
                    "tokenln repo search \"target text\" --path src --glob \"*.rs\"".to_string(),
                    "tokenln repo read <path> --start-line <n> --end-line <m>".to_string(),
                ],
            });
        }

        if is_large_existing_file(file, policy.proxy_cat_max_file_bytes) {
            return Some(PolicyViolation {
                code: "cat_large_file",
                message: format!(
                    "blocked `cat` of large file '{}' (> {} bytes)",
                    file, policy.proxy_cat_max_file_bytes
                ),
                suggestions: vec![format!(
                    "tokenln repo read {file} --start-line 1 --end-line 200 --max-chars 6000"
                )],
            });
        }
    }

    None
}

fn first_positional(args: &[String]) -> Option<&str> {
    args.iter().map(String::as_str).find(|arg| {
        !arg.starts_with('-') && *arg != "!" && *arg != "(" && *arg != ")" && *arg != ","
    })
}

fn positional_args(args: &[String]) -> Vec<&str> {
    args.iter()
        .map(String::as_str)
        .filter(|arg| {
            !arg.starts_with('-')
                && *arg != "-"
                && *arg != "!"
                && *arg != "("
                && *arg != ")"
                && *arg != ","
        })
        .collect()
}

fn has_option(args: &[String], option: &str) -> bool {
    args.iter().any(|arg| arg == option)
}

fn has_any_option(args: &[String], options: &[&str]) -> bool {
    args.iter()
        .any(|arg| options.iter().any(|option| arg == option))
}

fn parse_tree_depth(args: &[String]) -> Option<u32> {
    let mut index = 0;
    while index < args.len() {
        let arg = args[index].as_str();
        if arg == "-L" || arg == "--level" {
            if let Some(value) = args
                .get(index + 1)
                .and_then(|next| next.parse::<u32>().ok())
            {
                return Some(value);
            }
        } else if let Some(value) = arg
            .strip_prefix("-L")
            .and_then(|rest| rest.parse::<u32>().ok())
        {
            return Some(value);
        }
        index += 1;
    }
    None
}

fn is_root_scope(path: &str) -> bool {
    matches!(path, "." | "./" | "")
}

fn has_glob_wildcards(value: &str) -> bool {
    value.contains('*') || value.contains('?') || value.contains('[')
}

fn is_large_existing_file(path: &str, max_file_bytes: u64) -> bool {
    let candidate = PathBuf::from(path);
    if !candidate.exists() || !candidate.is_file() {
        return false;
    }
    fs::metadata(candidate)
        .map(|meta| meta.len() > max_file_bytes)
        .unwrap_or(false)
}

fn is_broad_regex(query: &str) -> bool {
    matches!(query.trim(), ".*" | "^.*$" | ".+" | "^.+$")
}

#[cfg(test)]
mod tests {
    use std::env;
    use std::path::PathBuf;
    use std::process;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::time::{SystemTime, UNIX_EPOCH};

    use super::{
        evaluate_proxy_command, evaluate_proxy_command_with_policy, load_policy_for_repo_root,
        load_policy_for_working_dir, validate_repo_query_request, validate_repo_read_request,
        validate_repo_search_request, validate_repo_tree_request, PolicySettings,
    };

    #[test]
    fn blocks_broad_find_from_repo_root() {
        let violation = evaluate_proxy_command("find", &[".".to_string()])
            .expect("broad find should be blocked");
        assert_eq!(violation.code, "broad_find");
    }

    #[test]
    fn allows_scoped_find_with_name_filter() {
        let violation = evaluate_proxy_command(
            "find",
            &[
                ".".to_string(),
                "-name".to_string(),
                "*.rs".to_string(),
                "-maxdepth".to_string(),
                "2".to_string(),
            ],
        );
        assert!(violation.is_none());
    }

    #[test]
    fn blocks_recursive_ls_from_repo_root() {
        let violation = evaluate_proxy_command("ls", &["-laR".to_string()])
            .expect("recursive root ls should be blocked");
        assert_eq!(violation.code, "recursive_ls_root");
    }

    #[test]
    fn enforces_repo_read_limits() {
        let violation = validate_repo_read_request(1, Some(900), 2_000)
            .expect_err("span too large should be blocked");
        assert_eq!(violation.code, "repo_read_span_too_large");
    }

    #[test]
    fn enforces_repo_search_limits() {
        let violation = validate_repo_search_request("needle", 999, true, true, true)
            .expect_err("max_results should be capped");
        assert_eq!(violation.code, "repo_search_results_too_large");
    }

    #[test]
    fn enforces_repo_query_and_tree_limits() {
        let query_violation =
            validate_repo_query_request(1_500, 8, 6).expect_err("budget limit should be enforced");
        assert_eq!(query_violation.code, "repo_query_budget_too_large");

        let tree_violation =
            validate_repo_tree_request(8, 200).expect_err("depth limit should be enforced");
        assert_eq!(tree_violation.code, "repo_tree_depth_too_large");
    }

    #[test]
    fn loads_policy_overrides_from_repo_root() {
        let root = make_temp_dir();
        let policy_dir = root.join(".tokenln");
        std::fs::create_dir_all(&policy_dir).expect("policy dir should be created");
        let policy_file = policy_dir.join("policy.toml");
        std::fs::write(
            &policy_file,
            r#"
[limits]
repo_query_budget = 480
repo_search_max_results = 70
repo_tree_max_depth = 2

[proxy]
block_broad_find = false
"#,
        )
        .expect("policy file should be written");

        let loaded = load_policy_for_repo_root(&root).expect("policy should load");
        assert_eq!(loaded.repo_query_budget, 480);
        assert_eq!(loaded.repo_search_max_results, 70);
        assert_eq!(loaded.repo_tree_max_depth, 2);
        assert!(!loaded.block_broad_find);

        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn discovers_policy_from_working_directory_ancestors() {
        let root = make_temp_dir();
        let policy_dir = root.join(".tokenln");
        std::fs::create_dir_all(&policy_dir).expect("policy dir should be created");
        std::fs::write(
            policy_dir.join("policy.toml"),
            r#"
[limits]
repo_read_max_span_lines = 120
"#,
        )
        .expect("policy file should be written");

        let nested = root.join("src").join("deeper");
        std::fs::create_dir_all(&nested).expect("nested dir should be created");

        let loaded = load_policy_for_working_dir(&nested).expect("policy should be discovered");
        assert_eq!(loaded.repo_read_max_span_lines, 120);

        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn proxy_rule_can_be_disabled_in_policy() {
        let policy = PolicySettings {
            block_broad_find: false,
            ..PolicySettings::default()
        };
        let violation = evaluate_proxy_command_with_policy(&policy, "find", &[".".to_string()]);
        assert!(violation.is_none());
    }

    #[test]
    fn invalid_policy_key_returns_error() {
        let root = make_temp_dir();
        let policy_dir = root.join(".tokenln");
        std::fs::create_dir_all(&policy_dir).expect("policy dir should be created");
        std::fs::write(
            policy_dir.join("policy.toml"),
            r#"
[limits]
unknown_field = 1
"#,
        )
        .expect("policy file should be written");

        let error = load_policy_for_repo_root(&root).expect_err("invalid key should fail");
        assert!(error.contains("unknown limits key"));
        std::fs::remove_dir_all(&root).ok();
    }

    fn make_temp_dir() -> PathBuf {
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let millis = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock should be valid")
            .as_millis();
        let nonce = COUNTER.fetch_add(1, Ordering::Relaxed);
        let path =
            env::temp_dir().join(format!("tokenln-policy-{}-{millis}-{nonce}", process::id()));
        std::fs::create_dir_all(&path).expect("temp dir should be created");
        path
    }
}
