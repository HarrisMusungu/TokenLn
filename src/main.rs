mod proxy;

use std::collections::{HashMap, HashSet};
use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{self, Command};
use std::time::{SystemTime, UNIX_EPOCH};

use clap::{Args, Parser, Subcommand, ValueEnum};
use serde::Serialize;

use tokenln::fixlog::load_fix_signatures;

use tokenln::analysis::cargo_build::CargoBuildAnalyzer;
use tokenln::analysis::cargo_test::CargoTestAnalyzer;
use tokenln::analysis::go_test::GoTestAnalyzer;
use tokenln::analysis::jest::JestAnalyzer;
use tokenln::analysis::pytest::PytestAnalyzer;
use tokenln::context::{
    build_context_packet, deviation_signature, BuildPacketOptions, ContextPacket, EvidenceRef,
};
use tokenln::emitters::claude::ClaudeEmitter;
use tokenln::emitters::codex::CodexEmitter;
use tokenln::emitters::copilot::CopilotEmitter;
use tokenln::emitters::generic::GenericEmitter;
use tokenln::emitters::ollama::OllamaEmitter;
use tokenln::lexers::cargo_build::CargoBuildLexer;
use tokenln::lexers::cargo_test::CargoTestLexer;
use tokenln::lexers::go_test::GoTestLexer;
use tokenln::lexers::jest::JestLexer;
use tokenln::lexers::pytest::PytestLexer;
use tokenln::optimizer::BasicOptimizer;
use tokenln::parsers::cargo_build::CargoBuildParser;
use tokenln::parsers::cargo_test::CargoTestParser;
use tokenln::parsers::go_test::GoTestParser;
use tokenln::parsers::jest::JestParser;
use tokenln::parsers::pytest::PytestParser;
use tokenln::pipeline::{Emitter, Lexer, Optimizer, Parser as PipelineParser, SemanticAnalyzer};
use tokenln::policy::{
    evaluate_proxy_command_with_policy, load_policy_for_repo_root, load_policy_for_working_dir,
    validate_repo_query_request_with_policy, validate_repo_read_request_with_policy,
    validate_repo_search_request_with_policy, validate_repo_tree_request_with_policy,
};
use tokenln::postprocess::apply_low_confidence_fallback;
use tokenln::repo::{
    log_repo_file, query_repo_context, read_repo_file, search_repo, tree_repo, RepoLogOptions,
    RepoQueryOptions, RepoReadOptions, RepoSearchOptions, RepoTreeOptions,
};

#[derive(Debug, Parser)]
#[command(
    name = "tokenln",
    version,
    about = "Compile CLI output into deviation IR"
)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Debug, Subcommand)]
enum Commands {
    Cargo(CargoCommand),
    Go(GoCommand),
    Jest(JestArgs),
    Pytest(PytestArgs),
    Repo(RepoCommand),
    Proxy(ProxyCommand),
    Query(QueryArgs),
    Expand(ExpandArgs),
    Compare(CompareArgs),
    /// Start the MCP server (JSON-RPC 2.0 over stdio).
    Serve(ServeArgs),
    /// Record a deviation as fixed in the fix log.
    Fixed(FixedArgs),
    /// Re-run the command from a previous run and check if deviations are fixed.
    Replay(ReplayArgs),
}

#[derive(Debug, Args)]
struct CargoCommand {
    #[command(subcommand)]
    command: CargoSubcommand,
}

#[derive(Debug, Subcommand)]
enum CargoSubcommand {
    Test(CargoTestArgs),
    Build(CargoBuildArgs),
}

#[derive(Debug, Args)]
struct GoCommand {
    #[command(subcommand)]
    command: GoSubcommand,
}

#[derive(Debug, Subcommand)]
enum GoSubcommand {
    Test(GoTestArgs),
}

#[derive(Debug, Args)]
struct RepoCommand {
    #[command(subcommand)]
    command: RepoSubcommand,
}

#[derive(Debug, Subcommand)]
enum RepoSubcommand {
    Query(RepoQueryArgs),
    Search(RepoSearchArgs),
    Read(RepoReadArgs),
    Tree(RepoTreeArgs),
    Log(RepoLogArgs),
}

#[derive(Debug, Args)]
struct ProxyCommand {
    #[command(subcommand)]
    command: ProxySubcommand,
}

#[derive(Debug, Args)]
struct RepoSearchArgs {
    query: String,

    #[arg(long, value_name = "DIR", default_value = ".")]
    dir: PathBuf,

    #[arg(long, value_name = "PATH")]
    path: Option<PathBuf>,

    #[arg(long, value_name = "GLOB")]
    glob: Option<String>,

    #[arg(long, default_value_t = 80)]
    max_results: usize,

    #[arg(long)]
    ignore_case: bool,

    #[arg(long)]
    fixed_strings: bool,

    #[arg(long)]
    emit_json: bool,
}

#[derive(Debug, Args)]
struct RepoQueryArgs {
    objective: String,

    #[arg(long, value_name = "DIR", default_value = ".")]
    dir: PathBuf,

    #[arg(long, value_name = "PATH")]
    path: Option<PathBuf>,

    #[arg(long, default_value_t = 320)]
    budget: u32,

    #[arg(long, default_value_t = 8)]
    max_findings: usize,

    #[arg(long, default_value_t = 6)]
    max_hints: usize,

    #[arg(long, value_enum, default_value_t = EmitterTarget::Generic)]
    target: EmitterTarget,

    #[arg(long)]
    emit_json: bool,
}

#[derive(Debug, Args)]
struct RepoReadArgs {
    path: PathBuf,

    #[arg(long, value_name = "DIR", default_value = ".")]
    dir: PathBuf,

    #[arg(long, default_value_t = 1)]
    start_line: u32,

    #[arg(long)]
    end_line: Option<u32>,

    #[arg(long, default_value_t = 6000)]
    max_chars: usize,

    #[arg(long)]
    emit_json: bool,
}

#[derive(Debug, Args)]
struct RepoTreeArgs {
    #[arg(long, value_name = "DIR", default_value = ".")]
    dir: PathBuf,

    #[arg(long, value_name = "PATH")]
    path: Option<PathBuf>,

    #[arg(long, default_value_t = 3)]
    max_depth: u32,

    #[arg(long, default_value_t = 200)]
    max_entries: usize,

    #[arg(long)]
    emit_json: bool,
}

#[derive(Debug, Args)]
struct RepoLogArgs {
    /// File path relative to the repo root.
    path: PathBuf,

    #[arg(long, value_name = "DIR", default_value = ".")]
    dir: PathBuf,

    #[arg(long, default_value_t = 20)]
    limit: usize,

    #[arg(long)]
    emit_json: bool,
}

#[derive(Debug, Subcommand)]
enum ProxySubcommand {
    Run(ProxyRunArgs),
    Install(ProxyInstallArgs),
    Last(ProxyLastArgs),
}

#[derive(Debug, Args)]
struct ProxyRunArgs {
    #[arg(long, value_name = "FILE")]
    from_file: Option<PathBuf>,

    #[arg(long)]
    emit_ir: bool,

    #[arg(long, value_enum, default_value_t = EmitterTarget::Generic)]
    target: EmitterTarget,

    #[arg(long, value_enum, default_value_t = SuccessOutputMode::Compact)]
    success_output: SuccessOutputMode,

    #[arg(long, value_name = "DIR", default_value = ".tokenln/runs")]
    artifacts_dir: PathBuf,

    #[arg(long)]
    no_artifacts: bool,

    #[arg(long)]
    no_delta: bool,

    /// Bypass broad-command guardrails for passthrough shell commands.
    #[arg(long)]
    allow_broad: bool,

    #[arg(last = true, value_name = "COMMAND", required = true)]
    command: Vec<String>,
}

#[derive(Debug, Args)]
struct ProxyInstallArgs {
    #[arg(long, value_enum, default_value_t = EmitterTarget::Generic)]
    target: EmitterTarget,

    #[arg(long, value_name = "DIR", default_value = ".tokenln/bin")]
    dir: PathBuf,

    #[arg(long, value_name = "PATH")]
    tokenln_bin: Option<PathBuf>,

    #[arg(long)]
    force: bool,
}

#[derive(Debug, Args)]
struct ProxyLastArgs {
    #[arg(long, value_name = "DIR", default_value = ".tokenln/runs")]
    dir: PathBuf,

    #[arg(long, conflicts_with = "ir")]
    raw: bool,

    #[arg(long, conflicts_with = "raw")]
    ir: bool,
}

#[derive(Debug, Args)]
struct QueryArgs {
    #[arg(long, default_value_t = 400)]
    budget: u32,

    #[arg(long, value_enum, default_value_t = EmitterTarget::Generic)]
    target: EmitterTarget,

    #[arg(long)]
    emit_json: bool,

    #[arg(long, value_name = "TEXT", default_value = "fix deviations")]
    objective: String,

    #[arg(long, value_name = "DIR", default_value = ".tokenln/runs")]
    dir: PathBuf,

    #[arg(long, value_name = "RUN_DIR")]
    run: Option<PathBuf>,
}

#[derive(Debug, Args)]
struct ExpandArgs {
    deviation_id: String,

    #[arg(long, value_enum, default_value_t = ExpandView::Evidence)]
    view: ExpandView,

    #[arg(long, default_value_t = 180)]
    budget: u32,

    #[arg(long, value_enum, default_value_t = EmitterTarget::Generic)]
    target: EmitterTarget,

    #[arg(long)]
    emit_json: bool,

    #[arg(long, value_name = "TEXT", default_value = "fix deviations")]
    objective: String,

    #[arg(long, value_name = "DIR", default_value = ".tokenln/runs")]
    dir: PathBuf,

    #[arg(long, value_name = "RUN_DIR")]
    run: Option<PathBuf>,
}

#[derive(Debug, Args)]
struct CompareArgs {
    #[arg(long, value_name = "DIR", default_value = ".tokenln/runs")]
    dir: PathBuf,

    #[arg(long, value_name = "RUN_DIR", conflicts_with = "latest")]
    run: Option<PathBuf>,

    #[arg(long, value_name = "RUN_DIR", conflicts_with = "previous")]
    previous_run: Option<PathBuf>,

    #[arg(long)]
    latest: bool,

    #[arg(long)]
    previous: bool,

    #[arg(long, value_enum, default_value_t = EmitterTarget::Generic)]
    target: EmitterTarget,

    #[arg(long)]
    emit_json: bool,
}

#[derive(Debug, Args)]
struct CargoTestArgs {
    #[arg(long, value_name = "FILE")]
    from_file: Option<PathBuf>,

    #[arg(long)]
    emit_ir: bool,

    #[arg(long, value_enum, default_value_t = EmitterTarget::Generic)]
    target: EmitterTarget,

    /// Save artifacts and show delta from previous run if one exists.
    #[arg(long)]
    delta: bool,

    #[arg(long, value_name = "DIR", default_value = ".tokenln/runs")]
    artifacts_dir: PathBuf,

    #[arg(last = true, value_name = "ARGS")]
    passthrough: Vec<String>,
}

#[derive(Debug, Args)]
struct CargoBuildArgs {
    #[arg(long, value_name = "FILE")]
    from_file: Option<PathBuf>,

    #[arg(long)]
    emit_ir: bool,

    #[arg(long, value_enum, default_value_t = EmitterTarget::Generic)]
    target: EmitterTarget,

    /// Save artifacts and show delta from previous run if one exists.
    #[arg(long)]
    delta: bool,

    #[arg(long, value_name = "DIR", default_value = ".tokenln/runs")]
    artifacts_dir: PathBuf,

    #[arg(last = true, value_name = "ARGS")]
    passthrough: Vec<String>,
}

#[derive(Debug, Args)]
struct GoTestArgs {
    #[arg(long, value_name = "FILE")]
    from_file: Option<PathBuf>,

    #[arg(long)]
    emit_ir: bool,

    #[arg(long, value_enum, default_value_t = EmitterTarget::Generic)]
    target: EmitterTarget,

    /// Save artifacts and show delta from previous run if one exists.
    #[arg(long)]
    delta: bool,

    #[arg(long, value_name = "DIR", default_value = ".tokenln/runs")]
    artifacts_dir: PathBuf,

    #[arg(last = true, value_name = "ARGS")]
    passthrough: Vec<String>,
}

#[derive(Debug, Args)]
struct PytestArgs {
    #[arg(long, value_name = "FILE")]
    from_file: Option<PathBuf>,

    #[arg(long)]
    emit_ir: bool,

    #[arg(long, value_enum, default_value_t = EmitterTarget::Generic)]
    target: EmitterTarget,

    /// Save artifacts and show delta from previous run if one exists.
    #[arg(long)]
    delta: bool,

    #[arg(long, value_name = "DIR", default_value = ".tokenln/runs")]
    artifacts_dir: PathBuf,

    #[arg(last = true, value_name = "ARGS")]
    passthrough: Vec<String>,
}

#[derive(Debug, Args)]
struct JestArgs {
    #[arg(long, value_name = "FILE")]
    from_file: Option<PathBuf>,

    #[arg(long)]
    emit_ir: bool,

    #[arg(long, value_enum, default_value_t = EmitterTarget::Generic)]
    target: EmitterTarget,

    /// Save artifacts and show delta from previous run if one exists.
    #[arg(long)]
    delta: bool,

    #[arg(long, value_name = "DIR", default_value = ".tokenln/runs")]
    artifacts_dir: PathBuf,

    #[arg(last = true, value_name = "ARGS")]
    passthrough: Vec<String>,
}

#[derive(Debug, Args)]
struct ServeArgs {
    /// Artifacts directory to read run data from.
    #[arg(long, value_name = "DIR", default_value = ".tokenln/runs")]
    dir: PathBuf,

    /// Path to the fix log file.
    #[arg(long, value_name = "FILE", default_value = ".tokenln/fix_log.jsonl")]
    fix_log: PathBuf,

    /// Repository root for repo tools (`repo_search`, `repo_read`, `repo_tree`).
    #[arg(long, value_name = "DIR", default_value = ".")]
    repo_root: PathBuf,
}

#[derive(Debug, Args)]
struct FixedArgs {
    /// Deviation ID to mark as fixed (e.g. d1, d2).
    deviation_id: String,

    /// Artifacts directory containing the run to reference.
    #[arg(long, value_name = "DIR", default_value = ".tokenln/runs")]
    dir: PathBuf,

    /// Path to the fix log file.
    #[arg(long, value_name = "FILE", default_value = ".tokenln/fix_log.jsonl")]
    fix_log: PathBuf,

    /// Optional note describing what was fixed.
    #[arg(long, value_name = "TEXT")]
    note: Option<String>,
}

/// Arguments for `tokenln replay`.
#[derive(Debug, Args)]
struct ReplayArgs {
    /// Artifacts directory to look up the run from.
    #[arg(long, value_name = "DIR", default_value = ".tokenln/runs")]
    dir: PathBuf,

    /// Specific run directory to replay (defaults to latest).
    #[arg(long, value_name = "RUN_DIR")]
    run: Option<PathBuf>,

    #[arg(long, value_enum, default_value_t = EmitterTarget::Generic)]
    target: EmitterTarget,

    #[arg(long)]
    emit_json: bool,
}

#[derive(Debug, Serialize)]
struct ReplayResult {
    run_id: String,
    source: String,
    command: String,
    original_count: usize,
    fixed_count: usize,
    still_failing_count: usize,
    new_count: usize,
    verdict: String,
    fixed: Vec<ReplayDeviation>,
    still_failing: Vec<ReplayDeviation>,
    new: Vec<ReplayDeviation>,
}

#[derive(Debug, Serialize)]
struct ReplayDeviation {
    id: String,
    summary: String,
    location: String,
    confidence: f32,
}

struct CommandRun {
    stdout: String,
    stderr: String,
    exit_code: Option<i32>,
}

impl CommandRun {
    fn combined_output(&self) -> String {
        if self.stderr.is_empty() {
            return self.stdout.clone();
        }

        if self.stdout.is_empty() {
            return self.stderr.clone();
        }

        if self.stdout.ends_with('\n') {
            format!("{}{}", self.stdout, self.stderr)
        } else {
            format!("{}\n{}", self.stdout, self.stderr)
        }
    }
}

#[derive(Debug, Clone)]
struct ProxyArtifacts {
    run_dir: PathBuf,
}

#[derive(Debug)]
struct LoadedRunArtifacts {
    run_dir: PathBuf,
    run_id: String,
    report: tokenln::ir::DeviationReport,
    report_artifact: String,
    raw_output: String,
}

#[derive(Debug, Serialize)]
struct ExpansionResult {
    run_id: String,
    deviation_id: String,
    view: String,
    budget_tokens: u32,
    used_tokens: u32,
    summary: String,
    expected: String,
    actual: String,
    location: String,
    confidence: f32,
    confidence_reasons: Vec<String>,
    trace: Vec<String>,
    evidence_refs: Vec<EvidenceRef>,
    raw_excerpt: Option<String>,
}

#[derive(Debug, Serialize)]
struct CompareResult {
    current_run_id: String,
    previous_run_id: String,
    source: String,
    current_total: usize,
    previous_total: usize,
    new_count: usize,
    resolved_count: usize,
    persistent_count: usize,
    new: Vec<ComparedDeviation>,
    resolved: Vec<ComparedDeviation>,
    persistent: Vec<PersistentDeviation>,
}

#[derive(Debug, Serialize)]
struct ComparedDeviation {
    id: String,
    summary: String,
    expected: String,
    actual: String,
    location: String,
    confidence: f32,
}

#[derive(Debug, Serialize)]
struct PersistentDeviation {
    current_id: String,
    previous_id: String,
    summary: String,
    location: String,
    confidence_current: f32,
    confidence_previous: f32,
    confidence_delta: f32,
}

#[derive(Debug, Copy, Clone, Eq, PartialEq, ValueEnum)]
enum EmitterTarget {
    Generic,
    Claude,
    Ollama,
    Codex,
    Copilot,
}

#[derive(Debug, Copy, Clone, Eq, PartialEq, ValueEnum)]
enum SuccessOutputMode {
    Compact,
    Passthrough,
}

#[derive(Debug, Copy, Clone, Eq, PartialEq, ValueEnum)]
enum ExpandView {
    Evidence,
    Trace,
    Full,
}

impl EmitterTarget {
    fn as_cli_value(self) -> &'static str {
        match self {
            EmitterTarget::Generic => "generic",
            EmitterTarget::Claude => "claude",
            EmitterTarget::Ollama => "ollama",
            EmitterTarget::Codex => "codex",
            EmitterTarget::Copilot => "copilot",
        }
    }
}

fn main() {
    let cli = Cli::parse();

    let exit_code = match run(cli) {
        Ok(code) => code,
        Err(err) => {
            eprintln!("tokenln error: {err}");
            1
        }
    };

    process::exit(exit_code);
}

fn run(cli: Cli) -> Result<i32, String> {
    match cli.command {
        Commands::Cargo(cargo) => match cargo.command {
            CargoSubcommand::Test(args) => run_cargo_test(args),
            CargoSubcommand::Build(args) => run_cargo_build(args),
        },
        Commands::Go(go) => match go.command {
            GoSubcommand::Test(args) => run_go_test(args),
        },
        Commands::Repo(repo) => run_repo(repo),
        Commands::Jest(args) => run_jest(args),
        Commands::Pytest(args) => run_pytest(args),
        Commands::Proxy(proxy) => run_proxy(proxy),
        Commands::Query(args) => run_query(args),
        Commands::Expand(args) => run_expand(args),
        Commands::Compare(args) => run_compare(args),
        Commands::Serve(args) => run_serve(args),
        Commands::Fixed(args) => run_fixed(args),
        Commands::Replay(args) => run_replay(args),
    }
}

fn run_repo(repo: RepoCommand) -> Result<i32, String> {
    match repo.command {
        RepoSubcommand::Query(args) => run_repo_query(args),
        RepoSubcommand::Search(args) => run_repo_search(args),
        RepoSubcommand::Read(args) => run_repo_read(args),
        RepoSubcommand::Tree(args) => run_repo_tree(args),
        RepoSubcommand::Log(args) => run_repo_log(args),
    }
}

fn run_repo_query(args: RepoQueryArgs) -> Result<i32, String> {
    let policy =
        load_policy_for_repo_root(&args.dir).map_err(|err| format!("tokenln policy: {err}"))?;
    validate_repo_query_request_with_policy(
        &policy,
        args.budget,
        args.max_findings,
        args.max_hints,
    )
    .map_err(|violation| violation.render())?;

    let packet = query_repo_context(RepoQueryOptions {
        repo_root: &args.dir,
        scope: args.path.as_deref(),
        objective: &args.objective,
        budget_tokens: args.budget,
        max_findings: args.max_findings,
        max_hints: args.max_hints,
    })?;

    if args.emit_json {
        let text = serde_json::to_string_pretty(&packet)
            .map_err(|err| format!("failed to serialize repo query packet: {err}"))?;
        println!("{text}");
    } else {
        println!("{}", render_repo_query(&packet, args.target));
    }
    Ok(0)
}

fn run_repo_search(args: RepoSearchArgs) -> Result<i32, String> {
    let policy =
        load_policy_for_repo_root(&args.dir).map_err(|err| format!("tokenln policy: {err}"))?;
    validate_repo_search_request_with_policy(
        &policy,
        &args.query,
        args.max_results,
        args.fixed_strings,
        args.path.is_some(),
        args.glob.is_some(),
    )
    .map_err(|violation| violation.render())?;

    let result = search_repo(RepoSearchOptions {
        repo_root: &args.dir,
        scope: args.path.as_deref(),
        query: &args.query,
        glob: args.glob.as_deref(),
        max_results: args.max_results,
        ignore_case: args.ignore_case,
        fixed_strings: args.fixed_strings,
    })?;

    if args.emit_json {
        let text = serde_json::to_string_pretty(&result)
            .map_err(|err| format!("failed to serialize repo search result: {err}"))?;
        println!("{text}");
    } else {
        println!("{}", render_repo_search_plain(&result));
    }
    Ok(0)
}

fn run_repo_read(args: RepoReadArgs) -> Result<i32, String> {
    let policy =
        load_policy_for_repo_root(&args.dir).map_err(|err| format!("tokenln policy: {err}"))?;
    validate_repo_read_request_with_policy(&policy, args.start_line, args.end_line, args.max_chars)
        .map_err(|violation| violation.render())?;

    let result = read_repo_file(RepoReadOptions {
        repo_root: &args.dir,
        path: &args.path,
        start_line: args.start_line,
        end_line: args.end_line,
        max_chars: args.max_chars,
    })?;

    if args.emit_json {
        let text = serde_json::to_string_pretty(&result)
            .map_err(|err| format!("failed to serialize repo read result: {err}"))?;
        println!("{text}");
    } else {
        println!("{}", render_repo_read_plain(&result));
    }
    Ok(0)
}

fn run_repo_tree(args: RepoTreeArgs) -> Result<i32, String> {
    let policy =
        load_policy_for_repo_root(&args.dir).map_err(|err| format!("tokenln policy: {err}"))?;
    validate_repo_tree_request_with_policy(&policy, args.max_depth, args.max_entries)
        .map_err(|violation| violation.render())?;

    let result = tree_repo(RepoTreeOptions {
        repo_root: &args.dir,
        scope: args.path.as_deref(),
        max_depth: args.max_depth,
        max_entries: args.max_entries,
    })?;

    if args.emit_json {
        let text = serde_json::to_string_pretty(&result)
            .map_err(|err| format!("failed to serialize repo tree result: {err}"))?;
        println!("{text}");
    } else {
        println!("{}", render_repo_tree_plain(&result));
    }
    Ok(0)
}

fn run_repo_log(args: RepoLogArgs) -> Result<i32, String> {
    let result = log_repo_file(RepoLogOptions {
        repo_root: &args.dir,
        path: &args.path,
        limit: args.limit,
    })?;

    if args.emit_json {
        let text = serde_json::to_string_pretty(&result)
            .map_err(|err| format!("failed to serialize repo log result: {err}"))?;
        println!("{text}");
    } else {
        let mut lines = Vec::new();
        lines.push(format!(
            "REPO_LOG path={} returned={}",
            result.path, result.returned
        ));
        if result.entries.is_empty() {
            lines.push("  (no commits found)".to_string());
        } else {
            for entry in &result.entries {
                lines.push(format!(
                    "  {} {} {}",
                    entry.hash, entry.date, entry.subject
                ));
            }
        }
        println!("{}", lines.join("\n"));
    }
    Ok(0)
}

fn render_repo_search_plain(result: &tokenln::repo::RepoSearchResult) -> String {
    let mut lines = Vec::new();
    lines.push(format!(
        "REPO_SEARCH backend={} query={:?} scope={} returned={} truncated={}",
        result.backend, result.query, result.scope, result.returned, result.truncated
    ));
    if result.matches.is_empty() {
        lines.push("matches: none".to_string());
    } else {
        lines.push("matches:".to_string());
        for entry in &result.matches {
            lines.push(format!(
                "  - {}:{}:{} {}",
                entry.path, entry.line, entry.column, entry.snippet
            ));
        }
    }
    lines.join("\n")
}

fn render_repo_read_plain(result: &tokenln::repo::RepoReadResult) -> String {
    format!(
        "REPO_READ path={} lines={}:{} total_lines={} truncated={}\n{}",
        result.path,
        result.start_line,
        result.end_line,
        result.total_lines,
        result.truncated,
        result.content
    )
}

fn render_repo_tree_plain(result: &tokenln::repo::RepoTreeResult) -> String {
    let mut lines = Vec::new();
    lines.push(format!(
        "REPO_TREE scope={} depth={} returned={} truncated={}",
        result.scope, result.max_depth, result.returned, result.truncated
    ));
    for entry in &result.entries {
        lines.push(format!("  - [{}] {}", entry.kind, entry.path));
    }
    lines.join("\n")
}

fn render_repo_query(packet: &tokenln::repo::RepoContextPacket, target: EmitterTarget) -> String {
    match target {
        EmitterTarget::Generic => render_repo_query_plain(packet),
        EmitterTarget::Claude
        | EmitterTarget::Codex
        | EmitterTarget::Copilot
        | EmitterTarget::Ollama => render_repo_query_markdown(packet),
    }
}

fn render_repo_query_plain(packet: &tokenln::repo::RepoContextPacket) -> String {
    let mut lines = Vec::new();
    lines.push(format!(
        "REPO_CONTEXT_PACKET id={} scope={} budget={} used={}",
        packet.packet_id, packet.scope, packet.budget_tokens, packet.used_tokens
    ));
    lines.push(format!("objective: {}", packet.objective));
    lines.push(format!("findings: {}", packet.findings.len()));

    for finding in &packet.findings {
        let line = finding
            .line
            .map(|value| value.to_string())
            .unwrap_or_else(|| "?".to_string());
        lines.push(format!(
            "[{}] {:.2} {}:{} {}",
            finding.id, finding.relevance_score, finding.path, line, finding.summary
        ));
        lines.push(format!("  snippet: {}", finding.snippet));
        lines.push(format!("  read_hint: {}", finding.read_hint));
    }

    if !packet.expansion_hints.is_empty() {
        lines.push("expansion_hints:".to_string());
        let shown = packet.expansion_hints.iter().take(6);
        for hint in shown {
            lines.push(format!(
                "  - {} ({}) est_tokens={} :: {}",
                hint.finding_id, hint.reason, hint.estimated_tokens, hint.hint
            ));
        }
        let hidden = packet
            .expansion_hints
            .len()
            .saturating_sub(6)
            .saturating_add(packet.omitted_hints);
        if hidden > 0 {
            lines.push(format!("  - ... {} additional hints omitted", hidden));
        }
    } else if packet.omitted_hints > 0 {
        lines.push(format!(
            "expansion_hints: ... {} additional hints omitted",
            packet.omitted_hints
        ));
    }

    lines.join("\n")
}

fn render_repo_query_markdown(packet: &tokenln::repo::RepoContextPacket) -> String {
    let mut sections = Vec::new();
    sections.push("# TokenLn Repo Context Packet".to_string());
    sections.push(format!(
        "Packet: `{}`  \nScope: `{}`  \nBudget: `{}` tokens  \nUsed: `{}` tokens  \nObjective: `{}`",
        packet.packet_id, packet.scope, packet.budget_tokens, packet.used_tokens, packet.objective
    ));
    sections.push(format!("Included findings: `{}`", packet.findings.len()));

    for finding in &packet.findings {
        let line = finding
            .line
            .map(|value| value.to_string())
            .unwrap_or_else(|| "?".to_string());
        sections.push(format!(
            "## {} · relevance {:.2}\nSummary: {}\nPath: `{}:{}`\nSnippet: `{}`\nRead hint: `{}`",
            finding.id,
            finding.relevance_score,
            finding.summary,
            finding.path,
            line,
            finding.snippet,
            finding.read_hint
        ));
    }

    if !packet.expansion_hints.is_empty() {
        let hints = packet
            .expansion_hints
            .iter()
            .take(6)
            .map(|hint| {
                format!(
                    "- `{}` ({}) est `{}` tokens -> `{}`",
                    hint.finding_id, hint.reason, hint.estimated_tokens, hint.hint
                )
            })
            .collect::<Vec<_>>()
            .join("\n");
        let hidden = packet
            .expansion_hints
            .len()
            .saturating_sub(6)
            .saturating_add(packet.omitted_hints);
        if hidden > 0 {
            sections.push(format!(
                "## Expansion Hints\n{}\n- `...` {} additional hints omitted",
                hints, hidden
            ));
        } else {
            sections.push(format!("## Expansion Hints\n{hints}"));
        }
    } else if packet.omitted_hints > 0 {
        sections.push(format!(
            "## Expansion Hints\n- `...` {} additional hints omitted",
            packet.omitted_hints
        ));
    }

    sections.join("\n\n")
}

fn emit_report(report: &tokenln::ir::DeviationReport, target: EmitterTarget) -> String {
    match target {
        EmitterTarget::Generic => GenericEmitter.emit(report),
        EmitterTarget::Claude => ClaudeEmitter.emit(report),
        EmitterTarget::Ollama => OllamaEmitter.emit(report),
        EmitterTarget::Codex => CodexEmitter.emit(report),
        EmitterTarget::Copilot => CopilotEmitter.emit(report),
    }
}

fn run_proxy(proxy: ProxyCommand) -> Result<i32, String> {
    match proxy.command {
        ProxySubcommand::Run(args) => run_proxy_run(args),
        ProxySubcommand::Install(args) => run_proxy_install(args),
        ProxySubcommand::Last(args) => run_proxy_last(args),
    }
}

fn run_proxy_run(args: ProxyRunArgs) -> Result<i32, String> {
    let (program, command_args) = args
        .command
        .split_first()
        .ok_or_else(|| "proxy command requires at least one argument".to_string())?;

    if args.from_file.is_none() && !args.allow_broad {
        let current_dir = env::current_dir()
            .map_err(|err| format!("failed to resolve current directory: {err}"))?;
        let policy = load_policy_for_working_dir(&current_dir)
            .map_err(|err| format!("tokenln policy: {err}"))?;
        if let Some(violation) = evaluate_proxy_command_with_policy(&policy, program, command_args)
        {
            eprintln!("{}", violation.render());
            return Ok(2);
        }
    }

    let route = proxy::classify_command(program, command_args);
    let run =
        if let Some(path) = args.from_file.as_ref() {
            match route {
                proxy::ProxyRoute::Analyze(_) => read_output_from_file(path)?,
                proxy::ProxyRoute::Passthrough => return Err(
                    "proxy --from-file only supports cargo test/build, go test, pytest, and jest"
                        .to_string(),
                ),
            }
        } else {
            execute_command(program, command_args)?
        };

    match route {
        proxy::ProxyRoute::Analyze(frontend) => {
            let raw_output = run.combined_output();
            let report = compile_report(frontend, &raw_output);
            let artifacts = if args.no_artifacts {
                None
            } else {
                Some(persist_proxy_artifacts(
                    frontend,
                    program,
                    command_args,
                    run.exit_code.unwrap_or(0),
                    &raw_output,
                    &report,
                    &args.artifacts_dir,
                )?)
            };

            if args.emit_ir {
                let output = serde_json::to_string_pretty(&report)
                    .map_err(|err| format!("failed to render IR: {err}"))?;
                println!("{output}");
            } else {
                let delta_output = if args.no_delta {
                    None
                } else {
                    artifacts.as_ref().and_then(|artifacts| {
                        render_delta_for_run(
                            &artifacts.run_dir,
                            &args.artifacts_dir,
                            &report.source,
                            args.target,
                        )
                    })
                };

                if let Some(output) = delta_output {
                    println!("{output}");
                } else if report.deviations.is_empty() {
                    print_success_output(frontend, &run, args.success_output);
                } else {
                    println!("{}", emit_report(&report, args.target));
                }
            }

            if !args.emit_ir {
                print_artifact_hint(artifacts.as_ref());
            }
        }
        proxy::ProxyRoute::Passthrough => {
            if args.emit_ir {
                return Err(
                    "proxy --emit-ir only supports cargo test/build, go test, pytest, and jest"
                        .to_string(),
                );
            }
            print_passthrough(&run);
        }
    }

    Ok(run.exit_code.unwrap_or(0))
}

fn render_delta_for_run(
    run_dir: &Path,
    artifacts_dir: &Path,
    source: &str,
    target: EmitterTarget,
) -> Option<String> {
    let previous_run_dir =
        resolve_previous_run_dir_for_source(run_dir, artifacts_dir, source).ok()?;
    let current = load_run_artifacts(run_dir).ok()?;
    let previous = load_run_artifacts(&previous_run_dir).ok()?;
    if previous.report.source != current.report.source {
        return None;
    }

    let comparison = compare_runs(&current, &previous);
    if comparison.current_total == 0 && comparison.previous_total == 0 {
        return None;
    }

    Some(render_compare(&comparison, target))
}

fn run_proxy_install(args: ProxyInstallArgs) -> Result<i32, String> {
    fs::create_dir_all(&args.dir).map_err(|err| {
        format!(
            "failed to create proxy directory '{}': {err}",
            args.dir.display()
        )
    })?;

    let tokenln_bin = match args.tokenln_bin {
        Some(path) => path,
        None => env::current_exe()
            .map_err(|err| format!("failed to resolve current tokenln binary path: {err}"))?,
    };

    let tokenln_bin = tokenln_bin.canonicalize().map_err(|err| {
        format!(
            "failed to canonicalize tokenln binary path '{}': {err}",
            tokenln_bin.display()
        )
    })?;

    let mut installed = Vec::new();
    let mut skipped = Vec::new();

    for command_name in proxy::supported_wrapper_commands() {
        let Some(real_program) = proxy::resolve_real_program(command_name, &args.dir) else {
            skipped.push(format!("{command_name} (not found on PATH)"));
            continue;
        };

        let wrapper_path = args.dir.join(command_name);
        if wrapper_path.exists() && !args.force {
            return Err(format!(
                "refusing to overwrite existing wrapper '{}'; rerun with --force",
                wrapper_path.display()
            ));
        }

        let script =
            proxy::render_wrapper_script(&tokenln_bin, args.target.as_cli_value(), &real_program);
        fs::write(&wrapper_path, script).map_err(|err| {
            format!(
                "failed to write wrapper script '{}': {err}",
                wrapper_path.display()
            )
        })?;

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let permissions = fs::Permissions::from_mode(0o755);
            fs::set_permissions(&wrapper_path, permissions).map_err(|err| {
                format!(
                    "failed to mark wrapper '{}' executable: {err}",
                    wrapper_path.display()
                )
            })?;
        }

        installed.push(wrapper_path);
    }

    if installed.is_empty() {
        return Err(format!(
            "no wrappers were installed in '{}'; ensure at least one of cargo/go/pytest/jest exists on PATH",
            args.dir.display()
        ));
    }

    println!(
        "Installed {} proxy wrappers in {}",
        installed.len(),
        args.dir.display()
    );
    for wrapper in installed {
        println!("  - {}", wrapper.display());
    }
    if !skipped.is_empty() {
        println!("Skipped missing commands:");
        for item in skipped {
            println!("  - {item}");
        }
    }
    println!("Add to PATH before launching your agent:");
    println!("  export PATH=\"{}:$PATH\"", args.dir.display());

    Ok(0)
}

fn run_proxy_last(args: ProxyLastArgs) -> Result<i32, String> {
    let run_dir = resolve_latest_run_dir(&args.dir)?;
    if args.raw {
        let raw_path = run_dir.join("raw_output.txt");
        let raw = fs::read_to_string(&raw_path)
            .map_err(|err| format!("failed to read raw output '{}': {err}", raw_path.display()))?;
        print!("{raw}");
    } else if args.ir {
        let ir_path = run_dir.join("report.ir.json");
        let ir = fs::read_to_string(&ir_path)
            .map_err(|err| format!("failed to read IR report '{}': {err}", ir_path.display()))?;
        print!("{ir}");
    } else {
        println!("{}", run_dir.display());
    }

    Ok(0)
}

fn run_query(args: QueryArgs) -> Result<i32, String> {
    let run_dir = resolve_query_run_dir(args.run.as_ref(), &args.dir)?;
    let loaded = load_run_artifacts(&run_dir)?;
    let previous_signatures =
        previous_run_signatures(&loaded.run_dir, &args.dir, &loaded.report.source)?;
    let fix_log_path = args.dir.parent().unwrap_or(&args.dir).join("fix_log.jsonl");
    let fixed_signatures = load_fix_signatures(&fix_log_path);

    let packet = build_context_packet(BuildPacketOptions {
        run_id: &loaded.run_id,
        source: &loaded.report.source,
        objective: &args.objective,
        budget_tokens: args.budget,
        report: &loaded.report,
        raw_output: &loaded.raw_output,
        report_artifact: &loaded.report_artifact,
        previous_signatures: &previous_signatures,
        fixed_signatures: &fixed_signatures,
    });

    if args.emit_json {
        let json = serde_json::to_string_pretty(&packet)
            .map_err(|err| format!("failed to render context packet: {err}"))?;
        println!("{json}");
    } else {
        println!("{}", render_context_packet(&packet, args.target));
    }

    Ok(0)
}

fn run_expand(args: ExpandArgs) -> Result<i32, String> {
    let run_dir = resolve_query_run_dir(args.run.as_ref(), &args.dir)?;
    let loaded = load_run_artifacts(&run_dir)?;
    let previous_signatures =
        previous_run_signatures(&loaded.run_dir, &args.dir, &loaded.report.source)?;
    let fix_log_path = args.dir.parent().unwrap_or(&args.dir).join("fix_log.jsonl");
    let fixed_signatures = load_fix_signatures(&fix_log_path);

    let packet = build_context_packet(BuildPacketOptions {
        run_id: &loaded.run_id,
        source: &loaded.report.source,
        objective: &args.objective,
        budget_tokens: u32::MAX / 4,
        report: &loaded.report,
        raw_output: &loaded.raw_output,
        report_artifact: &loaded.report_artifact,
        previous_signatures: &previous_signatures,
        fixed_signatures: &fixed_signatures,
    });

    let deviation_index = parse_deviation_id(&args.deviation_id).ok_or_else(|| {
        format!(
            "invalid deviation id '{}'; expected formats: d1, d2, ...",
            args.deviation_id
        )
    })?;

    if deviation_index >= loaded.report.deviations.len() {
        return Err(format!(
            "deviation '{}' not found in run '{}' (available: d1..d{})",
            args.deviation_id,
            loaded.run_id,
            loaded.report.deviations.len()
        ));
    }

    let deviation = &loaded.report.deviations[deviation_index];
    let target_id = format!("d{}", deviation_index + 1);
    let packet_slice = packet
        .deviations
        .iter()
        .find(|slice| slice.id == target_id)
        .ok_or_else(|| format!("deviation '{}' missing from packet assembly", target_id))?;

    let raw_excerpt = match args.view {
        ExpandView::Trace => None,
        ExpandView::Evidence | ExpandView::Full => Some(compose_raw_excerpt(
            &packet_slice.evidence_refs,
            &loaded.raw_output,
            &loaded.report_artifact,
        )),
    };

    let mut result = ExpansionResult {
        run_id: loaded.run_id,
        deviation_id: target_id,
        view: expand_view_label(args.view).to_string(),
        budget_tokens: args.budget,
        used_tokens: 0,
        summary: deviation.summary.clone(),
        expected: deviation.expected.description.clone(),
        actual: deviation.actual.description.clone(),
        location: format_deviation_location(deviation),
        confidence: deviation.confidence,
        confidence_reasons: deviation.confidence_reasons.clone(),
        trace: if matches!(args.view, ExpandView::Evidence) {
            Vec::new()
        } else {
            deviation.trace.frames.clone()
        },
        evidence_refs: if matches!(args.view, ExpandView::Trace) {
            Vec::new()
        } else {
            packet_slice.evidence_refs.clone()
        },
        raw_excerpt,
    };

    enforce_expansion_budget(&mut result);

    if args.emit_json {
        let json = serde_json::to_string_pretty(&result)
            .map_err(|err| format!("failed to render expansion JSON: {err}"))?;
        println!("{json}");
    } else {
        println!("{}", render_expansion(&result, args.target));
    }

    Ok(0)
}

fn run_compare(args: CompareArgs) -> Result<i32, String> {
    let _latest_selected = args.latest || args.run.is_none();
    let _previous_selected = args.previous || args.previous_run.is_none();

    let run_dir = match args.run.as_ref() {
        Some(run) => resolve_query_run_dir(Some(run), &args.dir)?,
        None => resolve_latest_run_dir(&args.dir)?,
    };
    let current = load_run_artifacts(&run_dir)?;
    let previous_run_dir = match args.previous_run.as_ref() {
        Some(run) => resolve_query_run_dir(Some(run), &args.dir)?,
        None => resolve_previous_run_dir_for_source(&run_dir, &args.dir, &current.report.source)?,
    };

    let current_canonical = run_dir.canonicalize().unwrap_or_else(|_| run_dir.clone());
    let previous_canonical = previous_run_dir
        .canonicalize()
        .unwrap_or_else(|_| previous_run_dir.clone());
    if current_canonical == previous_canonical {
        return Err("compare requires two distinct run directories".to_string());
    }

    let previous = load_run_artifacts(&previous_run_dir)?;
    if current.report.source != previous.report.source {
        return Err(format!(
            "cannot compare runs with different sources ('{}' vs '{}')",
            current.report.source, previous.report.source
        ));
    }

    let comparison = compare_runs(&current, &previous);

    if args.emit_json {
        let json = serde_json::to_string_pretty(&comparison)
            .map_err(|err| format!("failed to render compare JSON: {err}"))?;
        println!("{json}");
    } else {
        println!("{}", render_compare(&comparison, args.target));
    }

    Ok(0)
}

fn run_serve(args: ServeArgs) -> Result<i32, String> {
    tokenln::mcp::run_mcp_server(args.dir, args.fix_log, args.repo_root)
}

fn run_fixed(args: FixedArgs) -> Result<i32, String> {
    use tokenln::context::deviation_signature;
    use tokenln::fixlog::{record_fix, FixLogEntry};

    let loaded = load_run_artifacts(&resolve_latest_run_dir(&args.dir)?)?;
    let deviation_index = parse_deviation_id(&args.deviation_id).ok_or_else(|| {
        format!(
            "invalid deviation id '{}'; expected formats: d1, d2, ...",
            args.deviation_id
        )
    })?;

    if deviation_index >= loaded.report.deviations.len() {
        return Err(format!(
            "deviation '{}' not found in run '{}' (available: d1..d{})",
            args.deviation_id,
            loaded.run_id,
            loaded.report.deviations.len()
        ));
    }

    let deviation = &loaded.report.deviations[deviation_index];
    let signature = deviation_signature(deviation);
    let entry = FixLogEntry::new(&signature, &loaded.report.source, &loaded.run_id, args.note);
    record_fix(&args.fix_log, &entry)?;

    println!(
        "Recorded fix: deviation '{}' from run '{}'",
        args.deviation_id, loaded.run_id
    );
    println!("Signature: {signature}");
    println!("Fix log:   {}", args.fix_log.display());
    println!(
        "Future `tokenln query` results will deprioritize this deviation (novelty_score = 0.10)."
    );

    Ok(0)
}

/// Parsed fields from a run's `meta.txt` file.
struct RunMeta {
    frontend_label: String,
    command: String,
}

fn parse_run_meta(run_dir: &Path) -> Result<RunMeta, String> {
    let meta_path = run_dir.join("meta.txt");
    let text = fs::read_to_string(&meta_path).map_err(|err| {
        format!(
            "failed to read meta file '{}': {err}",
            meta_path.display()
        )
    })?;

    let mut frontend_label = String::new();
    let mut command = String::new();

    for line in text.lines() {
        if let Some(value) = line.strip_prefix("frontend:") {
            frontend_label = value.trim().to_string();
        } else if let Some(value) = line.strip_prefix("command:") {
            command = value.trim().to_string();
        }
    }

    if command.is_empty() {
        return Err(format!(
            "meta file '{}' has no command field",
            meta_path.display()
        ));
    }

    Ok(RunMeta {
        frontend_label,
        command,
    })
}

fn frontend_from_label(label: &str) -> Option<proxy::FrontendKind> {
    match label {
        "cargo_test" => Some(proxy::FrontendKind::CargoTest),
        "cargo_build" => Some(proxy::FrontendKind::CargoBuild),
        "go_test" => Some(proxy::FrontendKind::GoTest),
        "pytest" => Some(proxy::FrontendKind::Pytest),
        "jest" => Some(proxy::FrontendKind::Jest),
        _ => None,
    }
}

fn run_replay(args: ReplayArgs) -> Result<i32, String> {
    let run_dir = resolve_query_run_dir(args.run.as_ref(), &args.dir)?;
    let loaded = load_run_artifacts(&run_dir)?;
    let meta = parse_run_meta(&run_dir)?;

    let frontend = frontend_from_label(&meta.frontend_label).ok_or_else(|| {
        format!(
            "unrecognised frontend '{}' in meta.txt; cannot replay",
            meta.frontend_label
        )
    })?;

    // Split stored command into program + args by whitespace.
    // This mirrors how format_command() stored it: `{program} {args.join(" ")}`.
    let parts = meta.command.split_whitespace().collect::<Vec<_>>();
    let (program, cmd_args) = parts
        .split_first()
        .ok_or_else(|| "meta.txt command is empty".to_string())?;
    let cmd_args = cmd_args
        .iter()
        .map(|s| s.to_string())
        .collect::<Vec<_>>();

    let run = execute_command(program, &cmd_args)?;
    let raw_output = run.combined_output();
    let new_report = compile_report(frontend, &raw_output);

    // Build signature sets for comparison.
    let original_signatures: std::collections::HashSet<String> = loaded
        .report
        .deviations
        .iter()
        .map(deviation_signature)
        .collect();

    let new_signatures: std::collections::HashSet<String> = new_report
        .deviations
        .iter()
        .map(deviation_signature)
        .collect();

    // Classify each original deviation.
    let mut fixed = Vec::new();
    let mut still_failing = Vec::new();
    for (idx, deviation) in loaded.report.deviations.iter().enumerate() {
        let sig = deviation_signature(deviation);
        let entry = ReplayDeviation {
            id: format!("d{}", idx + 1),
            summary: deviation.summary.clone(),
            location: format_deviation_location(deviation),
            confidence: deviation.confidence,
        };
        if new_signatures.contains(&sig) {
            still_failing.push(entry);
        } else {
            fixed.push(entry);
        }
    }

    // Detect new deviations (regressions introduced since the original run).
    let new: Vec<ReplayDeviation> = new_report
        .deviations
        .iter()
        .enumerate()
        .filter(|(_, d)| !original_signatures.contains(&deviation_signature(d)))
        .map(|(idx, d)| ReplayDeviation {
            id: format!("new-d{}", idx + 1),
            summary: d.summary.clone(),
            location: format_deviation_location(d),
            confidence: d.confidence,
        })
        .collect();

    let verdict = if !fixed.is_empty() && still_failing.is_empty() && new.is_empty() {
        "all_fixed"
    } else if !fixed.is_empty() && new.is_empty() {
        "partial_fix"
    } else if !fixed.is_empty() && !new.is_empty() {
        "fixed_with_regression"
    } else if fixed.is_empty() && !new.is_empty() {
        "regression"
    } else {
        "no_change"
    };

    let result = ReplayResult {
        run_id: loaded.run_id,
        source: loaded.report.source,
        command: meta.command,
        original_count: loaded.report.deviations.len(),
        fixed_count: fixed.len(),
        still_failing_count: still_failing.len(),
        new_count: new.len(),
        verdict: verdict.to_string(),
        fixed,
        still_failing,
        new,
    };

    if args.emit_json {
        let json = serde_json::to_string_pretty(&result)
            .map_err(|err| format!("failed to render replay JSON: {err}"))?;
        println!("{json}");
    } else {
        println!("{}", render_replay(&result, args.target));
    }

    // Exit 0 if all fixed or no deviations, 1 if any remain.
    Ok(if result.still_failing_count == 0 && result.new_count == 0 {
        0
    } else {
        1
    })
}

fn render_replay(result: &ReplayResult, target: EmitterTarget) -> String {
    match target {
        EmitterTarget::Generic => render_replay_plain(result),
        EmitterTarget::Claude
        | EmitterTarget::Codex
        | EmitterTarget::Copilot
        | EmitterTarget::Ollama => render_replay_markdown(result),
    }
}

fn render_replay_plain(result: &ReplayResult) -> String {
    let mut lines = Vec::new();
    lines.push(format!(
        "REPLAY run={} source={} verdict={}",
        result.run_id, result.source, result.verdict
    ));
    lines.push(format!("command: {}", result.command));
    lines.push(format!(
        "original={} fixed={} still_failing={} new={}",
        result.original_count, result.fixed_count, result.still_failing_count, result.new_count
    ));

    if !result.fixed.is_empty() {
        lines.push("fixed:".to_string());
        for d in &result.fixed {
            lines.push(format!(
                "  + {} {} ({}) conf={:.2}",
                d.id, d.summary, d.location, d.confidence
            ));
        }
    }
    if !result.still_failing.is_empty() {
        lines.push("still_failing:".to_string());
        for d in &result.still_failing {
            lines.push(format!(
                "  ! {} {} ({}) conf={:.2}",
                d.id, d.summary, d.location, d.confidence
            ));
        }
    }
    if !result.new.is_empty() {
        lines.push("new (regression):".to_string());
        for d in &result.new {
            lines.push(format!(
                "  ~ {} {} ({}) conf={:.2}",
                d.id, d.summary, d.location, d.confidence
            ));
        }
    }

    lines.join("\n")
}

fn render_replay_markdown(result: &ReplayResult) -> String {
    let mut sections = Vec::new();
    sections.push("# TokenLn Replay".to_string());
    sections.push(format!(
        "Run: `{}`  \nSource: `{}`  \nCommand: `{}`  \nVerdict: **{}**",
        result.run_id, result.source, result.command, result.verdict
    ));
    sections.push(format!(
        "Original: `{}`  \nFixed: `{}`  \nStill failing: `{}`  \nNew: `{}`",
        result.original_count, result.fixed_count, result.still_failing_count, result.new_count
    ));

    if !result.fixed.is_empty() {
        let rows = result
            .fixed
            .iter()
            .map(|d| format!("- ✓ `{}` {} (`{}`) conf `{:.2}`", d.id, d.summary, d.location, d.confidence))
            .collect::<Vec<_>>()
            .join("\n");
        sections.push(format!("## Fixed\n{rows}"));
    }
    if !result.still_failing.is_empty() {
        let rows = result
            .still_failing
            .iter()
            .map(|d| format!("- ✗ `{}` {} (`{}`) conf `{:.2}`", d.id, d.summary, d.location, d.confidence))
            .collect::<Vec<_>>()
            .join("\n");
        sections.push(format!("## Still Failing\n{rows}"));
    }
    if !result.new.is_empty() {
        let rows = result
            .new
            .iter()
            .map(|d| format!("- ~ `{}` {} (`{}`) conf `{:.2}`", d.id, d.summary, d.location, d.confidence))
            .collect::<Vec<_>>()
            .join("\n");
        sections.push(format!("## New (Regression)\n{rows}"));
    }

    sections.join("\n\n")
}

fn compare_runs(current: &LoadedRunArtifacts, previous: &LoadedRunArtifacts) -> CompareResult {
    let mut current_map = HashMap::new();
    for (idx, deviation) in current.report.deviations.iter().enumerate() {
        let signature = deviation_signature(deviation);
        current_map
            .entry(signature)
            .or_insert_with(|| (format!("d{}", idx + 1), deviation));
    }

    let mut previous_map = HashMap::new();
    for (idx, deviation) in previous.report.deviations.iter().enumerate() {
        let signature = deviation_signature(deviation);
        previous_map
            .entry(signature)
            .or_insert_with(|| (format!("d{}", idx + 1), deviation));
    }

    let mut new = current_map
        .iter()
        .filter(|(signature, _)| !previous_map.contains_key(*signature))
        .map(|(_, (id, deviation))| compared_deviation(id, deviation))
        .collect::<Vec<_>>();
    sort_compared_deviations(&mut new);

    let mut resolved = previous_map
        .iter()
        .filter(|(signature, _)| !current_map.contains_key(*signature))
        .map(|(_, (id, deviation))| compared_deviation(id, deviation))
        .collect::<Vec<_>>();
    sort_compared_deviations(&mut resolved);

    let mut persistent = current_map
        .iter()
        .filter_map(|(signature, (current_id, current_deviation))| {
            let (previous_id, previous_deviation) = previous_map.get(signature)?;
            Some(PersistentDeviation {
                current_id: current_id.clone(),
                previous_id: previous_id.clone(),
                summary: current_deviation.summary.clone(),
                location: format_deviation_location(current_deviation),
                confidence_current: current_deviation.confidence,
                confidence_previous: previous_deviation.confidence,
                confidence_delta: round2(
                    current_deviation.confidence - previous_deviation.confidence,
                ),
            })
        })
        .collect::<Vec<_>>();
    persistent.sort_by(|left, right| {
        compare_deviation_id(&left.current_id, &right.current_id)
            .then_with(|| left.summary.cmp(&right.summary))
    });

    CompareResult {
        current_run_id: current.run_id.clone(),
        previous_run_id: previous.run_id.clone(),
        source: current.report.source.clone(),
        current_total: current.report.deviations.len(),
        previous_total: previous.report.deviations.len(),
        new_count: new.len(),
        resolved_count: resolved.len(),
        persistent_count: persistent.len(),
        new,
        resolved,
        persistent,
    }
}

fn compared_deviation(id: &str, deviation: &tokenln::ir::Deviation) -> ComparedDeviation {
    ComparedDeviation {
        id: id.to_string(),
        summary: deviation.summary.clone(),
        expected: deviation.expected.description.clone(),
        actual: deviation.actual.description.clone(),
        location: format_deviation_location(deviation),
        confidence: deviation.confidence,
    }
}

fn sort_compared_deviations(entries: &mut [ComparedDeviation]) {
    entries.sort_by(|left, right| {
        compare_deviation_id(&left.id, &right.id).then_with(|| left.summary.cmp(&right.summary))
    });
}

fn compare_deviation_id(left: &str, right: &str) -> std::cmp::Ordering {
    let left_idx = parse_deviation_id(left).unwrap_or(usize::MAX);
    let right_idx = parse_deviation_id(right).unwrap_or(usize::MAX);
    left_idx.cmp(&right_idx).then_with(|| left.cmp(right))
}

fn round2(value: f32) -> f32 {
    (value * 100.0).round() / 100.0
}

fn render_compare(result: &CompareResult, target: EmitterTarget) -> String {
    match target {
        EmitterTarget::Generic => render_compare_plain(result),
        EmitterTarget::Claude
        | EmitterTarget::Codex
        | EmitterTarget::Copilot
        | EmitterTarget::Ollama => render_compare_markdown(result),
    }
}

fn render_compare_plain(result: &CompareResult) -> String {
    let mut lines = Vec::new();
    lines.push(format!(
        "RUN_COMPARE current={} previous={} source={}",
        result.current_run_id, result.previous_run_id, result.source
    ));
    lines.push(format!(
        "totals: current={} previous={} new={} resolved={} persistent={}",
        result.current_total,
        result.previous_total,
        result.new_count,
        result.resolved_count,
        result.persistent_count
    ));

    lines.push("new:".to_string());
    if result.new.is_empty() {
        lines.push("  - none".to_string());
    } else {
        for deviation in &result.new {
            lines.push(format!(
                "  - {} {} ({}) conf={:.2}",
                deviation.id, deviation.summary, deviation.location, deviation.confidence
            ));
        }
    }

    lines.push("resolved:".to_string());
    if result.resolved.is_empty() {
        lines.push("  - none".to_string());
    } else {
        for deviation in &result.resolved {
            lines.push(format!(
                "  - {} {} ({}) conf={:.2}",
                deviation.id, deviation.summary, deviation.location, deviation.confidence
            ));
        }
    }

    lines.push("persistent:".to_string());
    if result.persistent.is_empty() {
        lines.push("  - none".to_string());
    } else {
        for deviation in &result.persistent {
            lines.push(format!(
                "  - {} (prev {}) {} ({}) delta={:+.2}",
                deviation.current_id,
                deviation.previous_id,
                deviation.summary,
                deviation.location,
                deviation.confidence_delta
            ));
        }
    }

    lines.join("\n")
}

fn render_compare_markdown(result: &CompareResult) -> String {
    let mut sections = Vec::new();
    sections.push("# TokenLn Run Comparison".to_string());
    sections.push(format!(
        "Current: `{}`  \nPrevious: `{}`  \nSource: `{}`",
        result.current_run_id, result.previous_run_id, result.source
    ));
    sections.push(format!(
        "Current deviations: `{}`  \nPrevious deviations: `{}`  \nNew: `{}`  \nResolved: `{}`  \nPersistent: `{}`",
        result.current_total,
        result.previous_total,
        result.new_count,
        result.resolved_count,
        result.persistent_count
    ));

    sections.push(render_compare_section_markdown("New", &result.new));
    sections.push(render_compare_section_markdown(
        "Resolved",
        &result.resolved,
    ));
    sections.push(render_compare_persistent_markdown(&result.persistent));

    sections.join("\n\n")
}

fn render_compare_section_markdown(title: &str, entries: &[ComparedDeviation]) -> String {
    if entries.is_empty() {
        return format!("## {title}\n- none");
    }

    let rows = entries
        .iter()
        .map(|deviation| {
            format!(
                "- `{}` {} ({}) conf `{:.2}`",
                deviation.id, deviation.summary, deviation.location, deviation.confidence
            )
        })
        .collect::<Vec<_>>()
        .join("\n");
    format!("## {title}\n{rows}")
}

fn render_compare_persistent_markdown(entries: &[PersistentDeviation]) -> String {
    if entries.is_empty() {
        return "## Persistent\n- none".to_string();
    }

    let rows = entries
        .iter()
        .map(|deviation| {
            format!(
                "- `{}` (prev `{}`) {} ({}) conf delta `{:+.2}`",
                deviation.current_id,
                deviation.previous_id,
                deviation.summary,
                deviation.location,
                deviation.confidence_delta
            )
        })
        .collect::<Vec<_>>()
        .join("\n");
    format!("## Persistent\n{rows}")
}

fn load_run_artifacts(run_dir: &Path) -> Result<LoadedRunArtifacts, String> {
    let report_path = run_dir.join("report.ir.json");
    let raw_path = run_dir.join("raw_output.txt");

    let report_artifact = fs::read_to_string(&report_path).map_err(|err| {
        format!(
            "failed to read IR artifact '{}': {err}",
            report_path.display()
        )
    })?;
    let report: tokenln::ir::DeviationReport =
        serde_json::from_str(&report_artifact).map_err(|err| {
            format!(
                "failed to parse IR artifact '{}': {err}",
                report_path.display()
            )
        })?;
    let raw_output = fs::read_to_string(&raw_path).unwrap_or_default();
    let run_id = run_dir
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("run")
        .to_string();

    Ok(LoadedRunArtifacts {
        run_dir: run_dir.to_path_buf(),
        run_id,
        report,
        report_artifact,
        raw_output,
    })
}

fn parse_deviation_id(input: &str) -> Option<usize> {
    let trimmed = input.trim();
    let value = trimmed.strip_prefix('d').unwrap_or(trimmed);
    let index = value.parse::<usize>().ok()?;
    index.checked_sub(1)
}

fn expand_view_label(view: ExpandView) -> &'static str {
    match view {
        ExpandView::Evidence => "evidence",
        ExpandView::Trace => "trace",
        ExpandView::Full => "full",
    }
}

fn compose_raw_excerpt(
    evidence_refs: &[EvidenceRef],
    raw_output: &str,
    report_artifact: &str,
) -> String {
    let mut sections = Vec::new();

    for evidence in evidence_refs {
        let content = match evidence.artifact.as_str() {
            "raw_output.txt" => {
                extract_line_window(raw_output, evidence.line_start, evidence.line_end)
            }
            "report.ir.json" => {
                extract_line_window(report_artifact, evidence.line_start, evidence.line_end)
            }
            _ => None,
        };

        if let Some(content) = content {
            sections.push(format!(
                "artifact: {} [{}:{}]\n{}",
                evidence.artifact, evidence.line_start, evidence.line_end, content
            ));
        }
    }

    if sections.is_empty() {
        "no evidence excerpts available".to_string()
    } else {
        sections.join("\n\n")
    }
}

fn extract_line_window(text: &str, start: u32, end: u32) -> Option<String> {
    if start == 0 || end == 0 || end < start {
        return None;
    }

    let lines = text.lines().collect::<Vec<_>>();
    if lines.is_empty() {
        return None;
    }

    let start_idx = (start as usize).saturating_sub(1);
    if start_idx >= lines.len() {
        return None;
    }
    let end_idx = (end as usize).min(lines.len());
    Some(lines[start_idx..end_idx].join("\n"))
}

fn format_deviation_location(deviation: &tokenln::ir::Deviation) -> String {
    let file = deviation.location.file.as_deref().unwrap_or("unknown");
    let line = deviation
        .location
        .line
        .map(|value| value.to_string())
        .unwrap_or_else(|| "?".to_string());
    let column = deviation
        .location
        .column
        .map(|value| value.to_string())
        .unwrap_or_else(|| "?".to_string());
    format!("{file}:{line}:{column}")
}

fn enforce_expansion_budget(result: &mut ExpansionResult) {
    let used = estimate_expansion_tokens(result);
    if used <= result.budget_tokens {
        result.used_tokens = used;
        return;
    }

    // Drop heavy sections first while preserving actionable skeleton.
    if !result.raw_excerpt.as_deref().unwrap_or("").is_empty() {
        result.raw_excerpt = Some(truncate_for_budget(
            result.raw_excerpt.as_deref().unwrap_or_default(),
            result.budget_tokens / 2,
        ));
    }
    if estimate_expansion_tokens(result) > result.budget_tokens {
        result.evidence_refs.clear();
    }
    if estimate_expansion_tokens(result) > result.budget_tokens {
        result.trace.truncate(4);
    }
    if estimate_expansion_tokens(result) > result.budget_tokens {
        result.confidence_reasons.truncate(4);
    }

    result.used_tokens = estimate_expansion_tokens(result).min(result.budget_tokens);
}

fn estimate_expansion_tokens(result: &ExpansionResult) -> u32 {
    let mut text = String::new();
    text.push_str(&result.summary);
    text.push_str(&result.expected);
    text.push_str(&result.actual);
    text.push_str(&result.location);
    text.push_str(&result.trace.join(" "));
    text.push_str(&result.confidence_reasons.join(" "));
    if let Some(raw) = result.raw_excerpt.as_ref() {
        text.push_str(raw);
    }
    let base = estimate_text_tokens(&text);
    let evidence_cost = (result.evidence_refs.len() as u32) * 8;
    base + evidence_cost + 16
}

fn estimate_text_tokens(text: &str) -> u32 {
    let chars = text.chars().count() as u32;
    chars.div_ceil(4).max(1)
}

fn truncate_for_budget(text: &str, budget_tokens: u32) -> String {
    let max_chars = (budget_tokens.saturating_mul(4) as usize).max(40);
    if text.chars().count() <= max_chars {
        return text.to_string();
    }

    let truncated = text
        .chars()
        .take(max_chars.saturating_sub(32))
        .collect::<String>();
    format!("{truncated}\n...[truncated; increase --budget to expand]")
}

fn render_expansion(result: &ExpansionResult, target: EmitterTarget) -> String {
    match target {
        EmitterTarget::Generic => render_expansion_plain(result),
        EmitterTarget::Claude
        | EmitterTarget::Codex
        | EmitterTarget::Copilot
        | EmitterTarget::Ollama => render_expansion_markdown(result),
    }
}

fn render_expansion_plain(result: &ExpansionResult) -> String {
    let mut lines = Vec::new();
    lines.push(format!(
        "EXPANSION run={} deviation={} view={} budget={} used={}",
        result.run_id, result.deviation_id, result.view, result.budget_tokens, result.used_tokens
    ));
    lines.push(format!("summary: {}", result.summary));
    lines.push(format!("expected: {}", result.expected));
    lines.push(format!("actual:   {}", result.actual));
    lines.push(format!("location: {}", result.location));
    lines.push(format!("confidence: {:.2}", result.confidence));
    if !result.trace.is_empty() {
        lines.push(format!("trace: {}", result.trace.join(" -> ")));
    }
    if !result.confidence_reasons.is_empty() {
        lines.push(format!(
            "confidence_reasons: {}",
            result.confidence_reasons.join(", ")
        ));
    }
    if !result.evidence_refs.is_empty() {
        lines.push("evidence_refs:".to_string());
        for evidence in &result.evidence_refs {
            lines.push(format!(
                "  - {} [{}:{}] hash={}",
                evidence.artifact, evidence.line_start, evidence.line_end, evidence.hash
            ));
        }
    }
    if let Some(raw_excerpt) = result.raw_excerpt.as_ref() {
        lines.push("raw_excerpt:".to_string());
        lines.push(raw_excerpt.clone());
    }
    lines.join("\n")
}

fn render_expansion_markdown(result: &ExpansionResult) -> String {
    let mut sections = Vec::new();
    sections.push("# TokenLn Expansion".to_string());
    sections.push(format!(
        "Run: `{}`  \nDeviation: `{}`  \nView: `{}`  \nBudget: `{}` tokens  \nUsed: `{}` tokens",
        result.run_id, result.deviation_id, result.view, result.budget_tokens, result.used_tokens
    ));
    sections.push(format!(
        "Summary: {}\nExpected: {}\nActual: {}\nLocation: {}\nConfidence: {:.2}",
        result.summary, result.expected, result.actual, result.location, result.confidence
    ));

    if !result.trace.is_empty() {
        sections.push(format!("Trace: `{}`", result.trace.join(" -> ")));
    }
    if !result.confidence_reasons.is_empty() {
        sections.push(format!(
            "Confidence reasons: {}",
            result.confidence_reasons.join(", ")
        ));
    }
    if !result.evidence_refs.is_empty() {
        let refs = result
            .evidence_refs
            .iter()
            .map(|evidence| {
                format!(
                    "- `{}` [{}:{}] hash `{}`",
                    evidence.artifact, evidence.line_start, evidence.line_end, evidence.hash
                )
            })
            .collect::<Vec<_>>()
            .join("\n");
        sections.push(format!("## Evidence Refs\n{refs}"));
    }
    if let Some(raw_excerpt) = result.raw_excerpt.as_ref() {
        sections.push(format!("## Raw Excerpt\n```text\n{raw_excerpt}\n```"));
    }

    sections.join("\n\n")
}

fn resolve_query_run_dir(run: Option<&PathBuf>, artifacts_dir: &Path) -> Result<PathBuf, String> {
    match run {
        Some(path) => {
            if path.is_dir() {
                Ok(path.clone())
            } else {
                Err(format!(
                    "query run directory '{}' does not exist",
                    path.display()
                ))
            }
        }
        None => resolve_latest_run_dir(artifacts_dir),
    }
}

fn resolve_latest_run_dir(artifacts_dir: &Path) -> Result<PathBuf, String> {
    let latest_file = artifacts_dir.join("latest.txt");
    let latest_content = fs::read_to_string(&latest_file).map_err(|err| {
        format!(
            "failed to read latest artifact pointer '{}': {err}",
            latest_file.display()
        )
    })?;

    let latest_path = latest_content.trim();
    if latest_path.is_empty() {
        return Err(format!(
            "latest artifact pointer '{}' is empty",
            latest_file.display()
        ));
    }

    let run_dir = PathBuf::from(latest_path);
    if !run_dir.is_dir() {
        return Err(format!(
            "latest run directory '{}' does not exist",
            run_dir.display()
        ));
    }

    Ok(run_dir)
}

fn list_run_dirs(artifacts_dir: &Path) -> Result<Vec<PathBuf>, String> {
    let mut run_dirs = fs::read_dir(artifacts_dir)
        .map_err(|err| {
            format!(
                "failed to list artifacts directory '{}': {err}",
                artifacts_dir.display()
            )
        })?
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| path.is_dir())
        .filter(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.starts_with("run-"))
        })
        .collect::<Vec<_>>();
    run_dirs.sort();
    Ok(run_dirs)
}

fn previous_run_signatures(
    run_dir: &Path,
    artifacts_dir: &Path,
    source: &str,
) -> Result<HashSet<String>, String> {
    let previous_dir = match resolve_previous_run_dir_for_source(run_dir, artifacts_dir, source) {
        Ok(path) => path,
        Err(_) => return Ok(HashSet::new()),
    };

    let previous_report_path = previous_dir.join("report.ir.json");
    let previous_text = match fs::read_to_string(&previous_report_path) {
        Ok(text) => text,
        Err(_) => return Ok(HashSet::new()),
    };
    let previous_report: tokenln::ir::DeviationReport = match serde_json::from_str(&previous_text) {
        Ok(report) => report,
        Err(_) => return Ok(HashSet::new()),
    };

    Ok(previous_report
        .deviations
        .iter()
        .map(deviation_signature)
        .collect::<HashSet<_>>())
}

fn resolve_previous_run_dir_for_source(
    run_dir: &Path,
    artifacts_dir: &Path,
    source: &str,
) -> Result<PathBuf, String> {
    let run_dirs = list_run_dirs(artifacts_dir)?;
    let current = run_dir
        .canonicalize()
        .unwrap_or_else(|_| run_dir.to_path_buf());

    let current_idx = run_dirs
        .iter()
        .position(|path| path.canonicalize().unwrap_or_else(|_| path.clone()) == current)
        .ok_or_else(|| {
            format!(
                "run directory '{}' is not tracked under '{}'",
                run_dir.display(),
                artifacts_dir.display()
            )
        })?;

    for candidate in run_dirs[..current_idx].iter().rev() {
        if run_source(candidate).as_deref() == Some(source) {
            return Ok(candidate.clone());
        }
    }

    Err(format!(
        "no previous run found before '{}' for source '{}'",
        run_dir.display(),
        source
    ))
}

fn run_source(run_dir: &Path) -> Option<String> {
    let report_path = run_dir.join("report.ir.json");
    let text = fs::read_to_string(report_path).ok()?;
    let report: tokenln::ir::DeviationReport = serde_json::from_str(&text).ok()?;
    Some(report.source)
}

fn render_context_packet(packet: &ContextPacket, target: EmitterTarget) -> String {
    match target {
        EmitterTarget::Generic => render_packet_plain(packet),
        EmitterTarget::Claude
        | EmitterTarget::Codex
        | EmitterTarget::Copilot
        | EmitterTarget::Ollama => render_packet_markdown(packet),
    }
}

fn render_packet_plain(packet: &ContextPacket) -> String {
    let mut lines = Vec::new();
    lines.push(format!(
        "CONTEXT_PACKET run={} budget={} used={} source={}",
        packet.run_id, packet.budget_tokens, packet.used_tokens, packet.source
    ));
    lines.push(format!("objective: {}", packet.objective));
    lines.push(format!(
        "deviations: {}/{}",
        packet.deviations.len(),
        packet.unresolved_count
    ));

    for deviation in &packet.deviations {
        lines.push(format!(
            "[{}] {} | utility={:.2} novelty={:.2} confidence={:.2}",
            deviation.id,
            deviation.summary,
            deviation.utility_score,
            deviation.novelty_score,
            deviation.confidence
        ));
        lines.push(format!("  expected: {}", deviation.expected));
        lines.push(format!("  actual:   {}", deviation.actual));
        lines.push(format!("  location: {}", deviation.location));
    }

    if !packet.expansion_hints.is_empty() {
        lines.push("expansion_hints:".to_string());
        for hint in &packet.expansion_hints {
            lines.push(format!(
                "  - {} ({}) est_tokens={} :: {}",
                hint.deviation_id, hint.reason, hint.estimated_tokens, hint.hint
            ));
        }
    }

    lines.join("\n")
}

fn render_packet_markdown(packet: &ContextPacket) -> String {
    let mut sections = Vec::new();
    sections.push("# TokenLn Context Packet".to_string());
    sections.push(format!(
        "Run: `{}`  \nBudget: `{}` tokens  \nUsed: `{}` tokens  \nSource: `{}`  \nObjective: `{}`",
        packet.run_id, packet.budget_tokens, packet.used_tokens, packet.source, packet.objective
    ));
    sections.push(format!(
        "Included deviations: `{}` / `{}`",
        packet.deviations.len(),
        packet.unresolved_count
    ));

    for deviation in &packet.deviations {
        sections.push(format!(
            "## {} · utility {:.2} · novelty {:.2} · confidence {:.2}\n\
Summary: {}\n\
Expected: {}\n\
Actual: {}\n\
Location: {}\n\
Evidence refs: {}",
            deviation.id,
            deviation.utility_score,
            deviation.novelty_score,
            deviation.confidence,
            deviation.summary,
            deviation.expected,
            deviation.actual,
            deviation.location,
            deviation.evidence_refs.len()
        ));
    }

    if !packet.expansion_hints.is_empty() {
        let mut hints = String::from("## Expansion Hints");
        for hint in &packet.expansion_hints {
            hints.push_str(&format!(
                "\n- `{}` ({}) est `{}` tokens -> `{}`",
                hint.deviation_id, hint.reason, hint.estimated_tokens, hint.hint
            ));
        }
        sections.push(hints);
    }

    sections.join("\n\n")
}

fn compile_and_render(
    frontend: proxy::FrontendKind,
    run: &CommandRun,
    emit_ir: bool,
    target: EmitterTarget,
) -> Result<String, String> {
    let raw_output = run.combined_output();
    let report = compile_report(frontend, &raw_output);

    if emit_ir {
        serde_json::to_string_pretty(&report).map_err(|err| format!("failed to render IR: {err}"))
    } else {
        Ok(emit_report(&report, target))
    }
}

fn compile_report(frontend: proxy::FrontendKind, raw_output: &str) -> tokenln::ir::DeviationReport {
    let optimizer = BasicOptimizer;

    let mut report = match frontend {
        proxy::FrontendKind::CargoTest => {
            let lexer = CargoTestLexer;
            let parser = CargoTestParser;
            let analyzer = CargoTestAnalyzer;

            let tokens = lexer.lex(raw_output);
            let parsed = parser.parse(&tokens);
            let report = analyzer.analyze(&parsed);
            optimizer.optimize(report)
        }
        proxy::FrontendKind::CargoBuild => {
            let lexer = CargoBuildLexer;
            let parser = CargoBuildParser;
            let analyzer = CargoBuildAnalyzer;

            let tokens = lexer.lex(raw_output);
            let parsed = parser.parse(&tokens);
            let report = analyzer.analyze(&parsed);
            optimizer.optimize(report)
        }
        proxy::FrontendKind::GoTest => {
            let lexer = GoTestLexer;
            let parser = GoTestParser;
            let analyzer = GoTestAnalyzer;

            let tokens = lexer.lex(raw_output);
            let parsed = parser.parse(&tokens);
            let report = analyzer.analyze(&parsed);
            optimizer.optimize(report)
        }
        proxy::FrontendKind::Pytest => {
            let lexer = PytestLexer;
            let parser = PytestParser;
            let analyzer = PytestAnalyzer;

            let tokens = lexer.lex(raw_output);
            let parsed = parser.parse(&tokens);
            let report = analyzer.analyze(&parsed);
            optimizer.optimize(report)
        }
        proxy::FrontendKind::Jest => {
            let lexer = JestLexer;
            let parser = JestParser;
            let analyzer = JestAnalyzer;

            let tokens = lexer.lex(raw_output);
            let parsed = parser.parse(&tokens);
            let report = analyzer.analyze(&parsed);
            optimizer.optimize(report)
        }
    };

    apply_low_confidence_fallback(&mut report, raw_output);
    report
}

fn print_passthrough(run: &CommandRun) {
    if !run.stdout.is_empty() {
        print!("{}", run.stdout);
    }
    if !run.stderr.is_empty() {
        eprint!("{}", run.stderr);
    }
}

fn print_success_output(frontend: proxy::FrontendKind, run: &CommandRun, mode: SuccessOutputMode) {
    match mode {
        SuccessOutputMode::Passthrough => {
            print_passthrough(run);
            if run.stdout.is_empty() && run.stderr.is_empty() {
                println!("No deviations detected.");
            }
        }
        SuccessOutputMode::Compact => {
            // Never synthesize success text for failing commands.
            if run.exit_code.unwrap_or(0) != 0 {
                print_passthrough(run);
                return;
            }

            let raw_output = run.combined_output();
            match compact_success_output(frontend, &raw_output) {
                Some(compact) => println!("{compact}"),
                None => {
                    if run.stdout.is_empty() && run.stderr.is_empty() {
                        println!("No deviations detected.");
                    } else {
                        print_passthrough(run);
                    }
                }
            }
        }
    }
}

fn compact_success_output(frontend: proxy::FrontendKind, raw_output: &str) -> Option<String> {
    match frontend {
        proxy::FrontendKind::CargoTest => compact_cargo_test_success(raw_output),
        proxy::FrontendKind::CargoBuild => compact_cargo_build_success(raw_output),
        proxy::FrontendKind::GoTest => compact_go_test_success(raw_output),
        proxy::FrontendKind::Pytest => compact_pytest_success(raw_output),
        proxy::FrontendKind::Jest => compact_jest_success(raw_output),
    }
}

#[derive(Debug, Copy, Clone, Default)]
struct CargoResultTotals {
    passed: u64,
    failed: u64,
    ignored: u64,
    measured: u64,
    filtered: u64,
}

impl CargoResultTotals {
    fn add_assign(&mut self, other: CargoResultTotals) {
        self.passed += other.passed;
        self.failed += other.failed;
        self.ignored += other.ignored;
        self.measured += other.measured;
        self.filtered += other.filtered;
    }
}

fn compact_cargo_test_success(raw_output: &str) -> Option<String> {
    let mut running_total = 0_u64;
    let mut totals = CargoResultTotals::default();
    let mut result_lines = 0_u64;

    for line in raw_output.lines() {
        let trimmed = line.trim();
        if let Some(n) = parse_running_tests_line(trimmed) {
            running_total += n;
        }
        if let Some(result) = parse_cargo_result_line(trimmed) {
            totals.add_assign(result);
            result_lines += 1;
        }
    }

    if result_lines == 0 {
        return None;
    }

    let discovered_total = totals.passed + totals.failed + totals.ignored + totals.measured;
    let total_tests = if running_total > 0 {
        running_total
    } else {
        discovered_total
    };

    let mut lines = Vec::new();
    if total_tests > 0 {
        lines.push(format!("running {total_tests} tests"));
    }
    lines.push(format!(
        "test result: ok. {} passed; {} failed; {} ignored; {} measured; {} filtered out;",
        totals.passed, totals.failed, totals.ignored, totals.measured, totals.filtered
    ));

    if result_lines > 1 {
        lines.push(format!(
            "tokenln: no deviations detected ({result_lines} test binaries summarized)"
        ));
    } else {
        lines.push("tokenln: no deviations detected".to_string());
    }

    Some(lines.join("\n"))
}

fn compact_cargo_build_success(raw_output: &str) -> Option<String> {
    let finished_line = raw_output
        .lines()
        .rev()
        .map(str::trim)
        .find(|line| line.starts_with("Finished "));

    finished_line.map(|line| format!("{line}\ntokenln: no deviations detected"))
}

fn compact_go_test_success(raw_output: &str) -> Option<String> {
    let ok_packages = raw_output
        .lines()
        .map(str::trim_start)
        .filter(|line| line.starts_with("ok\t") || line.starts_with("ok  "))
        .count();

    if ok_packages == 0 {
        return None;
    }

    Some(format!(
        "ok\t{ok_packages} packages\n\
tokenln: no deviations detected"
    ))
}

fn compact_pytest_success(raw_output: &str) -> Option<String> {
    let summary_line = raw_output
        .lines()
        .rev()
        .map(str::trim)
        .map(|line| line.trim_matches('=').trim())
        .find(|line| {
            line.contains(" passed")
                && line.contains(" in ")
                && !line.contains(" failed")
                && !line.contains(" error")
        })?;

    Some(format!("{summary_line}\ntokenln: no deviations detected"))
}

fn compact_jest_success(raw_output: &str) -> Option<String> {
    let mut lines = Vec::new();

    if let Some(line) = find_last_prefixed_line(raw_output, "Test Suites:") {
        lines.push(line);
    }
    if let Some(line) = find_last_prefixed_line(raw_output, "Tests:") {
        lines.push(line);
    }
    if let Some(line) = find_last_prefixed_line(raw_output, "Snapshots:") {
        lines.push(line);
    }
    if let Some(line) = find_last_prefixed_line(raw_output, "Time:") {
        lines.push(line);
    }
    if let Some(line) = find_last_prefixed_line(raw_output, "Ran all test suites") {
        lines.push(line);
    }

    if lines.is_empty() {
        return None;
    }

    lines.push("tokenln: no deviations detected".to_string());
    Some(lines.join("\n"))
}

fn find_last_prefixed_line(raw_output: &str, prefix: &str) -> Option<String> {
    raw_output.lines().rev().find_map(|line| {
        let trimmed = line.trim();
        if trimmed.starts_with(prefix) {
            Some(trimmed.to_string())
        } else {
            None
        }
    })
}

fn parse_running_tests_line(line: &str) -> Option<u64> {
    let rest = line.strip_prefix("running ")?;
    let count = rest
        .strip_suffix(" tests")
        .or_else(|| rest.strip_suffix(" test"))?;
    count.trim().parse::<u64>().ok()
}

fn parse_cargo_result_line(line: &str) -> Option<CargoResultTotals> {
    if !line.contains("test result: ok.") {
        return None;
    }

    Some(CargoResultTotals {
        passed: parse_number_between(line, "ok. ", " passed;")?,
        failed: parse_number_between(line, " passed; ", " failed;")?,
        ignored: parse_number_between(line, " failed; ", " ignored;")?,
        measured: parse_number_between(line, " ignored; ", " measured;")?,
        filtered: parse_number_between(line, " measured; ", " filtered out;")?,
    })
}

fn parse_number_between(line: &str, prefix: &str, suffix: &str) -> Option<u64> {
    let start = line.find(prefix)? + prefix.len();
    let remaining = &line[start..];
    let end = remaining.find(suffix)?;
    remaining[..end].trim().parse::<u64>().ok()
}

fn persist_proxy_artifacts(
    frontend: proxy::FrontendKind,
    program: &str,
    args: &[String],
    exit_code: i32,
    raw_output: &str,
    report: &tokenln::ir::DeviationReport,
    artifacts_dir: &Path,
) -> Result<ProxyArtifacts, String> {
    fs::create_dir_all(artifacts_dir).map_err(|err| {
        format!(
            "failed to create artifacts directory '{}': {err}",
            artifacts_dir.display()
        )
    })?;

    let now_millis = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|err| format!("system clock error while creating artifacts: {err}"))?
        .as_millis();
    let run_dir = artifacts_dir.join(format!("run-{now_millis}-{}", process::id()));
    fs::create_dir_all(&run_dir).map_err(|err| {
        format!(
            "failed to create run artifact '{}': {err}",
            run_dir.display()
        )
    })?;

    let raw_path = run_dir.join("raw_output.txt");
    fs::write(&raw_path, raw_output)
        .map_err(|err| format!("failed to write raw output '{}': {err}", raw_path.display()))?;

    let ir_path = run_dir.join("report.ir.json");
    let ir_json = serde_json::to_string_pretty(report)
        .map_err(|err| format!("failed to render IR for artifacts: {err}"))?;
    fs::write(&ir_path, ir_json)
        .map_err(|err| format!("failed to write IR report '{}': {err}", ir_path.display()))?;

    let meta_path = run_dir.join("meta.txt");
    let command = format_command(program, args);
    let meta = format!(
        "frontend: {}\ncommand: {}\nexit_code: {}\ndeviations: {}\n",
        frontend_label(frontend),
        command,
        exit_code,
        report.deviations.len()
    );
    fs::write(&meta_path, meta).map_err(|err| {
        format!(
            "failed to write artifact metadata '{}': {err}",
            meta_path.display()
        )
    })?;

    let canonical_run_dir = run_dir.canonicalize().unwrap_or(run_dir);
    let latest_file = artifacts_dir.join("latest.txt");
    fs::write(&latest_file, format!("{}\n", canonical_run_dir.display())).map_err(|err| {
        format!(
            "failed to update latest artifact pointer '{}': {err}",
            latest_file.display()
        )
    })?;

    Ok(ProxyArtifacts {
        run_dir: canonical_run_dir,
    })
}

fn frontend_label(frontend: proxy::FrontendKind) -> &'static str {
    match frontend {
        proxy::FrontendKind::CargoTest => "cargo_test",
        proxy::FrontendKind::CargoBuild => "cargo_build",
        proxy::FrontendKind::GoTest => "go_test",
        proxy::FrontendKind::Pytest => "pytest",
        proxy::FrontendKind::Jest => "jest",
    }
}

fn print_artifact_hint(artifacts: Option<&ProxyArtifacts>) {
    if let Some(artifacts) = artifacts {
        println!(
            "tokenln: full-fidelity artifacts -> {}",
            artifacts.run_dir.display()
        );
    }
}

fn read_output_from_file(path: &PathBuf) -> Result<CommandRun, String> {
    let raw_output = fs::read_to_string(path)
        .map_err(|err| format!("failed to read '{}': {err}", path.display()))?;

    Ok(CommandRun {
        stdout: raw_output,
        stderr: String::new(),
        exit_code: None,
    })
}

fn execute_command(program: &str, args: &[String]) -> Result<CommandRun, String> {
    let output = Command::new(program).args(args).output().map_err(|err| {
        format!(
            "failed to execute '{}': {err}",
            format_command(program, args)
        )
    })?;

    Ok(CommandRun {
        stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
        exit_code: output.status.code(),
    })
}

fn format_command(program: &str, args: &[String]) -> String {
    if args.is_empty() {
        program.to_string()
    } else {
        format!("{} {}", program, args.join(" "))
    }
}

fn run_cargo_test(args: CargoTestArgs) -> Result<i32, String> {
    let run = if let Some(path) = args.from_file.as_ref() {
        read_output_from_file(path)?
    } else {
        execute_cargo_test(&args.passthrough)?
    };

    run_direct_command(
        proxy::FrontendKind::CargoTest,
        "cargo test",
        &[],
        run,
        args.emit_ir,
        args.target,
        args.delta,
        &args.artifacts_dir,
    )
}

fn execute_cargo_test(passthrough: &[String]) -> Result<CommandRun, String> {
    let mut args = Vec::with_capacity(1 + passthrough.len());
    args.push("test".to_string());
    args.extend_from_slice(passthrough);
    execute_command("cargo", &args)
}

fn run_cargo_build(args: CargoBuildArgs) -> Result<i32, String> {
    let run = if let Some(path) = args.from_file.as_ref() {
        read_output_from_file(path)?
    } else {
        execute_cargo_build(&args.passthrough)?
    };

    run_direct_command(
        proxy::FrontendKind::CargoBuild,
        "cargo build",
        &[],
        run,
        args.emit_ir,
        args.target,
        args.delta,
        &args.artifacts_dir,
    )
}

fn execute_cargo_build(passthrough: &[String]) -> Result<CommandRun, String> {
    let mut args = Vec::with_capacity(1 + passthrough.len());
    args.push("build".to_string());
    args.extend_from_slice(passthrough);
    execute_command("cargo", &args)
}

fn run_go_test(args: GoTestArgs) -> Result<i32, String> {
    let run = if let Some(path) = args.from_file.as_ref() {
        read_output_from_file(path)?
    } else {
        execute_go_test(&args.passthrough)?
    };

    run_direct_command(
        proxy::FrontendKind::GoTest,
        "go test",
        &[],
        run,
        args.emit_ir,
        args.target,
        args.delta,
        &args.artifacts_dir,
    )
}

fn execute_go_test(passthrough: &[String]) -> Result<CommandRun, String> {
    let mut args = Vec::with_capacity(1 + passthrough.len());
    args.push("test".to_string());
    args.extend_from_slice(passthrough);
    execute_command("go", &args)
}

fn run_pytest(args: PytestArgs) -> Result<i32, String> {
    let run = if let Some(path) = args.from_file.as_ref() {
        read_output_from_file(path)?
    } else {
        execute_pytest(&args.passthrough)?
    };

    run_direct_command(
        proxy::FrontendKind::Pytest,
        "pytest",
        &[],
        run,
        args.emit_ir,
        args.target,
        args.delta,
        &args.artifacts_dir,
    )
}

fn execute_pytest(passthrough: &[String]) -> Result<CommandRun, String> {
    execute_command("pytest", passthrough)
}

fn run_jest(args: JestArgs) -> Result<i32, String> {
    let run = if let Some(path) = args.from_file.as_ref() {
        read_output_from_file(path)?
    } else {
        execute_jest(&args.passthrough)?
    };

    run_direct_command(
        proxy::FrontendKind::Jest,
        "jest",
        &[],
        run,
        args.emit_ir,
        args.target,
        args.delta,
        &args.artifacts_dir,
    )
}

/// Shared handler for direct (non-proxy) run commands.
/// When `delta` is true, persists artifacts and shows delta from prior run.
/// Otherwise emits the full report (or IR).
#[allow(clippy::too_many_arguments)]
fn run_direct_command(
    frontend: proxy::FrontendKind,
    program: &str,
    command_args: &[String],
    run: CommandRun,
    emit_ir: bool,
    target: EmitterTarget,
    delta: bool,
    artifacts_dir: &Path,
) -> Result<i32, String> {
    let exit_code = run.exit_code.unwrap_or(0);

    if !delta {
        // Original behaviour: compile and print.
        let rendered = compile_and_render(frontend, &run, emit_ir, target)?;
        println!("{rendered}");
        return Ok(exit_code);
    }

    // Delta mode: persist artifacts then compare.
    let raw_output = run.combined_output();
    let report = compile_report(frontend, &raw_output);
    let artifacts = persist_proxy_artifacts(
        frontend,
        program,
        command_args,
        exit_code,
        &raw_output,
        &report,
        artifacts_dir,
    )?;

    if emit_ir {
        let output = serde_json::to_string_pretty(&report)
            .map_err(|err| format!("failed to render IR: {err}"))?;
        println!("{output}");
    } else {
        let delta_output =
            render_delta_for_run(&artifacts.run_dir, artifacts_dir, &report.source, target);
        if let Some(output) = delta_output {
            println!("{output}");
        } else {
            println!("{}", emit_report(&report, target));
        }
    }

    print_artifact_hint(Some(&artifacts));
    Ok(exit_code)
}

fn execute_jest(passthrough: &[String]) -> Result<CommandRun, String> {
    execute_command("jest", passthrough)
}

#[cfg(test)]
mod tests {
    use super::{
        compact_cargo_test_success, compact_jest_success, compact_pytest_success, compare_runs,
        extract_line_window, parse_cargo_result_line, parse_deviation_id, persist_proxy_artifacts,
        previous_run_signatures, resolve_previous_run_dir_for_source, LoadedRunArtifacts,
    };
    use crate::proxy::FrontendKind;
    use std::fs;
    use std::path::PathBuf;
    use tokenln::ir::{
        Behavior, Deviation, DeviationKind, DeviationReport, ExecutionTrace, Expectation, Location,
    };

    #[test]
    fn compacts_cargo_test_success_across_multiple_binaries() {
        let raw = "\
running 2 tests
..
test result: ok. 2 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out;

running 1 tests
.
test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out;
";
        let compact = compact_cargo_test_success(raw).expect("should build compact summary");
        assert!(compact.contains("running 3 tests"));
        assert!(compact.contains(
            "test result: ok. 3 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out;"
        ));
    }

    #[test]
    fn parses_cargo_result_line_counts() {
        let line = "test result: ok. 21 passed; 0 failed; 1 ignored; 0 measured; 4 filtered out;";
        let parsed = parse_cargo_result_line(line).expect("result line should parse");
        assert_eq!(parsed.passed, 21);
        assert_eq!(parsed.failed, 0);
        assert_eq!(parsed.ignored, 1);
        assert_eq!(parsed.filtered, 4);
    }

    #[test]
    fn compacts_pytest_summary_line() {
        let raw = "\
============================= test session starts ==============================
...
============================== 18 passed in 0.62s ==============================
";
        let compact = compact_pytest_success(raw).expect("pytest summary should compact");
        assert!(compact.contains("18 passed in 0.62s"));
    }

    #[test]
    fn compacts_jest_summary_lines() {
        let raw = "\
PASS  src/math.test.ts
Test Suites: 3 passed, 3 total
Tests:       18 passed, 18 total
Snapshots:   0 total
Time:        1.31 s
Ran all test suites.
";
        let compact = compact_jest_success(raw).expect("jest summary should compact");
        assert!(compact.contains("Test Suites: 3 passed, 3 total"));
        assert!(compact.contains("Tests:       18 passed, 18 total"));
    }

    #[test]
    fn writes_full_fidelity_artifacts() {
        let root = PathBuf::from("/tmp/tokenln-artifact-test");
        let _ = fs::remove_dir_all(&root);

        let report = DeviationReport::new("pytest", Vec::new());
        let artifacts = persist_proxy_artifacts(
            FrontendKind::Pytest,
            "pytest",
            &["-q".to_string()],
            0,
            "raw output",
            &report,
            &root,
        )
        .expect("artifact writing should succeed");

        assert!(artifacts.run_dir.join("raw_output.txt").exists());
        assert!(artifacts.run_dir.join("report.ir.json").exists());
        assert!(artifacts.run_dir.join("meta.txt").exists());
        assert!(root.join("latest.txt").exists());
    }

    #[test]
    fn parses_deviation_id_formats() {
        assert_eq!(parse_deviation_id("d1"), Some(0));
        assert_eq!(parse_deviation_id("2"), Some(1));
        assert_eq!(parse_deviation_id("d10"), Some(9));
        assert_eq!(parse_deviation_id("d0"), None);
        assert_eq!(parse_deviation_id("dx"), None);
    }

    #[test]
    fn extracts_line_windows_by_one_indexed_bounds() {
        let text = "a\nb\nc\nd";
        let lines = extract_line_window(text, 2, 3).expect("window should be present");
        assert_eq!(lines, "b\nc");
        assert!(extract_line_window(text, 0, 2).is_none());
        assert!(extract_line_window(text, 5, 6).is_none());
    }

    #[test]
    fn compares_runs_into_new_resolved_and_persistent_buckets() {
        let previous = loaded_run(
            "run-prev",
            DeviationReport::new(
                "pytest",
                vec![
                    sample_deviation("old only", "tests/a.py", 10, "t::a", 0.70),
                    sample_deviation("stays", "tests/b.py", 20, "t::b", 0.80),
                ],
            ),
        );
        let current = loaded_run(
            "run-cur",
            DeviationReport::new(
                "pytest",
                vec![
                    sample_deviation("stays", "tests/b.py", 20, "t::b", 0.92),
                    sample_deviation("new only", "tests/c.py", 30, "t::c", 0.88),
                ],
            ),
        );

        let compared = compare_runs(&current, &previous);

        assert_eq!(compared.new_count, 1);
        assert_eq!(compared.resolved_count, 1);
        assert_eq!(compared.persistent_count, 1);
        assert_eq!(compared.new[0].summary, "new only");
        assert_eq!(compared.resolved[0].summary, "old only");
        assert_eq!(compared.persistent[0].summary, "stays");
        assert_eq!(compared.persistent[0].confidence_delta, 0.12);
    }

    #[test]
    fn resolves_previous_run_using_matching_source() {
        let root = PathBuf::from("/tmp/tokenln-previous-source-test");
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).expect("test root should be created");

        let run_a = root.join("run-100");
        let run_b = root.join("run-200");
        let run_c = root.join("run-300");
        fs::create_dir_all(&run_a).expect("run a should be created");
        fs::create_dir_all(&run_b).expect("run b should be created");
        fs::create_dir_all(&run_c).expect("run c should be created");

        write_report(
            &run_a,
            &DeviationReport::new(
                "pytest",
                vec![sample_deviation("pytest old", "tests/a.py", 10, "a", 0.8)],
            ),
        );
        write_report(
            &run_b,
            &DeviationReport::new(
                "jest",
                vec![sample_deviation("jest mid", "src/a.test.ts", 12, "b", 0.7)],
            ),
        );
        write_report(
            &run_c,
            &DeviationReport::new(
                "pytest",
                vec![sample_deviation(
                    "pytest current",
                    "tests/c.py",
                    30,
                    "c",
                    0.9,
                )],
            ),
        );

        let previous = resolve_previous_run_dir_for_source(&run_c, &root, "pytest")
            .expect("should find prior pytest run");
        assert_eq!(
            previous.file_name().and_then(|name| name.to_str()),
            Some("run-100")
        );

        let signatures = previous_run_signatures(&run_c, &root, "pytest")
            .expect("signature set should be loaded");
        assert_eq!(signatures.len(), 1);
    }

    fn loaded_run(run_id: &str, report: DeviationReport) -> LoadedRunArtifacts {
        LoadedRunArtifacts {
            run_dir: PathBuf::from(format!("/tmp/{run_id}")),
            run_id: run_id.to_string(),
            report,
            report_artifact: String::new(),
            raw_output: String::new(),
        }
    }

    fn sample_deviation(
        summary: &str,
        file: &str,
        line: u32,
        symbol: &str,
        confidence: f32,
    ) -> Deviation {
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
                column: Some(1),
                symbol: Some(symbol.to_string()),
            },
            trace: ExecutionTrace {
                frames: vec!["pytest".to_string()],
            },
            confidence,
            confidence_reasons: vec![],
            raw_excerpt: None,
            summary: summary.to_string(),
            group_id: None,
            is_root_cause: None,
        }
    }

    fn write_report(run_dir: &PathBuf, report: &DeviationReport) {
        let path = run_dir.join("report.ir.json");
        let json = serde_json::to_string_pretty(report).expect("report should serialize");
        fs::write(path, json).expect("report should be written");
    }
}
