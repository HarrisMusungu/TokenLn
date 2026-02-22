use crate::pipeline::{Lexer, Token};

pub struct CargoBuildLexer;

impl Lexer for CargoBuildLexer {
    fn lex(&self, input: &str) -> Vec<Token> {
        let mut tokens = Vec::new();

        for line in input.lines() {
            let trimmed = line.trim();

            if let Some((code, message)) = parse_build_error_header(trimmed) {
                tokens.push(Token::BuildErrorHeader { code, message });
                continue;
            }

            if let Some((file, line_num, column_num)) = parse_build_location(trimmed) {
                tokens.push(Token::BuildLocation {
                    file,
                    line: line_num,
                    column: column_num,
                });
                continue;
            }

            if let Some(message) = trimmed.strip_prefix("help:") {
                tokens.push(Token::BuildHelp {
                    message: message.trim().to_string(),
                });
                continue;
            }

            if let Some(code) = parse_build_explain_code(trimmed) {
                tokens.push(Token::BuildExplainCode { code });
                continue;
            }

            if let Some((line_num, snippet)) = parse_build_source_snippet(trimmed) {
                tokens.push(Token::BuildSourceSnippet {
                    line: line_num,
                    snippet,
                });
                continue;
            }

            if let Some(marker) = parse_build_caret_marker(trimmed) {
                tokens.push(Token::BuildCaretMarker { marker });
            }
        }

        tokens
    }
}

fn parse_build_error_header(line: &str) -> Option<(Option<String>, String)> {
    if let Some(rest) = line.strip_prefix("error[") {
        let (code, message) = rest.split_once("]: ")?;
        if code.is_empty() {
            return None;
        }
        return Some((Some(code.to_string()), message.trim().to_string()));
    }

    if let Some(message) = line.strip_prefix("error:") {
        let message = message.trim();
        if message.starts_with("could not compile") {
            return None;
        }
        if message.is_empty() {
            return None;
        }
        return Some((None, message.to_string()));
    }

    None
}

fn parse_build_location(line: &str) -> Option<(String, u32, u32)> {
    let location = line.strip_prefix("--> ")?;
    parse_file_line_column(location.trim())
}

fn parse_build_explain_code(line: &str) -> Option<String> {
    let prefix = "For more information about this error, try `rustc --explain ";
    let rest = line.strip_prefix(prefix)?;
    let code = rest.strip_suffix("`.")?.trim();
    if code.is_empty() {
        return None;
    }
    Some(code.to_string())
}

fn parse_build_source_snippet(line: &str) -> Option<(Option<u32>, String)> {
    let (left, right) = line.split_once('|')?;
    let left = left.trim();
    if left.is_empty() {
        return None;
    }

    let line_num = left.parse::<u32>().ok()?;
    let snippet = right.trim();
    if snippet.is_empty() {
        return None;
    }

    Some((Some(line_num), snippet.to_string()))
}

fn parse_build_caret_marker(line: &str) -> Option<String> {
    let marker = line.strip_prefix('|')?.trim();
    if marker.starts_with('^') {
        return Some(marker.to_string());
    }
    None
}

fn parse_file_line_column(location: &str) -> Option<(String, u32, u32)> {
    let mut parts = location.rsplitn(3, ':');
    let col = parts.next()?.parse::<u32>().ok()?;
    let line = parts.next()?.parse::<u32>().ok()?;
    let file = parts.next()?.to_string();
    if file.is_empty() {
        return None;
    }
    Some((file, line, col))
}

#[cfg(test)]
mod tests {
    use super::CargoBuildLexer;
    use crate::pipeline::{Lexer, Token};

    #[test]
    fn lexes_build_error_with_code_and_location() {
        let input = "\
error[E0425]: cannot find value `x` in this scope
 --> src/main.rs:4:20
4 |     println!(\"{}\", x);
  |                    ^ not found in this scope
help: a local variable with a similar name exists: `y`
For more information about this error, try `rustc --explain E0425`.";

        let tokens = CargoBuildLexer.lex(input);
        assert!(tokens.iter().any(|t| matches!(
            t,
            Token::BuildErrorHeader { code, message } if code.as_deref() == Some("E0425") && message.contains("cannot find value")
        )));
        assert!(tokens.iter().any(|t| matches!(
            t,
            Token::BuildLocation { file, line, column } if file == "src/main.rs" && *line == 4 && *column == 20
        )));
        assert!(tokens.iter().any(|t| matches!(
            t,
            Token::BuildHelp { message } if message.contains("similar name")
        )));
        assert!(tokens.iter().any(|t| matches!(
            t,
            Token::BuildExplainCode { code } if code == "E0425"
        )));
        assert!(tokens.iter().any(|t| matches!(
            t,
            Token::BuildSourceSnippet { line, snippet } if *line == Some(4) && snippet.contains("println!")
        )));
        assert!(tokens.iter().any(|t| matches!(
            t,
            Token::BuildCaretMarker { marker } if marker.starts_with('^') && marker.contains("not found")
        )));
    }

    #[test]
    fn ignores_could_not_compile_summary_line() {
        let input = "error: could not compile `demo` due to 1 previous error";
        let tokens = CargoBuildLexer.lex(input);
        assert!(tokens.is_empty());
    }
}
