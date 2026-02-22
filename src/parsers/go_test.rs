use crate::pipeline::{ParsedFailure, Parser, Token};

pub struct GoTestParser;

impl Parser for GoTestParser {
    fn parse(&self, tokens: &[Token]) -> Vec<ParsedFailure> {
        let mut failures = Vec::new();
        let mut current: Option<ParsedFailure> = None;

        for token in tokens {
            match token {
                Token::FailureHeader { test_name } => {
                    if let Some(existing) = current.as_mut() {
                        if existing.test_name.is_none() {
                            existing.test_name = Some(test_name.clone());
                            if existing.trace.is_empty() {
                                existing.trace = vec!["go test".to_string(), test_name.clone()];
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
                        trace: vec!["go test".to_string(), test_name.clone()],
                        ..ParsedFailure::default()
                    });
                }
                Token::PanicLocation { file, line, column } => {
                    let current = current.get_or_insert_with(ParsedFailure::default);
                    current.file = Some(file.clone());
                    current.line = Some(*line);
                    current.column = Some(*column);
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
    use super::GoTestParser;
    use crate::pipeline::{Parser, Token};

    #[test]
    fn groups_go_test_tokens_into_one_failure() {
        let tokens = vec![
            Token::FailureHeader {
                test_name: "TestValidateToken".to_string(),
            },
            Token::PanicLocation {
                file: "auth_test.go".to_string(),
                line: 42,
                column: 1,
            },
            Token::AssertionRight {
                value: "401".to_string(),
            },
            Token::AssertionLeft {
                value: "403".to_string(),
            },
            Token::PanicMessage {
                message: "expected 401, got 403".to_string(),
            },
        ];

        let parsed = GoTestParser.parse(&tokens);
        assert_eq!(parsed.len(), 1);
        let first = &parsed[0];
        assert_eq!(first.test_name.as_deref(), Some("TestValidateToken"));
        assert_eq!(first.file.as_deref(), Some("auth_test.go"));
        assert_eq!(first.line, Some(42));
        assert_eq!(first.column, Some(1));
        assert_eq!(first.assertion_left.as_deref(), Some("403"));
        assert_eq!(first.assertion_right.as_deref(), Some("401"));
        assert_eq!(first.trace, vec!["go test", "TestValidateToken"]);
    }

    #[test]
    fn merges_late_failure_header_into_current_failure() {
        let tokens = vec![
            Token::AssertionRight {
                value: "401".to_string(),
            },
            Token::AssertionLeft {
                value: "403".to_string(),
            },
            Token::PanicLocation {
                file: "auth_test.go".to_string(),
                line: 42,
                column: 1,
            },
            Token::FailureHeader {
                test_name: "TestValidateToken".to_string(),
            },
        ];

        let parsed = GoTestParser.parse(&tokens);
        assert_eq!(parsed.len(), 1);
        let first = &parsed[0];
        assert_eq!(first.test_name.as_deref(), Some("TestValidateToken"));
        assert_eq!(first.file.as_deref(), Some("auth_test.go"));
        assert_eq!(first.line, Some(42));
        assert_eq!(first.assertion_left.as_deref(), Some("403"));
        assert_eq!(first.assertion_right.as_deref(), Some("401"));
    }
}
