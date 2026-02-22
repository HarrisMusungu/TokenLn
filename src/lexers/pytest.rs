use crate::pipeline::{Lexer, Token};

pub struct PytestLexer;

impl Lexer for PytestLexer {
    fn lex(&self, input: &str) -> Vec<Token> {
        let mut tokens = Vec::new();

        for line in input.lines() {
            let trimmed = line.trim();

            if let Some(test_name) = parse_failure_summary_header(trimmed) {
                tokens.push(Token::FailureHeader { test_name });
                continue;
            }

            if let Some((file, line_num, message)) = parse_pytest_location(trimmed) {
                tokens.push(Token::PanicLocation {
                    file,
                    line: line_num,
                    column: 1,
                });
                if !message.is_empty() {
                    tokens.push(Token::PanicMessage { message });
                }
                continue;
            }

            if let Some((left, right)) = parse_pytest_assert_pair(trimmed) {
                tokens.push(Token::AssertionLeft { value: left });
                tokens.push(Token::AssertionRight { value: right });
                continue;
            }

            if let Some(message) = parse_pytest_error_message(trimmed) {
                tokens.push(Token::PanicMessage { message });
            }
        }

        tokens
    }
}

fn parse_failure_summary_header(line: &str) -> Option<String> {
    let summary = line.strip_prefix("FAILED ")?;
    let (nodeid, _) = summary.split_once(" - ")?;
    let nodeid = nodeid.trim();
    if nodeid.is_empty() {
        return None;
    }
    Some(nodeid.to_string())
}

fn parse_pytest_location(line: &str) -> Option<(String, u32, String)> {
    let mut parts = line.rsplitn(3, ':');
    let message = parts.next()?.trim().to_string();
    let line_num = parts.next()?.trim().parse::<u32>().ok()?;
    let file = parts.next()?.trim();

    if file.is_empty() {
        return None;
    }

    Some((file.to_string(), line_num, message))
}

fn parse_pytest_assert_pair(line: &str) -> Option<(String, String)> {
    let assertion = line.strip_prefix("E")?.trim();
    let expression = assertion.strip_prefix("assert ")?;
    let (left, right) = expression.split_once(" == ")?;
    let left = left.trim();
    let right = right.trim();
    if left.is_empty() || right.is_empty() {
        return None;
    }
    Some((left.to_string(), right.to_string()))
}

fn parse_pytest_error_message(line: &str) -> Option<String> {
    let message = line.strip_prefix("E")?.trim();
    if message.is_empty() {
        return None;
    }
    Some(message.to_string())
}

#[cfg(test)]
mod tests {
    use super::PytestLexer;
    use crate::pipeline::{Lexer, Token};

    #[test]
    fn lexes_pytest_assertion_failure() {
        let input = "\
=================================== FAILURES ===================================
________________________ test_auth_invalid_token ________________________

    def test_auth_invalid_token():
>       assert status_code == 401
E       assert 403 == 401

tests/test_auth.py:27: AssertionError
=========================== short test summary info ============================
FAILED tests/test_auth.py::test_auth_invalid_token - assert 403 == 401";

        let tokens = PytestLexer.lex(input);
        assert!(tokens.iter().any(|t| matches!(
            t,
            Token::FailureHeader { test_name } if test_name == "tests/test_auth.py::test_auth_invalid_token"
        )));
        assert!(tokens.iter().any(|t| matches!(
            t,
            Token::PanicLocation { file, line, column } if file == "tests/test_auth.py" && *line == 27 && *column == 1
        )));
        assert!(tokens.iter().any(|t| matches!(
            t,
            Token::AssertionLeft { value } if value == "403"
        )));
        assert!(tokens.iter().any(|t| matches!(
            t,
            Token::AssertionRight { value } if value == "401"
        )));
    }
}
