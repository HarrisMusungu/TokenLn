//! Regex-free symbol index for Rust, Python, Go, TypeScript, and JavaScript.
//!
//! Scans source files line-by-line using hand-written pattern matchers and
//! returns `IndexedSymbol` records. No new crate dependencies required.
//!
//! Accuracy trade-off: the matchers are intentionally conservative — they
//! produce few false positives at the cost of missing some definitions (e.g.
//! multi-line signatures, macros, complex generics). For query-routing
//! purposes this is acceptable.

use std::ffi::OsStr;
use std::fs;
use std::path::{Path, PathBuf};

/// A symbol definition found in a source file.
#[derive(Debug, Clone)]
pub struct IndexedSymbol {
    /// The symbol's unqualified name (e.g. `build_context_packet`).
    pub name: String,
    pub kind: SymbolKind,
    /// Relative path from the repo root.
    pub path: String,
    /// 1-indexed line number of the definition.
    pub line: u32,
    pub visibility: SymbolVisibility,
}

#[derive(Debug, Clone, Eq, PartialEq)]
pub enum SymbolKind {
    Function,
    Method,
    Struct,
    Enum,
    Trait,
    Impl,
    Class,
    Interface,
    Type,
}

impl SymbolKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            SymbolKind::Function => "fn",
            SymbolKind::Method => "method",
            SymbolKind::Struct => "struct",
            SymbolKind::Enum => "enum",
            SymbolKind::Trait => "trait",
            SymbolKind::Impl => "impl",
            SymbolKind::Class => "class",
            SymbolKind::Interface => "interface",
            SymbolKind::Type => "type",
        }
    }
}

#[derive(Debug, Clone, Eq, PartialEq)]
pub enum SymbolVisibility {
    Public,
    Private,
    Unknown,
}

enum Language {
    Rust,
    Python,
    Go,
    TypeScript,
    JavaScript,
}

// ─── Public API ───────────────────────────────────────────────────────────────

/// Index all symbols in `file_path`.  Returns an empty vec on read errors.
pub fn index_file(file_path: &Path, display_path: &str) -> Vec<IndexedSymbol> {
    let lang = match detect_language(file_path) {
        Some(lang) => lang,
        None => return Vec::new(),
    };

    let text = match fs::read_to_string(file_path) {
        Ok(t) => t,
        Err(_) => return Vec::new(),
    };

    let mut symbols = Vec::new();
    for (idx, line) in text.lines().enumerate() {
        let line_num = (idx + 1) as u32;
        if let Some(sym) = parse_symbol(line, &lang, display_path, line_num) {
            symbols.push(sym);
        }
    }
    symbols
}

/// Options for scanning a directory tree.
pub struct IndexOptions<'a> {
    pub repo_root: &'a Path,
    /// Scope within the root to scan (defaults to root).
    pub scope: Option<&'a Path>,
    pub max_symbols: usize,
}

/// Walk the directory tree under `options.scope` and index all source files.
/// Returns symbols sorted by file path then line number.
pub fn index_directory(options: IndexOptions<'_>) -> Vec<IndexedSymbol> {
    let root = options.repo_root;
    let scope = options.scope.unwrap_or(root);
    let mut symbols = Vec::new();
    collect_symbols(root, scope, &mut symbols, options.max_symbols);
    symbols.sort_by(|a, b| a.path.cmp(&b.path).then_with(|| a.line.cmp(&b.line)));
    symbols
}

/// Search the index for symbols matching a name query (case-insensitive
/// substring match on `name`).  Returns matches sorted by relevance.
pub fn search_symbols<'a>(
    symbols: &'a [IndexedSymbol],
    query: &str,
    max_results: usize,
) -> Vec<&'a IndexedSymbol> {
    let query_lower = query.to_lowercase();
    let mut exact: Vec<&IndexedSymbol> = Vec::new();
    let mut prefix: Vec<&IndexedSymbol> = Vec::new();
    let mut contains: Vec<&IndexedSymbol> = Vec::new();

    for sym in symbols {
        let name_lower = sym.name.to_lowercase();
        if name_lower == query_lower {
            exact.push(sym);
        } else if name_lower.starts_with(&query_lower) {
            prefix.push(sym);
        } else if name_lower.contains(&query_lower) {
            contains.push(sym);
        }
    }

    let mut result: Vec<&IndexedSymbol> = Vec::new();
    result.extend_from_slice(&exact);
    result.extend_from_slice(&prefix);
    result.extend_from_slice(&contains);
    result.truncate(max_results);
    result
}

// ─── Internals ────────────────────────────────────────────────────────────────

const EXCLUDED_DIRS: &[&str] = &[".git", "target", ".tokenln", "node_modules", "vendor", "dist"];

fn collect_symbols(root: &Path, dir: &Path, out: &mut Vec<IndexedSymbol>, limit: usize) {
    if out.len() >= limit {
        return;
    }

    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };

    let mut children: Vec<PathBuf> = entries
        .filter_map(Result::ok)
        .map(|e| e.path())
        .collect();
    children.sort();

    for path in children {
        if out.len() >= limit {
            return;
        }

        let name = path
            .file_name()
            .and_then(OsStr::to_str)
            .unwrap_or_default();
        if EXCLUDED_DIRS.contains(&name) {
            continue;
        }

        if path.is_dir() {
            collect_symbols(root, &path, out, limit);
        } else if path.is_file() {
            let display = path
                .strip_prefix(root)
                .unwrap_or(&path)
                .to_string_lossy()
                .into_owned();
            let file_syms = index_file(&path, &display);
            let remaining = limit.saturating_sub(out.len());
            out.extend(file_syms.into_iter().take(remaining));
        }
    }
}

fn detect_language(path: &Path) -> Option<Language> {
    match path.extension().and_then(OsStr::to_str)? {
        "rs" => Some(Language::Rust),
        "py" => Some(Language::Python),
        "go" => Some(Language::Go),
        "ts" | "tsx" => Some(Language::TypeScript),
        "js" | "jsx" => Some(Language::JavaScript),
        _ => None,
    }
}

fn parse_symbol(line: &str, lang: &Language, path: &str, line_num: u32) -> Option<IndexedSymbol> {
    match lang {
        Language::Rust => parse_rust(line, path, line_num),
        Language::Python => parse_python(line, path, line_num),
        Language::Go => parse_go(line, path, line_num),
        Language::TypeScript | Language::JavaScript => parse_ts_js(line, path, line_num),
    }
}

// ─── Rust parser ─────────────────────────────────────────────────────────────

fn parse_rust(line: &str, path: &str, line_num: u32) -> Option<IndexedSymbol> {
    let trimmed = line.trim();

    // Skip comments and attributes.
    if trimmed.starts_with("//") || trimmed.starts_with('#') || trimmed.starts_with('*') {
        return None;
    }

    // Strip visibility prefix.
    let (vis, rest) = strip_rust_visibility(trimmed);

    // fn
    if let Some(after_fn) = rest.strip_prefix("fn ") {
        let name = first_ident(after_fn)?;
        return Some(make_symbol(name, SymbolKind::Function, vis, path, line_num));
    }
    // async fn
    if let Some(after) = rest
        .strip_prefix("async fn ")
        .or_else(|| rest.strip_prefix("async  fn "))
    {
        let name = first_ident(after)?;
        return Some(make_symbol(name, SymbolKind::Function, vis, path, line_num));
    }
    // struct
    if let Some(after) = rest.strip_prefix("struct ") {
        let name = first_ident(after)?;
        return Some(make_symbol(name, SymbolKind::Struct, vis, path, line_num));
    }
    // enum
    if let Some(after) = rest.strip_prefix("enum ") {
        let name = first_ident(after)?;
        return Some(make_symbol(name, SymbolKind::Enum, vis, path, line_num));
    }
    // trait
    if let Some(after) = rest.strip_prefix("trait ") {
        let name = first_ident(after)?;
        return Some(make_symbol(name, SymbolKind::Trait, vis, path, line_num));
    }
    // type alias
    if let Some(after) = rest.strip_prefix("type ") {
        let name = first_ident(after)?;
        return Some(make_symbol(name, SymbolKind::Type, vis, path, line_num));
    }
    // impl (impl Trait for Type  OR  impl Type)
    if let Some(after) = rest.strip_prefix("impl") {
        let rest2 = after.trim_start_matches(|c: char| c != ' ' && c != '{'); // skip generics
        let rest2 = rest2.trim();
        if rest2.is_empty() || rest2.starts_with('{') {
            return None;
        }
        // `impl Trait for Type` → use the type after `for`
        let name = if let Some(after_for) = find_word_after(rest2, "for") {
            first_ident(after_for)?
        } else {
            first_ident(rest2)?
        };
        return Some(make_symbol(name, SymbolKind::Impl, vis, path, line_num));
    }

    None
}

/// Strip a Rust visibility prefix (`pub`, `pub(crate)`, `pub(super)`, etc.)
/// and return whether it was public plus the remainder.
fn strip_rust_visibility(s: &str) -> (SymbolVisibility, &str) {
    if let Some(rest) = s.strip_prefix("pub(") {
        // e.g. `pub(crate) fn …`
        let close = rest.find(')').map(|i| i + 1).unwrap_or(0);
        let after = rest[close..].trim_start();
        return (SymbolVisibility::Public, after);
    }
    if let Some(rest) = s.strip_prefix("pub ") {
        return (SymbolVisibility::Public, rest.trim_start());
    }
    (SymbolVisibility::Private, s)
}

/// Find the word `needle` as a whole word in `s` and return the slice after it.
fn find_word_after<'a>(s: &'a str, needle: &str) -> Option<&'a str> {
    let mut pos = 0;
    while let Some(idx) = s[pos..].find(needle) {
        let abs = pos + idx;
        let before_ok = abs == 0 || !s[abs - 1..].starts_with(|c: char| c.is_alphanumeric() || c == '_');
        let after_pos = abs + needle.len();
        let after_ok = after_pos >= s.len() || !s[after_pos..].starts_with(|c: char| c.is_alphanumeric() || c == '_');
        if before_ok && after_ok {
            return Some(s[after_pos..].trim_start());
        }
        pos = abs + 1;
    }
    None
}

// ─── Python parser ────────────────────────────────────────────────────────────

fn parse_python(line: &str, path: &str, line_num: u32) -> Option<IndexedSymbol> {
    let trimmed = line.trim();

    if trimmed.starts_with('#') {
        return None;
    }

    // def (function or method — same syntax in Python)
    if let Some(after) = trimmed.strip_prefix("def ").or_else(|| trimmed.strip_prefix("async def ")) {
        let name = first_ident(after)?;
        let vis = if name.starts_with('_') {
            SymbolVisibility::Private
        } else {
            SymbolVisibility::Public
        };
        return Some(make_symbol(name, SymbolKind::Function, vis, path, line_num));
    }
    // class
    if let Some(after) = trimmed.strip_prefix("class ") {
        let name = first_ident(after)?;
        let vis = if name.starts_with('_') {
            SymbolVisibility::Private
        } else {
            SymbolVisibility::Public
        };
        return Some(make_symbol(name, SymbolKind::Class, vis, path, line_num));
    }

    None
}

// ─── Go parser ───────────────────────────────────────────────────────────────

fn parse_go(line: &str, path: &str, line_num: u32) -> Option<IndexedSymbol> {
    let trimmed = line.trim();

    if trimmed.starts_with("//") {
        return None;
    }

    // func — may be a top-level function or a method with a receiver.
    if let Some(after) = trimmed.strip_prefix("func ") {
        let after = after.trim();
        if after.starts_with('(') {
            // Method with receiver: `func (r ReceiverType) MethodName(...)`
            let close = after.find(')')?;
            let rest = after[close + 1..].trim();
            let name = first_ident(rest)?;
            let vis = go_visibility(&name);
            return Some(make_symbol(name, SymbolKind::Method, vis, path, line_num));
        } else {
            // Top-level function
            let name = first_ident(after)?;
            let vis = go_visibility(&name);
            return Some(make_symbol(name, SymbolKind::Function, vis, path, line_num));
        }
    }
    // type T struct | type T interface
    if let Some(after) = trimmed.strip_prefix("type ") {
        let name = first_ident(after)?;
        let vis = go_visibility(&name);
        let rest_after_name = after[name.len()..].trim();
        let kind = if rest_after_name.starts_with("struct") || rest_after_name.starts_with('{') {
            SymbolKind::Struct
        } else if rest_after_name.starts_with("interface") {
            SymbolKind::Interface
        } else {
            SymbolKind::Type
        };
        return Some(make_symbol(name, kind, vis, path, line_num));
    }

    None
}

fn go_visibility(name: &str) -> SymbolVisibility {
    match name.chars().next() {
        Some(c) if c.is_uppercase() => SymbolVisibility::Public,
        Some(_) => SymbolVisibility::Private,
        None => SymbolVisibility::Unknown,
    }
}

// ─── TypeScript / JavaScript parser ──────────────────────────────────────────

fn parse_ts_js(line: &str, path: &str, line_num: u32) -> Option<IndexedSymbol> {
    let trimmed = line.trim();

    if trimmed.starts_with("//") || trimmed.starts_with('*') {
        return None;
    }

    // Strip export / export default prefix.
    let (vis, rest) = strip_ts_export(trimmed);

    // function | async function
    if let Some(after) = rest
        .strip_prefix("function ")
        .or_else(|| rest.strip_prefix("async function "))
    {
        let name = first_ident(after)?;
        return Some(make_symbol(name, SymbolKind::Function, vis, path, line_num));
    }
    // class
    if let Some(after) = rest.strip_prefix("class ") {
        let name = first_ident(after)?;
        return Some(make_symbol(name, SymbolKind::Class, vis, path, line_num));
    }
    // interface (TypeScript only)
    if let Some(after) = rest.strip_prefix("interface ") {
        let name = first_ident(after)?;
        return Some(make_symbol(name, SymbolKind::Interface, vis, path, line_num));
    }
    // type alias (TypeScript)
    if let Some(after) = rest.strip_prefix("type ") {
        // Distinguish `type Foo = ...` from `typeof`
        let name = first_ident(after)?;
        if name == "of" {
            return None; // typeof keyword
        }
        return Some(make_symbol(name, SymbolKind::Type, vis, path, line_num));
    }
    // Arrow function assigned to const/let: `const name = (...) =>`
    if let Some(after) = rest
        .strip_prefix("const ")
        .or_else(|| rest.strip_prefix("let "))
    {
        let name = first_ident(after)?;
        let rest_after_name = after[name.len()..].trim_start();
        // Only capture if followed by `=` and the value looks like an arrow fn.
        if rest_after_name.starts_with('=') {
            let after_eq = rest_after_name[1..].trim();
            if after_eq.starts_with('(') || after_eq.starts_with("async") {
                return Some(make_symbol(name, SymbolKind::Function, vis, path, line_num));
            }
        }
    }

    None
}

fn strip_ts_export(s: &str) -> (SymbolVisibility, &str) {
    if let Some(rest) = s.strip_prefix("export default ") {
        return (SymbolVisibility::Public, rest.trim_start());
    }
    if let Some(rest) = s.strip_prefix("export ") {
        return (SymbolVisibility::Public, rest.trim_start());
    }
    (SymbolVisibility::Private, s)
}

// ─── Shared helpers ───────────────────────────────────────────────────────────

/// Extract the first identifier (alphanumeric + `_`) from the start of `s`.
fn first_ident(s: &str) -> Option<String> {
    let s = s.trim_start();
    let end = s
        .find(|c: char| !c.is_alphanumeric() && c != '_')
        .unwrap_or(s.len());
    if end == 0 {
        None
    } else {
        Some(s[..end].to_string())
    }
}

fn make_symbol(
    name: String,
    kind: SymbolKind,
    visibility: SymbolVisibility,
    path: &str,
    line: u32,
) -> IndexedSymbol {
    IndexedSymbol {
        name,
        kind,
        path: path.to_string(),
        line,
        visibility,
    }
}

// ─── Unit tests ───────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn rust_syms(src: &str) -> Vec<IndexedSymbol> {
        src.lines()
            .enumerate()
            .filter_map(|(i, line)| parse_rust(line, "src/lib.rs", (i + 1) as u32))
            .collect()
    }

    fn py_syms(src: &str) -> Vec<IndexedSymbol> {
        src.lines()
            .enumerate()
            .filter_map(|(i, line)| parse_python(line, "mod.py", (i + 1) as u32))
            .collect()
    }

    fn go_syms(src: &str) -> Vec<IndexedSymbol> {
        src.lines()
            .enumerate()
            .filter_map(|(i, line)| parse_go(line, "main.go", (i + 1) as u32))
            .collect()
    }

    fn ts_syms(src: &str) -> Vec<IndexedSymbol> {
        src.lines()
            .enumerate()
            .filter_map(|(i, line)| parse_ts_js(line, "index.ts", (i + 1) as u32))
            .collect()
    }

    // ── Rust ─────────────────────────────────────────────────────────────────

    #[test]
    fn rust_public_function() {
        let syms = rust_syms("pub fn my_fn(x: u32) -> u32 {");
        assert_eq!(syms.len(), 1);
        assert_eq!(syms[0].name, "my_fn");
        assert_eq!(syms[0].kind, SymbolKind::Function);
        assert_eq!(syms[0].visibility, SymbolVisibility::Public);
    }

    #[test]
    fn rust_private_function() {
        let syms = rust_syms("fn helper(x: &str) -> bool {");
        assert_eq!(syms.len(), 1);
        assert_eq!(syms[0].name, "helper");
        assert_eq!(syms[0].visibility, SymbolVisibility::Private);
    }

    #[test]
    fn rust_async_function() {
        let syms = rust_syms("pub async fn handle_request() {");
        assert_eq!(syms.len(), 1);
        assert_eq!(syms[0].name, "handle_request");
        assert_eq!(syms[0].kind, SymbolKind::Function);
        assert_eq!(syms[0].visibility, SymbolVisibility::Public);
    }

    #[test]
    fn rust_struct_and_enum() {
        let syms = rust_syms(
            "pub struct Foo {\npub(crate) enum Bar {\nstruct Private {",
        );
        assert_eq!(syms.len(), 3);
        assert_eq!(syms[0].name, "Foo");
        assert_eq!(syms[0].kind, SymbolKind::Struct);
        assert_eq!(syms[1].name, "Bar");
        assert_eq!(syms[1].kind, SymbolKind::Enum);
        assert_eq!(syms[1].visibility, SymbolVisibility::Public);
        assert_eq!(syms[2].name, "Private");
        assert_eq!(syms[2].visibility, SymbolVisibility::Private);
    }

    #[test]
    fn rust_trait() {
        let syms = rust_syms("pub trait Pipeline {");
        assert_eq!(syms.len(), 1);
        assert_eq!(syms[0].kind, SymbolKind::Trait);
        assert_eq!(syms[0].name, "Pipeline");
    }

    #[test]
    fn rust_impl_type() {
        let syms = rust_syms("impl BasicOptimizer {");
        assert_eq!(syms.len(), 1);
        assert_eq!(syms[0].kind, SymbolKind::Impl);
        assert_eq!(syms[0].name, "BasicOptimizer");
    }

    #[test]
    fn rust_impl_trait_for() {
        let syms = rust_syms("impl Optimizer for BasicOptimizer {");
        assert_eq!(syms.len(), 1);
        assert_eq!(syms[0].kind, SymbolKind::Impl);
        assert_eq!(syms[0].name, "BasicOptimizer");
    }

    #[test]
    fn rust_type_alias() {
        let syms = rust_syms("pub type Result<T> = std::result::Result<T, Error>;");
        assert_eq!(syms.len(), 1);
        assert_eq!(syms[0].kind, SymbolKind::Type);
        assert_eq!(syms[0].name, "Result");
    }

    #[test]
    fn rust_comments_skipped() {
        let syms = rust_syms("// fn not_a_symbol()\n/// pub fn also_not()\n#[derive(Debug)]");
        assert!(syms.is_empty(), "comments and attributes must not produce symbols");
    }

    // ── Python ───────────────────────────────────────────────────────────────

    #[test]
    fn python_function_and_class() {
        let syms = py_syms("def compute(x):\nclass MyModel:\n    def _private(self):");
        assert_eq!(syms.len(), 3);
        assert_eq!(syms[0].name, "compute");
        assert_eq!(syms[0].kind, SymbolKind::Function);
        assert_eq!(syms[1].name, "MyModel");
        assert_eq!(syms[1].kind, SymbolKind::Class);
        assert_eq!(syms[2].name, "_private");
        assert_eq!(syms[2].visibility, SymbolVisibility::Private);
    }

    #[test]
    fn python_async_def() {
        let syms = py_syms("async def fetch(url: str) -> dict:");
        assert_eq!(syms.len(), 1);
        assert_eq!(syms[0].name, "fetch");
        assert_eq!(syms[0].kind, SymbolKind::Function);
    }

    // ── Go ───────────────────────────────────────────────────────────────────

    #[test]
    fn go_function_visibility() {
        let syms = go_syms("func Public() {}\nfunc private() {}");
        assert_eq!(syms.len(), 2);
        assert_eq!(syms[0].visibility, SymbolVisibility::Public);
        assert_eq!(syms[1].visibility, SymbolVisibility::Private);
    }

    #[test]
    fn go_method() {
        let syms = go_syms("func (r *Router) Handle(path string) {}");
        assert_eq!(syms.len(), 1);
        assert_eq!(syms[0].name, "Handle");
        assert_eq!(syms[0].kind, SymbolKind::Method);
        assert_eq!(syms[0].visibility, SymbolVisibility::Public);
    }

    #[test]
    fn go_struct_and_interface() {
        let syms = go_syms("type Server struct {\ntype Handler interface {");
        assert_eq!(syms.len(), 2);
        assert_eq!(syms[0].kind, SymbolKind::Struct);
        assert_eq!(syms[1].kind, SymbolKind::Interface);
    }

    // ── TypeScript / JS ──────────────────────────────────────────────────────

    #[test]
    fn ts_exported_function_and_class() {
        let syms = ts_syms(
            "export function greet(name: string): void {\nexport class ApiClient {",
        );
        assert_eq!(syms.len(), 2);
        assert_eq!(syms[0].name, "greet");
        assert_eq!(syms[0].kind, SymbolKind::Function);
        assert_eq!(syms[0].visibility, SymbolVisibility::Public);
        assert_eq!(syms[1].name, "ApiClient");
        assert_eq!(syms[1].kind, SymbolKind::Class);
    }

    #[test]
    fn ts_interface_and_type_alias() {
        let syms = ts_syms("export interface Config {\nexport type Id = string;");
        assert_eq!(syms.len(), 2);
        assert_eq!(syms[0].kind, SymbolKind::Interface);
        assert_eq!(syms[1].kind, SymbolKind::Type);
    }

    #[test]
    fn ts_arrow_function() {
        let syms = ts_syms("const handler = async (req, res) => {");
        assert_eq!(syms.len(), 1);
        assert_eq!(syms[0].name, "handler");
        assert_eq!(syms[0].kind, SymbolKind::Function);
    }

    // ── search_symbols ───────────────────────────────────────────────────────

    #[test]
    fn search_prefers_exact_then_prefix_then_substring() {
        let syms = vec![
            make_symbol("build_context".to_string(), SymbolKind::Function, SymbolVisibility::Public, "a.rs", 1),
            make_symbol("build".to_string(), SymbolKind::Function, SymbolVisibility::Public, "b.rs", 1),
            make_symbol("rebuild_index".to_string(), SymbolKind::Function, SymbolVisibility::Public, "c.rs", 1),
        ];
        let results = search_symbols(&syms, "build", 10);
        assert_eq!(results[0].name, "build");          // exact
        assert_eq!(results[1].name, "build_context");  // prefix
        assert_eq!(results[2].name, "rebuild_index");  // contains
    }
}
