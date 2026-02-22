use crate::pipeline::{ParsedFailure, Parser, Token};

pub struct PytestParser;

impl Parser for PytestParser {
    fn parse(&self, tokens: &[Token]) -> Vec<ParsedFailure> {
        let mut failures = Vec::new();
        let mut current: Option<ParsedFailure> = None;

        for token in tokens {
            match token {
                Token::FailureHeader { test_name } => {
                    let (derived_file, derived_symbol) = parse_nodeid(test_name);
                    if let Some(existing) = current.as_mut() {
                        if existing.test_name.is_none() {
                            existing.test_name = Some(test_name.clone());
                            if existing.file.is_none() {
                                existing.file = derived_file;
                            }
                            if existing.trace.is_empty() {
                                existing.trace = vec!["pytest".to_string(), test_name.clone()];
                            }
                            if existing.panic_message.is_none() {
                                existing.panic_message =
                                    derived_symbol.map(|symbol| format!("failed test: {symbol}"));
                            }
                            continue;
                        }

                        if existing.test_name.as_deref() == Some(test_name) {
                            continue;
                        }
                    }

                    push_if_meaningful(&mut failures, current.take());
                    current = Some(ParsedFailure {
                        test_name: Some(test_name.clone()),
                        file: derived_file,
                        trace: vec!["pytest".to_string(), test_name.clone()],
                        panic_message: derived_symbol
                            .map(|symbol| format!("failed test: {symbol}")),
                        ..ParsedFailure::default()
                    });
                }
                Token::PanicLocation { file, line, .. } => {
                    let current = current.get_or_insert_with(ParsedFailure::default);
                    current.file = Some(file.clone());
                    current.line = Some(*line);
                }
                Token::AssertionLeft { value } => {
                    let current = current.get_or_insert_with(ParsedFailure::default);
                    current.assertion_left = Some(value.clone());
                }
                Token::AssertionRight { value } => {
                    let current = current.get_or_insert_with(ParsedFailure::default);
                    current.assertion_right = Some(value.clone());
                }
                Token::PanicMessage { message } => {
                    let current = current.get_or_insert_with(ParsedFailure::default);
                    current.panic_message = Some(message.clone());
                }
                Token::PanicThread { .. }
                | Token::BuildErrorHeader { .. }
                | Token::BuildLocation { .. }
                | Token::BuildHelp { .. }
                | Token::BuildExplainCode { .. }
                | Token::BuildSourceSnippet { .. }
                | Token::BuildCaretMarker { .. } => {}
            }
        }

        push_if_meaningful(&mut failures, current);
        failures
    }
}

fn parse_nodeid(nodeid: &str) -> (Option<String>, Option<String>) {
    if let Some((file, symbol)) = nodeid.split_once("::") {
        let file = file.trim();
        let symbol = symbol.trim();
        return (
            if file.is_empty() {
                None
            } else {
                Some(file.to_string())
            },
            if symbol.is_empty() {
                None
            } else {
                Some(symbol.to_string())
            },
        );
    }

    (None, None)
}

fn push_if_meaningful(target: &mut Vec<ParsedFailure>, candidate: Option<ParsedFailure>) {
    if let Some(candidate) = candidate {
        if candidate.test_name.is_some()
            || candidate.assertion_left.is_some()
            || candidate.assertion_right.is_some()
            || candidate.panic_message.is_some()
            || candidate.file.is_some()
            || candidate.line.is_some()
        {
            target.push(candidate);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::PytestParser;
    use crate::pipeline::{Parser, Token};

    #[test]
    fn groups_pytest_tokens_into_one_failure() {
        let tokens = vec![
            Token::FailureHeader {
                test_name: "tests/test_auth.py::test_auth_invalid_token".to_string(),
            },
            Token::PanicLocation {
                file: "tests/test_auth.py".to_string(),
                line: 27,
                column: 1,
            },
            Token::AssertionLeft {
                value: "403".to_string(),
            },
            Token::AssertionRight {
                value: "401".to_string(),
            },
            Token::PanicMessage {
                message: "AssertionError".to_string(),
            },
        ];

        let parsed = PytestParser.parse(&tokens);
        assert_eq!(parsed.len(), 1);
        let first = &parsed[0];
        assert_eq!(
            first.test_name.as_deref(),
            Some("tests/test_auth.py::test_auth_invalid_token")
        );
        assert_eq!(first.file.as_deref(), Some("tests/test_auth.py"));
        assert_eq!(first.line, Some(27));
        assert_eq!(first.assertion_left.as_deref(), Some("403"));
        assert_eq!(first.assertion_right.as_deref(), Some("401"));
        assert_eq!(
            first.trace,
            vec!["pytest", "tests/test_auth.py::test_auth_invalid_token"]
        );
    }

    #[test]
    fn merges_late_summary_header_into_current_failure() {
        let tokens = vec![
            Token::AssertionLeft {
                value: "403".to_string(),
            },
            Token::AssertionRight {
                value: "401".to_string(),
            },
            Token::PanicLocation {
                file: "tests/test_auth.py".to_string(),
                line: 27,
                column: 1,
            },
            Token::PanicMessage {
                message: "AssertionError".to_string(),
            },
            Token::FailureHeader {
                test_name: "tests/test_auth.py::test_auth_invalid_token".to_string(),
            },
        ];

        let parsed = PytestParser.parse(&tokens);
        assert_eq!(parsed.len(), 1);
        let first = &parsed[0];
        assert_eq!(
            first.test_name.as_deref(),
            Some("tests/test_auth.py::test_auth_invalid_token")
        );
        assert_eq!(first.file.as_deref(), Some("tests/test_auth.py"));
        assert_eq!(first.line, Some(27));
        assert_eq!(first.assertion_left.as_deref(), Some("403"));
        assert_eq!(first.assertion_right.as_deref(), Some("401"));
    }
}
