use crate::pipeline::{ParsedFailure, Parser, Token};

pub struct CargoBuildParser;

impl Parser for CargoBuildParser {
    fn parse(&self, tokens: &[Token]) -> Vec<ParsedFailure> {
        let mut failures = Vec::new();
        let mut current: Option<ParsedFailure> = None;

        for token in tokens {
            match token {
                Token::BuildErrorHeader { code, message } => {
                    push_if_meaningful(&mut failures, current.take());
                    current = Some(ParsedFailure {
                        test_name: code.clone().or(Some("rustc".to_string())),
                        panic_message: Some(message.clone()),
                        build_error_code: code.clone(),
                        trace: vec!["cargo build".to_string()],
                        ..ParsedFailure::default()
                    });
                }
                Token::BuildLocation { file, line, column } => {
                    let current = current.get_or_insert_with(ParsedFailure::default);
                    current.file = Some(file.clone());
                    current.line = Some(*line);
                    current.column = Some(*column);
                }
                Token::BuildHelp { message } => {
                    let current = current.get_or_insert_with(ParsedFailure::default);
                    current.build_help = Some(message.clone());
                }
                Token::BuildExplainCode { code } => {
                    let current = current.get_or_insert_with(ParsedFailure::default);
                    current.build_explain_code = Some(code.clone());
                    if current.build_error_code.is_none() {
                        current.build_error_code = Some(code.clone());
                    }
                }
                Token::BuildSourceSnippet { line, snippet } => {
                    let current = current.get_or_insert_with(ParsedFailure::default);
                    if current.build_source_line.is_none() {
                        current.build_source_line = *line;
                    }
                    current.build_source_snippet = Some(snippet.clone());
                }
                Token::BuildCaretMarker { marker } => {
                    let current = current.get_or_insert_with(ParsedFailure::default);
                    current.build_caret_marker = Some(marker.clone());
                }
                Token::FailureHeader { .. }
                | Token::PanicThread { .. }
                | Token::PanicLocation { .. }
                | Token::AssertionLeft { .. }
                | Token::AssertionRight { .. }
                | Token::PanicMessage { .. } => {}
            }
        }

        push_if_meaningful(&mut failures, current);
        failures
    }
}

fn push_if_meaningful(target: &mut Vec<ParsedFailure>, candidate: Option<ParsedFailure>) {
    if let Some(candidate) = candidate {
        if candidate.build_error_code.is_some()
            || candidate.panic_message.is_some()
            || candidate.file.is_some()
        {
            target.push(candidate);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::CargoBuildParser;
    use crate::pipeline::{Parser, Token};

    #[test]
    fn groups_build_error_tokens() {
        let tokens = vec![
            Token::BuildErrorHeader {
                code: Some("E0425".to_string()),
                message: "cannot find value `x` in this scope".to_string(),
            },
            Token::BuildLocation {
                file: "src/main.rs".to_string(),
                line: 4,
                column: 20,
            },
            Token::BuildHelp {
                message: "a local variable with a similar name exists: `y`".to_string(),
            },
            Token::BuildExplainCode {
                code: "E0425".to_string(),
            },
            Token::BuildSourceSnippet {
                line: Some(4),
                snippet: "println!(\"{}\", x);".to_string(),
            },
            Token::BuildCaretMarker {
                marker: "^ not found in this scope".to_string(),
            },
        ];

        let parsed = CargoBuildParser.parse(&tokens);
        assert_eq!(parsed.len(), 1);
        let first = &parsed[0];
        assert_eq!(first.build_error_code.as_deref(), Some("E0425"));
        assert_eq!(first.file.as_deref(), Some("src/main.rs"));
        assert_eq!(
            first.build_help.as_deref(),
            Some("a local variable with a similar name exists: `y`")
        );
        assert_eq!(first.build_explain_code.as_deref(), Some("E0425"));
        assert_eq!(first.build_source_line, Some(4));
        assert_eq!(
            first.build_source_snippet.as_deref(),
            Some("println!(\"{}\", x);")
        );
        assert_eq!(
            first.build_caret_marker.as_deref(),
            Some("^ not found in this scope")
        );
    }
}
