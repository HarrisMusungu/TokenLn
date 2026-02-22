# TokenLn Agent Instructions

This project uses TokenLn as an MCP-backed context layer.

## Primary Rule

Prefer TokenLn MCP tools over broad shell exploration.

Use:
- `repo_query` for repo understanding/planning
- `repo_search` for locating symbols/text
- `repo_read` for bounded file slices
- `repo_tree` for structure overview
- `query` / `expand` / `compare` for deviation workflows

Do not manually build JSON-RPC messages in Bash (no `echo "Content-Length:..."` protocol calls).

## Shell Usage

Shell is still allowed for:
- Running build/test commands (`cargo test`, etc.)
- Small focused checks

Avoid:
- Broad recursive scans when MCP tools can answer the same question
- Huge unbounded reads

## Failure Fallback

If an MCP tool call fails:
1. State that MCP failed.
2. Use the narrowest shell fallback possible.
3. Return to MCP tools as soon as available.
