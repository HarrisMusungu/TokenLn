/// MCP (Model Context Protocol) server for TokenLn.
///
/// Exposes TokenLn's deviation compilation pipeline as MCP tools that LLM agents
/// can call directly — no command wrapping or PATH shim installation required.
///
/// Transport: JSON-RPC 2.0 over stdio with Content-Length framing (MCP spec).
///
/// # Usage
///
/// ```bash
/// tokenln serve [--dir .tokenln/runs] [--fix-log .tokenln/fix_log.jsonl] [--repo-root .]
/// ```
///
/// # Tools
///
/// | Tool    | Purpose |
/// |---------|---------|
/// | analyze | Compile raw CLI output into a deviation report |
/// | query   | Budget-bounded context packet from the latest run |
/// | expand  | Full evidence for one deviation |
/// | compare | Delta between the latest two runs |
/// | fixed   | Record a deviation as fixed |
/// | last    | Retrieve last run artifacts |
/// | repo_query | Budget-bounded repository context packet |
/// | repo_search | Search repository code (`rg` with fallback) |
/// | repo_read | Read file slices with line bounds |
/// | repo_tree | Compact repository tree summary |
/// | repo_log | Per-file git commit history (`git log --follow`) |
use std::collections::HashMap;
use std::fs;
use std::io::{self, BufRead, Read, Write};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::analysis::cargo_build::CargoBuildAnalyzer;
use crate::analysis::cargo_test::CargoTestAnalyzer;
use crate::analysis::go_test::GoTestAnalyzer;
use crate::analysis::jest::JestAnalyzer;
use crate::analysis::pytest::PytestAnalyzer;
use crate::context::{build_context_packet, deviation_signature, BuildPacketOptions, EvidenceRef};
use crate::emitters::claude::ClaudeEmitter;
use crate::emitters::codex::CodexEmitter;
use crate::emitters::copilot::CopilotEmitter;
use crate::emitters::generic::GenericEmitter;
use crate::emitters::ollama::OllamaEmitter;
use crate::fixlog::{load_fix_signatures, record_fix, FixLogEntry};
use crate::ir::{Deviation, DeviationReport};
use crate::lexers::cargo_build::CargoBuildLexer;
use crate::lexers::cargo_test::CargoTestLexer;
use crate::lexers::go_test::GoTestLexer;
use crate::lexers::jest::JestLexer;
use crate::lexers::pytest::PytestLexer;
use crate::optimizer::BasicOptimizer;
use crate::parsers::cargo_build::CargoBuildParser;
use crate::parsers::cargo_test::CargoTestParser;
use crate::parsers::go_test::GoTestParser;
use crate::parsers::jest::JestParser;
use crate::parsers::pytest::PytestParser;
use crate::pipeline::{Emitter, Lexer, Optimizer, Parser as PipelineParser, SemanticAnalyzer};
use crate::policy::{
    load_policy_for_repo_root, validate_repo_query_request_with_policy,
    validate_repo_read_request_with_policy, validate_repo_search_request_with_policy,
    validate_repo_tree_request_with_policy,
};
use crate::postprocess::apply_low_confidence_fallback;
use crate::root_cause::RootCauseStore;
use crate::repo::{
    log_repo_file, query_repo_context, read_repo_file, search_repo, tree_repo, RepoLogOptions,
    RepoQueryOptions, RepoReadOptions, RepoSearchOptions, RepoTreeOptions,
};

// ──────────────────────────────────────────────────────────────────────────────
// Types
// ──────────────────────────────────────────────────────────────────────────────

/// Which LLM agent format to use when rendering output.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Target {
    Generic,
    Claude,
    Ollama,
    Codex,
    Copilot,
}

/// Supported tool frontends.
#[derive(Debug, Clone, Copy)]
enum Frontend {
    CargoTest,
    CargoBuild,
    GoTest,
    Pytest,
    Jest,
}

struct RunArtifacts {
    run_id: String,
    report: DeviationReport,
    report_artifact: String,
    raw_output: String,
}

// ──────────────────────────────────────────────────────────────────────────────
// JSON-RPC types
// ──────────────────────────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
struct JsonRpcRequest {
    jsonrpc: String,
    #[serde(default)]
    id: Option<Value>,
    method: String,
    #[serde(default)]
    params: Value,
}

#[derive(Debug, Serialize)]
struct JsonRpcResponse {
    jsonrpc: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    id: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    result: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<JsonRpcError>,
}

#[derive(Debug, Serialize)]
struct JsonRpcError {
    code: i64,
    message: String,
}

impl JsonRpcResponse {
    fn ok(id: Option<Value>, result: Value) -> Self {
        Self {
            jsonrpc: "2.0",
            id,
            result: Some(result),
            error: None,
        }
    }

    fn err(id: Option<Value>, code: i64, message: impl Into<String>) -> Self {
        Self {
            jsonrpc: "2.0",
            id,
            result: None,
            error: Some(JsonRpcError {
                code,
                message: message.into(),
            }),
        }
    }
}

// ──────────────────────────────────────────────────────────────────────────────
// Server entry point
// ──────────────────────────────────────────────────────────────────────────────

/// Run the MCP server loop. Reads JSON-RPC 2.0 messages from stdin with
/// Content-Length framing and writes responses to stdout.
pub fn run_mcp_server(
    artifacts_dir: PathBuf,
    fix_log_path: PathBuf,
    repo_root: PathBuf,
) -> Result<i32, String> {
    let stdin = io::stdin();
    let mut stdin_lock = stdin.lock();
    let stdout = io::stdout();
    let mut stdout_lock = stdout.lock();

    loop {
        let content_length = match read_content_length(&mut stdin_lock) {
            Ok(Some(len)) => len,
            Ok(None) => break, // EOF
            Err(err) => {
                eprintln!("tokenln mcp: header read error: {err}");
                break;
            }
        };

        let mut body = vec![0u8; content_length];
        if let Err(err) = stdin_lock.read_exact(&mut body) {
            eprintln!("tokenln mcp: body read error: {err}");
            break;
        }

        let body_str = match std::str::from_utf8(&body) {
            Ok(s) => s,
            Err(err) => {
                eprintln!("tokenln mcp: invalid UTF-8 in body: {err}");
                continue;
            }
        };

        let request: JsonRpcRequest = match serde_json::from_str(body_str) {
            Ok(req) => req,
            Err(err) => {
                send_response(
                    &mut stdout_lock,
                    &JsonRpcResponse::err(None, -32700, format!("parse error: {err}")),
                );
                continue;
            }
        };

        if request.jsonrpc != "2.0" {
            send_response(
                &mut stdout_lock,
                &JsonRpcResponse::err(
                    request.id.clone(),
                    -32600,
                    "invalid request: jsonrpc must be \"2.0\"",
                ),
            );
            continue;
        }

        let response = handle_request(request, &artifacts_dir, &fix_log_path, &repo_root);
        if let Some(resp) = response {
            send_response(&mut stdout_lock, &resp);
        }
    }

    Ok(0)
}

// ──────────────────────────────────────────────────────────────────────────────
// I/O helpers
// ──────────────────────────────────────────────────────────────────────────────

fn read_content_length(reader: &mut impl BufRead) -> Result<Option<usize>, String> {
    let mut content_length: Option<usize> = None;
    loop {
        let mut line = String::new();
        let bytes_read = reader
            .read_line(&mut line)
            .map_err(|err| format!("read_line: {err}"))?;
        if bytes_read == 0 {
            return Ok(None);
        }
        let line = line.trim_end_matches(['\r', '\n']);
        if line.is_empty() {
            if content_length.is_some() {
                break;
            }
            continue;
        }
        if let Some((name, value)) = line.split_once(':') {
            if name.trim().eq_ignore_ascii_case("content-length") {
                if let Ok(n) = value.trim().parse::<usize>() {
                    content_length = Some(n);
                }
            }
        }
    }
    content_length
        .map(Some)
        .ok_or_else(|| "missing Content-Length header".to_string())
}

fn send_response(writer: &mut impl Write, response: &JsonRpcResponse) {
    let body = match serde_json::to_string(response) {
        Ok(s) => s,
        Err(err) => {
            eprintln!("tokenln mcp: failed to serialize response: {err}");
            return;
        }
    };
    let header = format!("Content-Length: {}\r\n\r\n", body.len());
    let _ = writer.write_all(header.as_bytes());
    let _ = writer.write_all(body.as_bytes());
    let _ = writer.flush();
}

// ──────────────────────────────────────────────────────────────────────────────
// Request dispatcher
// ──────────────────────────────────────────────────────────────────────────────

fn handle_request(
    req: JsonRpcRequest,
    artifacts_dir: &Path,
    fix_log_path: &Path,
    repo_root: &Path,
) -> Option<JsonRpcResponse> {
    let id = req.id.clone();
    match req.method.as_str() {
        "initialize" => Some(JsonRpcResponse::ok(
            id,
            json!({
                "protocolVersion": negotiated_protocol_version(&req.params),
                "capabilities": {"tools": {}},
                "serverInfo": {
                    "name": "tokenln",
                    "version": env!("CARGO_PKG_VERSION")
                }
            }),
        )),
        "notifications/initialized" => None,
        "ping" => Some(JsonRpcResponse::ok(id, json!({}))),
        "tools/list" => Some(JsonRpcResponse::ok(id, tools_list_result())),
        "tools/call" => Some(dispatch_tool(
            id,
            &req.params,
            artifacts_dir,
            fix_log_path,
            repo_root,
        )),
        other => Some(JsonRpcResponse::err(
            id,
            -32601,
            format!("method not found: {other}"),
        )),
    }
}

fn negotiated_protocol_version(params: &Value) -> String {
    params
        .get("protocolVersion")
        .and_then(|value| value.as_str())
        .unwrap_or("2024-11-05")
        .to_string()
}

fn dispatch_tool(
    id: Option<Value>,
    params: &Value,
    artifacts_dir: &Path,
    fix_log_path: &Path,
    repo_root: &Path,
) -> JsonRpcResponse {
    let name = match params.get("name").and_then(|v| v.as_str()) {
        Some(n) => n,
        None => return JsonRpcResponse::err(id, -32602, "tools/call missing required field: name"),
    };
    let args = params.get("arguments").cloned().unwrap_or(json!({}));

    let result = match name {
        "analyze" => tool_analyze(&args),
        "query" => tool_query(&args, artifacts_dir, fix_log_path),
        "expand" => tool_expand(&args, artifacts_dir, fix_log_path),
        "compare" => tool_compare(&args, artifacts_dir),
        "fixed" => tool_fixed(&args, artifacts_dir, fix_log_path),
        "last" => tool_last(&args, artifacts_dir),
        "repo_query" => tool_repo_query(&args, repo_root),
        "repo_search" => tool_repo_search(&args, repo_root),
        "repo_read" => tool_repo_read(&args, repo_root),
        "repo_tree" => tool_repo_tree(&args, repo_root),
        "repo_log" => tool_repo_log(&args, repo_root),
        other => Err(format!("unknown tool: {other}")),
    };

    match result {
        Ok(text) => JsonRpcResponse::ok(id, json!({"content": [{"type": "text", "text": text}]})),
        Err(err) => JsonRpcResponse::err(id, -32603, err),
    }
}

// ──────────────────────────────────────────────────────────────────────────────
// Tool implementations
// ──────────────────────────────────────────────────────────────────────────────

fn tool_analyze(args: &Value) -> Result<String, String> {
    let tool = args
        .get("tool")
        .and_then(|v| v.as_str())
        .ok_or("analyze: missing required field: tool")?;
    let raw_output = args
        .get("raw_output")
        .and_then(|v| v.as_str())
        .ok_or("analyze: missing required field: raw_output")?;
    let frontend = parse_frontend(tool).ok_or_else(|| format!("analyze: unknown tool '{tool}'"))?;
    let emit_ir = args
        .get("emit_ir")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let target = parse_target(args);

    let report = compile_report(frontend, raw_output);

    if emit_ir {
        serde_json::to_string_pretty(&report)
            .map_err(|err| format!("analyze: failed to serialize IR: {err}"))
    } else {
        Ok(emit_report(&report, target))
    }
}

fn tool_query(args: &Value, artifacts_dir: &Path, fix_log_path: &Path) -> Result<String, String> {
    let dir = resolve_dir(args, artifacts_dir);
    let loaded = load_latest_run(&dir)?;
    let fixed_signatures = load_fix_signatures(fix_log_path);
    let rc_store = RootCauseStore::load(&rc_graph_path(artifacts_dir));

    let budget = args.get("budget").and_then(|v| v.as_u64()).unwrap_or(400) as u32;
    let objective = args
        .get("objective")
        .and_then(|v| v.as_str())
        .unwrap_or("fix deviations");
    let target = parse_target(args);
    let emit_json = args
        .get("emit_json")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);

    let packet = build_context_packet(BuildPacketOptions {
        run_id: &loaded.run_id,
        source: &loaded.report.source,
        objective,
        budget_tokens: budget,
        report: &loaded.report,
        raw_output: &loaded.raw_output,
        report_artifact: &loaded.report_artifact,
        occurrence_counts: &rc_store.occurrence_counts(),
        regression_signatures: &rc_store.regression_signatures(),
        fixed_signatures: &fixed_signatures,
    });

    if emit_json {
        serde_json::to_string_pretty(&packet)
            .map_err(|err| format!("query: failed to serialize packet: {err}"))
    } else {
        Ok(render_context_packet(&packet, target))
    }
}

fn tool_expand(args: &Value, artifacts_dir: &Path, fix_log_path: &Path) -> Result<String, String> {
    let deviation_id = args
        .get("deviation_id")
        .and_then(|v| v.as_str())
        .ok_or("expand: missing required field: deviation_id")?;
    let view = args
        .get("view")
        .and_then(|v| v.as_str())
        .unwrap_or("evidence");
    let budget = args.get("budget").and_then(|v| v.as_u64()).unwrap_or(180) as u32;
    let target = parse_target(args);
    let dir = resolve_dir(args, artifacts_dir);

    let loaded = load_latest_run(&dir)?;
    let fixed_signatures = load_fix_signatures(fix_log_path);
    let rc_store = RootCauseStore::load(&rc_graph_path(artifacts_dir));

    let packet = build_context_packet(BuildPacketOptions {
        run_id: &loaded.run_id,
        source: &loaded.report.source,
        objective: "expand deviation evidence",
        budget_tokens: u32::MAX / 4,
        report: &loaded.report,
        raw_output: &loaded.raw_output,
        report_artifact: &loaded.report_artifact,
        occurrence_counts: &rc_store.occurrence_counts(),
        regression_signatures: &rc_store.regression_signatures(),
        fixed_signatures: &fixed_signatures,
    });

    let deviation_index = parse_deviation_id(deviation_id).ok_or_else(|| {
        format!("expand: invalid deviation id '{deviation_id}'; expected d1, d2, ...")
    })?;

    if deviation_index >= loaded.report.deviations.len() {
        return Err(format!(
            "expand: deviation '{}' not found (available: d1..d{})",
            deviation_id,
            loaded.report.deviations.len()
        ));
    }

    let deviation = &loaded.report.deviations[deviation_index];
    let target_id = format!("d{}", deviation_index + 1);

    let evidence_refs = packet
        .deviations
        .iter()
        .find(|s| s.id == target_id)
        .map(|s| s.evidence_refs.clone())
        .unwrap_or_default();

    let raw_excerpt = if view != "trace" {
        Some(compose_raw_excerpt(
            &evidence_refs,
            &loaded.raw_output,
            &loaded.report_artifact,
        ))
    } else {
        None
    };

    let trace = if view == "evidence" {
        &[][..]
    } else {
        &deviation.trace.frames[..]
    };

    Ok(render_expansion_text(
        &loaded.run_id,
        &target_id,
        view,
        budget,
        deviation,
        trace,
        &evidence_refs,
        raw_excerpt.as_deref(),
        target,
    ))
}

fn tool_compare(args: &Value, artifacts_dir: &Path) -> Result<String, String> {
    let dir = resolve_dir(args, artifacts_dir);
    let target = parse_target(args);

    let current_dir = latest_run_dir(&dir)?;
    let current = load_run_artifacts(&current_dir)?;
    let prev_dir = previous_run_dir_for_source(&current_dir, &dir, &current.report.source)?;
    let previous = load_run_artifacts(&prev_dir)?;

    if current.report.source != previous.report.source {
        return Err(format!(
            "compare: source mismatch ('{}' vs '{}')",
            current.report.source, previous.report.source
        ));
    }

    Ok(render_compare_result(&current, &previous, target))
}

fn tool_fixed(args: &Value, artifacts_dir: &Path, fix_log_path: &Path) -> Result<String, String> {
    let deviation_id = args
        .get("deviation_id")
        .and_then(|v| v.as_str())
        .ok_or("fixed: missing required field: deviation_id")?;
    let note = args
        .get("note")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());
    let dir = resolve_dir(args, artifacts_dir);

    let loaded = load_latest_run(&dir)?;
    let deviation_index = parse_deviation_id(deviation_id).ok_or_else(|| {
        format!("fixed: invalid deviation id '{deviation_id}'; expected d1, d2, ...")
    })?;

    if deviation_index >= loaded.report.deviations.len() {
        return Err(format!(
            "fixed: deviation '{}' not found (available: d1..d{})",
            deviation_id,
            loaded.report.deviations.len()
        ));
    }

    let deviation = &loaded.report.deviations[deviation_index];
    let signature = deviation_signature(deviation);
    let entry = FixLogEntry::new(&signature, &loaded.report.source, &loaded.run_id, note);
    record_fix(fix_log_path, &entry)?;

    Ok(format!(
        "Recorded fix for deviation '{}' from run '{}'.\n\
         Future query results will deprioritize this deviation (novelty_score = 0.10).\n\
         Signature: {}",
        deviation_id, loaded.run_id, signature
    ))
}

fn tool_last(args: &Value, artifacts_dir: &Path) -> Result<String, String> {
    let dir = resolve_dir(args, artifacts_dir);
    let run_dir = latest_run_dir(&dir)?;
    let mode = args.get("mode").and_then(|v| v.as_str()).unwrap_or("path");

    match mode {
        "raw" => {
            let path = run_dir.join("raw_output.txt");
            fs::read_to_string(&path)
                .map_err(|err| format!("last: failed to read raw output: {err}"))
        }
        "ir" => {
            let path = run_dir.join("report.ir.json");
            fs::read_to_string(&path)
                .map_err(|err| format!("last: failed to read IR report: {err}"))
        }
        _ => Ok(run_dir.display().to_string()),
    }
}

fn tool_repo_query(args: &Value, repo_root: &Path) -> Result<String, String> {
    let policy =
        load_policy_for_repo_root(repo_root).map_err(|err| format!("tokenln policy: {err}"))?;

    let objective = args
        .get("objective")
        .and_then(|v| v.as_str())
        .ok_or("repo_query: missing required field: objective")?;
    let path = args.get("path").and_then(|v| v.as_str()).map(Path::new);
    let budget = args.get("budget").and_then(|v| v.as_u64()).unwrap_or(320) as u32;
    let max_findings = args
        .get("max_findings")
        .and_then(|v| v.as_u64())
        .unwrap_or(8) as usize;
    let max_hints = args.get("max_hints").and_then(|v| v.as_u64()).unwrap_or(6) as usize;
    let emit_json = args
        .get("emit_json")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);

    validate_repo_query_request_with_policy(&policy, budget, max_findings, max_hints)
        .map_err(|violation| violation.render())?;

    let packet = query_repo_context(RepoQueryOptions {
        repo_root,
        scope: path,
        objective,
        budget_tokens: budget,
        max_findings,
        max_hints,
    })?;

    if emit_json {
        serde_json::to_string_pretty(&packet)
            .map_err(|err| format!("repo_query: failed to serialize JSON: {err}"))
    } else {
        Ok(render_repo_query_text(&packet))
    }
}

fn tool_repo_search(args: &Value, repo_root: &Path) -> Result<String, String> {
    let policy =
        load_policy_for_repo_root(repo_root).map_err(|err| format!("tokenln policy: {err}"))?;

    let query = args
        .get("query")
        .and_then(|v| v.as_str())
        .ok_or("repo_search: missing required field: query")?;

    let path = args.get("path").and_then(|v| v.as_str()).map(Path::new);
    let glob = args.get("glob").and_then(|v| v.as_str());
    let max_results = args
        .get("max_results")
        .and_then(|v| v.as_u64())
        .unwrap_or(80) as usize;
    let ignore_case = args
        .get("ignore_case")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let fixed_strings = args
        .get("fixed_strings")
        .and_then(|v| v.as_bool())
        .unwrap_or(true);
    let emit_json = args
        .get("emit_json")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);

    validate_repo_search_request_with_policy(
        &policy,
        query,
        max_results,
        fixed_strings,
        path.is_some(),
        glob.is_some(),
    )
    .map_err(|violation| violation.render())?;

    let result = search_repo(RepoSearchOptions {
        repo_root,
        scope: path,
        query,
        glob,
        max_results,
        ignore_case,
        fixed_strings,
    })?;

    if emit_json {
        serde_json::to_string_pretty(&result)
            .map_err(|err| format!("repo_search: failed to serialize JSON: {err}"))
    } else {
        Ok(render_repo_search_text(&result))
    }
}

fn tool_repo_read(args: &Value, repo_root: &Path) -> Result<String, String> {
    let policy =
        load_policy_for_repo_root(repo_root).map_err(|err| format!("tokenln policy: {err}"))?;

    let path = args
        .get("path")
        .and_then(|v| v.as_str())
        .ok_or("repo_read: missing required field: path")?;
    let start_line = args.get("start_line").and_then(|v| v.as_u64()).unwrap_or(1) as u32;
    let end_line = args
        .get("end_line")
        .and_then(|v| v.as_u64())
        .map(|v| v as u32);
    let max_chars = args
        .get("max_chars")
        .and_then(|v| v.as_u64())
        .unwrap_or(6000) as usize;
    let emit_json = args
        .get("emit_json")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);

    validate_repo_read_request_with_policy(&policy, start_line, end_line, max_chars)
        .map_err(|violation| violation.render())?;

    let result = read_repo_file(RepoReadOptions {
        repo_root,
        path: Path::new(path),
        start_line,
        end_line,
        max_chars,
    })?;

    if emit_json {
        serde_json::to_string_pretty(&result)
            .map_err(|err| format!("repo_read: failed to serialize JSON: {err}"))
    } else {
        Ok(render_repo_read_text(&result))
    }
}

fn tool_repo_tree(args: &Value, repo_root: &Path) -> Result<String, String> {
    let policy =
        load_policy_for_repo_root(repo_root).map_err(|err| format!("tokenln policy: {err}"))?;

    let path = args.get("path").and_then(|v| v.as_str()).map(Path::new);
    let max_depth = args.get("max_depth").and_then(|v| v.as_u64()).unwrap_or(3) as u32;
    let max_entries = args
        .get("max_entries")
        .and_then(|v| v.as_u64())
        .unwrap_or(200) as usize;
    let emit_json = args
        .get("emit_json")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);

    validate_repo_tree_request_with_policy(&policy, max_depth, max_entries)
        .map_err(|violation| violation.render())?;

    let result = tree_repo(RepoTreeOptions {
        repo_root,
        scope: path,
        max_depth,
        max_entries,
    })?;

    if emit_json {
        serde_json::to_string_pretty(&result)
            .map_err(|err| format!("repo_tree: failed to serialize JSON: {err}"))
    } else {
        Ok(render_repo_tree_text(&result))
    }
}

fn tool_repo_log(args: &Value, repo_root: &Path) -> Result<String, String> {
    let path = args
        .get("path")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "repo_log: missing required parameter 'path'".to_string())?;
    let limit = args
        .get("limit")
        .and_then(|v| v.as_u64())
        .unwrap_or(20) as usize;
    let emit_json = args
        .get("emit_json")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);

    let result = log_repo_file(RepoLogOptions {
        repo_root,
        path: std::path::Path::new(path),
        limit,
    })?;

    if emit_json {
        serde_json::to_string_pretty(&result)
            .map_err(|err| format!("repo_log: failed to serialize JSON: {err}"))
    } else {
        Ok(render_repo_log_text(&result))
    }
}

fn render_repo_log_text(result: &crate::repo::RepoLogResult) -> String {
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
    lines.join("\n")
}

// ──────────────────────────────────────────────────────────────────────────────
// Pipeline helpers (self-contained, parallel to main.rs)
// ──────────────────────────────────────────────────────────────────────────────

fn compile_report(frontend: Frontend, raw_output: &str) -> DeviationReport {
    let optimizer = BasicOptimizer;
    let mut report = match frontend {
        Frontend::CargoTest => {
            let tokens = CargoTestLexer.lex(raw_output);
            let parsed = CargoTestParser.parse(&tokens);
            optimizer.optimize(CargoTestAnalyzer.analyze(&parsed))
        }
        Frontend::CargoBuild => {
            let tokens = CargoBuildLexer.lex(raw_output);
            let parsed = CargoBuildParser.parse(&tokens);
            optimizer.optimize(CargoBuildAnalyzer.analyze(&parsed))
        }
        Frontend::GoTest => {
            let tokens = GoTestLexer.lex(raw_output);
            let parsed = GoTestParser.parse(&tokens);
            optimizer.optimize(GoTestAnalyzer.analyze(&parsed))
        }
        Frontend::Pytest => {
            let tokens = PytestLexer.lex(raw_output);
            let parsed = PytestParser.parse(&tokens);
            optimizer.optimize(PytestAnalyzer.analyze(&parsed))
        }
        Frontend::Jest => {
            let tokens = JestLexer.lex(raw_output);
            let parsed = JestParser.parse(&tokens);
            optimizer.optimize(JestAnalyzer.analyze(&parsed))
        }
    };
    apply_low_confidence_fallback(&mut report, raw_output);
    report
}

fn emit_report(report: &DeviationReport, target: Target) -> String {
    match target {
        Target::Generic => GenericEmitter.emit(report),
        Target::Claude => ClaudeEmitter.emit(report),
        Target::Ollama => OllamaEmitter.emit(report),
        Target::Codex => CodexEmitter.emit(report),
        Target::Copilot => CopilotEmitter.emit(report),
    }
}

// ──────────────────────────────────────────────────────────────────────────────
// ── Root-cause graph helpers ───────────────────────────────────────────────────

fn rc_graph_path(artifacts_dir: &Path) -> PathBuf {
    artifacts_dir
        .parent()
        .unwrap_or(artifacts_dir)
        .join("root_cause_graph.jsonl")
}

// Artifact I/O helpers
// ──────────────────────────────────────────────────────────────────────────────

fn load_latest_run(artifacts_dir: &Path) -> Result<RunArtifacts, String> {
    let run_dir = latest_run_dir(artifacts_dir)?;
    load_run_artifacts(&run_dir)
}

fn latest_run_dir(artifacts_dir: &Path) -> Result<PathBuf, String> {
    let latest_file = artifacts_dir.join("latest.txt");
    let content = fs::read_to_string(&latest_file).map_err(|err| {
        format!(
            "no run artifacts found at '{}': {err}\n\
             Run a command through the proxy first: tokenln proxy run -- cargo test",
            latest_file.display()
        )
    })?;
    let path = PathBuf::from(content.trim());
    if !path.is_dir() {
        return Err(format!(
            "latest run directory '{}' does not exist",
            path.display()
        ));
    }
    Ok(path)
}

fn load_run_artifacts(run_dir: &Path) -> Result<RunArtifacts, String> {
    let report_path = run_dir.join("report.ir.json");
    let raw_path = run_dir.join("raw_output.txt");

    let report_artifact = fs::read_to_string(&report_path).map_err(|err| {
        format!(
            "failed to read IR artifact '{}': {err}",
            report_path.display()
        )
    })?;
    let report: DeviationReport = serde_json::from_str(&report_artifact)
        .map_err(|err| format!("failed to parse IR artifact: {err}"))?;
    let raw_output = fs::read_to_string(&raw_path).unwrap_or_default();
    let run_id = run_dir
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("run")
        .to_string();

    Ok(RunArtifacts {
        run_id,
        report,
        report_artifact,
        raw_output,
    })
}

fn previous_run_dir_for_source(
    run_dir: &Path,
    artifacts_dir: &Path,
    source: &str,
) -> Result<PathBuf, String> {
    let mut run_dirs = fs::read_dir(artifacts_dir)
        .map_err(|err| format!("failed to list '{}': {err}", artifacts_dir.display()))?
        .filter_map(Result::ok)
        .map(|e| e.path())
        .filter(|p| {
            p.is_dir()
                && p.file_name()
                    .and_then(|n| n.to_str())
                    .is_some_and(|n| n.starts_with("run-"))
        })
        .collect::<Vec<_>>();
    run_dirs.sort();

    let current = run_dir
        .canonicalize()
        .unwrap_or_else(|_| run_dir.to_path_buf());
    let current_idx = run_dirs
        .iter()
        .position(|p| p.canonicalize().unwrap_or_else(|_| p.clone()) == current)
        .ok_or_else(|| format!("run '{}' not tracked", run_dir.display()))?;

    for candidate in run_dirs[..current_idx].iter().rev() {
        if run_source(candidate).as_deref() == Some(source) {
            return Ok(candidate.clone());
        }
    }

    Err(format!("no previous run for source '{source}'"))
}

fn run_source(run_dir: &Path) -> Option<String> {
    let text = fs::read_to_string(run_dir.join("report.ir.json")).ok()?;
    let report: DeviationReport = serde_json::from_str(&text).ok()?;
    Some(report.source)
}

// ──────────────────────────────────────────────────────────────────────────────
// Rendering helpers
// ──────────────────────────────────────────────────────────────────────────────

fn render_context_packet(packet: &crate::context::ContextPacket, target: Target) -> String {
    match target {
        Target::Generic => {
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
            for d in &packet.deviations {
                lines.push(format!(
                    "[{}] {} | utility={:.2} novelty={:.2} confidence={:.2}{}",
                    d.id,
                    d.summary,
                    d.utility_score,
                    d.novelty_score,
                    d.confidence,
                    d.fix_hint
                        .as_deref()
                        .map(|h| format!(" | {h}"))
                        .unwrap_or_default()
                ));
                lines.push(format!("  expected: {}", d.expected));
                lines.push(format!("  actual:   {}", d.actual));
                lines.push(format!("  location: {}", d.location));
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
        _ => {
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
            for d in &packet.deviations {
                let fix = d
                    .fix_hint
                    .as_deref()
                    .map(|h| format!("\n> {h}"))
                    .unwrap_or_default();
                sections.push(format!(
                    "## {} · utility {:.2} · novelty {:.2} · confidence {:.2}\n\
                     Summary: {}\nExpected: {}\nActual: {}\nLocation: {}{}",
                    d.id,
                    d.utility_score,
                    d.novelty_score,
                    d.confidence,
                    d.summary,
                    d.expected,
                    d.actual,
                    d.location,
                    fix,
                ));
            }
            if !packet.expansion_hints.is_empty() {
                let mut hints = "## Expansion Hints".to_string();
                for hint in &packet.expansion_hints {
                    hints.push_str(&format!(
                        "\n- `{}` ({}) est `{}` tokens",
                        hint.deviation_id, hint.reason, hint.estimated_tokens
                    ));
                }
                sections.push(hints);
            }
            sections.join("\n\n")
        }
    }
}

fn render_compare_result(
    current: &RunArtifacts,
    previous: &RunArtifacts,
    target: Target,
) -> String {
    let mut current_map: HashMap<String, (String, &Deviation)> = HashMap::new();
    for (idx, d) in current.report.deviations.iter().enumerate() {
        current_map
            .entry(deviation_signature(d))
            .or_insert_with(|| (format!("d{}", idx + 1), d));
    }

    let mut previous_map: HashMap<String, (String, &Deviation)> = HashMap::new();
    for (idx, d) in previous.report.deviations.iter().enumerate() {
        previous_map
            .entry(deviation_signature(d))
            .or_insert_with(|| (format!("d{}", idx + 1), d));
    }

    let new_deviations: Vec<_> = current_map
        .iter()
        .filter(|(sig, _)| !previous_map.contains_key(*sig))
        .map(|(_, (id, d))| (id.clone(), d.summary.clone(), d.confidence))
        .collect();

    let resolved_deviations: Vec<_> = previous_map
        .iter()
        .filter(|(sig, _)| !current_map.contains_key(*sig))
        .map(|(_, (id, d))| (id.clone(), d.summary.clone(), d.confidence))
        .collect();

    let persistent_count = current_map
        .keys()
        .filter(|sig| previous_map.contains_key(*sig))
        .count();

    match target {
        Target::Generic => {
            let mut lines = Vec::new();
            lines.push(format!(
                "RUN_COMPARE current={} previous={} source={}",
                current.run_id, previous.run_id, current.report.source
            ));
            lines.push(format!(
                "totals: current={} previous={} new={} resolved={} persistent={}",
                current.report.deviations.len(),
                previous.report.deviations.len(),
                new_deviations.len(),
                resolved_deviations.len(),
                persistent_count
            ));
            lines.push("new:".to_string());
            if new_deviations.is_empty() {
                lines.push("  - none".to_string());
            } else {
                for (id, summary, confidence) in &new_deviations {
                    lines.push(format!("  - {id} {summary} conf={confidence:.2}"));
                }
            }
            lines.push("resolved:".to_string());
            if resolved_deviations.is_empty() {
                lines.push("  - none".to_string());
            } else {
                for (id, summary, confidence) in &resolved_deviations {
                    lines.push(format!("  - {id} {summary} conf={confidence:.2}"));
                }
            }
            lines.join("\n")
        }
        _ => {
            let mut sections = Vec::new();
            sections.push("# TokenLn Run Comparison".to_string());
            sections.push(format!(
                "Current: `{}`  \nPrevious: `{}`  \nSource: `{}`",
                current.run_id, previous.run_id, current.report.source
            ));
            sections.push(format!(
                "New: `{}`  \nResolved: `{}`  \nPersistent: `{}`",
                new_deviations.len(),
                resolved_deviations.len(),
                persistent_count
            ));
            if new_deviations.is_empty() {
                sections.push("## New\n- none".to_string());
            } else {
                let rows = new_deviations
                    .iter()
                    .map(|(id, summary, confidence)| {
                        format!("- `{id}` {summary} conf `{confidence:.2}`")
                    })
                    .collect::<Vec<_>>()
                    .join("\n");
                sections.push(format!("## New\n{rows}"));
            }
            if resolved_deviations.is_empty() {
                sections.push("## Resolved\n- none".to_string());
            } else {
                let rows = resolved_deviations
                    .iter()
                    .map(|(id, summary, confidence)| {
                        format!("- `{id}` {summary} conf `{confidence:.2}`")
                    })
                    .collect::<Vec<_>>()
                    .join("\n");
                sections.push(format!("## Resolved\n{rows}"));
            }
            sections.join("\n\n")
        }
    }
}

fn render_expansion_text(
    run_id: &str,
    deviation_id: &str,
    view: &str,
    budget: u32,
    deviation: &Deviation,
    trace: &[String],
    evidence_refs: &[EvidenceRef],
    raw_excerpt: Option<&str>,
    target: Target,
) -> String {
    match target {
        Target::Generic => {
            let mut lines = Vec::new();
            lines.push(format!(
                "EXPANSION run={} deviation={} view={} budget={}",
                run_id, deviation_id, view, budget
            ));
            lines.push(format!("summary: {}", deviation.summary));
            lines.push(format!("expected: {}", deviation.expected.description));
            lines.push(format!("actual:   {}", deviation.actual.description));
            lines.push(format!(
                "location: {}",
                format_deviation_location(deviation)
            ));
            lines.push(format!("confidence: {:.2}", deviation.confidence));
            if !trace.is_empty() {
                lines.push(format!("trace: {}", trace.join(" -> ")));
            }
            if !evidence_refs.is_empty() {
                lines.push("evidence_refs:".to_string());
                for ev in evidence_refs {
                    lines.push(format!(
                        "  - {} [{}:{}] hash={}",
                        ev.artifact, ev.line_start, ev.line_end, ev.hash
                    ));
                }
            }
            if let Some(excerpt) = raw_excerpt {
                lines.push("raw_excerpt:".to_string());
                lines.push(excerpt.to_string());
            }
            lines.join("\n")
        }
        _ => {
            let mut sections = Vec::new();
            sections.push("# TokenLn Expansion".to_string());
            sections.push(format!(
                "Run: `{run_id}`  \nDeviation: `{deviation_id}`  \nView: `{view}`  \nBudget: `{budget}` tokens"
            ));
            sections.push(format!(
                "Summary: {}\nExpected: {}\nActual: {}\nLocation: {}\nConfidence: {:.2}",
                deviation.summary,
                deviation.expected.description,
                deviation.actual.description,
                format_deviation_location(deviation),
                deviation.confidence
            ));
            if !trace.is_empty() {
                sections.push(format!("Trace: `{}`", trace.join(" -> ")));
            }
            if !evidence_refs.is_empty() {
                let refs = evidence_refs
                    .iter()
                    .map(|ev| {
                        format!(
                            "- `{}` [{}:{}] hash `{}`",
                            ev.artifact, ev.line_start, ev.line_end, ev.hash
                        )
                    })
                    .collect::<Vec<_>>()
                    .join("\n");
                sections.push(format!("## Evidence Refs\n{refs}"));
            }
            if let Some(excerpt) = raw_excerpt {
                sections.push(format!("## Raw Excerpt\n```text\n{excerpt}\n```"));
            }
            sections.join("\n\n")
        }
    }
}

fn render_repo_query_text(packet: &crate::repo::RepoContextPacket) -> String {
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
        for hint in packet.expansion_hints.iter().take(6) {
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

fn render_repo_search_text(result: &crate::repo::RepoSearchResult) -> String {
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

fn render_repo_read_text(result: &crate::repo::RepoReadResult) -> String {
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

fn render_repo_tree_text(result: &crate::repo::RepoTreeResult) -> String {
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

fn compose_raw_excerpt(
    evidence_refs: &[EvidenceRef],
    raw_output: &str,
    report_artifact: &str,
) -> String {
    let mut sections = Vec::new();
    for ev in evidence_refs {
        let content = match ev.artifact.as_str() {
            "raw_output.txt" => extract_line_window(raw_output, ev.line_start, ev.line_end),
            "report.ir.json" => extract_line_window(report_artifact, ev.line_start, ev.line_end),
            _ => None,
        };
        if let Some(content) = content {
            sections.push(format!(
                "artifact: {} [{}:{}]\n{}",
                ev.artifact, ev.line_start, ev.line_end, content
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
    let lines: Vec<_> = text.lines().collect();
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

fn format_deviation_location(deviation: &Deviation) -> String {
    let file = deviation.location.file.as_deref().unwrap_or("unknown");
    let line = deviation
        .location
        .line
        .map(|l| l.to_string())
        .unwrap_or_else(|| "?".to_string());
    let col = deviation
        .location
        .column
        .map(|c| c.to_string())
        .unwrap_or_else(|| "?".to_string());
    format!("{file}:{line}:{col}")
}

fn parse_deviation_id(input: &str) -> Option<usize> {
    let trimmed = input.trim();
    let value = trimmed.strip_prefix('d').unwrap_or(trimmed);
    let index = value.parse::<usize>().ok()?;
    index.checked_sub(1)
}

fn parse_frontend(tool: &str) -> Option<Frontend> {
    match tool {
        "cargo_test" => Some(Frontend::CargoTest),
        "cargo_build" => Some(Frontend::CargoBuild),
        "go_test" => Some(Frontend::GoTest),
        "pytest" => Some(Frontend::Pytest),
        "jest" => Some(Frontend::Jest),
        _ => None,
    }
}

fn parse_target(args: &Value) -> Target {
    match args
        .get("target")
        .and_then(|v| v.as_str())
        .unwrap_or("generic")
    {
        "claude" => Target::Claude,
        "ollama" => Target::Ollama,
        "codex" => Target::Codex,
        "copilot" => Target::Copilot,
        _ => Target::Generic,
    }
}

fn resolve_dir(args: &Value, default: &Path) -> PathBuf {
    args.get("dir")
        .and_then(|v| v.as_str())
        .map(PathBuf::from)
        .unwrap_or_else(|| default.to_path_buf())
}

// ──────────────────────────────────────────────────────────────────────────────
// Tools list (separate function to keep handle_request readable)
// ──────────────────────────────────────────────────────────────────────────────

fn tools_list_result() -> Value {
    json!({
        "tools": [
            {
                "name": "analyze",
                "description": "Compile raw CLI output (cargo test, pytest, jest, go test, cargo build) into a minimal deviation report. Pass the raw stdout/stderr of the command directly — no file, no proxy setup needed.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "tool": {
                            "type": "string",
                            "enum": ["cargo_test", "cargo_build", "go_test", "pytest", "jest"],
                            "description": "The tool that produced the output"
                        },
                        "raw_output": {
                            "type": "string",
                            "description": "The raw stdout/stderr from the tool"
                        },
                        "target": {
                            "type": "string",
                            "enum": ["generic", "claude", "ollama", "codex", "copilot"],
                            "description": "LLM target format (default: generic)"
                        },
                        "emit_ir": {
                            "type": "boolean",
                            "description": "Emit raw Deviation IR JSON (default: false)"
                        }
                    },
                    "required": ["tool", "raw_output"]
                }
            },
            {
                "name": "query",
                "description": "Build a budget-bounded context packet from the latest run. Returns the highest-utility deviations that fit within the token budget.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "budget": {"type": "number", "description": "Max token budget (default: 400)"},
                        "objective": {"type": "string", "description": "Goal for the context (default: fix deviations)"},
                        "target": {"type": "string", "enum": ["generic", "claude", "ollama", "codex", "copilot"]},
                        "dir": {"type": "string", "description": "Artifacts directory (default: .tokenln/runs)"},
                        "emit_json": {"type": "boolean", "description": "Return raw JSON packet"}
                    }
                }
            },
            {
                "name": "expand",
                "description": "Expand one deviation with full evidence, trace, and raw output excerpts. Use after query to get more detail.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "deviation_id": {"type": "string", "description": "Deviation ID from a prior query (e.g. d1)"},
                        "view": {"type": "string", "enum": ["evidence", "trace", "full"]},
                        "budget": {"type": "number", "description": "Max token budget (default: 180)"},
                        "target": {"type": "string", "enum": ["generic", "claude", "ollama", "codex", "copilot"]},
                        "dir": {"type": "string"}
                    },
                    "required": ["deviation_id"]
                }
            },
            {
                "name": "compare",
                "description": "Compare the latest two runs for the same source. Reports new, resolved, and persistent deviations.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "target": {"type": "string", "enum": ["generic", "claude", "ollama", "codex", "copilot"]},
                        "dir": {"type": "string"}
                    }
                }
            },
            {
                "name": "fixed",
                "description": "Record a deviation as fixed. Future query results will deprioritize it (novelty_score = 0.10).",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "deviation_id": {"type": "string", "description": "Deviation ID to mark as fixed (e.g. d1)"},
                        "note": {"type": "string", "description": "Optional description of what was fixed"},
                        "dir": {"type": "string"}
                    },
                    "required": ["deviation_id"]
                }
            },
            {
                "name": "last",
                "description": "Retrieve information about the last run.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "mode": {"type": "string", "enum": ["path", "raw", "ir"]},
                        "dir": {"type": "string"}
                    }
                }
            },
            {
                "name": "repo_query",
                "description": "Build a budget-bounded repository context packet for a non-debug objective (understanding, planning, location discovery).",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "objective": {"type": "string", "description": "What you want to accomplish or understand"},
                        "path": {"type": "string", "description": "Optional scope path within repo root"},
                        "budget": {"type": "number", "description": "Token budget for included findings and hints (default: 320, capped at 900)"},
                        "max_findings": {"type": "number", "description": "Maximum findings to include (default: 8, capped at 16)"},
                        "max_hints": {"type": "number", "description": "Maximum expansion hints to include (default: 6, capped at 12)"},
                        "emit_json": {"type": "boolean", "description": "Return JSON packet"}
                    },
                    "required": ["objective"]
                }
            },
            {
                "name": "repo_search",
                "description": "Search repository files (ripgrep-powered with safe fallback). Use this instead of broad file-by-file exploration.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "query": {"type": "string", "description": "Search query or regex pattern"},
                        "path": {"type": "string", "description": "Optional scope path within repo root"},
                        "glob": {"type": "string", "description": "Optional glob filter (e.g. *.rs)"},
                        "max_results": {"type": "number", "description": "Maximum matches to return (default: 80, capped at 120)"},
                        "ignore_case": {"type": "boolean", "description": "Case-insensitive search"},
                        "fixed_strings": {"type": "boolean", "description": "Treat query as literal text (default: true)"},
                        "emit_json": {"type": "boolean", "description": "Return JSON result"}
                    },
                    "required": ["query"]
                }
            },
            {
                "name": "repo_read",
                "description": "Read a bounded file slice with line controls and truncation safeguards.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "path": {"type": "string", "description": "File path relative to repo root"},
                        "start_line": {"type": "number", "description": "Start line (default: 1)"},
                        "end_line": {"type": "number", "description": "Optional end line (default: start+199, span capped at 400 lines)"},
                        "max_chars": {"type": "number", "description": "Maximum returned characters (default: 6000, capped at 12000)"},
                        "emit_json": {"type": "boolean", "description": "Return JSON result"}
                    },
                    "required": ["path"]
                }
            },
            {
                "name": "repo_tree",
                "description": "Return a compact repository tree to understand project structure before deeper reads.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "path": {"type": "string", "description": "Optional subtree path"},
                        "max_depth": {"type": "number", "description": "Tree depth (default: 3, capped at 4)"},
                        "max_entries": {"type": "number", "description": "Maximum entries (default: 200, capped at 400)"},
                        "emit_json": {"type": "boolean", "description": "Return JSON result"}
                    }
                }
            },
            {
                "name": "repo_log",
                "description": "Return recent git commit history for a single file. Useful for 'what changed', 'recent commits', or 'history of' queries.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "path": {"type": "string", "description": "File path relative to repo root"},
                        "limit": {"type": "number", "description": "Maximum number of commits to return (default: 20, max: 200)"},
                        "emit_json": {"type": "boolean", "description": "Return JSON result"}
                    },
                    "required": ["path"]
                }
            }
        ]
    })
}
