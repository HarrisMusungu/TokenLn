use crate::pipeline::{ParsedFailure, Parser, Token};

pub struct CargoTestParser;

impl Parser for CargoTestParser {
    fn parse(&self, tokens: &[Token]) -> Vec<ParsedFailure> {
        let mut failures = Vec::new();
        let mut current: Option<ParsedFailure> = None;

        for token in tokens {
            match token {
                Token::FailureHeader { test_name } => {
                    push_if_meaningful(&mut failures, current.take());
                    current = Some(ParsedFailure {
                        test_name: Some(test_name.clone()),
                        trace: vec!["cargo test".to_string(), test_name.clone()],
                        ..ParsedFailure::default()
                    });
                }
                Token::PanicThread { test_name } => {
                    let current = current.get_or_insert_with(ParsedFailure::default);
                    if current.test_name.is_none() {
                        current.test_name = Some(test_name.clone());
                    }
                    if current.trace.is_empty() {
                        current.trace = vec!["cargo test".to_string(), test_name.clone()];
                    }
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
                Token::BuildErrorHeader { .. }
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
        {
            target.push(candidate);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::CargoTestParser;
    use crate::pipeline::{Parser, Token};

    #[test]
    fn groups_tokens_into_one_failure() {
        let tokens = vec![
            Token::FailureHeader {
                test_name: "tests::auth_invalid_token".to_string(),
            },
            Token::PanicLocation {
                file: "src/auth.rs".to_string(),
                line: 89,
                column: 9,
            },
            Token::PanicMessage {
                message: "assertion `left == right` failed".to_string(),
            },
            Token::AssertionLeft {
                value: "403".to_string(),
            },
            Token::AssertionRight {
                value: "401".to_string(),
            },
        ];

        let parsed = CargoTestParser.parse(&tokens);
        assert_eq!(parsed.len(), 1);
        let first = &parsed[0];
        assert_eq!(
            first.test_name.as_deref(),
            Some("tests::auth_invalid_token")
        );
        assert_eq!(first.file.as_deref(), Some("src/auth.rs"));
        assert_eq!(first.assertion_left.as_deref(), Some("403"));
        assert_eq!(first.assertion_right.as_deref(), Some("401"));
    }
}
